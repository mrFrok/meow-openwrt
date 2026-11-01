#!/bin/bash
# Mihomo Lightweight — CPU profile of one baseline scenario on the router.
# Bind-mounts a pprof-enabled build (IsDebug forced, built without no_pprof) over
# /usr/bin/mihomo, restarts the service, runs bench/baseline-run.sh for ONLY=<scenario>,
# grabs /debug/pprof/profile while iperf3 runs, then restores the production binary.
# Usage: bench/prof-run.sh <label> <scenario> [router-binary]
#   e.g. bench/prof-run.sh tiny udp-direct-dl /tmp/prof/mihomo
set -u
LABEL=${1:?label}; SCEN=${2:?scenario}; BIN=${3:-/tmp/prof/mihomo}
ROUTER=${ROUTER:-root@192.168.1.1}; SEC=${SEC:-20}
OUT=out/prof/$LABEL; mkdir -p "$OUT"
r() { timeout 60 ssh -o BatchMode=yes "$ROUTER" "sh -c '$1'"; }
restore() { r "/etc/init.d/mihomo stop; sleep 1; umount /usr/bin/mihomo; /etc/init.d/mihomo start; sleep 3; grep -c usr/bin/mihomo /proc/mounts; ls -l /usr/bin/mihomo"; }
trap restore EXIT
r "mount --bind $BIN /usr/bin/mihomo && /etc/init.d/mihomo restart; sleep 5; /usr/bin/mihomo -v | head -1; pidof mihomo"
ONLY=$SCEN bench/baseline-run.sh "$LABEL" "$OUT" > "$OUT/run.log" 2>&1 &
RUN=$!
until pgrep -f "iperf3 -c" >/dev/null || ! kill -0 $RUN 2>/dev/null; do sleep 0.5; done # client only: iperf3 -s may run here
sleep 3
curl -s -o "$OUT/$SCEN.cpu.pprof" "http://${ROUTER#*@}:9090/debug/pprof/profile?seconds=$SEC"
wait $RUN
tail -5 "$OUT/run.log"
go tool pprof -top -nodecount=40 "$OUT/$SCEN.cpu.pprof" 2>/dev/null | head -50
