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
//! IPv4 only for now: IPv6 traffic is left alone.
//!
//! On OpenWrt, fw4's `input` / `forward` chains drop what its zones do
//! not accept, the device included; like sing-tun, an include in
//! `/etc/nftables.d` accepts traffic from and to the device, and fw4 is
//! reloaded to pick it up (and again to drop it on shutdown).

use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::net::Ipv4Addr;
use std::process::{Command, Stdio};

use ipnet::Ipv4Net;
use tracing::{debug, info, warn};

pub(super) const TABLE: &str = "meow";
/// Packets of captured UDP/ICMP flows: policy-routed into the device.
pub(super) const ROUTE_MARK: u32 = 0x2023;
/// meow's own sockets and flows decided not to be captured.
pub(super) const DEFAULT_BYPASS_MARK: u32 = meow_common::nftset::AUTO_REDIRECT_BYPASS_MARK;

const FW4_INCLUDE: &str = "/etc/nftables.d/0-meow-auto-redirect.nft";

/// What the ruleset is built from.
pub(super) struct Params {
    pub dev: String,
    pub redirect_port: u16,
    pub bypass_mark: u32,
    /// The device's subnet (local) and its peer address (DNS hijack target).
    pub tun_net: Ipv4Net,
    pub dns_target: Option<Ipv4Addr>,
    /// `None`: capture every destination.
    pub route: Option<Vec<(Ipv4Addr, Ipv4Addr)>>,
    pub exclude: Vec<(Ipv4Addr, Ipv4Addr)>,
    /// `route-address-dns`: an empty timeout set (`route4_dns`) the DNS
    /// answer hook fills; captured like `route4`.
    pub dns_set: bool,
}

/// The installed table (and fw4 include); dropping it removes both.
pub(super) struct NftRedirect {
    fw4: bool,
}

