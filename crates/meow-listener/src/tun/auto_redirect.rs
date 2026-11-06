//! `tun.auto-redirect` on Linux (meow-openwrt): mihomo's / sing-tun's
//! nftables layout, so a gateway needs no firewall scripting of its own.
//!
//! One table, `inet meow`, applied in a single `nft -f` transaction:
//!
//! - TCP — the host's own (`output`) and forwarded (`prerouting`) — is
//!   `redirect`ed to an internal listener that recovers the destination
//!   with `SO_ORIGINAL_DST`.
//! - UDP and ICMP get [`ROUTE_MARK`] (kept on the conntrack entry, so every
//!   packet of the flow follows) and policy routing sends that mark to the
//!   device (`policy_route::PolicyRoutes::setup_marked`).
//! - DNS from the LAN and the host (not to loopback) is `dnat`ed to the
//!   device's peer address, where `dns-hijack` answers it.
//! - meow's own sockets carry the bypass mark (the global `routing-mark`)
//!   and are never captured; neither are local destinations
//!   (`local4`: every address of the host's interfaces, loopback, the
//!   device's subnet). `route-address(-set)` limits capture to its
//!   destinations, `route-exclude-address(-set)` exempts some.
//!
//! IPv6 gets the same chains (`output6`, ...) when the device has an IPv6
//! address; otherwise IPv6 traffic is left alone.
//!
//! On OpenWrt, fw4's `input` / `forward` chains drop what its zones do
//! not accept, the device included; like sing-tun, an include in
//! `/etc/nftables.d` accepts traffic from and to the device, and fw4 is
//! reloaded to pick it up (and again to drop it on shutdown).

use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::process::{Command, Stdio};

use ipnet::{Ipv4Net, Ipv6Net};
use tracing::{debug, info, warn};

pub(super) const TABLE: &str = "meow";
/// Packets of captured UDP/ICMP flows: policy-routed into the device.
pub(super) const ROUTE_MARK: u32 = 0x2023;
/// meow's own sockets and flows decided not to be captured.
pub(super) const DEFAULT_BYPASS_MARK: u32 = meow_common::nftset::AUTO_REDIRECT_BYPASS_MARK;

const FW4_INCLUDE: &str = "/etc/nftables.d/0-meow-auto-redirect.nft";

/// Inclusive address range.
pub(super) type Range<A> = (A, A);

/// An address family's spelling in nftables.
pub(super) trait Family: Copy + Ord + std::fmt::Display {
    /// `4` / `6`: set and chain name suffix.
    const SUFFIX: &'static str;
    /// `ip` / `ip6`.
    const KW: &'static str;
    const NFPROTO: &'static str;
    const ICMP: &'static str;
    const TYPE: &'static str;
    /// `b` directly follows `a`.
    fn next_to(a: Self, b: Self) -> bool;
}

impl Family for Ipv4Addr {
    const SUFFIX: &'static str = "4";
    const KW: &'static str = "ip";
    const NFPROTO: &'static str = "ipv4";
    const ICMP: &'static str = "icmp";
    const TYPE: &'static str = "ipv4_addr";
    fn next_to(a: Self, b: Self) -> bool {
        u32::from(b) <= u32::from(a).saturating_add(1)
    }
}

impl Family for Ipv6Addr {
    const SUFFIX: &'static str = "6";
    const KW: &'static str = "ip6";
    const NFPROTO: &'static str = "ipv6";
    const ICMP: &'static str = "icmpv6";
    const TYPE: &'static str = "ipv6_addr";
    fn next_to(a: Self, b: Self) -> bool {
        u128::from(b) <= u128::from(a).saturating_add(1)
    }
}

/// One family's capture.
pub(super) struct Fam<A> {
    pub redirect_port: u16,
    /// DNS hijack target: the device's peer address.
    pub dns_target: Option<A>,
    /// `None`: capture every destination.
    pub route: Option<Vec<(A, A)>>,
    pub exclude: Vec<(A, A)>,
    /// Never captured: the host's subnets, loopback, the device's subnet
    /// (and for IPv6 link-local and multicast).
    pub local: Vec<(A, A)>,
}

/// What the ruleset is built from.
pub(super) struct Params {
    pub dev: String,
    pub bypass_mark: u32,
    /// `route-address-dns`: an empty timeout set (`route4_dns` /
    /// `route6_dns`) the DNS answer hook fills; captured like `route4`.
    pub dns_set: bool,
    pub v4: Fam<Ipv4Addr>,
    pub v6: Option<Fam<Ipv6Addr>>,
}

