// SPDX-License-Identifier: GPL-3.0-only
//! Kernel side of the `wireguard` outbound (meow-openwrt stage 8): rtnetlink
//! for the link, its addresses, a per-link routing table and the `oif` rule
//! that sends bound sockets there, and generic netlink for the device
//! configuration of the kernel's `wireguard` or AmneziaWG's `amneziawg`
//! family. Both families share one uapi; AmneziaWG adds attributes after
//! `WGDEVICE_A_PEERS`. Messages are framed by hand (no netlink crate).

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const NETLINK_ROUTE: i32 = 0;
const NETLINK_GENERIC: i32 = 16;

const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_REPLACE: u16 = 0x100;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLA_F_NESTED: u16 = 0x8000;

const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const RTM_NEWADDR: u16 = 20;
const RTM_NEWROUTE: u16 = 24;
const RTM_NEWRULE: u16 = 32;
const RTM_DELRULE: u16 = 33;

const IFLA_IFNAME: u16 = 3;
const IFLA_MTU: u16 = 4;
const IFLA_LINKINFO: u16 = 18;
const IFLA_INFO_KIND: u16 = 1;
const IFF_UP: u32 = 1;

const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;

const RTA_OIF: u16 = 4;
const RTA_TABLE: u16 = 15;
const RTPROT_STATIC: u8 = 4;
const RT_SCOPE_LINK: u8 = 253;
const RTN_UNICAST: u8 = 1;

const FRA_PRIORITY: u16 = 6;
const FRA_FWMARK: u16 = 10;
const FRA_TABLE: u16 = 15;
const FRA_FWMASK: u16 = 16;
const FRA_OIFNAME: u16 = 17;
const FR_ACT_TO_TBL: u8 = 1;

const GENL_ID_CTRL: u16 = 0x10;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const CTRL_ATTR_VERSION: u16 = 3;

const WG_CMD_SET_DEVICE: u8 = 1;
const WGDEVICE_A_IFNAME: u16 = 2;
const WGDEVICE_A_PRIVATE_KEY: u16 = 3;
const WGDEVICE_A_FLAGS: u16 = 5;
const WGDEVICE_A_FWMARK: u16 = 7;
const WGDEVICE_A_PEERS: u16 = 8;
const WGDEVICE_A_JC: u16 = 9;
const WGDEVICE_A_JMIN: u16 = 10;
const WGDEVICE_A_JMAX: u16 = 11;
const WGDEVICE_A_S1: u16 = 12;
const WGDEVICE_A_S2: u16 = 13;
const WGDEVICE_A_H1: u16 = 14;
const WGDEVICE_A_S3: u16 = 19;
const WGDEVICE_A_S4: u16 = 20;
const WGDEVICE_A_I1: u16 = 21;
const WGDEVICE_A_HEADER_PROTECTION_KEY: u16 = 26;
const WGDEVICE_A_CONTENT_PADDING_ADDITION: u16 = 27;
const WGDEVICE_A_REKEY_AFTER_TIME: u16 = 28;
const WGDEVICE_A_REKEY_TIMEOUT: u16 = 29;
const WGDEVICE_A_REJECT_AFTER_TIME: u16 = 30;
const WGDEVICE_A_KEEPALIVE_TIMEOUT: u16 = 31;
const WGDEVICE_A_MAX_HANDSHAKE_ATTEMPTS: u16 = 32;
const WGDEVICE_A_RANDOM_TRAILERS: u16 = 33;
const WGDEVICE_A_DISABLE_COOKIES: u16 = 34;
const WGDEVICE_F_REPLACE_PEERS: u32 = 1;
const WGPEER_A_PUBLIC_KEY: u16 = 1;
const WGPEER_A_PRESHARED_KEY: u16 = 2;
const WGPEER_A_FLAGS: u16 = 3;
const WGPEER_A_ENDPOINT: u16 = 4;
const WGPEER_A_PERSISTENT_KEEPALIVE_INTERVAL: u16 = 5;
const WGPEER_A_ALLOWEDIPS: u16 = 9;
const WGPEER_F_REPLACE_ALLOWEDIPS: u32 = 2;
const WGALLOWEDIP_A_FAMILY: u16 = 1;
const WGALLOWEDIP_A_IPADDR: u16 = 2;
const WGALLOWEDIP_A_CIDR_MASK: u16 = 3;

