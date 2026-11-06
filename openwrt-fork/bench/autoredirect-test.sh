#!/bin/sh
# meow-openwrt — functional test of tun.auto-redirect, entirely in three netns
# on the router; nothing on the host (routes, firewall, the live core) is
# touched.
#
#   mwc (LAN client)         mwg (gateway: meow)            mws (server)
#   192.168.77.2 ──veth── 192.168.77.1 | 10.78.0.1 ──veth── 10.78.0.2
#                                                  on lo: 203.0.113.2 (captured)
#                                                         203.0.113.3 (not)
# (Off the gateway's own subnets, which auto-redirect never captures.)
#
# meow runs in mwg with tun.auto-redirect and route-address 203.0.113.2/32.
# Checks: LAN TCP and UDP to .2 go through meow, to .3 they do not; LAN DNS
# is answered by meow (hosts entry); the gateway's own TCP to .2 goes through
# meow; on SIGTERM the nft table and policy rules are gone.
# Usage: sh autoredirect-test.sh <meow-bin>
LC_ALL=C; export LC_ALL
BIN=${1:?meow binary}
W=/tmp/mwar; rm -rf $W; mkdir -p $W/www; echo hello > $W/www/x
fail=0
ok() { echo "PASS $*"; }
bad() { echo "FAIL $*"; fail=1; }
G="ip netns exec mwg"; C="ip netns exec mwc"; S="ip netns exec mws"

cleanup() {
	[ -n "$MP" ] && kill $MP 2>/dev/null
	for p in $SRV; do kill $p 2>/dev/null; done
	for n in mwc mwg mws; do ip netns del $n 2>/dev/null; done
}
trap cleanup EXIT

for n in mwc mwg mws; do ip netns del $n 2>/dev/null; ip netns add $n; ip netns exec $n ip link set lo up; done
ip link add mwc0 type veth peer name mwg0
ip link set mwc0 netns mwc; ip link set mwg0 netns mwg
ip link add mwg1 type veth peer name mws0
ip link set mwg1 netns mwg; ip link set mws0 netns mws
$C ip addr add 192.168.77.2/24 dev mwc0; $C ip link set mwc0 up; $C ip route add default via 192.168.77.1
$G ip addr add 192.168.77.1/24 dev mwg0; $G ip link set mwg0 up
$G ip addr add 10.78.0.1/24 dev mwg1; $G ip link set mwg1 up; $G ip route add default via 10.78.0.2
$G sysctl -qw net.ipv4.ip_forward=1
$S ip addr add 10.78.0.2/24 dev mws0; $S ip link set mws0 up
$S ip addr add 203.0.113.2/32 dev lo; $S ip addr add 203.0.113.3/32 dev lo
$S ip route add 192.168.77.0/24 via 10.78.0.1
$G ip route add 203.0.113.0/24 via 10.78.0.2
SRV=""
for a in 203.0.113.2 203.0.113.3; do $S uhttpd -f -p $a:8080 -h $W/www & SRV="$SRV $!"; done
$S iperf3 -s -D -B 203.0.113.2 >/dev/null 2>&1

cat > $W/config.yaml <<EOF
log-level: info
external-controller: 127.0.0.1:19099
hosts:
  test.lan: 1.2.3.4
dns:
  enable: true
  listen: 127.0.0.1:11053
  enhanced-mode: redir-host
  use-hosts: true
  nameserver: [203.0.113.3]
tun:
  enable: true
  stack: system
  device: mwtun
  auto-route: true
  auto-redirect: true
  route-address: [203.0.113.2/32]
rules:
  - MATCH,DIRECT
EOF
$G $BIN -d $W -f $W/config.yaml > $W/meow.log 2>&1 & MP=$!
sleep 4
$G nft list table inet meow > $W/nft.txt 2>&1 && ok "table inet meow installed" || bad "no table inet meow: $(head -2 $W/nft.txt)"
$G ip rule | grep -q 'fwmark 0x2023' && ok "fwmark rule" || bad "no fwmark rule"