/// The installed table (and fw4 include); dropping it removes both.
pub(super) struct NftRedirect {
    fw4: bool,
}

impl NftRedirect {
    pub(super) fn setup(p: &Params) -> io::Result<Self> {
        nft(&ruleset(p))?;
        // The DNS set starts empty: the answer hook must add everything anew.
        meow_common::nftset::bump_epoch();
        info!(
            "tun auto-redirect: nftables table inet {TABLE} installed (TCP -> :{}{}, UDP via '{}')",
            p.v4.redirect_port,
            p.v6.as_ref()
                .map_or(String::new(), |f| format!(" / [::]:{}", f.redirect_port)),
            p.dev
        );
        let fw4 = fw4_accept(&p.dev);
        Ok(Self { fw4 })
    }
}

impl Drop for NftRedirect {
    fn drop(&mut self) {
        if let Err(e) = nft(&format!("table inet {TABLE}\ndelete table inet {TABLE}\n")) {
            warn!("tun auto-redirect: cannot remove table inet {TABLE}: {e}");
        }
        if self.fw4 {
            match std::fs::remove_file(FW4_INCLUDE) {
                Ok(()) => fw4_reload(),
                Err(e) => debug!("tun auto-redirect: {FW4_INCLUDE}: {e}"),
            }
        }
    }
}

/// Sort and merge inclusive ranges: nftables refuses overlapping interval
/// elements.
pub(super) fn coalesce<A: Family>(mut r: Vec<(A, A)>) -> Vec<(A, A)> {
    r.sort_unstable();
    let mut out: Vec<(A, A)> = Vec::with_capacity(r.len());
    for (s, e) in r {
        if let Some(last) = out.last_mut() {
            if A::next_to(last.1, s) {
                if e > last.1 {
                    last.1 = e;
                }
                continue;
            }
        }
        out.push((s, e));
    }
    out
}

pub(super) fn net_range(n: Ipv4Net) -> (Ipv4Addr, Ipv4Addr) {
    (n.network(), n.broadcast())
}

pub(super) fn net6_range(n: Ipv6Net) -> (Ipv6Addr, Ipv6Addr) {
    (n.network(), n.broadcast())
}

fn elements<A: Family>(r: &[(A, A)]) -> String {
    let mut s = String::with_capacity(r.len() * 32);
    for (i, (a, b)) in r.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        if a == b {
            let _ = write!(s, "{a}");
        } else {
            let _ = write!(s, "{a}-{b}");
        }
    }
    s
}

fn set<A: Family>(name: &str, r: &[(A, A)]) -> String {
    let mut s = format!(
        "\tset {name}{} {{\n\t\ttype {}\n\t\tflags interval\n",
        A::SUFFIX,
        A::TYPE
    );
    if !r.is_empty() {
        let _ = writeln!(s, "\t\telements = {{ {} }}", elements(r));
    }
    s.push_str("\t}\n");
    s
}

