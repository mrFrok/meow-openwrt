#!/bin/sh
# meow-openwrt — read-only hardware/kernel probe for OpenWrt routers.
# Changes nothing, installs nothing, sends nothing anywhere. Prints a report to stdout.
# Addresses, MACs, hostnames and keys are not printed.
# Usage: sh hwprobe.sh > probe.txt   (then share probe.txt)
LC_ALL=C; export LC_ALL
sec() { echo; echo "=== $*"; }
mask() { sed -E 's/([0-9]{1,3}\.){3}[0-9]{1,3}/x.x.x.x/g; s/([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}/xx:xx:xx:xx:xx:xx/g; s/[0-9a-fA-F]{0,4}(:[0-9a-fA-F]{0,4}){2,7}(\/[0-9]+)?/x::x/g'; }
have() { command -v "$1" >/dev/null 2>&1; }

sec board
if have ubus; then ubus call system board 2>/dev/null | grep -E '"(kernel|model|board_name|system|release|distribution|version|revision|target|description)"' | grep -v hostname; fi
uname -m -r -v
sec cpu
grep -E '^(model name|cpu model|system type|isa|ASEs implemented|Features|CPU implementer|CPU part|BogoMIPS|Hardware)' /proc/cpuinfo | sort | uniq -c | head -12
echo "cores: $(grep -c '^processor' /proc/cpuinfo)"
[ -r /sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq ] && echo "max_freq_khz $(cat /sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq)"
grep -qiE 'fpu|vfp|neon|asimd' /proc/cpuinfo && echo "fpu-flags: present" || echo "fpu-flags: none listed"
sec memory
grep -E '^(MemTotal|MemFree|MemAvailable|Buffers|Cached|Shmem|SwapTotal|Slab|SUnreclaim|KernelStack|PageTables|CommitLimit|Committed_AS)' /proc/meminfo
[ -e /sys/block/zram0 ] && echo "zram0 present"
cat /proc/sys/vm/overcommit_memory 2>/dev/null | sed 's/^/overcommit_memory /'
sec storage
df -h / /overlay /tmp 2>/dev/null
mount | grep -E ' / | /overlay | /rom | /tmp ' | awk '{print $1, $3, $5}'
sec packages
if have apk; then echo "pm: apk"; apk list --installed 2>/dev/null | grep -oE '^(kmod-[^ ]+|nftables[^ ]*|firewall4[^ ]*|dnsmasq[^ ]*|mihomo[^ ]*|luci-app-(clashwrt|ssclash|passwall[^ ]*|podkop)[^ ]*|bpftool[^ ]*|tc-[^ ]+|ip-full[^ ]*|iperf3[^ ]*)' | sort
elif have opkg; then echo "pm: opkg"; opkg list-installed 2>/dev/null | grep -E '^(kmod-|nftables|firewall4|dnsmasq|mihomo|luci-app-(clashwrt|ssclash|passwall|podkop)|bpftool|tc-|ip-full|iperf3)' | awk '{print $1, $3}'; fi
sec kernel-config
if [ -r /proc/config.gz ]; then zcat /proc/config.gz | grep -E '^(# )?CONFIG_(BPF_SYSCALL|BPF_JIT|BPF_JIT_ALWAYS_ON|NET_SOCK_MSG|BPF_STREAM_PARSER|CGROUPS|CGROUP_BPF|DEBUG_INFO_BTF|XDP_SOCKETS|NET_CLS_BPF|NET_ACT_BPF|NF_FLOW_TABLE|NFT_FLOW_OFFLOAD|NF_FLOW_TABLE_INET|NFT_TPROXY|NF_TPROXY_IPV4|TUN|IO_URING|TLS|NET_SCH_CAKE|NF_CONNTRACK_NETLINK)[ =]'; else echo "no /proc/config.gz (normal on OpenWrt); see modules below"; fi
echo "bpf_jit_enable=$(cat /proc/sys/net/core/bpf_jit_enable 2>/dev/null || echo n/a)"
echo "unprivileged_bpf_disabled=$(cat /proc/sys/kernel/unprivileged_bpf_disabled 2>/dev/null || echo n/a)"
[ -d /sys/fs/bpf ] && echo "bpffs dir present: $(mount | grep -c ' /sys/fs/bpf ') mounted"
[ -d /sys/fs/cgroup ] && echo "cgroup fs: $(mount | grep -E ' /sys/fs/cgroup' | awk '{print $5}' | tr '\n' ' ')"
grep -cE '' /proc/kallsyms >/dev/null 2>&1 && for s in sk_psock_start_verdict sock_map_update_elem nf_flow_offload_add tcp_bpf_update_proto io_uring_setup tls_sw_sendmsg; do grep -q " $s\$" /proc/kallsyms 2>/dev/null && echo "sym $s: yes" || echo "sym $s: no"; done
sec modules
lsmod 2>/dev/null | awk 'NR>1{print $1}' | grep -E '^(nf_|nft_|xt_|tun|cls_|act_|sch_|mtk|nf_flow|tls|ip6?table|nfnetlink)' | sort | tr '\n' ' '; echo
sec firewall
uci -q get firewall.@defaults[0].flow_offloading | sed 's/^/flow_offloading=/'
uci -q get firewall.@defaults[0].flow_offloading_hw | sed 's/^/flow_offloading_hw=/'
if have nft; then nft list flowtables 2>/dev/null | grep -E 'flowtable|hook|devices|flags' | mask; nft list tables 2>/dev/null; fi
sec network
for d in /sys/class/net/*; do [ -d "$d" ] || continue; n=${d##*/}; [ "$n" = lo ] && continue
	drv=$(basename "$(readlink "$d/device/driver" 2>/dev/null)" 2>/dev/null); dsa=$( [ -e "$d/dsa" ] && echo dsa )
	echo "$n driver=${drv:--} mtu=$(cat $d/mtu) ${dsa} speed=$(cat $d/speed 2>/dev/null || echo -)"
done | sed 's/^\(br-[a-z]*\|wan\|lan[0-9]*\|eth[0-9]*\|wl[a-z0-9-]*\|phy[0-9a-z-]*\|tun[0-9]*\|Meta\|utun\)/&/'
if have ethtool; then for n in $(ls /sys/class/net | grep -E '^eth[0-9]+$'); do echo "-- ethtool -k $n"; ethtool -k $n 2>/dev/null | grep -E '^(rx-checksumming|tx-checksumming|scatter-gather|tcp-segmentation-offload|generic-segmentation-offload|generic-receive-offload|hw-tc-offload|rx-gro-list|rx-udp-gro-forwarding):'; done; else echo "no ethtool"; fi
[ -d /sys/kernel/debug/mtk_ppe ] && echo "mtk_ppe debugfs present"
grep -lE 'mtk_ppe|ppe' /proc/interrupts >/dev/null 2>&1 && echo "ppe irq present"
sec proxy-core
for b in /usr/bin/mihomo /usr/bin/clash /etc/clashwrt/bin/mihomo /tmp/mihomo; do [ -x "$b" ] && echo "$b $(ls -l "$b" | awk '{print $5}') bytes ($("$b" -v 2>/dev/null | head -1 | cut -c1-80))"; done
for p in $(pidof mihomo clash 2>/dev/null); do echo "pid $p: $(grep -E '^(VmRSS|RssAnon|RssFile|Threads)' /proc/$p/status | tr -s ' \t' ' ' | tr '\n' ' ')"; tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -E '^(GOGC|GOMEMLIMIT|GOMAXPROCS|GODEBUG)='; done
echo; echo "=== end"
