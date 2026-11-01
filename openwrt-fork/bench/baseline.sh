#!/bin/sh
# Mihomo Lightweight — router-side sampler (BusyBox ash). Read-only: changes nothing.
# Records, once per second, the proxy core's CPU and memory plus system CPU/softirq and
# kernel memory, while YOU generate load from a LAN PC (see BENCHMARKS.md scenarios).
# Usage: sh baseline.sh <label> [seconds=60] [process=mihomo] > <label>.csv
# Example: sh baseline.sh upstream-idle 120 ; sh baseline.sh upstream-tcp-direct-iperf 60
LC_ALL=C; export LC_ALL
LABEL=${1:?label}; SECS=${2:-60}; NAME=${3:-mihomo}
PID=$(pidof "$NAME" | awk '{print $1}'); [ -n "$PID" ] || { echo "no $NAME process" >&2; exit 1; }
HZ=100; NCPU=$(grep -c '^processor' /proc/cpuinfo)
st() { awk '{print $14+$15}' /proc/$PID/stat; }                       # utime+stime ticks; core_cpu_pct is % of ONE core (200 = two cores busy)
cpu() { awk '/^cpu /{print $2+$3+$4+$5+$6+$7+$8, $5+$6, $7, $8}' /proc/stat; }  # total idle+iowait irq softirq
mi() { awk '/^(MemAvailable|Slab|SUnreclaim|KernelStack|PageTables|Shmem):/{printf "%s ", $2}' /proc/meminfo; }
ps1() { awk '/^(VmRSS|RssAnon|RssFile|Threads):/{printf "%s ", $2}' /proc/$PID/status; awk '/^Pss:/{printf "%s ", $2}' /proc/$PID/smaps_rollup 2>/dev/null; }
nconn() { awk 'END{print NR-1}' /proc/net/nf_conntrack 2>/dev/null || cat /proc/sys/net/netfilter/nf_conntrack_count 2>/dev/null; }
echo "# label=$LABEL pid=$PID ncpu=$NCPU kernel=$(uname -r) core=$(readlink /proc/$PID/exe) $(tr '\0' ' ' < /proc/$PID/cmdline | cut -c1-120)"
echo "# env: $(tr '\0' '\n' < /proc/$PID/environ 2>/dev/null | grep -E '^(GOGC|GOMEMLIMIT|GOMAXPROCS|GODEBUG)=' | tr '\n' ' ')"
echo "t,core_cpu_pct,sys_busy_pct,softirq_pct,rss_kb,anon_kb,file_kb,threads,pss_kb,memavail_kb,shmem_kb,slab_kb,sunreclaim_kb,kstack_kb,pagetables_kb,conntrack"
p0=$(st); set -- $(cpu); t0=$1 i0=$2 s0=$4
n=0; while [ $n -lt "$SECS" ]; do
	sleep 1; n=$((n+1))
	p1=$(st); set -- $(cpu); t1=$1 i1=$2 s1=$4
	dt=$((t1-t0)); [ $dt -gt 0 ] || dt=1
	core=$(awk -v a=$p0 -v b=$p1 -v d=$dt -v n=$NCPU 'BEGIN{printf "%.1f", (b-a)*100*n/d}')
	busy=$(awk -v d=$dt -v i=$((i1-i0)) 'BEGIN{printf "%.1f", (d-i)*100/d}')
	sirq=$(awk -v d=$dt -v s=$((s1-s0)) 'BEGIN{printf "%.1f", s*100/d}')
	set -- $(ps1); r=$1 a=$2 f=$3 th=$4 pss=${5:-}
	set -- $(mi)
	echo "$n,$core,$busy,$sirq,$r,$a,$f,$th,$pss,$1,$2,$3,$4,$5,$6,$(nconn)"
	p0=$p1 t0=$t1 i0=$i1 s0=$s1
done
