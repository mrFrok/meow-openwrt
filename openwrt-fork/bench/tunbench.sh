#!/bin/sh
# meow-openwrt — UDP through a core's TUN, entirely inside two netns on the
# router: nothing on the host (routes, firewall, the live core) is touched.
#
#   mwc (client side)                          mwb (server side)
#   iperf3 -c 192.0.2.2 ──► <core TUN mwtun>   iperf3 -s, 192.0.2.2 on lo
#   core DIRECT / hy2 ── fwmark 0x99 ── veth 203.0.113.1 ──► 203.0.113.2
#                                              mihomo hysteria2 listener :8444
#
# The core runs in mwc with routing-mark 0x99; marked sockets use table 100
# (192.0.2.0/24 via the veth), everything else to 192.0.2.0/24 goes into the
# TUN — the clashwrt redirect_tun shape, where the TUN carries LAN UDP.
#
# Usage (on the router):
#   sh tunbench.sh up                       # netns, iperf3 server, hy2 server
#   sh tunbench.sh run <label> <core-bin> [scenario...]
#   sh tunbench.sh down
# Scenarios: direct-ul direct-dl hy2-ul hy2-dl (UDP at RATE, default: all),
#            tcp-ul tcp-dl (DIRECT TCP, unlimited rate, 4 streams).
# Env: RATE=1G LEN=1200 SECS=15 CORE_CPU=0,1 CLI_CPU=2 SRV_CPU=3
#      SRVBIN=/usr/bin/mihomo (hy2 server), MTU=9000, EXTRA= (extra tun: lines)
# Prints: received Mbit/s, loss, core CPU (% of one core), %/Mbit.
set -u
W=/tmp/tunbench; C=mwc; S=mwb
RATE=${RATE:-1G}; LEN=${LEN:-1200}; SECS=${SECS:-15}
CORE_CPU=${CORE_CPU:-0,1}; CLI_CPU=${CLI_CPU:-2}; SRV_CPU=${SRV_CPU:-3}
SRVBIN=${SRVBIN:-/usr/bin/mihomo}; MTU=${MTU:-9000}; EXTRA=${EXTRA:-}

up() {
	mkdir -p $W/srv
	ip netns add $C; ip netns add $S
	ip link add mwc0 type veth peer name mwb0
	ip link set mwc0 netns $C; ip link set mwb0 netns $S
	ip -n $C addr add 203.0.113.1/24 dev mwc0; ip -n $C link set mwc0 up; ip -n $C link set lo up
	ip -n $S addr add 203.0.113.2/24 dev mwb0; ip -n $S link set mwb0 up; ip -n $S link set lo up
	ip -n $S addr add 192.0.2.2/32 dev lo
	# iperf3 connect()s its UDP socket to the first peer: replies must leave
	# from 192.0.2.2, or later datagrams to 192.0.2.2 hit no socket.
	ip -n $S route replace 203.0.113.0/24 dev mwb0 src 192.0.2.2
	ip -n $C rule add fwmark 0x99 lookup 100 pref 100
	ip -n $C route add 192.0.2.0/24 via 203.0.113.2 dev mwc0 table 100
	ip netns exec $S taskset -c "$SRV_CPU" iperf3 -s -D -p 5201
	[ -f $W/srv/cert.pem ] || { echo "need $W/srv/cert.pem + key.pem"; exit 1; }
	cat > $W/srv/config.yaml <<EOF
log-level: warning
listeners:
  - {name: hy, type: hysteria2, port: 8444, listen: 203.0.113.2, users: {u: bench}, certificate: $W/srv/cert.pem, private-key: $W/srv/key.pem}
rules: ['MATCH,DIRECT']
EOF
	ip netns exec $S taskset -c "$SRV_CPU" "$SRVBIN" -d $W/srv -f $W/srv/config.yaml > $W/srv/log 2>&1 &
	sleep 2; echo "up (hy2 server pid $!)"
}

down() {
	for n in $C $S; do ip netns pids $n 2>/dev/null | xargs -r kill; done
	sleep 1; ip netns del $C 2>/dev/null; ip netns del $S 2>/dev/null; echo down
}

