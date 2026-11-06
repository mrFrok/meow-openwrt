#!/bin/sh
# meow-openwrt — functional test of the `type: wireguard` outbound (stage 8)
# on the kernel's WireGuard module, in two netns:
#
#   mwg (meow, client)  10.20.0.1 ──veth── 10.20.0.2  mws (server)
#                                                     wg0 10.99.0.1/24, :51820
#                                                     HTTP + UDP echo on 10.99.0.1
#
# The server answers with the peer address it saw: 10.99.0.2 means the
# request came through the tunnel. The server side is set up by a small
# python generic-netlink client (no wireguard-tools needed); keys come from
# python `cryptography`.
#
# Checks: TCP and UDP through the node; meow created mwg* with its oif rule;
# a link deleted from outside is recreated on the next dial; SIGTERM removes
# the link and the rules.
#
# Usage: sh wg-test.sh <meow-bin>   (root, or re-runs itself under
# `unshare -Urnm` — needs unprivileged user namespaces and the wireguard
# kernel module on the host)
LC_ALL=C; export LC_ALL
BIN=$(realpath "${1:?meow binary}")
if [ "$(id -u)" != 0 ] && [ -z "$MW_IN_NS" ]; then
	MW_IN_NS=1 exec unshare -Urnm sh "$0" "$BIN"
fi
[ -n "$MW_IN_NS" ] && mount -t tmpfs none /run && mkdir -p /run/netns
W=${TMPDIR:-/tmp}/mwwg.$$; rm -rf "$W"; mkdir -p "$W"
fail=0
ok() { echo "PASS $*"; }
bad() { echo "FAIL $*"; fail=1; }
G="ip netns exec mwg"; S="ip netns exec mws"
MP=""; SRV=""
cleanup() {
	[ -n "$MP" ] && kill $MP 2>/dev/null
	for p in $SRV; do kill $p 2>/dev/null; done
	for n in mwg mws; do ip netns del $n 2>/dev/null; done
	rm -rf "$W"
}
trap cleanup EXIT

for n in mwg mws; do ip netns del $n 2>/dev/null; ip netns add $n; ip netns exec $n ip link set lo up; done
ip link add mwg0 type veth peer name mws0
ip link set mwg0 netns mwg; ip link set mws0 netns mws
$G ip addr add 10.20.0.1/24 dev mwg0; $G ip link set mwg0 up
# As on OpenWrt (the host's strict default would drop tunnel replies).
$G sysctl -qw net.ipv4.conf.all.rp_filter=0 net.ipv4.conf.default.rp_filter=0
$S ip addr add 10.20.0.2/24 dev mws0; $S ip link set mws0 up

# Keys and the server's wg0, over generic netlink.
cat > "$W/wg.py" <<'EOF'
import base64, socket, struct, sys
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
from cryptography.hazmat.primitives import serialization as ser
raw = lambda k: k.private_bytes(ser.Encoding.Raw, ser.PrivateFormat.Raw, ser.NoEncryption())
pub = lambda k: k.public_key().public_bytes(ser.Encoding.Raw, ser.PublicFormat.Raw)
if sys.argv[1] == "keys":
    for k in (X25519PrivateKey.generate(), X25519PrivateKey.generate()):
        print(base64.b64encode(raw(k)).decode(), base64.b64encode(pub(k)).decode())
    sys.exit()
# set <ifname> <server-priv-b64> <client-pub-b64> <client-allowed-ip>
ifname, spriv, cpub, allowed = sys.argv[2:6]
def nla(t, d):
    a = struct.pack("HH", 4 + len(d), t) + d
    return a + b"\0" * (-len(a) % 4)
def msg(ty, payload):
    return struct.pack("IHHII", 16 + len(payload), ty, 5, 1, 0) + payload
s = socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, 16)
s.send(msg(0x10, bytes([3, 1, 0, 0]) + nla(2, b"wireguard\0")))
r = s.recv(65536); a = r[20:]; fam = None
while len(a) >= 4:
    l, t = struct.unpack("HH", a[:4])
    if t == 1: fam = struct.unpack("H", a[4:6])[0]
    a = a[(l + 3) & ~3:]
ip = socket.inet_aton(allowed)
aip = nla(0x8000, nla(1, struct.pack("H", socket.AF_INET)) + nla(2, ip) + nla(3, bytes([32])))
peer = nla(0x8000, nla(1, base64.b64decode(cpub)) + nla(9 | 0x8000, aip))
body = nla(2, ifname.encode() + b"\0") + nla(3, base64.b64decode(spriv)) + nla(6, struct.pack("H", 51820)) + nla(8 | 0x8000, peer)
s.send(msg(fam, bytes([1, 1, 0, 0]) + body))
r = s.recv(65536)
err = struct.unpack("i", r[16:20])[0]
sys.exit(0 if err == 0 else f"wg set: errno {-err}")
EOF
set -- $(python3 "$W/wg.py" keys); SPRIV=$1 SPUB=$2 CPRIV=$3 CPUB=$4
$S ip link add wg0 type wireguard || { echo "SKIP: no wireguard module"; exit 0; }
$S python3 "$W/wg.py" set wg0 "$SPRIV" "$CPUB" 10.99.0.2 || bad "server wg0 setup"
$S ip addr add 10.99.0.1/24 dev wg0; $S ip link set wg0 up