/// One netlink message under construction: header, fixed part, attributes.
struct Msg {
    buf: Vec<u8>,
    nests: Vec<usize>,
}

impl Msg {
    fn new(ty: u16, flags: u16, fixed: &[u8]) -> Self {
        let mut buf = Vec::with_capacity(256);
        buf.extend_from_slice(&[0; 4]); // length, patched in `finish`
        buf.extend_from_slice(&ty.to_ne_bytes());
        buf.extend_from_slice(&(flags | NLM_F_REQUEST | NLM_F_ACK).to_ne_bytes());
        buf.extend_from_slice(&[0; 8]); // seq, pid
        buf.extend_from_slice(fixed);
        Self {
            buf,
            nests: Vec::new(),
        }
    }

    fn attr(&mut self, ty: u16, data: &[u8]) -> &mut Self {
        let len = u16::try_from(4 + data.len()).expect("netlink attribute fits u16");
        self.buf.extend_from_slice(&len.to_ne_bytes());
        self.buf.extend_from_slice(&ty.to_ne_bytes());
        self.buf.extend_from_slice(data);
        self.pad();
        self
    }

    fn u8(&mut self, ty: u16, v: u8) -> &mut Self {
        self.attr(ty, &[v])
    }
    fn u16(&mut self, ty: u16, v: u16) -> &mut Self {
        self.attr(ty, &v.to_ne_bytes())
    }
    fn u32(&mut self, ty: u16, v: u32) -> &mut Self {
        self.attr(ty, &v.to_ne_bytes())
    }
    fn u64(&mut self, ty: u16, v: u64) -> &mut Self {
        self.attr(ty, &v.to_ne_bytes())
    }
    fn strz(&mut self, ty: u16, s: &str) -> &mut Self {
        let mut d = Vec::with_capacity(s.len() + 1);
        d.extend_from_slice(s.as_bytes());
        d.push(0);
        self.attr(ty, &d)
    }

    fn nest(&mut self, ty: u16) -> &mut Self {
        self.nests.push(self.buf.len());
        self.buf.extend_from_slice(&[0, 0]);
        self.buf
            .extend_from_slice(&(ty | NLA_F_NESTED).to_ne_bytes());
        self
    }

    fn end(&mut self) -> &mut Self {
        let at = self.nests.pop().expect("nest/end pairs");
        let len = u16::try_from(self.buf.len() - at).expect("nested attribute fits u16");
        self.buf[at..at + 2].copy_from_slice(&len.to_ne_bytes());
        self
    }

    fn pad(&mut self) {
        while !self.buf.len().is_multiple_of(4) {
            self.buf.push(0);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        let len = u32::try_from(self.buf.len()).expect("netlink message fits u32");
        self.buf[..4].copy_from_slice(&len.to_ne_bytes());
        self.buf
    }
}

fn socket(proto: i32) -> io::Result<OwnedFd> {
    // SAFETY: plain socket(2); the fd is owned right away.
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, proto) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a fresh, valid descriptor nobody else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Send one request, wait for its ACK; returns the payloads of data replies
/// that came before it (the family lookup reads one).
fn request(proto: i32, msg: Msg) -> io::Result<Vec<Vec<u8>>> {
    let fd = socket(proto)?;
    let req = msg.finish();
    // SAFETY: req is a valid buffer of req.len() bytes.
    let n = unsafe { libc::send(fd.as_raw_fd(), req.as_ptr().cast(), req.len(), 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut out = Vec::new();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        // SAFETY: buf is a valid writable buffer of buf.len() bytes.
        let n = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut rest = &buf[..usize::try_from(n).unwrap_or(0)];
        while rest.len() >= 16 {
            let len = u32::from_ne_bytes(rest[0..4].try_into().expect("4 bytes")) as usize;
            let ty = u16::from_ne_bytes(rest[4..6].try_into().expect("2 bytes"));
            if len < 16 || len > rest.len() {
                return Err(io::Error::other("netlink: truncated reply"));
            }
            match ty {
                NLMSG_ERROR => {
                    let code = i32::from_ne_bytes(rest[16..20].try_into().expect("4 bytes"));
                    return if code == 0 {
                        Ok(out)
                    } else {
                        Err(io::Error::from_raw_os_error(-code))
                    };
                }
                NLMSG_DONE => return Ok(out),
                _ => out.push(rest[16..len].to_vec()),
            }
            rest = &rest[(len + 3) & !3..];
        }
    }
}

fn ifinfo(index: i32, flags: u32, change: u32) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0] = libc::AF_UNSPEC as u8;
    b[4..8].copy_from_slice(&index.to_ne_bytes());
    b[8..12].copy_from_slice(&flags.to_ne_bytes());
    b[12..16].copy_from_slice(&change.to_ne_bytes());
    b
}

