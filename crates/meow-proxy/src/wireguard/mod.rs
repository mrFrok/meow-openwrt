// SPDX-License-Identifier: GPL-3.0-only
//! meow-openwrt stage 8: mihomo's `type: wireguard` outbound (with
//! `amnezia-wg-option`) on the kernel's WireGuard or AmneziaWG module
//! instead of a userspace stack. meow creates the link the first time the
//! node is used (a dial or a health check), configures it over netlink,
//! and dials through it like DIRECT bound to the link: the kernel does the
//! crypto, so the node costs no CPU in meow and almost no binary size.
//!
//! Routing: sockets bound to the link (`SO_BINDTODEVICE`) find a default
//! route in the link's own table through an `oif` rule (priority
//! `RULE_PRIORITY`, ahead of the TUN rules at 9200). The link's encrypted
//! packets carry meow's routing mark (or [`WG_MARK`] with a rule back to
//! the main table), so TUN / auto-redirect capture never takes them.
//!
//! The link goes away with the adapter (config reload, provider refresh)
//! and at exit through [`cleanup_all`].

mod netlink;

pub use netlink::{Amnezia, HRange};

use crate::direct::DirectAdapter;
use async_trait::async_trait;
use meow_common::{
    AdapterType, MeowError, Metadata, ProxyAdapter, ProxyConn, ProxyHealth, ProxyPacketConn, Result,
};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Mutex as StdMutex;
use tracing::{info, warn};

/// Priority of the per-link `oif` rules, before the TUN rules (9200–9205).
const RULE_PRIORITY: u32 = 9100;
/// Priority of the rule that returns marked tunnel packets to the main table.
const MARK_RULE_PRIORITY: u32 = 9101;
/// The link's own packets carry this mark when meow has no routing mark.
pub const WG_MARK: u32 = 0x2025;
const RT_TABLE_MAIN: u32 = 254;

/// Links meow created and still owns (name → table, v4, v6): removed with
/// their rules at exit by [`cleanup_all`].
static LIVE: StdMutex<BTreeMap<String, (u32, bool, bool)>> = StdMutex::new(BTreeMap::new());

/// A parsed `type: wireguard` node.
#[derive(Clone)]
pub struct WireGuardConfig {
    pub name: String,
    pub server: String,
    pub port: u16,
    pub ip: Option<(Ipv4Addr, u8)>,
    pub ipv6: Option<(Ipv6Addr, u8)>,
    pub private_key: [u8; 32],
    pub public_key: [u8; 32],
    pub pre_shared_key: Option<[u8; 32]>,
    pub allowed_ips: Vec<(IpAddr, u8)>,
    pub mtu: u32,
    pub persistent_keepalive: u16,
    pub udp: bool,
    pub amnezia: Option<Amnezia>,
}

/// A link meow created, removed again on drop.
struct Link {
    ifname: String,
    index: u32,
    table: u32,
    v4: bool,
    v6: bool,
}

impl Drop for Link {
    fn drop(&mut self) {
        // A newer adapter for the same node may have replaced the link
        // under the same name (config reload): leave that one alone.
        if netlink::link_index(&self.ifname).ok() != Some(self.index) {
            return;
        }
        for v6 in [false, true] {
            if (v6 && self.v6) || (!v6 && self.v4) {
                let _ = netlink::rule_del(&self.ifname, self.table, RULE_PRIORITY, v6);
            }
        }
        let _ = netlink::link_del(&self.ifname);
        if let Ok(mut live) = LIVE.lock() {
            live.remove(&self.ifname);
        }
        info!("wireguard: removed {}", self.ifname);
    }
}

pub struct WireGuardAdapter {
    cfg: WireGuardConfig,
    addr: String,
    ifname: String,
    table: u32,
    inner: DirectAdapter,
    link: tokio::sync::Mutex<Option<Link>>,
    health: ProxyHealth,
}