impl NftRedirect {
    pub(super) fn setup(p: &Params) -> io::Result<Self> {
        let local = local_ranges(p.tun_net);
        let script = ruleset(p, &local);
        nft(&script)?;
        // The DNS set starts empty: the answer hook must add everything anew.
        meow_common::nftset::bump_epoch();
        info!(
            "tun auto-redirect: nftables table inet {TABLE} installed (TCP -> :{}, UDP via '{}')",
            p.redirect_port, p.dev
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
pub(super) fn coalesce(mut r: Vec<(Ipv4Addr, Ipv4Addr)>) -> Vec<(Ipv4Addr, Ipv4Addr)> {
    r.sort_unstable();
    let mut out: Vec<(Ipv4Addr, Ipv4Addr)> = Vec::with_capacity(r.len());
    for (s, e) in r {
        if let Some(last) = out.last_mut() {
            if u32::from(s) <= u32::from(last.1).saturating_add(1) {
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

fn elements(r: &[(Ipv4Addr, Ipv4Addr)]) -> String {
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

fn set(name: &str, r: &[(Ipv4Addr, Ipv4Addr)]) -> String {
    let mut s = format!("\tset {name} {{\n\t\ttype ipv4_addr\n\t\tflags interval\n");
    if !r.is_empty() {
        let _ = writeln!(s, "\t\telements = {{ {} }}", elements(r));
    }
    s.push_str("\t}\n");
    s
}

/// The whole table, preceded by a create-then-delete so a stale one from a
/// crashed run is replaced atomically.
pub(super) fn ruleset(p: &Params, local: &[(Ipv4Addr, Ipv4Addr)]) -> String {
    let bypass = format!("0x{:08x}", p.bypass_mark);
    let route = format!("0x{ROUTE_MARK:08x}");
    let dev = &p.dev;
    let port = p.redirect_port;
    // Destination filters shared by every chain: local, excluded and (when
    // narrowed) not-routed destinations are left alone.
    let mut filter = String::from("\t\tip daddr @local4 return\n\t\tip daddr @exclude4 return\n");
    match (p.route.is_some(), p.dns_set) {
        (true, true) => filter.push_str("\t\tip daddr != @route4 ip daddr != @route4_dns return\n"),
        (true, false) => filter.push_str("\t\tip daddr != @route4 return\n"),
        (false, true) => filter.push_str("\t\tip daddr != @route4_dns return\n"),
        (false, false) => {}
    }
    let dns_out = p.dns_target.map_or(String::new(), |t| {
        format!("\t\tmeta nfproto ipv4 oifname != \"lo\" udp dport 53 dnat ip to {t}\n")
    });
    let dns_pre = p.dns_target.map_or(String::new(), |t| {
        format!("\t\tip saddr @local4 udp dport 53 dnat ip to {t}\n")
    });

    let mut s = format!("table inet {TABLE}\ndelete table inet {TABLE}\ntable inet {TABLE} {{\n");
    s.push_str(&set("local4", local));
    s.push_str(&set("exclude4", &p.exclude));
    if let Some(r) = &p.route {
        s.push_str(&set("route4", r));
    }
    if p.dns_set {
        s.push_str("\tset route4_dns {\n\t\ttype ipv4_addr\n\t\tflags timeout\n\t}\n");
    }
    let _ = write!(
        s,
        "\tchain output {{
\t\ttype nat hook output priority mangle; policy accept;
\t\tmeta mark {bypass} return
{dns_out}{filter}\t\tmeta nfproto ipv4 meta l4proto tcp redirect to :{port}
\t}}
\tchain output_udp {{
\t\ttype route hook output priority mangle; policy accept;
\t\tmeta nfproto != ipv4 return
\t\tmeta l4proto != {{ icmp, udp }} return
\t\tmeta mark {bypass} return
\t\tct mark {bypass} return
\t\toifname \"lo\" return
{filter}\t\tmeta mark set {route} ct mark set meta mark
\t}}
\tchain prerouting {{
\t\ttype nat hook prerouting priority dstnat + 1; policy accept;
\t\tiifname \"{dev}\" return
\t\tmeta nfproto != ipv4 return
{dns_pre}{filter}\t\tmeta l4proto tcp redirect to :{port}
\t\tmeta l4proto {{ icmp, udp }} meta mark set {route} ct mark set meta mark
\t}}
\tchain prerouting_udp {{
\t\ttype filter hook prerouting priority dstnat + 2; policy accept;
\t\tmeta nfproto != ipv4 return
\t\tmeta l4proto != {{ icmp, udp }} return
\t\tiifname \"{dev}\" return
\t\tct mark {route} meta mark set ct mark return
\t\tmeta mark set {bypass} ct mark set meta mark
\t}}
}}
"
    );
    s
}

/// Replace `route4` (when narrowed) and `exclude4` in one transaction.
pub(super) fn sets_update(
    route: Option<&[(Ipv4Addr, Ipv4Addr)]>,
    exclude: &[(Ipv4Addr, Ipv4Addr)],
) -> String {
    let mut s = String::new();
    let mut one = |name: &str, r: &[(Ipv4Addr, Ipv4Addr)]| {
        let _ = writeln!(s, "flush set inet {TABLE} {name}");
        if !r.is_empty() {
            let _ = writeln!(s, "add element inet {TABLE} {name} {{ {} }}", elements(r));
        }
    };
    if let Some(r) = route {
        one("route4", r);
    }
    one("exclude4", exclude);
    s
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

/// Every IPv4 address range of the host's interfaces, loopback and the
/// device's subnet: destinations never captured.
fn local_ranges(tun_net: Ipv4Net) -> Vec<(Ipv4Addr, Ipv4Addr)> {
    let mut r = vec![
        net_range(Ipv4Net::new_assert(Ipv4Addr::new(127, 0, 0, 0), 8)),
        net_range(tun_net.trunc()),
    ];
    // SAFETY: getifaddrs/freeifaddrs pair; entries are read only while the
    // list is alive, and sockaddr_in is read only for AF_INET entries.
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&raw mut head) == 0 {
            let mut cur = head;
            while !cur.is_null() {
                let ifa = &*cur;
                if !ifa.ifa_addr.is_null()
                    && i32::from((*ifa.ifa_addr).sa_family) == libc::AF_INET
                    && !ifa.ifa_netmask.is_null()
                {
                    let a = &*ifa.ifa_addr.cast::<libc::sockaddr_in>();
                    let m = &*ifa.ifa_netmask.cast::<libc::sockaddr_in>();
                    let addr = Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr));
                    let mask = u32::from_be(m.sin_addr.s_addr);
                    let prefix = u8::try_from(mask.leading_ones()).unwrap_or(32);
                    if let Ok(net) = Ipv4Net::new(addr, prefix) {
                        r.push(net_range(net.trunc()));
                    }
                }
                cur = ifa.ifa_next;
            }
            libc::freeifaddrs(head);
        }
    }
    coalesce(r)
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

    #[test]
    fn sets_update_flushes_and_refills() {
        let s = sets_update(Some(&[(a("1.0.0.0"), a("1.0.0.255"))]), &[]);
        assert_eq!(
            s,
            "flush set inet meow route4\nadd element inet meow route4 { 1.0.0.0-1.0.0.255 }\n\
             flush set inet meow exclude4\n"
        );
        assert_eq!(sets_update(None, &[]), "flush set inet meow exclude4\n");
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
    }

    #[test]
    fn ruleset_narrowed_has_route_set_and_filters() {
        let p = Params {
            dev: "Meta".into(),
            redirect_port: 7895,
            bypass_mark: DEFAULT_BYPASS_MARK,
            tun_net: "198.18.0.1/30".parse().unwrap(),
            dns_target: Some(a("198.18.0.2")),
            route: Some(vec![(a("91.108.4.0"), a("91.108.7.255"))]),
            exclude: Vec::new(),
            dns_set: true,
        };
        let s = ruleset(&p, &[(a("127.0.0.0"), a("127.255.255.255"))]);
        assert!(s.starts_with("table inet meow\ndelete table inet meow\n"));
        assert!(s.contains("elements = { 91.108.4.0-91.108.7.255 }"));
        assert!(s.contains("ip daddr != @route4 ip daddr != @route4_dns return"));
        assert!(s.contains("set route4_dns {\n\t\ttype ipv4_addr\n\t\tflags timeout"));
        assert!(s.contains("ip daddr @exclude4 return"));
        assert!(s.contains("redirect to :7895"));
        assert!(s.contains("udp dport 53 dnat ip to 198.18.0.2"));
        assert!(s.contains("meta mark 0x00002024 return"));
    }

    #[test]
    fn ruleset_capture_all_has_no_route_set() {
        let p = Params {
            dev: "Meta".into(),
            redirect_port: 1,
            bypass_mark: 7,
            tun_net: "198.18.0.1/30".parse().unwrap(),
            dns_target: None,
            route: None,
            exclude: vec![(a("1.1.1.1"), a("1.1.1.1"))],
            dns_set: false,
        };
        let s = ruleset(&p, &[]);
        assert!(!s.contains("@route4"));
        assert!(s.contains("elements = { 1.1.1.1 }"));
        assert!(s.contains("ip daddr @exclude4 return"));
        assert!(!s.contains("dport 53"));
    }
}