cat > "$W/srv.py" <<'EOF'
import http.server, socket, threading
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = self.client_address[0].encode()
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
def udp():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("10.99.0.1", 9000))
    while True:
        d, a = s.recvfrom(2048); s.sendto(a[0].encode() + b" " + d, a)
threading.Thread(target=udp, daemon=True).start()
http.server.ThreadingHTTPServer(("10.99.0.1", 8080), H).serve_forever()
EOF
$S python3 "$W/srv.py" & SRV="$SRV $!"

cat > "$W/config.yaml" <<EOF
log-level: info
mixed-port: 17891
external-controller: 127.0.0.1:19097
proxies:
  - name: wg-node
    type: wireguard
    server: 10.20.0.2
    port: 51820
    ip: 10.99.0.2
    private-key: $CPRIV
    public-key: $SPUB
    allowed-ips: ['10.99.0.0/24']
    udp: true
rules:
  - MATCH,wg-node
EOF
$G "$BIN" -d "$W" -f "$W/config.yaml" > "$W/meow.log" 2>&1 & MP=$!
for i in $(seq 1 50); do $G python3 -c "import socket;socket.create_connection(('127.0.0.1',17891),0.2)" 2>/dev/null && break; sleep 0.2; done

http() { $G python3 -c "
import urllib.request
h = urllib.request.ProxyHandler({'http': 'http://127.0.0.1:17891'})
print(urllib.request.build_opener(h).open('http://10.99.0.1:8080/', timeout=8).read().decode())" 2>&1 | tail -1; }
udp() { $G python3 -c "
import socket, struct
c = socket.create_connection(('127.0.0.1', 17891), 4)
c.sendall(b'\x05\x01\x00'); c.recv(2)
c.sendall(b'\x05\x03\x00\x01' + bytes(4) + b'\x00\x00'); r = c.recv(10)
relay = (socket.inet_ntoa(r[4:8]), struct.unpack('>H', r[8:10])[0])
u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); u.settimeout(4)
u.sendto(b'\x00\x00\x00\x01' + socket.inet_aton('10.99.0.1') + struct.pack('>H', 9000) + b'ping', relay)
try: print(u.recv(2048)[10:].decode())
except Exception as e: print('TIMEOUT', e)" 2>&1 | tail -1; }

r=$(http); [ "$r" = 10.99.0.2 ] && ok "TCP through wg-node ($r)" || bad "TCP: '$r'"
r=$(udp); [ "$r" = "10.99.0.2 ping" ] && ok "UDP through wg-node ($r)" || bad "UDP: '$r'"
if [ -n "$MW_DEBUG" ]; then
	IF0=$($G ip -o link show type wireguard | awk -F': ' '{print $2}' | head -1)
	echo "--- gw:"; $G ip -s -o link show type wireguard; $G ip addr show type wireguard | grep inet
	$G ip rule; $G ip route show table all | grep -E 'mwg|wg'
	echo "--- srv:"; $S ip -s -o link show wg0
	$G sh -c 'for f in /proc/sys/net/ipv4/conf/*/rp_filter; do echo "$f $(cat $f)"; done'
	$G python3 -c "
import socket
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, 25, b'$IF0'); s.settimeout(4)
try: s.connect(('10.99.0.1', 8080)); print('raw bound connect OK', s.getsockname())
except Exception as e: print('raw bound connect FAIL', e)"
fi
IF=$($G ip -o link show type wireguard | awk -F': ' '{print $2}' | head -1)
case "$IF" in mwg*) ok "meow created $IF";; *) bad "no mwg* link";; esac
$G ip rule | grep -q "oif $IF" && ok "  ...with its oif rule" || bad "  ...no oif rule"
$G ip link del "$IF"
r=$(http); [ "$r" = 10.99.0.2 ] && ok "link deleted from outside: recreated on next dial" || bad "after external delete: '$r'"
kill $MP; sleep 1; MP=""
$G ip -o link show type wireguard | grep -q mwg && bad "link left after SIGTERM" || ok "link removed on SIGTERM"
$G ip rule | grep -q 'oif mwg' && bad "rule left after SIGTERM" || ok "rules removed"
grep -E 'ERROR|panicked' "$W/meow.log" | head -5
[ $fail = 0 ] && echo "ALL PASS" || { echo "SOME FAILED — meow log tail:"; tail -15 "$W/meow.log"; }
exit $fail