fn family_of(v6: bool) -> u8 {
    if v6 {
        libc::AF_INET6 as u8
    } else {
        libc::AF_INET as u8
    }
}

/// `ip link add <name> type <kind>`.
pub(super) fn link_add(name: &str, kind: &str) -> io::Result<()> {
    let mut m = Msg::new(RTM_NEWLINK, NLM_F_CREATE | NLM_F_EXCL, &ifinfo(0, 0, 0));
    m.strz(IFLA_IFNAME, name)
        .nest(IFLA_LINKINFO)
        .strz(IFLA_INFO_KIND, kind)
        .end();
    request(NETLINK_ROUTE, m).map(drop)
}

/// `ip link del <name>`; a missing link is not an error.
pub(super) fn link_del(name: &str) -> io::Result<()> {
    let mut m = Msg::new(RTM_DELLINK, 0, &ifinfo(0, 0, 0));
    m.strz(IFLA_IFNAME, name);
    match request(NETLINK_ROUTE, m) {
        Err(e) if e.raw_os_error() == Some(libc::ENODEV) => Ok(()),
        r => r.map(drop),
    }
}

pub(super) fn link_index(name: &str) -> io::Result<u32> {
    let c = std::ffi::CString::new(name).map_err(io::Error::other)?;
    // SAFETY: c is a valid NUL-terminated string.
    match unsafe { libc::if_nametoindex(c.as_ptr()) } {
        0 => Err(io::Error::last_os_error()),
        i => Ok(i),
    }
}

/// `ip link set <index> mtu <mtu> up`.
pub(super) fn link_up(index: u32, mtu: u32) -> io::Result<()> {
    let idx = i32::try_from(index).map_err(io::Error::other)?;
    let mut m = Msg::new(RTM_NEWLINK, 0, &ifinfo(idx, IFF_UP, IFF_UP));
    m.u32(IFLA_MTU, mtu);
    request(NETLINK_ROUTE, m).map(drop)
}

/// `ip addr add <addr>/<prefix> dev <index>`.
pub(super) fn addr_add(index: u32, addr: IpAddr, prefix: u8) -> io::Result<()> {
    let mut fixed = [0u8; 8];
    fixed[0] = family_of(addr.is_ipv6());
    fixed[1] = prefix;
    fixed[4..8].copy_from_slice(&index.to_ne_bytes());
    let mut m = Msg::new(RTM_NEWADDR, NLM_F_CREATE | NLM_F_REPLACE, &fixed);
    let bytes = ip_bytes(addr);
    m.attr(IFA_LOCAL, &bytes).attr(IFA_ADDRESS, &bytes);
    request(NETLINK_ROUTE, m).map(drop)
}

/// `ip route replace default dev <index> table <table>`; the routes go with
/// the link when it is deleted.
pub(super) fn default_route(index: u32, table: u32, v6: bool) -> io::Result<()> {
    let mut fixed = [0u8; 12];
    fixed[0] = family_of(v6);
    fixed[5] = RTPROT_STATIC;
    fixed[6] = RT_SCOPE_LINK;
    fixed[7] = RTN_UNICAST;
    let mut m = Msg::new(RTM_NEWROUTE, NLM_F_CREATE | NLM_F_REPLACE, &fixed);
    m.u32(RTA_TABLE, table).u32(RTA_OIF, index);
    request(NETLINK_ROUTE, m).map(drop)
}

fn rule_msg(ty: u16, flags: u16, oif: &str, table: u32, priority: u32, v6: bool) -> Msg {
    let mut fixed = [0u8; 12];
    fixed[0] = family_of(v6);
    fixed[7] = FR_ACT_TO_TBL;
    let mut m = Msg::new(ty, flags, &fixed);
    m.u32(FRA_PRIORITY, priority)
        .u32(FRA_TABLE, table)
        .strz(FRA_OIFNAME, oif);
    m
}

