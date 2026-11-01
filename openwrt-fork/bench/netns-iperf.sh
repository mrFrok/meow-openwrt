#!/bin/sh
# Mihomo Lightweight — local iperf3 target on the router instead of a VPS.
# Runs ON THE ROUTER: iperf3 -s in a netns behind a veth, address 203.0.113.2
# (TEST-NET-3: not in clashwrt's private bypass set, so LAN traffic to it is
# forwarded through mihomo; mihomo's DIRECT socket reaches it via the veth).
# No ISP link in the path: the ceiling is the 1 Gbit/s LAN, not the uplink.
# Usage: sh netns-iperf.sh up|down    (iperf3 on 5201-5204 and 5211-5214, like the VPS)
# If /tmp/lwsrv/mihomo exists (+ config.yaml, cert.pem, key.pem from bench/reality-server.yaml),
# a VLESS+Reality server also runs in the netns on :8443, pinned to CPU ${SRV_CPU:-3}.
NS=lwbench; H=vlwb0; P=vlwb1
case "$1" in
up)
	ip netns add $NS
	ip link add $H type veth peer name $P
	ip link set $P netns $NS
	ip addr add 203.0.113.1/24 dev $H; ip link set $H up
	ip -n $NS addr add 203.0.113.2/24 dev $P; ip -n $NS link set $P up; ip -n $NS link set lo up
	ip -n $NS route add default via 203.0.113.1
	for p in 5201 5202 5203 5204 5211 5212 5213 5214; do ip netns exec $NS iperf3 -s -D -p $p; done
	[ -x /tmp/lwsrv/mihomo ] && { ip netns exec $NS taskset -c ${SRV_CPU:-3} /tmp/lwsrv/mihomo -d /tmp/lwsrv > /tmp/lwsrv/log 2>&1 & sleep 2; echo "reality: $(ip netns exec $NS netstat -ltn | grep -c ':8443 ')"; }
	echo up ;;
down)
	ip netns pids $NS 2>/dev/null | xargs -r kill
	ip link del $H 2>/dev/null; ip netns del $NS 2>/dev/null
	echo down ;;
*) echo "usage: $0 up|down"; exit 1 ;;
esac
