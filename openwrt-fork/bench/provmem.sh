#!/bin/sh
# Mihomo Lightweight — RssAnon around a rule-provider update (D4, lw_free_after_load
# hook in component/resource/fetcher_lw.go). Router side, BusyBox ash.
# Runs BIN with the given config dir, lets it settle, forces an update of each named
# rule-provider through the REST API, samples RssAnon every second. Changes no files.
# Stop the running core first (ports), start it again afterwards.
# Usage: sh provmem.sh <bin> <config-dir> <provider>[,<provider>...] [settle=60] [after=30]
# Env: ENVS="GOMEMLIMIT=48MiB GOGC=50" (same for every binary you compare)
LC_ALL=C; export LC_ALL
BIN=${1:?binary}; DIR=${2:?config dir}; PROVS=${3:?providers}; SETTLE=${4:-60}; AFTER=${5:-30}
ENVS=${ENVS:-"GOMEMLIMIT=48MiB GOGC=50"}
C=$DIR/config.yaml
S=$(sed -n 's/^secret: *//p' "$C"); P=$(sed -n 's/^external-controller: *[^:]*:\([0-9]*\).*/\1/p' "$C")
anon() { awk '/^RssAnon/{print $2}' /proc/$pid/status 2>/dev/null; }
env $ENVS "$BIN" -d "$DIR" >/dev/null 2>&1 & pid=$!
sleep "$SETTLE"
a0=$(anon); [ -n "$a0" ] || { echo "core died"; exit 1; }
t0=$(date +%s)
for p in $(echo "$PROVS" | tr ',' ' '); do
	code=$(curl -s -o /dev/null -w '%{http_code}' -X PUT -H "Authorization: Bearer $S" "http://127.0.0.1:$P/providers/rules/$p")
	echo "# update $p: $code ($(( $(date +%s) - t0 ))s)"
done
peak=$a0; i=0; a5=; a10=
while [ $i -lt "$AFTER" ]; do sleep 1; i=$((i+1))
	a=$(anon); [ -n "$a" ] || break
	[ "$a" -gt "$peak" ] && peak=$a
	[ $i -eq 5 ] && a5=$a; [ $i -eq 10 ] && a10=$a
done
end=$(anon)
kill $pid; wait $pid 2>/dev/null
echo "bin=$BIN env=$ENVS"
echo "anon_kb before=$a0 peak=$peak after5s=$a5 after10s=$a10 after${AFTER}s=$end"