/// `ip rule add oif <name> lookup <table> priority <priority>`: sockets bound
/// to the link (SO_BINDTODEVICE) find its default route there.
pub(super) fn rule_add(oif: &str, table: u32, priority: u32, v6: bool) -> io::Result<()> {
    let m = rule_msg(
        RTM_NEWRULE,
        NLM_F_CREATE | NLM_F_EXCL,
        oif,
        table,
        priority,
        v6,
    );
    match request(NETLINK_ROUTE, m) {
        Err(e) if e.raw_os_error() == Some(libc::EEXIST) => Ok(()),
        r => r.map(drop),
    }
}

/// Removes the rule [`rule_add`] made; a missing rule is not an error.
pub(super) fn rule_del(oif: &str, table: u32, priority: u32, v6: bool) -> io::Result<()> {
    let m = rule_msg(RTM_DELRULE, 0, oif, table, priority, v6);
    match request(NETLINK_ROUTE, m) {
        Err(e) if e.raw_os_error() == Some(libc::ENOENT) => Ok(()),
        r => r.map(drop),
    }
}

fn fwmark_rule(ty: u16, flags: u16, mark: u32, table: u32, priority: u32, v6: bool) -> Msg {
    let mut fixed = [0u8; 12];
    fixed[0] = family_of(v6);
    fixed[7] = FR_ACT_TO_TBL;
    let mut m = Msg::new(ty, flags, &fixed);
    m.u32(FRA_PRIORITY, priority)
        .u32(FRA_TABLE, table)
        .u32(FRA_FWMARK, mark)
        .u32(FRA_FWMASK, u32::MAX);
    m
}

/// `ip rule add fwmark <mark> lookup <table> priority <priority>`.
pub(super) fn rule_fwmark(mark: u32, table: u32, priority: u32, v6: bool) -> io::Result<()> {
    let m = fwmark_rule(
        RTM_NEWRULE,
        NLM_F_CREATE | NLM_F_EXCL,
        mark,
        table,
        priority,
        v6,
    );
    match request(NETLINK_ROUTE, m) {
        Err(e) if e.raw_os_error() == Some(libc::EEXIST) => Ok(()),
        r => r.map(drop),
    }
}

/// Removes the rule [`rule_fwmark`] made; a missing rule is not an error.
pub(super) fn rule_fwmark_del(mark: u32, table: u32, priority: u32, v6: bool) -> io::Result<()> {
    let m = fwmark_rule(RTM_DELRULE, 0, mark, table, priority, v6);
    match request(NETLINK_ROUTE, m) {
        Err(e) if e.raw_os_error() == Some(libc::ENOENT) => Ok(()),
        r => r.map(drop),
    }
}

fn ip_bytes(ip: IpAddr) -> Vec<u8> {
    match ip {
        IpAddr::V4(a) => a.octets().to_vec(),
        IpAddr::V6(a) => a.octets().to_vec(),
    }
}

/// A generic netlink family: its id and version (AmneziaWG encodes H1–H4
/// differently per version).
#[derive(Clone, Copy, Debug)]
pub(super) struct Family {
    pub id: u16,
    pub version: u32,
    /// `amneziawg` rather than `wireguard`: its policy types the keepalive
    /// interval as u32 (the kernel's own module: u16).
    pub amnezia: bool,
}

/// Look up a generic netlink family; `Ok(None)` when its module is not loaded.
pub(super) fn genl_family(name: &str) -> io::Result<Option<Family>> {
    let mut m = Msg::new(GENL_ID_CTRL, 0, &[CTRL_CMD_GETFAMILY, 1, 0, 0]);
    m.strz(CTRL_ATTR_FAMILY_NAME, name);
    let replies = match request(NETLINK_GENERIC, m) {
        Err(e) if e.raw_os_error() == Some(libc::ENOENT) => return Ok(None),
        r => r?,
    };
    for r in replies {
        let (mut id, mut version) = (None, 1);
        let mut a = r.get(4..).unwrap_or_default();
        while a.len() >= 4 {
            let len = u16::from_ne_bytes([a[0], a[1]]) as usize;
            let ty = u16::from_ne_bytes([a[2], a[3]]) & !NLA_F_NESTED;
            if len < 4 || len > a.len() {
                break;
            }
            let v = &a[4..len];
            match ty {
                CTRL_ATTR_FAMILY_ID if v.len() >= 2 => id = Some(u16::from_ne_bytes([v[0], v[1]])),
                CTRL_ATTR_VERSION if v.len() >= 4 => {
                    version = u32::from_ne_bytes(v[..4].try_into().expect("4 bytes"));
                }
                _ => {}
            }
            a = &a[((len + 3) & !3).min(a.len())..];
        }
        if let Some(id) = id {
            return Ok(Some(Family {
                id,
                version,
                amnezia: name == "amneziawg",
            }));
        }
    }
    Ok(None)
}