conns() { $G wget -qO- http://127.0.0.1:19099/connections 2>/dev/null; }
seen() { grep -c -- "$1" $W/meow.log; }

r=$($C wget -qO- -T 5 http://203.0.113.2:8080/x 2>&1)
[ "$r" = hello ] && ok "LAN TCP to captured .2" || bad "LAN TCP to .2: $r"
[ "$(seen '203.0.113.2:8080')" -ge 1 ] && ok "  ...went through meow" || bad "  ...not in meow log"
r=$($C wget -qO- -T 5 http://203.0.113.3:8080/x 2>&1)
[ "$r" = hello ] && ok "LAN TCP to uncaptured .3" || bad "LAN TCP to .3: $r"
[ "$(seen '203.0.113.3:8080')" -eq 0 ] && ok "  ...bypassed meow" || bad "  ...went through meow"

$C iperf3 -u -c 203.0.113.2 -b 10M -t 2 > $W/iperf.txt 2>&1
grep -q 'receiver' $W/iperf.txt && ok "LAN UDP iperf3 to .2" || bad "LAN UDP iperf3: $(tail -2 $W/iperf.txt)"
grep -q '203.0.113.2:5201' $W/meow.log && ok "  ...through meow" || bad "  ...UDP not in meow log"

r=$($C nslookup test.lan 203.0.113.3 2>&1 | grep -c 1.2.3.4)
[ "$r" -ge 1 ] && ok "LAN DNS hijacked (hosts answer)" || bad "LAN DNS not hijacked"

n0=$(seen '203.0.113.2:8080')
r=$($G wget -qO- -T 5 http://203.0.113.2:8080/x 2>&1)
[ "$r" = hello ] && [ "$(seen '203.0.113.2:8080')" -gt "$n0" ] && ok "gateway's own TCP through meow" || bad "gateway own TCP: $r"

kill $MP; sleep 2; MP=""
$G nft list table inet meow >/dev/null 2>&1 && bad "table left after SIGTERM" || ok "table removed"
$G ip rule | grep -q 'fwmark 0x2023' && bad "rules left after SIGTERM" || ok "rules removed"
echo "--- meow log (warn/error):"; grep -iE 'warn|error' $W/meow.log | head -8

# Phase 2: route-address-dns — capture only what the rules do not send
# DIRECT, learned from DNS answers.
cat > $W/config.yaml <<EOF
log-level: info
hosts:
  blocked.lan: 203.0.113.2
  ok.lan: 203.0.113.3
dns:
  enable: true
  listen: 127.0.0.1:11053
  enhanced-mode: redir-host
  use-hosts: true
  nameserver: [203.0.113.3]
tun:
  enable: true
  stack: system
  device: mwtun
  auto-route: true
  auto-redirect: true
  route-address-dns: true
rules:
  - DOMAIN,blocked.lan,REJECT
  - MATCH,DIRECT
EOF
$G $BIN -d $W -f $W/config.yaml > $W/meow2.log 2>&1 & MP=$!
sleep 4
r=$($C wget -qO- -T 5 http://203.0.113.2:8080/x 2>&1)
[ "$r" = hello ] && ! grep -q -- '203.0.113.2:8080' $W/meow2.log && ok "dns-set: .2 bypasses before any DNS answer" || bad "dns-set: .2 before DNS: $r"
$C nslookup blocked.lan 203.0.113.3 >/dev/null 2>&1; $C nslookup ok.lan 203.0.113.3 >/dev/null 2>&1
$G nft list set inet meow route4_dns 2>&1 | grep -q '203.0.113.2' && ok "dns-set: REJECT domain's address added" || bad "dns-set: .2 not in route4_dns"
$G nft list set inet meow route4_dns 2>&1 | grep -q '203.0.113.3' && bad "dns-set: DIRECT domain's address added" || ok "dns-set: DIRECT domain's address not added"
# (A hosts answer leaves no IP->domain mapping, so the flow itself matches
# by IP; what matters here is that it reaches meow.)
$C wget -qO- -T 5 http://203.0.113.2:8080/x >/dev/null 2>&1
grep -q -- '203.0.113.2:8080' $W/meow2.log && ok "dns-set: .2 now captured" || bad "dns-set: .2 still bypasses"
r=$($C wget -qO- -T 5 http://203.0.113.3:8080/x 2>&1)
[ "$r" = hello ] && ! grep -q -- '203.0.113.3:8080' $W/meow2.log && ok "dns-set: .3 still bypasses" || bad "dns-set: .3: $r"
kill $MP; sleep 2; MP=""
grep -iE 'warn|error' $W/meow2.log | head -5
[ $fail = 0 ] && echo ALL PASS || echo SOME FAILED