/// One family's sets and chains (each chain ignores the other family).
fn family<A: Family>(dev: &str, bypass: u32, dns_set: bool, f: &Fam<A>) -> String {
    let bypass = format!("0x{bypass:08x}");
    let route = format!("0x{ROUTE_MARK:08x}");
    let (sx, kw, nf, icmp, port) = (A::SUFFIX, A::KW, A::NFPROTO, A::ICMP, f.redirect_port);
    let mut s = set("local", &f.local);
    s.push_str(&set("exclude", &f.exclude));
    if let Some(r) = &f.route {
        s.push_str(&set("route", r));
    }
    if dns_set {
        let _ = write!(
            s,
            "\tset route{sx}_dns {{\n\t\ttype {}\n\t\tflags timeout\n\t}}\n",
            A::TYPE
        );
    }
    // Destination filters shared by every chain: local, excluded and (when
    // narrowed) not-routed destinations are left alone.
    let mut filter =
        format!("\t\t{kw} daddr @local{sx} return\n\t\t{kw} daddr @exclude{sx} return\n");
    match (f.route.is_some(), dns_set) {
        (true, true) => {
            let _ = writeln!(
                filter,
                "\t\t{kw} daddr != @route{sx} {kw} daddr != @route{sx}_dns return"
            );
        }
        (true, false) => {
            let _ = writeln!(filter, "\t\t{kw} daddr != @route{sx} return");
        }
        (false, true) => {
            let _ = writeln!(filter, "\t\t{kw} daddr != @route{sx}_dns return");
        }
        (false, false) => {}
    }
    let dns_out = f.dns_target.map_or(String::new(), |t| {
        format!("\t\toifname != \"lo\" udp dport 53 dnat {kw} to {t}\n")
    });
    let dns_pre = f.dns_target.map_or(String::new(), |t| {
        format!("\t\t{kw} saddr @local{sx} udp dport 53 dnat {kw} to {t}\n")
    });
    let _ = write!(
        s,
        "\tchain output{sx} {{
\t\ttype nat hook output priority mangle; policy accept;
\t\tmeta nfproto != {nf} return
\t\tmeta mark {bypass} return
{dns_out}{filter}\t\tmeta l4proto tcp redirect to :{port}
\t}}
\tchain output_udp{sx} {{
\t\ttype route hook output priority mangle; policy accept;
\t\tmeta nfproto != {nf} return
\t\tmeta l4proto != {{ {icmp}, udp }} return
\t\tmeta mark {bypass} return
\t\tct mark {bypass} return
\t\toifname \"lo\" return
{filter}\t\tmeta mark set {route} ct mark set meta mark
\t}}
\tchain prerouting{sx} {{
\t\ttype nat hook prerouting priority dstnat + 1; policy accept;
\t\tmeta nfproto != {nf} return
\t\tiifname \"{dev}\" return
{dns_pre}{filter}\t\tmeta l4proto tcp redirect to :{port}
\t\tmeta l4proto {{ {icmp}, udp }} meta mark set {route} ct mark set meta mark
\t}}
\tchain prerouting_udp{sx} {{
\t\ttype filter hook prerouting priority dstnat + 2; policy accept;
\t\tmeta nfproto != {nf} return
\t\tmeta l4proto != {{ {icmp}, udp }} return
\t\tiifname \"{dev}\" return
\t\tct mark {route} meta mark set ct mark return
\t\tmeta mark set {bypass} ct mark set meta mark
\t}}
"
    );
    s
}

/// The whole table, preceded by a create-then-delete so a stale one from a
/// crashed run is replaced atomically.
pub(super) fn ruleset(p: &Params) -> String {
    let mut s = format!("table inet {TABLE}\ndelete table inet {TABLE}\ntable inet {TABLE} {{\n");
    s.push_str(&family(&p.dev, p.bypass_mark, p.dns_set, &p.v4));
    if let Some(v6) = &p.v6 {
        s.push_str(&family(&p.dev, p.bypass_mark, p.dns_set, v6));
    }
    s.push_str("}\n");
    s
}

/// Flush and refill `route` (when narrowed) and `exclude` of one family.
pub(super) fn sets_update<A: Family>(s: &mut String, route: Option<&[(A, A)]>, exclude: &[(A, A)]) {
    let mut one = |name: &str, r: &[(A, A)]| {
        let _ = writeln!(s, "flush set inet {TABLE} {name}{}", A::SUFFIX);
        if !r.is_empty() {
            let _ = writeln!(
                s,
                "add element inet {TABLE} {name}{} {{ {} }}",
                A::SUFFIX,
                elements(r)
            );
        }
    };
    if let Some(r) = route {
        one("route", r);
    }
    one("exclude", exclude);
}

pub(super) fn apply(script: &str) -> io::Result<()> {
    nft(script)
}

