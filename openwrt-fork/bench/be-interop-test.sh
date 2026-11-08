#!/bin/sh
# meow-openwrt — interop of the TLS/QUIC paths against an official mihomo
# server, for a meow run through a wrapper (e.g. qemu-mips for the
# big-endian build: rustls + quinn instead of BoringSSL + quiche).
#
#   curl / python ─▶ meow (mixed) ─hysteria2 | trojan+TLS─▶ mihomo ─▶ local HTTP / UDP echo
#
# Per node: 8 MiB down, 3 MiB up, 4 parallel downloads (checksums), and for
# hysteria2 UDP through SOCKS5 UDP ASSOCIATE (64 datagrams echoed).
#
# Usage: sh be-interop-test.sh <meow-bin-or-wrapper> <mihomo-bin>   ONLY=<substring>
LC_ALL=C; export LC_ALL
BIN=$(realpath "${1:?meow binary or wrapper}"); MIHOMO=$(realpath "${2:?mihomo binary}")
W=${TMPDIR:-/tmp}/mwbe.$$; rm -rf "$W"; mkdir -p "$W/srv"
fail=0; MP=""; SP=""; HP=""; UP=""
ok() { echo "PASS $*"; }
bad() { echo "FAIL $*"; fail=1; }
cleanup() { for p in $MP $SP $HP $UP; do kill $p 2>/dev/null; done; rm -rf "$W"; }
trap cleanup EXIT

openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 \
	-subj /CN=bench.test -keyout "$W/srv/key.pem" -out "$W/srv/cert.pem" 2>/dev/null
head -c 8388608 /dev/urandom > "$W/big"; head -c 3145728 /dev/urandom > "$W/up"
cat > "$W/srv.py" <<'EOF'
import hashlib, http.server, socket, sys, threading
big = open(sys.argv[1], "rb").read()
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def do_GET(self):
        self.send_response(200); self.send_header("Content-Length", str(len(big))); self.end_headers(); self.wfile.write(big)
    def do_POST(self):
        n = int(self.headers["Content-Length"]); b = hashlib.sha256(self.rfile.read(n)).hexdigest().encode()
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
def echo():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1", 22053))
    while True:
        d, a = s.recvfrom(65535); s.sendto(d, a)
threading.Thread(target=echo, daemon=True).start()
http.server.ThreadingHTTPServer(("127.0.0.1", 22080), H).serve_forever()
EOF
python3 "$W/srv.py" "$W/big" & HP=$!

cat > "$W/mihomo.yaml" <<EOF
log-level: warning
listeners:
  - {name: hy, type: hysteria2, port: 22443, listen: 127.0.0.1, users: {u: bench}, certificate: $W/srv/cert.pem, private-key: $W/srv/key.pem}
  - {name: tj, type: trojan, port: 22444, listen: 127.0.0.1, users: [{username: u, password: bench}], certificate: $W/srv/cert.pem, private-key: $W/srv/key.pem}
rules: ['MATCH,DIRECT']
EOF
"$MIHOMO" -d "$W/srv" -f "$W/mihomo.yaml" > "$W/mihomo.log" 2>&1 & SP=$!
sleep 2

WANT_DOWN=$(sha256sum < "$W/big" | cut -d' ' -f1)
WANT_UP=$(sha256sum < "$W/up" | cut -d' ' -f1)

cat > "$W/udp.py" <<'EOF'
import socket, struct, sys
port = int(sys.argv[1])
c = socket.create_connection(("127.0.0.1", port), 10)
c.sendall(b"\x05\x01\x00"); assert c.recv(2) == b"\x05\x00"
c.sendall(b"\x05\x03\x00\x01" + bytes(4) + bytes(2))
r = c.recv(10); assert r[1] == 0, r
relay = (socket.inet_ntoa(r[4:8]), struct.unpack(">H", r[8:10])[0])
if relay[0] == "0.0.0.0": relay = ("127.0.0.1", relay[1])
u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); u.settimeout(3)
hdr = b"\x00\x00\x00\x01" + socket.inet_aton("127.0.0.1") + struct.pack(">H", 22053)
got = 0
for i in range(64):
    msg = bytes([i]) * (100 + i * 20)
    u.sendto(hdr + msg, relay)
    try:
        d, _ = u.recvfrom(65535)
        if d[10:] == msg: got += 1
    except socket.timeout:
        if got == 0 and i >= 3: break
print(got)
EOF

run() { # <name> <proxy yaml line> <udp yes|no>
	case "$1" in *"${ONLY:-}"*) ;; *) return;; esac
	cat > "$W/config.yaml" <<EOF
log-level: warning
mixed-port: 17894
proxies:
  - $2
rules:
  - MATCH,n
EOF
	"$BIN" -d "$W" -f "$W/config.yaml" > "$W/meow.log" 2>&1 & MP=$!
	for i in $(seq 1 100); do python3 -c "import socket;socket.create_connection(('127.0.0.1',17894),0.2)" 2>/dev/null && break; sleep 0.2; done
	P="-s --max-time 120 -x http://127.0.0.1:17894"
	d=$(curl $P http://127.0.0.1:22080/big | sha256sum | cut -d' ' -f1)
	u=$(curl $P -H 'Content-Type: application/octet-stream' --data-binary @"$W/up" http://127.0.0.1:22080/up)
	pids=""; for i in 1 2 3 4; do
		( curl $P http://127.0.0.1:22080/big | sha256sum | cut -d' ' -f1 > "$W/par.$i" ) & pids="$pids $!"; done
	for p in $pids; do wait $p; done
	par=0; for i in 1 2 3 4; do [ "$(cat "$W/par.$i")" = "$WANT_DOWN" ] && par=$((par+1)); done
	[ "$d" = "$WANT_DOWN" ] && ok "$1: download 8 MiB" || bad "$1: download"
	[ "$u" = "$WANT_UP" ] && ok "$1: upload 3 MiB" || bad "$1: upload ($u)"
	[ $par = 4 ] && ok "$1: 4 parallel downloads" || bad "$1: parallel $par/4"
	if [ "$3" = yes ]; then
		g=$(python3 "$W/udp.py" 17894 2>&1)
		[ "$g" = 64 ] && ok "$1: UDP 64/64 echoed" || bad "$1: UDP $g/64"
	fi
	kill $MP; for i in $(seq 1 50); do kill -0 $MP 2>/dev/null || break; sleep 0.1; done; MP=""
	grep -E 'ERROR|panicked' "$W/meow.log" | tail -4
}

run "hysteria2" "{name: n, type: hysteria2, server: 127.0.0.1, port: 22443, password: bench, sni: bench.test, skip-cert-verify: true, alpn: [h3]}" yes
run "hysteria2 2 conns" "{name: n, type: hysteria2, server: 127.0.0.1, port: 22443, password: bench, sni: bench.test, skip-cert-verify: true, alpn: [h3], connections: 2}" yes
run "trojan tls" "{name: n, type: trojan, server: 127.0.0.1, port: 22444, password: bench, sni: bench.test, skip-cert-verify: true}" no

[ $fail = 0 ] && echo "ALL PASS" || { echo "SOME FAILED — mihomo log tail:"; tail -${STAIL:-5} "$W/mihomo.log"; }
exit $fail