core_cfg() { # $1 = rule target
	cat <<EOF
log-level: warning
external-controller: 127.0.0.1:19095
routing-mark: 153
tun:
  enable: true
  device: mwtun
  stack: system
  mtu: $MTU
  inet4-address: [172.31.0.1/30]
  auto-route: false
  auto-detect-interface: false
  dns-hijack: []
  gso: true
  gso-max-size: 65536
$EXTRA
proxies:
  - {name: H, type: hysteria2, server: 203.0.113.2, port: 8444, password: bench, sni: bench.test, skip-cert-verify: true, alpn: [h3]}
rules: ['MATCH,$1']
EOF
}

ticks() { awk '{print $14+$15}' /proc/$1/stat 2>/dev/null; }

run1() { # label bin scenario
	lab=$1; bin=$2; sc=$3
	case $sc in hy2-*) tgt=H ;; *) tgt=DIRECT ;; esac
	case $sc in *-dl) rev=-R ;; *) rev= ;; esac
	case $sc in tcp-*) proto="-P 4"; rate=max ;; *) proto="-u -b $RATE -l $LEN"; rate=$RATE ;; esac
	d=$W/core; rm -rf $d; mkdir -p $d; core_cfg $tgt > $d/config.yaml
	ip netns exec $C taskset -c "$CORE_CPU" "$bin" -d $d -f $d/config.yaml > $d/log 2>&1 &
	pid=$!; i=0
	while [ $i -lt 50 ] && ! ip -n $C link show mwtun >/dev/null 2>&1; do sleep 0.2; i=$((i+1)); done
	ip -n $C link set mwtun up 2>/dev/null
	ip -n $C route replace 192.0.2.0/24 dev mwtun
	# warm-up: hy2 handshake, flow setup
	ip netns exec $C iperf3 -c 192.0.2.2 $proto -t 2 $rev > /dev/null 2>&1
	t0=$(ticks $pid); s0=$(date +%s)
	out=$(ip netns exec $C taskset -c "$CLI_CPU" iperf3 -c 192.0.2.2 $proto -t "$SECS" $rev 2>&1)
	t1=$(ticks $pid); s1=$(date +%s)
	kill $pid 2>/dev/null; wait $pid 2>/dev/null
	rcv=$(echo "$out" | grep receiver | grep -v '^\[ *[0-9]' | tail -1)
	[ -n "$rcv" ] || rcv=$(echo "$out" | grep receiver | tail -1)
	mbps=$(echo "$rcv" | awk '{for(i=1;i<=NF;i++) if($i ~ /bits\/sec/){v=$(i-1); u=$i; if(u ~ /^G/) v*=1000; if(u ~ /^K/) v/=1000; print v; exit}}')
	loss=$(echo "$rcv" | grep -o '([0-9.e+-]*%)' | tr -d '()')
	cpu=$(awk -v a="${t0:-0}" -v b="${t1:-0}" -v s=$((s1-s0)) 'BEGIN{ if(s<1)s=1; printf "%.0f", (b-a)/s }')
	per=$(awk -v c="$cpu" -v m="${mbps:-0}" 'BEGIN{ if(m>0) printf "%.2f", c/m; else print "-" }')
	printf '%-14s %-10s rate=%-5s recv=%6s Mbit/s loss=%-7s core=%4s%% %%/Mbit=%s\n' "$lab" "$sc" "$rate" "${mbps:--}" "${loss:--}" "$cpu" "$per"
	[ -n "$mbps" ] || { echo "$out" | tail -3; tail -5 $d/log; }
}

case "${1:-}" in
up) up ;;
down) down ;;
run)
	lab=${2:?label}; bin=${3:?core binary}; shift 3
	[ $# -gt 0 ] || set -- direct-ul direct-dl hy2-ul hy2-dl
	for sc in "$@"; do run1 "$lab" "$bin" "$sc"; done ;;
*) echo "usage: $0 up | run <label> <core-bin> [scenario...] | down"; exit 1 ;;
esac
