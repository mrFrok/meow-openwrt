# meow-openwrt benchmarks

## Binary size, mipsel_24kc (SDK snapshot mt7621, rust 1.96, 2026-11-01)

Built with `bench/size-sdk.sh` (package `openwrt/package/meow`), stripped, opt-level z, LTO, cgu=1.
xz = `xz -9e` of the binary (≈ cost inside a squashfs firmware image).

| variant | bin | xz | apk |
|---|---|---|---|
| minimal, PIE, dyn libstdc++ (base) | 10.18 MB | 2.83 | 4.30 |
| minimal + static libstdc++ | 10.24 | 2.86 | 4.34 |
| minimal + static libstdc++ + `-Cforce-unwind-tables=no` | 9.85 | 2.77 | 4.20 |
| minimal + static libstdc++ + non-PIE (`-Crelocation-model=static`) | 8.80 | 2.53 | 3.82 |
| minimal + static libstdc++ + opt-level s | 11.29 | 3.01 | 4.71 |
| minimal, all of: static libstdc++, non-PIE, no unwind tables | 8.47 | 2.45 | 3.69 |
| tiny, same flags | 10.77 | 3.04 | 4.62 |
| tiny + build-std `-Cpanic=immediate-abort` | 9.79 | 2.79 | 4.24 |
| E1: + `-Zlocation-detail=none`, std `optimize_for_size` | 9.46 | 2.71 | 4.09 |
| E2: + `log-max-info` (debug!/trace! compiled out) | 9.27 | 2.68 | 4.04 |
| E3: + regex without `perf` | **9.00** | 2.63 | 3.94 |
| E4: + `-Zfmt-debug=shallow` (not adopted) | 8.94 | 2.60 | 3.91 |

Per-feature cost on top of E3 (`tiny` 9.00 MB):

| removed | bin | Δ |
|---|---|---|
| listener-tun | 8.15 | −854 KB |
| hysteria2 | 8.48 | −525 KB |
| vless (+vision, reality) | 8.54 | −462 KB |
| listener-tproxy | 8.94 | −66 KB |
| everything (no protocols/listeners/dns-server: core only) | 6.44 | −2.56 MB |

Host `cargo bloat --crates` (x86_64, tiny, .text): meow_config 887K, std 661K, boring_sys 437K,
meow_proxy 435K, netlink_packet_route 390K (two versions: 0.25 via tun-rs/netconfig-rs, 0.33 via
meow-listener/route_manager), regex_automata 306K, meow_dns 302K, meow_api 279K, meow_transport 267K,
serde_saphyr 241K, axum 234K, h2 126K, clap 112K.

### Package profiles (`openwrt/package/meow` variants, all knobs on; jffs2 = `mkfs.jffs2 -x zlib` i.e. lzma, as OpenWrt kernels)

| step (tiny) | bin | xz | jffs2 |
|---|---|---|---|
| E3 + mux + dns-encrypted (new `tiny`) | 9.27 MB | — | 4.59 |
| + tun-rs without `address-management` on Linux (no netconfig-rs / 2nd netlink-packet-route), clap w/o color | 9.13 | 2.65 | 4.54 |
| + `policy_route.rs` frames netlink by hand, decodes only `RuleMessage`; Linux routes without route_manager | **8.68** | 2.56 | **4.38** |

`lightweight` (tiny + anytls, vmess, shadow-tls) at the 9.27 MB step: 9.60 MB, jffs2 4.78.

Current profiles (all three pass `meow -t` under qemu-mipsel):

| package | bin | xz | jffs2 |
|---|---|---|---|
| meow-tiny | 8.68 MB | 2.56 | 4.38 |
| meow (lightweight) | 9.07 | 2.66 | 4.56 |
| meow-full (no build-std, keeps panic messages) | 14.00 | 4.07 | 6.85 |
Live-checked in `unshare -rn` (x86 tiny): auto-route true (v4+v6 rules 9200–9205, table, addresses) and fake-ip
(main-table 198.18.0.0/16) install and fully clean up on SIGTERM.

## Release profiles on the BPI-R4 (2026-11-06)
aarch64 musl static, no PGO (`pgo-gen.sh PROFILE=… PGO_USE=none`). `autoredirect-test.sh` (5 phases) ALL PASS
for all three. tunbench SECS=8, mihomo server (`SRVBIN=/tmp/mihomo-srv`); memory: meowmem on the live config, idle 20 s.

| profile | size | DIRECT TCP ↓ | hy2 TCP ↓ | VLESS+Reality ↓ | bench anon peak | live idle anon (end) | live HWM |
|---|---|---|---|---|---|---|---|
| tiny | 8.48 MB | 5690 Mbit/s, 0.02 %/Mbit | 306, 0.39 | 2020, 0.08 | 1.3 MB | 5.1 MB | 14.1 MB |
| lightweight | 9.08 MB | 5450, 0.02 | 312, 0.44 | 2030, 0.08 | 1.3 MB | 5.1 MB | 14.0 MB |
| full | 12.18 MB | 5660, 0.02 | 323, 0.44 | 2000, 0.08 | 4.6 MB | 6.0 MB | 16.2 MB |

Speeds are the same across profiles; `full` costs +3.7 MB on flash and ~3 MB anon (logbook/traffic-ledger SQLite).
