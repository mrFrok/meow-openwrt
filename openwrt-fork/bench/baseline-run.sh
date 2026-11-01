#!/bin/bash
# Mihomo Lightweight — baseline driver, runs on a LAN PC (this machine).
# Temporarily switches the clashwrt router so all LAN traffic goes through mihomo
# (bypass_set_mode=off, watchdog=0), adds 3 rules for the iperf3 VPS, pins the
# proxy nodes, runs iperf3 scenarios while bench/baseline.sh samples the router,
# then restores everything. A failsafe on the router restores after FAILSAFE seconds
# even if this script dies.
# Usage: bench/baseline-run.sh <label> [outdir]     e.g. bench/baseline-run.sh upstream
# Env: ROUTER=root@192.168.1.1 VPS=217.60.33.169 T=30 (seconds per scenario)
#      PROXY_TCP="🇵🇱 TCP Reality" PROXY_UDP="🇵🇱 Hysteria2" UDP_BW=300M FAILSAFE=1800
#      ONLY="tcp-proxy-dl4 udp-proxy-dl" — run only these scenarios
#      PROXY_INJECT=bench/reality-client.yaml — extra top-level proxies: appended to the config
#      TCP_TARGET=Proxy — rule target for the TCP proxy ports (e.g. "LW Reality" from PROXY_INJECT)
#      UDP_LEN=1200 — iperf3 UDP datagram size (default 1460 > QUIC datagram: Hysteria2 fragments it in two)
#      UDP_TARGET=ProxyUDP — same for UDP (e.g. "LW Hysteria2")
#      IPERF_SSH=user@host — run the iperf3 client there over ssh (when this PC is the bench server)
# VPS side (prepared by the user): iperf3 -s on 5201-5204 (PROXY) and 5211-5214 (DIRECT).
set -u
LABEL=${1:?label}; OUT=${2:-out/baseline/$LABEL}
ROUTER=${ROUTER:-root@192.168.1.1}; VPS=${VPS:-217.60.33.169}; T=${T:-30}
PROXY_TCP=${PROXY_TCP:-"🇵🇱 TCP Reality"}; PROXY_UDP=${PROXY_UDP:-"🇵🇱 Hysteria2"}
UDP_BW=${UDP_BW:-300M}; FAILSAFE=${FAILSAFE:-1800}; TCP_TARGET=${TCP_TARGET:-Proxy}; UDP_TARGET=${UDP_TARGET:-ProxyUDP}; PROXY_INJECT=${PROXY_INJECT:-}; IPERF_SSH=${IPERF_SSH:-}
mkdir -p "$OUT"
r() { timeout 120 ssh -o BatchMode=yes "$ROUTER" "sh -c '$1'"; }   # router root shell is fish

# --- router-side scripts (uploaded once) ---
SETUP=$(cat <<EOF
set -e
D=/tmp/bl; mkdir -p \$D
C=/etc/mihomo/config.yaml
S=\$(sed -n "s/^secret: //p" \$C)
api() { curl -s -o /dev/null -w "%{http_code}" -X "\$1" -H "Authorization: Bearer \$S" -H "Content-Type: application/json" "http://127.0.0.1:9090\$2" -d "\$3"; }
if [ ! -f \$D/config.yaml.orig ]; then # an interrupted run left bench state: keep its originals
cp \$C \$D/config.yaml.orig
uci -q get clashwrt.config.bypass_set_mode > \$D/bypass_set_mode.orig || echo auto > \$D/bypass_set_mode.orig
uci -q get clashwrt.config.watchdog > \$D/watchdog.orig || echo 1 > \$D/watchdog.orig
for g in Proxy ProxyUDP; do curl -s -H "Authorization: Bearer \$S" http://127.0.0.1:9090/proxies/\$g | jsonfilter -e @.now > \$D/sel.\$g.orig; done
fi
awk '{print} /^rules:/ && !d {print "  - AND,((NETWORK,udp),(IP-CIDR,$VPS/32),(DST-PORT,5201-5204)),$UDP_TARGET"; print "  - AND,((IP-CIDR,$VPS/32),(DST-PORT,5201-5204)),$TCP_TARGET"; print "  - AND,((IP-CIDR,$VPS/32),(DST-PORT,5211-5214)),DIRECT"; d=1}' \$D/config.yaml.orig > \$C
if [ -s \$D/inject.yaml ]; then echo proxies: >> \$C; cat \$D/inject.yaml >> \$C; fi
echo "reload: \$(api PUT "/configs?force=true" "{\"path\":\"\$C\"}")"
sleep 3
echo "pin Proxy: \$(api PUT /proxies/Proxy "{\"name\":\"$PROXY_TCP\"}")"
echo "pin ProxyUDP: \$(api PUT /proxies/ProxyUDP "{\"name\":\"$PROXY_UDP\"}")"
uci set clashwrt.config.bypass_set_mode=off; uci set clashwrt.config.watchdog=0; uci commit clashwrt
/etc/init.d/clashwrt reload
( sleep $FAILSAFE; sh \$D/restore.sh >> \$D/failsafe.log 2>&1 ) >/dev/null 2>&1 &
echo \$! > \$D/failsafe.pid
EOF
)
RESTORE=$(cat <<'EOF'
D=/tmp/bl; C=/etc/mihomo/config.yaml
[ -f $D/config.yaml.orig ] || { echo "nothing to restore"; exit 0; }
S=$(sed -n "s/^secret: //p" $D/config.yaml.orig)
api() { curl -s -o /dev/null -w "%{http_code}" -X "$1" -H "Authorization: Bearer $S" -H "Content-Type: application/json" "http://127.0.0.1:9090$2" -d "$3"; }
cp $D/config.yaml.orig $C
echo "reload: $(api PUT "/configs?force=true" "{\"path\":\"$C\"}")"
sleep 3
for g in Proxy ProxyUDP; do n=$(cat $D/sel.$g.orig 2>/dev/null); [ -n "$n" ] && echo "restore $g: $(api PUT /proxies/$g "{\"name\":\"$n\"}")"; done
uci set clashwrt.config.bypass_set_mode="$(cat $D/bypass_set_mode.orig)"; uci set clashwrt.config.watchdog="$(cat $D/watchdog.orig)"; uci commit clashwrt
/etc/init.d/clashwrt reload
[ -f $D/failsafe.pid ] && kill "$(cat $D/failsafe.pid)" 2>/dev/null
rm -f $D/config.yaml.orig $D/failsafe.pid
echo restored
EOF
)

