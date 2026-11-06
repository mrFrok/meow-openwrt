# meow-openwrt

A mihomo-compatible proxy core for OpenWrt routers, written in Rust and sized for
stock 16 MB flash. It reads the same `config.yaml` and serves the same REST API
as mihomo, so clashwrt, OpenClash, nikki and the usual web dashboards run it
without changes.

meow-openwrt is an unofficial fork of [meow-rs](https://github.com/meow-rs/meow-rs).
It is early work: the router builds run daily on one aarch64 router and are
checked under qemu on MIPS. Reports from other hardware are welcome.

[Русская версия](README.ru.md)

## Profiles

One package per profile. They conflict with each other, so install one.

| Package | What it carries | aarch64 | armv7 | mipsel | mips |
|---|---|---|---|---|---|
| `meow-tiny` | Trojan, VLESS (Vision, Reality), Hysteria2, mux, DoH/DoT, mixed, TProxy and TUN inbounds | 6.8 MB | 7.5 MB | 9.0 MB | 8.5 MB |
| `meow` | tiny plus Shadowsocks, AnyTLS, VMess, ShadowTLS | 7.2 MB | 8.0 MB | 9.7 MB | 9.1 MB |
| `meow-full` | everything meow-rs ships, logbook and traffic ledger included | 10.2 MB | 11.7 MB | 14.8 MB | 14.2 MB |

Sizes are of the stripped static binary. Flash takes less than that: squashfs
stores `meow-tiny` in about 2.6 MB, JFFS2 in 4.4 MB, UBIFS with zlib in about
5.5 MB.

Shadowsocks nodes are left out of `meow-tiny` to save space. When a
subscription mixes protocols, those nodes load as unavailable and the rest
work.

## Architectures

| OpenWrt arch | Example devices | TLS and QUIC |
|---|---|---|
| `aarch64_cortex-a53`, `aarch64_generic`, `-a72`, `-a76` | MT7981/MT7986/MT7988 (Filogic), RK3328/RK3568/RK3588 | BoringSSL, quiche |
| `arm_cortex-a7_neon-vfpv4` | IPQ40xx | BoringSSL, quiche |
| `mipsel_24kc` | MT7621 | BoringSSL, quiche |
| `mips_24kc` | ath79 (big-endian) | rustls, quinn |

BoringSSL only builds for little-endian targets, so big-endian MIPS uses rustls
and quinn instead. On that target ECH, restls and JLS are unavailable and the
TLS fingerprint setting is ignored with a warning.

## Install

Download the package for your profile and architecture from
[Releases](../../releases), copy it to the router and install it.

```sh
# OpenWrt 25.x (apk)
apk add --allow-untrusted /tmp/meow-tiny_*.apk

# OpenWrt 22.03 to 24.10 (opkg)
opkg install /tmp/meow-tiny_*.ipk
```

The binary is fully static, so one package installs on any OpenWrt release
of its architecture, 22.03 and newer. `tun.auto-redirect` and `dns.nftset`
need nftables (fw4, 22.03+); on iptables firmware the TProxy, redirect and
TUN modes still work.

The package installs one file, `/usr/bin/meow`. Check that it reads your
current config before switching anything:

```sh
meow -d /etc/mihomo -f /etc/mihomo/config.yaml -t
```

### With ClashWrt

[ClashWrt](https://github.com/mrFrok/clashwrt) is a LuCI front-end for
transparent proxying on OpenWrt, by the author of this fork. It has a setup
wizard and four interception modes, one of which works on every kernel, and
it shows when traffic is captured but goes nowhere. It is the front-end meow
is tested under every day.

```sh
wget -qO- https://raw.githubusercontent.com/mrFrok/clashwrt/main/install.sh | sh
```

### Running it under a front-end

Front-ends start the core by path. Point their core setting at `/usr/bin/meow`,
or try it in place of mihomo for one boot with a bind mount. A reboot or
`umount` puts mihomo back.

```sh
/etc/init.d/mihomo stop
mount --bind /usr/bin/meow /usr/bin/mihomo
/etc/init.d/mihomo start
```

`meow -v` reports `Meow Meta`, and meow answers the flags front-ends pass
(`-d`, `-f`, `-t`, `-v`).

## What the fork adds

- **`tun.auto-redirect`**. One config key installs the capture rules: its own
  `inet meow` nftables table, policy routing, a DNS redirect and an fw4
  include. Narrow it with `route-address-set` and `route-exclude-address-set`.
  The sets follow rule-set refreshes, the table comes back if something
  removes it, and meow cleans everything up on exit. See [docs/tun.md](docs/tun.md).
- **WireGuard and AmneziaWG nodes** (`type: wireguard`, with
  `amnezia-wg-option` for AmneziaWG) run on the kernel module: install
  `kmod-wireguard` or `kmod-amneziawg`, and meow creates and configures the
  link itself the first time a node is used. The kernel does the crypto, so
  it is fast on MIPS and adds almost nothing to the binary. `reserved` (some
  WARP setups) is not supported by the module.
- **`dns.nftset`**. meow puts the addresses of domains that go to a proxy into
  an nftables set, so the firewall captures only that traffic. 80 000 domains
  take about 1.2 MB in meow, against about 28 MB as dnsmasq lists. See
  [docs/nftset.md](docs/nftset.md).
- **mihomo behaviour where meow-rs differed**: `tproxy-port` is a real TPROXY
  listener with UDP, `RULE-SET` works inside `AND`/`OR`/`NOT`, proxy providers
  run their health checks, and REST API answers were compared against mihomo
  with [`api-compare.py`](openwrt-fork/bench/api-compare.py).
- **Small-memory defaults**: `max-connections` scales with RAM (256 on 64 MB),
  rule sets stream from disk into compact tries, and idle memory is about
  5 MB.
- **Big-endian MIPS**, with the alternative TLS and QUIC stack described above.

## Measurements

Banana Pi BPI-R4 (MT7988A, aarch64), static musl builds without PGO, measured
on 2026-11-06. Throughput is a single download through meow to a mihomo server
on the router itself, so it shows meow's cost and not the link speed.

| Profile | DIRECT TCP | Hysteria2 | VLESS + Reality | Idle memory, live config | Peak RSS |
|---|---|---|---|---|---|
| tiny | 5690 Mbit/s | 306 Mbit/s | 2020 Mbit/s | 5.1 MB | 14.1 MB |
| lightweight | 5450 Mbit/s | 312 Mbit/s | 2030 Mbit/s | 5.1 MB | 14.0 MB |
| full | 5660 Mbit/s | 323 Mbit/s | 2000 Mbit/s | 6.0 MB | 16.2 MB |

Method and the full size history are in
[openwrt-fork/BENCHMARKS.md](openwrt-fork/BENCHMARKS.md).

## Known gaps

- XHTTP supports only `mode: stream-one`. Nodes with `packet-up` or
  `stream-up` load as unavailable.
- Config keys meow does not implement, such as `tun.auto-detect-interface`,
  are logged as warnings and ignored.
- No GUI of its own. Use a front-end or a dashboard on `external-controller`.

## Build

The OpenWrt SDK package is in [openwrt/package/meow](openwrt/package/meow).
Link it into an SDK's `package/` directory and build one variant:

```sh
ln -s "$PWD/openwrt/package/meow" ~/openwrt-sdk/package/meow
make -C ~/openwrt-sdk package/meow/compile V=s
```

The release workflow builds without the SDK's Rust and uses the scripts in
[openwrt-fork/bench](openwrt-fork/bench) (`pgo-gen.sh`, `mips-be-build.sh`)
with a pinned rustup toolchain. The roadmap and its status are in
[openwrt-fork/PLAN.md](openwrt-fork/PLAN.md).

## Help with hardware

If you have a router other than an aarch64 one, run the read-only probe and
attach its output to an issue. It changes nothing and prints no addresses,
MACs or keys.

```sh
wget -O /tmp/hwprobe.sh https://raw.githubusercontent.com/mrFrok/meow-openwrt/main/openwrt-fork/bench/hwprobe.sh
sh /tmp/hwprobe.sh > /tmp/probe.txt
```

## License

Changes in this fork are GPL-3.0. Code inherited from meow-rs stays available
under MIT. See [NOTICE-FORK.md](NOTICE-FORK.md). The upstream README with the
full protocol and config reference is kept as
[docs/meow-rs-README.md](docs/meow-rs-README.md).

This fork is not affiliated with or endorsed by the meow-rs authors.
