#!/bin/sh
# Mihomo Lightweight — live-config memory check (D4: measure before patching).
# Runs the core with YOUR config under several GC settings, one after another,
# and reports live heap (from gctrace) next to RssAnon/RssFile. Changes no files.
# Stop the service first so ports are free, start it again afterwards:
#   /etc/init.d/clashwrt stop ; sh livemem.sh /usr/bin/mihomo <config-dir> 90 ; /etc/init.d/clashwrt start
# <config-dir> is the value after -d in: tr '\0' ' ' < /proc/$(pidof mihomo)/cmdline
LC_ALL=C; export LC_ALL
BIN=${1:?binary}; DIR=${2:?config dir}; SECS=${3:-90}
VARIANTS=${VARIANTS:-"GOMEMLIMIT=64MiB GOMEMLIMIT=64MiB,GOGC=50 GOMEMLIMIT=32MiB GOMEMLIMIT=32MiB,GOGC=50"}
printf '%-26s %8s %8s %8s %8s %6s %10s\n' variant peakAnon endAnon endFile liveMB gcs startPeakMB
for v in $VARIANTS; do
	log=/tmp/livemem.$$.log
	env $(echo "$v" | tr ',' ' ') GODEBUG=gctrace=1 "$BIN" -d "$DIR" >/dev/null 2>"$log" & pid=$!
	peak=0; i=0
	while [ $i -lt "$SECS" ]; do sleep 1; i=$((i+1))
		a=$(awk '/^RssAnon/{print $2}' /proc/$pid/status 2>/dev/null) || break
		[ -n "$a" ] && [ "$a" -gt "$peak" ] && peak=$a
	done
	end=$(awk '/^RssAnon/{print $2}' /proc/$pid/status); file=$(awk '/^RssFile/{print $2}' /proc/$pid/status)
	kill $pid; wait $pid 2>/dev/null
	# gctrace: "... A->B->C MB, D MB goal ..." ; C = live heap after GC
	live=$(grep -oE '[0-9]+->[0-9]+->[0-9]+ MB' "$log" | tail -1 | awk -F'->' '{print $3+0}')
	spk=$(grep -oE '[0-9]+->[0-9]+->[0-9]+ MB' "$log" | head -5 | awk -F'->' '{if ($1+0>m) m=$1+0} END{print m+0}')
	gcs=$(grep -c '^gc ' "$log")
	printf '%-26s %8s %8s %8s %8s %6s %10s\n' "$v" "$peak" "$end" "$file" "$live" "$gcs" "$spk"
	rm -f "$log"; sleep 2
done
echo "(KB for Anon/File; MB for live heap and startup peak heap from the first 5 GCs)"
