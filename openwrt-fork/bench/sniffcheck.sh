#!/bin/sh
# meow-openwrt — TUN TLS sniffer check inside tunbench's netns (run
# `tunbench.sh up` first). A TLS ClientHello to 192.0.2.2 with SNI
# bench.test must be named by the sniffer and hit DOMAIN-SUFFIX,bench.test
# (REJECT); the same connect without SNI must fall through to MATCH,DIRECT.
# Usage (on the router): sh sniffcheck.sh <core-bin> [stack=system|lwip]
set -u
BIN=${1:?core-bin}; STACK=${2:-system}; W=/tmp/tunbench; C=mwc; d=$W/sniff
rm -rf $d; mkdir -p $d
cat > $d/config.yaml <<CFG
log-level: info
external-controller: 127.0.0.1:19096
routing-mark: 153
sniffer:
  enable: true
  sniff:
    TLS: {ports: [5201]}
tun:
  enable: true
  device: mwtun
  stack: $STACK
  inet4-address: [172.31.0.1/30]
  auto-route: false
  auto-detect-interface: false
  dns-hijack: []
rules: ['DOMAIN-SUFFIX,bench.test,REJECT', 'MATCH,DIRECT']
CFG
ip netns exec $C "$BIN" -d $d -f $d/config.yaml > $d/log 2>&1 &
pid=$!; i=0
while [ $i -lt 50 ] && ! ip -n $C link show mwtun >/dev/null 2>&1; do sleep 0.2; i=$((i+1)); done
ip -n $C link set mwtun up; ip -n $C route replace 192.0.2.0/24 dev mwtun
ip netns exec $C curl -sk -m 3 --resolve bench.test:5201:192.0.2.2 https://bench.test:5201/ >/dev/null 2>&1
ip netns exec $C curl -sk -m 3 https://192.0.2.2:5201/ >/dev/null 2>&1
sleep 1; kill $pid; wait $pid 2>/dev/null
ip -n $C route del 192.0.2.0/24 dev mwtun 2>/dev/null
echo "== $STACK"; grep -E 'bench\.test|192\.0\.2\.2:5201' $d/log | sed 's/\x1b\[[0-9;]*m//g' | cut -c28-220
