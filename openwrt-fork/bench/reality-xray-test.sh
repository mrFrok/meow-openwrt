#!/bin/sh
# meow-openwrt — REALITY interop against official Xray (26+).
#
#   curl ─▶ meow (mixed) ─VLESS+REALITY─▶ xray ─▶ local HTTP
#
# Xray serves REALITY on TCP+Vision, gRPC and XHTTP with dest www.sony.com
# (needs the Internet: the server fetches dest on every handshake). For each
# client variant — fingerprint, hybrid X25519MLKEM768 key share on/off,
# transport — download 8 MiB and upload 3 MiB through the node and compare
# checksums, then 8 parallel downloads over one node (gRPC/XHTTP share an
# h2 connection).
#
# Usage: sh reality-xray-test.sh <meow-bin> <xray-bin>   ONLY=<substring>
LC_ALL=C; export LC_ALL
BIN=$(realpath "${1:?meow binary}"); XRAY=$(realpath "${2:?xray binary}")
W=${TMPDIR:-/tmp}/mwrx.$$; rm -rf "$W"; mkdir -p "$W"
fail=0; MP=""; XP=""; HP=""
ok() { echo "PASS $*"; }
bad() { echo "FAIL $*"; fail=1; }
cleanup() { for p in $MP $XP $HP; do kill $p 2>/dev/null; done; rm -rf "$W"; }
trap cleanup EXIT
UUID=b831381d-6324-4d53-ad4f-8cda48b30811; SID=6ba85179e30d4fc2
DEST=${DEST:-www.sony.com}

KEYS=$("$XRAY" x25519)
PRIV=$(echo "$KEYS" | sed -n 's/^Private[ ]*[Kk]ey:[ ]*//p; s/^PrivateKey:[ ]*//p' | head -1)
PUB=$(echo "$KEYS" | sed -n 's/^Public[ ]*[Kk]ey:[ ]*//p; s/^Password[^:]*:[ ]*//p' | head -1)
[ -n "$PRIV" ] && [ -n "$PUB" ] || { echo "cannot parse xray x25519 output"; exit 2; }

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
http.server.ThreadingHTTPServer(("127.0.0.1", 21080), H).serve_forever()
EOF
python3 "$W/srv.py" "$W/big" & HP=$!

RS="\"security\":\"reality\",\"realitySettings\":{\"dest\":\"$DEST:443\",\"serverNames\":[\"$DEST\"],\"privateKey\":\"$PRIV\",\"shortIds\":[\"$SID\"]}"
cat > "$W/xray.json" <<EOF
{"log":{"loglevel":"${XLOG:-warning}"},
 "inbounds":[
  {"listen":"127.0.0.1","port":21443,"protocol":"vless",
   "settings":{"clients":[{"id":"$UUID","flow":"xtls-rprx-vision"}],"decryption":"none"},
   "streamSettings":{"network":"tcp",$RS}},
  {"listen":"127.0.0.1","port":21444,"protocol":"vless",
   "settings":{"clients":[{"id":"$UUID"}],"decryption":"none"},
   "streamSettings":{"network":"grpc","grpcSettings":{"serviceName":"g"},$RS}},
  {"listen":"127.0.0.1","port":21445,"protocol":"vless",
   "settings":{"clients":[{"id":"$UUID"}],"decryption":"none"},
   "streamSettings":{"network":"xhttp","xhttpSettings":{"path":"/xh","mode":"auto"},$RS}}],
 "outbounds":[{"protocol":"freedom","settings":{"finalRules":[{"action":"allow","ip":["127.0.0.1"]}]}}]}
EOF
"$XRAY" run -c "$W/xray.json" > "$W/xray.log" 2>&1 & XP=$!
sleep 1

WANT_DOWN=$(sha256sum < "$W/big" | cut -d' ' -f1)
WANT_UP=$(sha256sum < "$W/up" | cut -d' ' -f1)

run() { # <name> <port> <fingerprint> <mlkem true|false> <transport yaml, indented 4>
	case "$1" in *"${ONLY:-}"*) ;; *) return;; esac
	cat > "$W/config.yaml" <<EOF
log-level: warning
mixed-port: 17893
proxies:
  - name: n
    type: vless
    server: 127.0.0.1
    port: $2
    uuid: $UUID
    tls: true
    servername: $DEST
    client-fingerprint: $3
    reality-opts: {public-key: $PUB, short-id: $SID, support-x25519mlkem768: $4}
$5
rules:
  - MATCH,n
EOF
	"$BIN" -d "$W" -f "$W/config.yaml" > "$W/meow.log" 2>&1 & MP=$!
	for i in $(seq 1 50); do python3 -c "import socket;socket.create_connection(('127.0.0.1',17893),0.2)" 2>/dev/null && break; sleep 0.2; done
	P="-s --max-time 60 -x http://127.0.0.1:17893"
	t=$(curl $P -o /dev/null -w '%{time_total}' http://127.0.0.1:21080/big)
	d=$(curl $P http://127.0.0.1:21080/big | sha256sum | cut -d' ' -f1)
	u=$(curl $P -H 'Content-Type: application/octet-stream' --data-binary @"$W/up" http://127.0.0.1:21080/up)
	pids=""; for i in 1 2 3 4 5 6 7 8; do
		( curl $P http://127.0.0.1:21080/big | sha256sum | cut -d' ' -f1 > "$W/par.$i" ) & pids="$pids $!"; done
	for p in $pids; do wait $p; done
	par=0; for i in 1 2 3 4 5 6 7 8; do [ "$(cat "$W/par.$i")" = "$WANT_DOWN" ] && par=$((par+1)); done
	[ "$d" = "$WANT_DOWN" ] && ok "$1: download 8 MiB (first ${t}s)" || bad "$1: download"
	[ "$u" = "$WANT_UP" ] && ok "$1: upload 3 MiB" || bad "$1: upload ($u)"
	[ $par = 8 ] && ok "$1: 8 parallel downloads" || bad "$1: parallel $par/8"
	kill $MP; for i in $(seq 1 50); do kill -0 $MP 2>/dev/null || break; sleep 0.1; done; MP=""
	if grep -qE 'ERROR|panicked' "$W/meow.log" || [ "$d$u$par" != "$WANT_DOWN${WANT_UP}8" ]; then
		grep -E 'ERROR|WARN|panicked' "$W/meow.log" | tail -4; fi
}

VISION="    network: tcp
    flow: xtls-rprx-vision"
GRPC="    network: grpc
    grpc-opts: {grpc-service-name: g}"
XHTTP="    network: xhttp
    xhttp-opts: {path: /xh, mode: auto}"

run "vision chrome x25519" 21443 chrome false "$VISION"
run "vision chrome mlkem" 21443 chrome true "$VISION"
run "vision firefox mlkem" 21443 firefox true "$VISION"
run "vision firefox x25519" 21443 firefox false "$VISION"
run "vision safari mlkem" 21443 safari true "$VISION"
run "grpc chrome mlkem" 21444 chrome true "$GRPC"
run "xhttp chrome mlkem (auto = stream-one)" 21445 chrome true "$XHTTP"

[ $fail = 0 ] && echo "ALL PASS" || { echo "SOME FAILED — xray log tail:"; grep -vE "DNS|dns" "$W/xray.log" | tail -${XTAIL:-5}; }
exit $fail