/// A header-type range for AmneziaWG H1–H4: `lo` alone or `lo-hi`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HRange {
    pub lo: u32,
    pub hi: u32,
}

/// AmneziaWG device settings; `None` fields are left to the module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Amnezia {
    pub jc: Option<u16>,
    pub jmin: Option<u16>,
    pub jmax: Option<u16>,
    pub s: [Option<u16>; 4],
    pub h: [Option<HRange>; 4],
    pub i: [Option<String>; 5],
    pub header_protection_key: Option<[u8; 32]>,
    pub content_padding_addition: Option<u32>,
    pub rekey_after_time: Option<u32>,
    pub rekey_timeout: Option<u32>,
    pub reject_after_time: Option<u32>,
    pub keepalive_timeout: Option<u32>,
    pub max_handshake_attempts: Option<u32>,
    pub random_trailers: Option<bool>,
    pub disable_cookies: Option<bool>,
}

/// The device configuration meow sends: one peer, as mihomo's node has.
pub(crate) struct Device<'a> {
    pub ifname: &'a str,
    pub private_key: [u8; 32],
    pub fwmark: u32,
    pub peer_key: [u8; 32],
    pub psk: Option<[u8; 32]>,
    pub endpoint: SocketAddr,
    pub keepalive: u16,
    pub allowed: &'a [(IpAddr, u8)],
    pub amnezia: Option<&'a Amnezia>,
}

fn sockaddr(sa: SocketAddr) -> Vec<u8> {
    let mut b = Vec::with_capacity(28);
    match sa {
        SocketAddr::V4(a) => {
            b.extend_from_slice(&(libc::AF_INET as u16).to_ne_bytes());
            b.extend_from_slice(&a.port().to_be_bytes());
            b.extend_from_slice(&a.ip().octets());
            b.extend_from_slice(&[0; 8]);
        }
        SocketAddr::V6(a) => {
            b.extend_from_slice(&(libc::AF_INET6 as u16).to_ne_bytes());
            b.extend_from_slice(&a.port().to_be_bytes());
            b.extend_from_slice(&a.flowinfo().to_be_bytes());
            b.extend_from_slice(&a.ip().octets());
            b.extend_from_slice(&a.scope_id().to_ne_bytes());
        }
    }
    b
}

/// `wg set` / `awg set`: keys, the one peer and (for `amneziawg`) the
/// obfuscation parameters, replacing whatever peers the link had.
pub(super) fn wg_set(fam: Family, d: &Device<'_>) -> io::Result<()> {
    let mut m = Msg::new(fam.id, 0, &[WG_CMD_SET_DEVICE, 1, 0, 0]);
    m.strz(WGDEVICE_A_IFNAME, d.ifname)
        .attr(WGDEVICE_A_PRIVATE_KEY, &d.private_key)
        .u32(WGDEVICE_A_FLAGS, WGDEVICE_F_REPLACE_PEERS)
        .u32(WGDEVICE_A_FWMARK, d.fwmark);
    if let Some(a) = d.amnezia {
        put_amnezia(&mut m, fam.version, a);
    }
    m.nest(WGDEVICE_A_PEERS).nest(0);
    m.attr(WGPEER_A_PUBLIC_KEY, &d.peer_key);
    if let Some(psk) = &d.psk {
        m.attr(WGPEER_A_PRESHARED_KEY, psk);
    }
    m.u32(WGPEER_A_FLAGS, WGPEER_F_REPLACE_ALLOWEDIPS)
        .attr(WGPEER_A_ENDPOINT, &sockaddr(d.endpoint));
    if fam.amnezia {
        m.u32(
            WGPEER_A_PERSISTENT_KEEPALIVE_INTERVAL,
            u32::from(d.keepalive),
        );
    } else {
        m.u16(WGPEER_A_PERSISTENT_KEEPALIVE_INTERVAL, d.keepalive);
    }
    m.nest(WGPEER_A_ALLOWEDIPS);
    for (i, (ip, cidr)) in d.allowed.iter().enumerate() {
        let fam16 = u16::from(family_of(ip.is_ipv6()));
        m.nest(u16::try_from(i).unwrap_or(0))
            .u16(WGALLOWEDIP_A_FAMILY, fam16)
            .attr(WGALLOWEDIP_A_IPADDR, &ip_bytes(*ip))
            .u8(WGALLOWEDIP_A_CIDR_MASK, *cidr)
            .end();
    }
    m.end().end().end();
    request(NETLINK_GENERIC, m).map(drop)
}

