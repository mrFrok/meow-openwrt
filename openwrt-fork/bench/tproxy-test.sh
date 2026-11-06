#!/bin/sh
# meow-openwrt — functional test of the transparent-proxy inbounds the way
# OpenWrt front-ends (clashwrt, OpenClash, nikki) drive them, in three netns:
#
#   mtc (LAN client) ──veth── mtg (gateway: meow) ──veth── mts (server)
#   10.9.0.2                   10.9.0.1 | 10.10.0.1          10.10.0.2
#
# The gateway steers LAN traffic for 10.10.0.0/24 and the fake-ip range into
# meow with nftables: TPROXY (`tproxy ip to :PORT`, fwmark + `local` route
# table) for `tproxy-port`, REDIRECT for `redir-port`. IP forwarding stays on,
# so traffic that slips past meow still reaches the server — the servers
# answer with the peer address they saw, which tells the two apart (meow
# dials from 10.10.0.1, a bypass arrives from 10.9.0.2).
#
# Phases: TPROXY TCP and UDP to a real IP; DNS from meow in fake-ip mode (the
# server netns runs a stub DNS answering 10.10.0.2 for every name), then
# TPROXY TCP and UDP to the fake IP; REDIRECT TCP; TPROXY with
# `allow-lan: false` (listener on 127.0.0.1, rule `tproxy ip to 127.0.0.1:P`).
#
# Usage: sh tproxy-test.sh <meow-bin>     (root, or re-runs itself under
# `unshare -Urnm` — no sudo needed where unprivileged user namespaces are on)
LC_ALL=C; export LC_ALL
BIN=$(realpath "${1:?meow binary}")
if [ "$(id -u)" != 0 ] && [ -z "$MT_IN_NS" ]; then
	MT_IN_NS=1 exec unshare -Urnm sh "$0" "$BIN"
fi
if [ -n "$MT_IN_NS" ]; then
	mount -t tmpfs none /run && mkdir -p /run/netns
fi
W=${TMPDIR:-/tmp}/mwtp.$$; rm -rf "$W"; mkdir -p "$W"
fail=0
ok() { echo "PASS $*"; }
bad() { echo "FAIL $*"; fail=1; }
G="ip netns exec mtg"; C="ip netns exec mtc"; S="ip netns exec mts"
MP=""; SRV=""

cleanup() {
	[ -n "$MP" ] && kill $MP 2>/dev/null
	for p in $SRV; do kill $p 2>/dev/null; done
	for n in mtc mtg mts; do ip netns del $n 2>/dev/null; done
	rm -rf "$W"
}
trap cleanup EXIT

for n in mtc mtg mts; do ip netns del $n 2>/dev/null; ip netns add $n; ip netns exec $n ip link set lo up; done
ip link add mtc0 type veth peer name mtg0
ip link set mtc0 netns mtc; ip link set mtg0 netns mtg
ip link add mtg1 type veth peer name mts0
ip link set mtg1 netns mtg; ip link set mts0 netns mts
$C ip addr add 10.9.0.2/24 dev mtc0; $C ip link set mtc0 up; $C ip route add default via 10.9.0.1
$G ip addr add 10.9.0.1/24 dev mtg0; $G ip link set mtg0 up
$G ip addr add 10.10.0.1/24 dev mtg1; $G ip link set mtg1 up
$G sysctl -qw net.ipv4.ip_forward=1
$S ip addr add 10.10.0.2/24 dev mts0; $S ip link set mts0 up; $S ip route add default via 10.10.0.1

# Servers: HTTP and UDP echo, both answering with the peer address seen.
cat > "$W/srv.py" <<'EOF'
import http.server, socket, threading
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = self.client_address[0].encode()
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
def udp():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("10.10.0.2", 9000))
    while True:
        d, a = s.recvfrom(2048); s.sendto(a[0].encode() + b" " + d, a)
def dns():  # answers every A query with 10.10.0.2
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("10.10.0.2", 53))
    while True:
        q, a = s.recvfrom(512)
        end = q.index(b"\0", 12) + 5
        r = q[:2] + b"\x81\x80" + q[4:6] + b"\0\x01\0\0\0\0" + q[12:end]
        if q[end - 4:end - 2] == b"\0\x01":
            r = r[:6] + b"\0\x01" + r[8:] + b"\xc0\x0c\0\x01\0\x01\0\0\0\x3c\0\x04" + bytes([10, 10, 0, 2])
        s.sendto(r, a)
threading.Thread(target=udp, daemon=True).start()
threading.Thread(target=dns, daemon=True).start()
http.server.ThreadingHTTPServer(("10.10.0.2", 8080), H).serve_forever()
EOF
$S python3 "$W/srv.py" & SRV="$SRV $!"

