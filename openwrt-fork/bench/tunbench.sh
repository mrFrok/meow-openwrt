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
# TUN — the redirect+TUN shape, where the TUN carries LAN UDP.
#
# Usage (on the router):
#   sh tunbench.sh up                       # netns, iperf3 server, hy2 server
#   sh tunbench.sh run <label> <core-bin> [scenario...]
#   sh tunbench.sh down
# Scenarios: direct-ul direct-dl hy2-ul hy2-dl (UDP at RATE, default: all),
#            tcp-ul tcp-dl (DIRECT TCP, unlimited rate, PAR=4 streams),
#            hy2tcp-ul hy2tcp-dl (the same TCP through the hy2 proxy),
#            vrtcp-ul vrtcp-dl (the same TCP through VLESS+Vision+REALITY).
# REALITY: the server (mihomo in mwb, dest = its own TLS controller, no
# Internet) and the client take the bench-only throwaway keys from
# reality-server.yaml / reality-client.yaml next to this script.
# Env: RATE=1G LEN=1200 SECS=15 CORE_CPU=0,1 CLI_CPU=2 SRV_CPU=3
#      SRVBIN=/usr/bin/mihomo (hy2 server), MTU=9000, EXTRA= (extra tun: lines), STACK=system|lwip
#      MODE=tun|redir (redir: TCP scenarios via nft REDIRECT), HY2OPTS= (extra hy2 proxy keys), SHAPE= (tc tbf args: a slow client), PAR=4
# Prints: received Mbit/s, loss, core CPU (% of one core), %/Mbit, the core's
# RssAnon and VmHWM (KB) at the end of the run.
set -u
W=/tmp/tunbench; C=mwc; S=mwb; B=$(cd "$(dirname "$0")" && pwd)
# yval <file> <key> <nth>: a scalar of the bench REALITY yamls (never printed).
yval() { grep -o "$2: [^,}]*" "$1" | sed -n "${3:-1}p" | sed "s/^$2: //; s/[][ '\"]//g"; }
RUUID=$(yval "$B/reality-client.yaml" uuid); RPUB=$(yval "$B/reality-client.yaml" public-key)
RSID=$(yval "$B/reality-client.yaml" short-id); RPRIV=$(yval "$B/reality-server.yaml" private-key 2)
RATE=${RATE:-1G}; LEN=${LEN:-1200}; SECS=${SECS:-15}
CORE_CPU=${CORE_CPU:-0,1}; CLI_CPU=${CLI_CPU:-2}; SRV_CPU=${SRV_CPU:-3}
SRVBIN=${SRVBIN:-/usr/bin/mihomo}; MTU=${MTU:-9000}; EXTRA=${EXTRA:-}; STACK=${STACK:-system}; MODE=${MODE:-tun}

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
	[ -n "$RPRIV" ] && [ -n "$RPUB" ] || echo "no REALITY keys in $B/reality-*.yaml: vrtcp-* unavailable"
	cat > $W/srv/config.yaml <<EOF
log-level: warning
external-controller-tls: 127.0.0.1:9443
tls: {certificate: $W/srv/cert.pem, private-key: $W/srv/key.pem}
listeners:
  - {name: hy, type: hysteria2, port: 8444, listen: 203.0.113.2, users: {u: bench}, certificate: $W/srv/cert.pem, private-key: $W/srv/key.pem}
  - {name: vr, type: vless, port: 8443, listen: 203.0.113.2, users: [{username: bench, uuid: $RUUID, flow: xtls-rprx-vision}], reality-config: {dest: 127.0.0.1:9443, private-key: $RPRIV, short-id: [$RSID], server-names: [bench.test]}}
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
redir-port: 7899
tun:
  enable: $([ "$MODE" = tun ] && echo true || echo false)
  device: mwtun
  stack: $STACK
  mtu: $MTU
  inet4-address: [172.31.0.1/30]
  auto-route: false
  auto-detect-interface: false
  dns-hijack: []
  gso: true
  gso-max-size: 65536
$EXTRA
proxies:
  - {name: H, type: hysteria2, server: 203.0.113.2, port: 8444, password: bench, sni: bench.test, skip-cert-verify: true, alpn: [h3]${HY2OPTS:+, $HY2OPTS}}
  - {name: VR, type: vless, server: 203.0.113.2, port: 8443, uuid: $RUUID, network: tcp, flow: xtls-rprx-vision, tls: true, servername: bench.test, client-fingerprint: chrome, reality-opts: {public-key: $RPUB, short-id: $RSID}}
