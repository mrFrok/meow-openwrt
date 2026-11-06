#!/bin/sh
# meow-openwrt — memory of cores on a live router config, without touching the
# running service. Copies <config-dir>/config.yaml into /tmp with its own ports
# (mixed 17890, controller 127.0.0.1:19090, DNS 127.0.0.1:11053), TUN/tproxy/
# redir off; provider dirs are symlinked, so the same rule-sets and proxies load.
# Each binary runs in turn: SECS idle, then N parallel downloads through
# mixed-port (if curl and URL are given). Prints RssAnon/RssFile in KB.
# Usage: sh meowmem.sh <config-dir> <bin>... ; env: SECS=60 N=20 URL= RATE=1M ENVS=
# ENVS (e.g. "MIMALLOC_PURGE_DELAY=0") is applied to every binary.
LC_ALL=C; export LC_ALL
DIR=${1:?config dir}; shift
SECS=${SECS:-60}; N=${N:-20}; URL=${URL:-}
W=/tmp/meowmem; H=$W/home
rm -rf "$W"; mkdir -p "$H"
for e in "$DIR"/*; do
	case ${e##*/} in config.yaml|cache.db|backups|ui) ;; *) ln -s "$(readlink -f "$e")" "$H/${e##*/}" ;; esac
done
awk '
/^(tproxy-port|redir-port|external-controller|external-ui|secret|mixed-port|port|socks-port):/ {next}
/^[a-z]/ { sect=$1 }
sect=="tun:" && /^  enable:/ { print "  enable: false"; next }
sect=="dns:" && /^  listen:/ { print "  listen: 127.0.0.1:11053"; next }
sect=="listeners:" && /^    port:/ { sub(/[0-9]+/, "1" substr($2,1,4)) }
{ print }
END { print "mixed-port: 17890"; print "external-controller: 127.0.0.1:19090" }
' "$DIR/config.yaml" > "$H/config.yaml"

kb() { awk -v k="$2" '$1==k":"{print $2}' /proc/$1/status 2>/dev/null; }
printf '%-22s %9s %9s %9s %9s %9s %9s\n' binary idlePeak idleEnd loadPeak loadEnd File HWM
for bin in "$@"; do
	env $ENVS "$bin" -d "$H" -f "$H/config.yaml" >"$W/${bin##*/}.log" 2>&1 & pid=$!
	peak=0; i=0
	while [ $i -lt "$SECS" ]; do sleep 1; i=$((i+1))
		a=$(kb $pid RssAnon); [ -z "$a" ] && break
		[ "$a" -gt "$peak" ] && peak=$a
	done
	idle=$(kb $pid RssAnon); lpeak=-; lend=-
	if [ -n "$URL" ] && command -v curl >/dev/null && [ -n "$idle" ]; then
		j=0; cp=""; while [ $j -lt "$N" ]; do
			curl -s -o /dev/null -m 30 --limit-rate ${RATE:-1M} -x http://127.0.0.1:17890 "$URL" & cp="$cp $!"; j=$((j+1))
		done
		lpeak=0; k=0
		while [ $k -lt 30 ]; do sleep 1; k=$((k+1))
			a=$(kb $pid RssAnon); [ -n "$a" ] && [ "$a" -gt "$lpeak" ] && lpeak=$a
		done
		wait $cp 2>/dev/null; sleep 5; lend=$(kb $pid RssAnon)
	fi
	file=$(kb $pid RssFile); hwm=$(kb $pid VmHWM)
	printf '%-22s %9s %9s %9s %9s %9s %9s\n' "${bin##*/}" "$peak" "${idle:-dead}" "$lpeak" "$lend" "${file:--}" "${hwm:--}"
	kill $pid 2>/dev/null; wait $pid 2>/dev/null; sleep 2
done
echo "logs: $W/*.log"