up() { timeout 30 ssh -o BatchMode=yes "$ROUTER" "sh -c 'mkdir -p /tmp/bl; cat > $2'" < "$1"; }
printf '%s\n' "$SETUP" > "$OUT/.setup.sh"; printf '%s\n' "$RESTORE" > "$OUT/.restore.sh"
up "$OUT/.setup.sh" /tmp/bl/setup.sh && up "$OUT/.restore.sh" /tmp/bl/restore.sh && up bench/baseline.sh /tmp/bl/baseline.sh && up "${PROXY_INJECT:-/dev/null}" /tmp/bl/inject.yaml || { echo "upload failed"; exit 1; }
rm -f "$OUT/.setup.sh" "$OUT/.restore.sh"

restore() { echo "== restore"; r "sh /tmp/bl/restore.sh" 2>&1 | tee "$OUT/restore.log"; }
trap restore EXIT

echo "== setup"; r "sh /tmp/bl/setup.sh" 2>&1 | tee "$OUT/setup.log"
sleep 5
r "/usr/libexec/clashwrt/fw.sh status" > "$OUT/fw-status.txt" 2>&1
r "nft -t list chain inet clashwrt mangle" > "$OUT/nft-mangle.txt" 2>&1

# scenario: name, seconds, iperf3 args (empty = idle)
run() {
	local name=$1 secs=$2; shift 2
	if [ -n "${ONLY:-}" ] && ! printf ' %s ' $ONLY | grep -q " $name "; then return; fi
	echo "== $name"
	r "sh /tmp/bl/baseline.sh $LABEL-$name $secs" > "$OUT/$name.csv" 2>"$OUT/$name.err" &
	local sp=$!
	sleep 1
	if [ $# -gt 0 ]; then
		if [ -n "$IPERF_SSH" ]; then # iperf3 client on another LAN host (e.g. the PC is the bench server)
			timeout $((secs + 15)) ssh -o BatchMode=yes "$IPERF_SSH" iperf3 -c "$VPS" -t $((secs - 3)) --connect-timeout 5000 -J "$*" > "$OUT/$name.json" 2>&1
		else
			timeout $((secs + 15)) iperf3 -c "$VPS" -t $((secs - 3)) --connect-timeout 5000 -J "$@" > "$OUT/$name.json" 2>&1
		fi
	fi
	wait $sp
}
run idle 30
run tcp-direct-dl  $T -p 5211 -R
run tcp-direct-ul  $T -p 5212
run tcp-direct-dl4 $T -p 5213 -R -P 4
run tcp-proxy-dl   $T -p 5201 -R
run tcp-proxy-ul   $T -p 5202
run tcp-proxy-dl4  $T -p 5203 -R -P 4
run udp-direct-dl  $T -p 5214 -u -b "$UDP_BW" ${UDP_LEN:+-l $UDP_LEN} -R
run udp-direct-ul  $T -p 5214 -u -b "$UDP_BW" ${UDP_LEN:+-l $UDP_LEN}
run udp-proxy-dl   $T -p 5204 -u -b "$UDP_BW" ${UDP_LEN:+-l $UDP_LEN} -R
run udp-proxy-ul   $T -p 5204 -u -b "$UDP_BW" ${UDP_LEN:+-l $UDP_LEN}
echo "== done: $OUT"
