#!/bin/sh
# meow-openwrt — interop test of the VLESS XHTTP modes against official Xray.
#
#   curl ─▶ meow (mixed) ─xhttp h2c─▶ xray (VLESS, xhttp mode auto) ─▶ local HTTP
#
# For each client variant: download 8 MiB and upload 3 MiB through the node
# and compare checksums. Variants: stream-one, stream-up, packet-up (body),
# packet-up as some subscriptions ship it (GET, data in headers, 8 KiB
# requests, 1-5 ms gap), packet-up with cookies and query placements.
#
# Usage: sh xhttp-test.sh <meow-bin> <xray-bin>
LC_ALL=C; export LC_ALL
BIN=$(realpath "${1:?meow binary}"); XRAY=$(realpath "${2:?xray binary}")
W=${TMPDIR:-/tmp}/mwxh.$$; rm -rf "$W"; mkdir -p "$W"
fail=0; MP=""; XP=""; HP=""
ok() { echo "PASS $*"; }
bad() { echo "FAIL $*"; fail=1; }
cleanup() { for p in $MP $XP $HP; do kill $p 2>/dev/null; done; rm -rf "$W"; }
trap cleanup EXIT
UUID=b831381d-6324-4d53-ad4f-8cda48b30811

head -c 8388608 /dev/urandom > "$W/big"; head -c 3145728 /dev/urandom > "$W/up"
cat > "$W/srv.py" <<'EOF'
import hashlib, http.server, sys
big = open(sys.argv[1], "rb").read()
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def do_GET(self):
        self.send_response(200); self.send_header("Content-Length", str(len(big))); self.end_headers(); self.wfile.write(big)
    def do_POST(self):
        n = int(self.headers["Content-Length"]); b = hashlib.sha256(self.rfile.read(n)).hexdigest().encode()
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
http.server.ThreadingHTTPServer(("127.0.0.1", 20080), H).serve_forever()
EOF
python3 "$W/srv.py" "$W/big" & HP=$!

cat > "$W/xray.json" <<EOF
{"log":{"loglevel":"${XLOG:-warning}"},
 "inbounds":[{"listen":"127.0.0.1","port":20443,"protocol":"vless",
   "settings":{"clients":[{"id":"$UUID"}],"decryption":"none"},
   "streamSettings":{"network":"xhttp","xhttpSettings":{"path":"/xh","mode":"auto","serverMaxHeaderBytes":65536}}},
  {"listen":"127.0.0.1","port":20444,"protocol":"vless",
   "settings":{"clients":[{"id":"$UUID"}],"decryption":"none"},
   "streamSettings":{"network":"xhttp","xhttpSettings":{"path":"/xh","mode":"packet-up",
     "sessionIDPlacement":"query","seqPlacement":"header","uplinkDataPlacement":"cookie",
     "serverMaxHeaderBytes":65536}}}],
 "outbounds":[{"protocol":"freedom","settings":{"finalRules":[{"action":"allow","ip":["127.0.0.1"]}]}}]}
EOF
"$XRAY" run -c "$W/xray.json" > "$W/xray.log" 2>&1 & XP=$!
sleep 1

WANT_DOWN=$(sha256sum < "$W/big" | cut -d' ' -f1)
WANT_UP=$(sha256sum < "$W/up" | cut -d' ' -f1)

run() { # <name> <xhttp-opts yaml, indented 6>; ONLY=<substring> runs a subset
	case "$1" in *"${ONLY:-}"*) ;; *) return;; esac
	cat > "$W/config.yaml" <<EOF
log-level: warning
mixed-port: 17892
proxies:
  - name: n
    type: vless
    server: 127.0.0.1
    port: ${PORT:-20443}
    uuid: $UUID
    network: xhttp
    xhttp-opts:
      path: /xh
$2
rules:
  - MATCH,n
EOF
	"$BIN" -d "$W" -f "$W/config.yaml" > "$W/meow.log" 2>&1 & MP=$!
	for i in $(seq 1 50); do python3 -c "import socket;socket.create_connection(('127.0.0.1',17892),0.2)" 2>/dev/null && break; sleep 0.2; done
	P="-s --max-time 60 -x http://127.0.0.1:17892"
	d=$(curl $P http://127.0.0.1:20080/big | sha256sum | cut -d' ' -f1)
	u=$(curl $P -H 'Content-Type: application/octet-stream' --data-binary @"$W/up" http://127.0.0.1:20080/up)
	[ "$d" = "$WANT_DOWN" ] && ok "$1: download 8 MiB" || bad "$1: download"
	[ "$u" = "$WANT_UP" ] && ok "$1: upload 3 MiB" || bad "$1: upload ($u)"
	kill $MP; for i in $(seq 1 50); do kill -0 $MP 2>/dev/null || break; sleep 0.1; done; MP=""
	if grep -qE 'ERROR|panicked' "$W/meow.log" || [ "$d$u" != "$WANT_DOWN$WANT_UP" ]; then
		grep -E 'ERROR|WARN|panicked' "$W/meow.log" | grep -v plaintext | tail -4; fi
}

run stream-one "      mode: stream-one"
run stream-up "      mode: stream-up"
run packet-up "      mode: packet-up"
run "packet-up GET+header" "      mode: packet-up
      uplink-http-method: GET
      uplink-data-placement: header
      uplink-chunk-size: 0
      sc-max-each-post-bytes: 8192
      sc-min-posts-interval-ms: 1-5"
PORT=20444 run "packet-up cookie, session in query, seq in header" "      mode: packet-up
      uplink-data-placement: cookie
      session-placement: query
      seq-placement: header
      sc-max-each-post-bytes: 16384"
run "auto (no REALITY) = packet-up" "      mode: auto"

[ $fail = 0 ] && echo "ALL PASS" || { echo "SOME FAILED — xray log tail:"; grep -vE "DNS|dns" "$W/xray.log" | tail -${XTAIL:-5}; }
exit $fail