/// FNV-1a: a stable link name and table per node name.
fn node_hash(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

impl WireGuardAdapter {
    /// `inner` is the DIRECT the node dials with (resolver, timeouts); it
    /// gets bound to the node's link here.
    pub fn new(cfg: WireGuardConfig, inner: DirectAdapter) -> Self {
        let h = node_hash(&cfg.name);
        let ifname = format!("mwg{h:08x}");
        Self {
            addr: format!("{}:{}", cfg.server, cfg.port),
            table: 0x6d77_0000 | (h & 0xffff),
            inner: inner.with_interface(&ifname),
            ifname,
            cfg,
            link: tokio::sync::Mutex::new(None),
            health: ProxyHealth::new(),
        }
    }

    pub fn interface(&self) -> &str {
        &self.ifname
    }

    async fn ensure_link(&self) -> Result<()> {
        let mut link = self.link.lock().await;
        if let Some(l) = link.as_ref() {
            if netlink::link_index(&l.ifname).ok() == Some(l.index) {
                return Ok(());
            }
            warn!(
                "wireguard[{}]: link {} vanished, recreating",
                self.cfg.name, l.ifname
            );
            *link = None;
        }
        let endpoint = meow_common::resolve_host(&self.cfg.server, self.cfg.port)
            .await
            .map_err(MeowError::Io)?;
        let cfg = self.cfg.clone();
        let (ifname, table) = (self.ifname.clone(), self.table);
        let new = tokio::task::spawn_blocking(move || setup(&cfg, &ifname, table, endpoint))
            .await
            .map_err(|e| MeowError::Proxy(format!("wireguard: setup task: {e}")))?
            .map_err(|e| MeowError::Proxy(format!("wireguard[{}]: {e}", self.cfg.name)))?;
        *link = Some(new);
        Ok(())
    }
}

fn kind(cfg: &WireGuardConfig) -> (&'static str, &'static str) {
    if cfg.amnezia.is_some() {
        ("amneziawg", "kmod-amneziawg")
    } else {
        ("wireguard", "kmod-wireguard")
    }
}

fn setup(
    cfg: &WireGuardConfig,
    ifname: &str,
    table: u32,
    endpoint: SocketAddr,
) -> std::io::Result<Link> {
    let (kind, package) = kind(cfg);
    // A link left by a crashed meow, or by the adapter this one replaces.
    netlink::link_del(ifname)?;
    // Creating the link autoloads the module (and registers its family).
    netlink::link_add(ifname, kind).map_err(|e| match e.raw_os_error() {
        Some(libc::EOPNOTSUPP) | Some(libc::EINVAL) => std::io::Error::other(format!(
            "the kernel has no `{kind}` link type: install {package}"
        )),
        _ => e,
    })?;
    let index = netlink::link_index(ifname)?;
    let link = Link {
        ifname: ifname.to_owned(),
        index,
        table,
        v4: cfg.ip.is_some(),
        v6: cfg.ipv6.is_some(),
    };
    if let Ok(mut live) = LIVE.lock() {
        live.insert(ifname.to_owned(), (table, link.v4, link.v6));
    }
    let fam = netlink::genl_family(kind)?.ok_or_else(|| {
        std::io::Error::other(format!("no `{kind}` netlink family: install {package}"))
    })?;
    let mark = match meow_common::routing_mark() {
        0 => WG_MARK,
        mark => mark,
    };
    let allowed: Vec<(IpAddr, u8)> = if cfg.allowed_ips.is_empty() {
        vec![
            (Ipv4Addr::UNSPECIFIED.into(), 0),
            (Ipv6Addr::UNSPECIFIED.into(), 0),
        ]
    } else {
        cfg.allowed_ips.clone()
    };
    netlink::wg_set(
        fam,
        &netlink::Device {
            ifname,
            private_key: cfg.private_key,
            fwmark: mark,
            peer_key: cfg.public_key,
            psk: cfg.pre_shared_key,
            endpoint,
            keepalive: cfg.persistent_keepalive,
            allowed: &allowed,
            amnezia: cfg.amnezia.as_ref(),
        },
    )?;
    if let Some((ip, prefix)) = cfg.ip {
        netlink::addr_add(index, ip.into(), prefix)?;
    }
    if let Some((ip, prefix)) = cfg.ipv6 {
        netlink::addr_add(index, ip.into(), prefix)?;
    }
    netlink::link_up(index, cfg.mtu)?;
    relax_rp_filter(ifname);
    for (v6, on) in [(false, link.v4), (true, link.v6)] {
        if !on {
            continue;
        }
        netlink::default_route(index, table, v6)?;
        netlink::rule_add(ifname, table, RULE_PRIORITY, v6)?;
        if meow_common::routing_mark() == 0 {
            netlink::rule_fwmark(WG_MARK, RT_TABLE_MAIN, MARK_RULE_PRIORITY, v6)?;
        }
    }
    info!(
        "wireguard: {ifname} up ({kind}, family v{}) for {} via {endpoint}",
        fam.version, cfg.name
    );
    Ok(link)
}

/// Replies come in on the link from addresses only its own table routes, so
/// strict reverse-path filtering drops them: turn it off on the link. The
/// kernel takes the larger of `all` and the link's value, so a strict
/// `all` (not OpenWrt's default) still breaks the node; say so.
fn relax_rp_filter(ifname: &str) {
    let path = format!("/proc/sys/net/ipv4/conf/{ifname}/rp_filter");
    if let Err(e) = std::fs::write(&path, "0") {
        warn!("wireguard: {path}: {e}");
    }
    if std::fs::read_to_string("/proc/sys/net/ipv4/conf/all/rp_filter")
        .is_ok_and(|v| v.trim() == "1")
    {
        warn!(
            "wireguard: net.ipv4.conf.all.rp_filter=1 drops replies on {ifname}; \
             set it to 0 or 2 (OpenWrt default: 0)"
        );
    }
}

/// Delete every link meow created; for shutdown.
pub fn cleanup_all() {
    let links: Vec<(String, (u32, bool, bool))> = LIVE
        .lock()
        .map(|l| l.iter().map(|(k, v)| (k.clone(), *v)).collect())
        .unwrap_or_default();
    for (name, (table, v4, v6)) in links {
        for (fam6, on) in [(false, v4), (true, v6)] {
            if on {
                let _ = netlink::rule_del(&name, table, RULE_PRIORITY, fam6);
            }
        }
        let _ = netlink::link_del(&name);
    }
    for v6 in [false, true] {
        let _ = netlink::rule_fwmark_del(WG_MARK, RT_TABLE_MAIN, MARK_RULE_PRIORITY, v6);
    }
}

#[async_trait]
impl ProxyAdapter for WireGuardAdapter {
    fn name(&self) -> &str {
        &self.cfg.name
    }

    fn adapter_type(&self) -> AdapterType {
        AdapterType::WireGuard
    }

    fn addr(&self) -> &str {
        &self.addr
    }

    fn support_udp(&self) -> bool {
        self.cfg.udp
    }

    async fn dial_tcp(&self, metadata: &Metadata) -> Result<Box<dyn ProxyConn>> {
        self.ensure_link().await?;
        self.inner.dial_tcp(metadata).await
    }

    async fn dial_udp(&self, metadata: &Metadata) -> Result<Box<dyn ProxyPacketConn>> {
        if !self.cfg.udp {
            return Err(MeowError::Proxy(format!(
                "wireguard[{}]: udp is off",
                self.cfg.name
            )));
        }
        self.ensure_link().await?;
        self.inner.dial_udp(metadata).await
    }

    fn health(&self) -> &ProxyHealth {
        &self.health
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_name_fits_ifnamsiz() {
        let a = WireGuardAdapter::new(test_cfg("🇩🇪 a very long node name"), DirectAdapter::new());
        assert!(a.interface().len() <= 15);
        assert!(a.interface().starts_with("mwg"));
    }

    fn test_cfg(name: &str) -> WireGuardConfig {
        WireGuardConfig {
            name: name.into(),
            server: "1.2.3.4".into(),
            port: 51820,
            ip: Some((Ipv4Addr::new(10, 0, 0, 2), 32)),
            ipv6: None,
            private_key: [1; 32],
            public_key: [2; 32],
            pre_shared_key: None,
            allowed_ips: vec![],
            mtu: 1408,
            persistent_keepalive: 0,
            udp: true,
            amnezia: None,
        }
    }
}