fn nft(script: &str) -> io::Result<()> {
    let mut child = Command::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(script.as_bytes())?;
    let out = child.wait_with_output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "nft: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// The host's interface subnets, loopback and the device's subnet (IPv4),
/// plus link-local and multicast (IPv6): destinations never captured.
pub(super) fn local_ranges(
    tun4: Ipv4Net,
    tun6: Option<Ipv6Net>,
) -> (Vec<Range<Ipv4Addr>>, Vec<Range<Ipv6Addr>>) {
    let mut r4 = vec![
        net_range(Ipv4Net::new_assert(Ipv4Addr::new(127, 0, 0, 0), 8)),
        net_range(tun4.trunc()),
    ];
    let mut r6 = vec![
        (Ipv6Addr::LOCALHOST, Ipv6Addr::LOCALHOST),
        net6_range(Ipv6Net::new_assert(
            Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0),
            10,
        )),
        net6_range(Ipv6Net::new_assert(
            Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0),
            8,
        )),
    ];
    r6.extend(tun6.map(|n| net6_range(n.trunc())));
    // SAFETY: getifaddrs/freeifaddrs pair; entries are read only while the
    // list is alive, and each sockaddr only as the type its family names.
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&raw mut head) == 0 {
            let mut cur = head;
            while !cur.is_null() {
                let ifa = &*cur;
                if !ifa.ifa_addr.is_null() && !ifa.ifa_netmask.is_null() {
                    match i32::from((*ifa.ifa_addr).sa_family) {
                        libc::AF_INET => {
                            let a = &*ifa.ifa_addr.cast::<libc::sockaddr_in>();
                            let m = &*ifa.ifa_netmask.cast::<libc::sockaddr_in>();
                            let addr = Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr));
                            let mask = u32::from_be(m.sin_addr.s_addr);
                            let prefix = u8::try_from(mask.leading_ones()).unwrap_or(32);
                            if let Ok(net) = Ipv4Net::new(addr, prefix) {
                                r4.push(net_range(net.trunc()));
                            }
                        }
                        libc::AF_INET6 => {
                            let a = &*ifa.ifa_addr.cast::<libc::sockaddr_in6>();
                            let m = &*ifa.ifa_netmask.cast::<libc::sockaddr_in6>();
                            let addr = Ipv6Addr::from(a.sin6_addr.s6_addr);
                            let mask = u128::from_be_bytes(m.sin6_addr.s6_addr);
                            let prefix = u8::try_from(mask.leading_ones()).unwrap_or(128);
                            if let Ok(net) = Ipv6Net::new(addr, prefix) {
                                r6.push(net6_range(net.trunc()));
                            }
                        }
                        _ => {}
                    }
                }
                cur = ifa.ifa_next;
            }
            libc::freeifaddrs(head);
        }
    }
    (coalesce(r4), coalesce(r6))
}

/// On OpenWrt (fw4 present), accept traffic from and to the device in
/// fw4's filter chains. Returns whether the include was installed.
fn fw4_accept(dev: &str) -> bool {
    let fw4 = ["/sbin/fw4", "/usr/sbin/fw4"]
        .into_iter()
        .any(|p| std::path::Path::new(p).exists());
    if !fw4 || !std::path::Path::new("/etc/nftables.d").is_dir() {
        return false;
    }
    // fw4 must own this network namespace's firewall (a meow in a netns of
    // its own leaves the host's fw4 alone).
    let fw4_table = Command::new("nft")
        .args(["list", "table", "inet", "fw4"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !fw4_table {
        return false;
    }
    let rules = format!(
        "chain input {{
\ttype filter hook input priority filter; policy accept;
\tiifname \"{dev}\" counter accept comment \"!meow: accept traffic from tun\"
}}
chain forward {{
\ttype filter hook forward priority filter; policy accept;
\tiifname \"{dev}\" counter accept comment \"!meow: accept traffic from tun\"
\toifname \"{dev}\" counter accept comment \"!meow: accept traffic to tun\"
}}
"
    );
    if std::fs::read_to_string(FW4_INCLUDE).ok().as_deref() != Some(rules.as_str()) {
        if let Err(e) = std::fs::write(FW4_INCLUDE, &rules) {
            warn!("tun auto-redirect: cannot write {FW4_INCLUDE}: {e}; fw4 may drop '{dev}'");
            return false;
        }
    }
    fw4_reload();
    true
}