# UDP client: send from a connected socket (so only the server's address is
# accepted back), print the reply or TIMEOUT.
cat > "$W/udp.py" <<'EOF'
import socket, sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.settimeout(4)
s.connect((sys.argv[1], int(sys.argv[2]))); s.send(b"ping")
try: print(s.recv(2048).decode())
except Exception as e: print("TIMEOUT", e)
EOF

http_peer() { $C python3 -c "import urllib.request,sys;print(urllib.request.urlopen('http://$1:8080/',timeout=6).read().decode())" 2>&1 | tail -1; }
udp_peer() { $C python3 "$W/udp.py" "$1" 9000; }
wait_port() { for i in $(seq 1 50); do $G sh -c "exec 3<>/dev/tcp/127.0.0.1/$1" 2>/dev/null && return 0; $G python3 -c "import socket;socket.create_connection(('127.0.0.1',$1),0.2)" 2>/dev/null && return 0; sleep 0.2; done; return 1; }

start_meow() { # $1 = config file
	[ -n "$MP" ] && kill $MP 2>/dev/null && wait $MP 2>/dev/null
	$G "$BIN" -d "$W" -f "$1" > "$W/meow.log" 2>&1 & MP=$!
	wait_port 19098 || bad "meow did not come up: $(tail -3 "$W/meow.log")"
}
check() { # $1 label, $2 got, $3 want-prefix
	case "$2" in "$3"*) ok "$1 ($2)";; *) bad "$1: got '$2', want '$3…'";; esac
}

# TPROXY steering, as clashwrt / OpenClash install it.
tproxy_rules() { # $1 = "to :7894" form
	$G nft -f - <<EOF
table inet mt
delete table inet mt
table inet mt {
	chain pre {
		type filter hook prerouting priority mangle; policy accept;
		iifname "mtg0" ip daddr { 10.10.0.0/24, 198.18.0.0/16 } meta l4proto { tcp, udp } meta mark set 0x1 tproxy ip $1 accept
	}
}
EOF
	$G ip rule add fwmark 0x1 lookup 100 2>/dev/null
	$G ip route replace local 0.0.0.0/0 dev lo table 100
}

base() { # $1 = allow-lan
cat <<EOF
log-level: info
allow-lan: $1
bind-address: "*"
external-controller: 127.0.0.1:19098
tproxy-port: 7894
redir-port: 7892
dns:
  enable: true
  listen: 0.0.0.0:1053
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  nameserver: [10.10.0.2]
rules:
  - MATCH,DIRECT
EOF
}

# ── 1. allow-lan: true, `tproxy ip to :7894` ──
base true > "$W/c1.yaml"; start_meow "$W/c1.yaml"; tproxy_rules "to :7894"
check "TPROXY TCP to real IP via meow" "$(http_peer 10.10.0.2)" 10.10.0.1
check "TPROXY UDP to real IP via meow" "$(udp_peer 10.10.0.2)" "10.10.0.1 ping"
fake=$($C dig +short +time=3 +tries=1 @10.9.0.1 -p 1053 srv.test A 2>&1 | tail -1)
check "fake-ip DNS answer" "$fake" 198.18.
case "$fake" in 198.18.*)
	check "TPROXY TCP to fake IP via meow" "$(http_peer "$fake")" 10.10.0.1
	check "TPROXY UDP to fake IP via meow" "$(udp_peer "$fake")" "10.10.0.1 ping";;
esac
grep -q 'match Match' "$W/meow.log" && ok "meow logged rule matches" || bad "no rule matches in meow log"

# ── 2. REDIRECT (redir-port) ──
$G nft delete table inet mt
$G nft -f - <<EOF
table ip mtr {
	chain pre {
		type nat hook prerouting priority dstnat; policy accept;
		iifname "mtg0" ip daddr 10.10.0.0/24 tcp dport 8080 redirect to :7892
	}
}
EOF
check "REDIRECT TCP via meow (redir-port)" "$(http_peer 10.10.0.2)" 10.10.0.1
$G nft delete table ip mtr

# ── 3. allow-lan: false, `tproxy ip to 127.0.0.1:7894` ──
base false > "$W/c3.yaml"; start_meow "$W/c3.yaml"; tproxy_rules "to 127.0.0.1:7894"
check "TPROXY TCP, listener on 127.0.0.1" "$(http_peer 10.10.0.2)" 10.10.0.1
check "TPROXY UDP, listener on 127.0.0.1" "$(udp_peer 10.10.0.2)" "10.10.0.1 ping"

grep -E 'ERROR|panicked' "$W/meow.log" | head -5
[ $fail = 0 ] && echo "ALL PASS" || { echo "SOME FAILED — meow log tail:"; tail -15 "$W/meow.log"; }
exit $fail