rules: ['MATCH,$1']
EOF
}

ticks() { awk '{print $14+$15}' /proc/$1/stat 2>/dev/null; }

run1() { # label bin scenario
	lab=$1; bin=$2; sc=$3
	case $sc in hy2*) tgt=H ;; vr*) tgt=VR ;; *) tgt=DIRECT ;; esac
	case $sc in *-dl) rev=-R ;; *) rev= ;; esac
	case $sc in
	tcp-*|hy2tcp-*|vrtcp-*) proto="-P ${PAR:-4}"; warm="-P 4"; rate=max ;;
	*) proto="-u -b $RATE -l $LEN"; warm="-u -b 10M -l $LEN"; rate=$RATE ;;
	esac
	d=$W/core; rm -rf $d; mkdir -p $d; core_cfg $tgt > $d/config.yaml
	ip netns exec $C taskset -c "$CORE_CPU" "$bin" -d $d -f $d/config.yaml > $d/log 2>&1 &
	pid=$!; i=0
	if [ "$MODE" = redir ]; then
	# MODE=redir: the redirect TCP path — nft REDIRECT to redir-port.
	sleep 2
	ip -n $C route replace 192.0.2.0/24 via 203.0.113.2 dev mwc0
	ip netns exec $C nft -f - <<NFT
table ip mwredir { chain out { type nat hook output priority -100; meta mark 0x99 return; ip daddr 192.0.2.0/24 tcp dport 5201 redirect to :7899; }; }
NFT
	else
	while [ $i -lt 50 ] && ! ip -n $C link show mwtun >/dev/null 2>&1; do sleep 0.2; i=$((i+1)); done
	ip -n $C link set mwtun up 2>/dev/null
	ip -n $C route replace 192.0.2.0/24 dev mwtun
	fi
	# SHAPE="rate 20mbit": a slow client — the TUN's egress (the client's
	# ACKs) is throttled, so downloads back up into the core's buffers.
	[ -n "${SHAPE:-}" ] && ip netns exec $C tc qdisc replace dev mwtun root tbf $SHAPE burst 16k latency 500ms
	# warm-up: hy2 handshake, flow setup
	ip netns exec $C iperf3 -c 192.0.2.2 $warm -t 2 $rev > /dev/null 2>&1
	t0=$(ticks $pid); s0=$(date +%s)
	( m=0; while kill -0 $pid 2>/dev/null; do a=$(awk '/^RssAnon:/{print $2}' /proc/$pid/status 2>/dev/null); [ -n "$a" ] && [ "$a" -gt "$m" ] && { m=$a; echo $m > $W/anonpeak; }; sleep 1; done ) &
	smp=$!
	out=$(ip netns exec $C taskset -c "$CLI_CPU" iperf3 -c 192.0.2.2 $proto -t "$SECS" $rev 2>&1)
	t1=$(ticks $pid); s1=$(date +%s)
	mem="anonPeak=$(cat $W/anonpeak 2>/dev/null) $(awk '/^(RssAnon|VmHWM):/{printf "%s=%s ", $1, $2}' /proc/$pid/status 2>/dev/null)"
	kill $smp 2>/dev/null; rm -f $W/anonpeak
	kill $pid 2>/dev/null; wait $pid 2>/dev/null
	ip netns exec $C nft delete table ip mwredir 2>/dev/null
	rcv=$(echo "$out" | grep receiver | grep -v '^\[ *[0-9]' | tail -1)
	[ -n "$rcv" ] || rcv=$(echo "$out" | grep receiver | tail -1)
	mbps=$(echo "$rcv" | awk '{for(i=1;i<=NF;i++) if($i ~ /bits\/sec/){v=$(i-1); u=$i; if(u ~ /^G/) v*=1000; if(u ~ /^K/) v/=1000; print v; exit}}')
	loss=$(echo "$rcv" | grep -o '([0-9.e+-]*%)' | tr -d '()')
	cpu=$(awk -v a="${t0:-0}" -v b="${t1:-0}" -v s=$((s1-s0)) 'BEGIN{ if(s<1)s=1; printf "%.0f", (b-a)/s }')
	per=$(awk -v c="$cpu" -v m="${mbps:-0}" 'BEGIN{ if(m>0) printf "%.2f", c/m; else print "-" }')
	printf '%-14s %-10s rate=%-5s recv=%6s Mbit/s loss=%-7s core=%4s%% %%/Mbit=%s %s\n' "$lab" "$sc" "$rate" "${mbps:--}" "${loss:--}" "$cpu" "$per" "$mem"
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