fn fw4_reload() {
    match Command::new("fw4").arg("reload").output() {
        Ok(o) if o.status.success() => debug!("tun auto-redirect: fw4 reloaded"),
        Ok(o) => warn!(
            "tun auto-redirect: fw4 reload failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => warn!("tun auto-redirect: fw4 reload: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn fam4(
        route: Option<Vec<(Ipv4Addr, Ipv4Addr)>>,
        exclude: Vec<(Ipv4Addr, Ipv4Addr)>,
        dns: Option<Ipv4Addr>,
    ) -> Fam<Ipv4Addr> {
        Fam {
            redirect_port: 7895,
            dns_target: dns,
            route,
            exclude,
            local: vec![(a("127.0.0.0"), a("127.255.255.255"))],
        }
    }

    #[test]
    fn sets_update_flushes_and_refills() {
        let mut s = String::new();
        sets_update(&mut s, Some(&[(a("1.0.0.0"), a("1.0.0.255"))]), &[]);
        assert_eq!(
            s,
            "flush set inet meow route4\nadd element inet meow route4 { 1.0.0.0-1.0.0.255 }\n\
             flush set inet meow exclude4\n"
        );
        let mut s = String::new();
        sets_update::<Ipv6Addr>(&mut s, None, &[]);
        assert_eq!(s, "flush set inet meow exclude6\n");
    }

    #[test]
    fn coalesce_merges_overlaps_and_neighbours() {
        let r = coalesce(vec![
            (a("10.0.0.0"), a("10.0.0.255")),
            (a("10.0.1.0"), a("10.0.1.255")),
            (a("10.0.0.10"), a("10.0.0.20")),
            (a("192.168.0.0"), a("192.168.0.255")),
        ]);
        assert_eq!(
            r,
            vec![
                (a("10.0.0.0"), a("10.0.1.255")),
                (a("192.168.0.0"), a("192.168.0.255"))
            ]
        );
        let six = |s: &str| s.parse::<Ipv6Addr>().unwrap();
        assert_eq!(
            coalesce(vec![(six("::2"), six("::3")), (six("::1"), six("::1"))]),
            vec![(six("::1"), six("::3"))]
        );
    }

    #[test]
    fn ruleset_narrowed_has_route_set_and_filters() {
        let p = Params {
            dev: "Meta".into(),
            bypass_mark: DEFAULT_BYPASS_MARK,
            dns_set: true,
            v4: fam4(
                Some(vec![(a("91.108.4.0"), a("91.108.7.255"))]),
                Vec::new(),
                Some(a("198.18.0.2")),
            ),
            v6: None,
        };
        let s = ruleset(&p);
        assert!(s.starts_with("table inet meow\ndelete table inet meow\n"));
        assert!(s.contains("elements = { 91.108.4.0-91.108.7.255 }"));
        assert!(s.contains("ip daddr != @route4 ip daddr != @route4_dns return"));
        assert!(s.contains("set route4_dns {\n\t\ttype ipv4_addr\n\t\tflags timeout"));
        assert!(s.contains("ip daddr @exclude4 return"));
        assert!(s.contains("redirect to :7895"));
        assert!(s.contains("udp dport 53 dnat ip to 198.18.0.2"));
        assert!(s.contains("meta mark 0x00002024 return"));
        assert!(!s.contains("output6"));
    }

    #[test]
    fn ruleset_capture_all_has_no_route_set() {
        let p = Params {
            dev: "Meta".into(),
            bypass_mark: 7,
            dns_set: false,
            v4: fam4(None, vec![(a("1.1.1.1"), a("1.1.1.1"))], None),
            v6: None,
        };
        let s = ruleset(&p);
        assert!(!s.contains("@route4"));
        assert!(s.contains("elements = { 1.1.1.1 }"));
        assert!(s.contains("ip daddr @exclude4 return"));
        assert!(!s.contains("dport 53"));
    }

    #[test]
    fn ruleset_v6_gets_its_own_chains() {
        let six = |s: &str| s.parse::<Ipv6Addr>().unwrap();
        let p = Params {
            dev: "Meta".into(),
            bypass_mark: DEFAULT_BYPASS_MARK,
            dns_set: true,
            v4: fam4(None, Vec::new(), None),
            v6: Some(Fam {
                redirect_port: 7896,
                dns_target: Some(six("fdfe:dcba:9876::2")),
                route: None,
                exclude: Vec::new(),
                local: vec![(
                    six("fe80::"),
                    six("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
                )],
            }),
        };
        let s = ruleset(&p);
        assert!(s.contains("chain output6 {"));
        assert!(s.contains("meta nfproto != ipv6 return"));
        assert!(s.contains("meta l4proto != { icmpv6, udp } return"));
        assert!(s.contains("ip6 daddr != @route6_dns return"));
        assert!(s.contains("udp dport 53 dnat ip6 to fdfe:dcba:9876::2"));
        assert!(s.contains("redirect to :7896"));
        assert!(s.contains(
            "set local6 {\n\t\ttype ipv6_addr\n\t\tflags interval\n\t\telements = { fe80::-febf:"
        ));
    }
}