fn put_amnezia(m: &mut Msg, version: u32, a: &Amnezia) {
    for (ty, v) in [
        (WGDEVICE_A_JC, a.jc),
        (WGDEVICE_A_JMIN, a.jmin),
        (WGDEVICE_A_JMAX, a.jmax),
    ] {
        if let Some(v) = v {
            m.u16(ty, v);
        }
    }
    for (ty, v) in [WGDEVICE_A_S1, WGDEVICE_A_S2, WGDEVICE_A_S3, WGDEVICE_A_S4]
        .into_iter()
        .zip(a.s)
    {
        if let Some(v) = v {
            m.u16(ty, v);
        }
    }
    for (n, h) in a.h.iter().enumerate() {
        let Some(h) = h else { continue };
        let ty = WGDEVICE_A_H1 + u16::try_from(n).unwrap_or(0);
        // The module's encoding changed with its family version.
        match version {
            0 | 1 => m.u32(ty, h.lo),
            2 => m.strz(ty, &format!("{}-{}", h.lo, h.hi)),
            _ => m.u64(ty, u64::from(h.lo) | (u64::from(h.hi) << 32)),
        };
    }
    if version >= 2 {
        for (n, s) in a.i.iter().enumerate() {
            if let Some(s) = s {
                m.strz(WGDEVICE_A_I1 + u16::try_from(n).unwrap_or(0), s);
            }
        }
    }
    if version >= 3 {
        if let Some(k) = &a.header_protection_key {
            m.attr(WGDEVICE_A_HEADER_PROTECTION_KEY, k);
        }
        for (ty, v) in [
            (
                WGDEVICE_A_CONTENT_PADDING_ADDITION,
                a.content_padding_addition,
            ),
            (WGDEVICE_A_REKEY_AFTER_TIME, a.rekey_after_time),
            (WGDEVICE_A_REKEY_TIMEOUT, a.rekey_timeout),
            (WGDEVICE_A_REJECT_AFTER_TIME, a.reject_after_time),
            (WGDEVICE_A_KEEPALIVE_TIMEOUT, a.keepalive_timeout),
            (WGDEVICE_A_MAX_HANDSHAKE_ATTEMPTS, a.max_handshake_attempts),
        ] {
            if let Some(v) = v {
                m.u32(ty, v);
            }
        }
        for (ty, v) in [
            (WGDEVICE_A_RANDOM_TRAILERS, a.random_trailers),
            (WGDEVICE_A_DISABLE_COOKIES, a.disable_cookies),
        ] {
            if let Some(v) = v {
                m.u8(ty, u8::from(v));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_lengths_are_patched() {
        let mut m = Msg::new(1, 0, &[]);
        m.nest(8).u16(1, 7).end();
        let b = m.finish();
        assert_eq!(
            u32::from_ne_bytes(b[0..4].try_into().unwrap()) as usize,
            b.len()
        );
        // nest header (4) + u16 attr padded (8)
        assert_eq!(u16::from_ne_bytes([b[16], b[17]]), 12);
        assert_eq!(u16::from_ne_bytes([b[18], b[19]]), 8 | NLA_F_NESTED);
    }

    #[test]
    fn sockaddr_v4_layout() {
        let b = sockaddr("1.2.3.4:51820".parse().unwrap());
        assert_eq!(b.len(), 16);
        assert_eq!(&b[2..4], &51820u16.to_be_bytes());
        assert_eq!(&b[4..8], &[1, 2, 3, 4]);
    }
}
