#!/bin/sh
# RSS breakdown of mihomo builds on THIS host (desktop numbers, not a router):
# idle after 5 s, then with N parallel slow downloads through mixed-port.
# Usage: bench/mem.sh <binary>... ; env: N=100 PROCS="1 4" MEMLIMIT=""(e.g. 48MiB) GOGCV=""
set -eu
export LC_ALL=C
N=${N:-100}; PROCS=${PROCS:-"1 4"}
W=${W:-/tmp/mlw-mem}; mkdir -p "$W/www"
[ -f "$W/www/big" ] || head -c 268435456 /dev/zero > "$W/www/big"
python3 -m http.server 18080 --bind 127.0.0.1 --directory "$W/www" >/dev/null 2>&1 & HTTP=$!
trap 'kill $HTTP 2>/dev/null || true; pkill -f "limit-rate 1M -x http://127.0.0.1:17890" 2>/dev/null || true' EXIT
sleep 1
snap() { # pid -> "rss anon file pss" in KB
	r=$(awk '/^VmRSS/{print $2}' /proc/$1/status); a=$(awk '/^RssAnon/{print $2}' /proc/$1/status)
	f=$(awk '/^RssFile/{print $2}' /proc/$1/status); p=$(awk '/^Pss:/{print $2}' /proc/$1/smaps_rollup)
	echo "$r $a $f $p"; }
printf '%-28s %2s %-9s %7s %7s %7s %7s\n' binary P state RSS Anon File Pss
for bin in "$@"; do for P in $PROCS; do
	home=$W/home; rm -rf "$home"; mkdir -p "$home"
	printf 'mixed-port: 17890\nlog-level: silent\nmode: rule\ngeodata-mode: false\ngeo-auto-update: false\nrules:\n  - MATCH,DIRECT\n' > "$home/config.yaml"
	env GOMAXPROCS=$P ${MEMLIMIT:+GOMEMLIMIT=$MEMLIMIT} ${GOGCV:+GOGC=$GOGCV} "$bin" -d "$home" >/dev/null 2>&1 & pid=$!
	sleep 5; set -- $(snap $pid)
	printf '%-28s %2s %-9s %7s %7s %7s %7s\n' "$(basename "$bin")" $P idle $1 $2 $3 $4
	i=0; while [ $i -lt "$N" ]; do curl -s -o /dev/null --limit-rate 1M -x http://127.0.0.1:17890 http://127.0.0.1:18080/big & i=$((i+1)); done
	pr=0; pa=0; pf=0; pp=0
	for _ in 1 2 3 4 5 6 7 8; do sleep 1; set -- $(snap $pid); [ $1 -gt $pr ] && { pr=$1; pa=$2; pf=$3; pp=$4; }; done
	printf '%-28s %2s %-9s %7s %7s %7s %7s\n' "$(basename "$bin")" $P "peak$N" $pr $pa $pf $pp
	pkill -f "limit-rate 1M -x http://127.0.0.1:17890" || true; kill $pid; wait $pid 2>/dev/null || true; sleep 1
done; done
