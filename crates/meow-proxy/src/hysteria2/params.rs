//! Transport parameters and pin parsing shared by the quiche driver
//! (little-endian builds) and the quinn driver (big-endian builds).

use super::{Error, Result};
use std::time::Duration;

pub(super) const ALPN_H3: &[u8] = b"h3";
/// Initial windows, as Hysteria's client (8 MiB stream, 20 MiB connection).
pub(super) const STREAM_RECEIVE_WINDOW: u64 = 8 * 1024 * 1024;
pub(super) const CONN_RECEIVE_WINDOW: u64 = STREAM_RECEIVE_WINDOW * 5 / 2;
/// quiche's auto-tuning ceilings.
pub(super) const MAX_STREAM_WINDOW: u64 = 16 * 1024 * 1024;
pub(super) const MAX_CONN_WINDOW: u64 = 24 * 1024 * 1024;
/// Floor of the memory-scaled connection ceiling.
pub(super) const MIN_AUTO_CONN_WINDOW: u64 = 4 * 1024 * 1024;
pub(super) const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const MAX_CONCURRENT_STREAMS: u64 = 1024;
pub(super) const DGRAM_QUEUE_LEN: usize = 1024;
/// quic-go's ceiling (1500 − IPv6 40 − UDP 8); PMTUD finds the path's.
pub(super) const MAX_SEND_UDP_PAYLOAD: usize = 1452;

/// Flow-control windows in effect, bytes.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Windows {
    pub(super) initial_stream: u64,
    pub(super) max_stream: u64,
    pub(super) initial_conn: u64,
    pub(super) max_conn: u64,
}

/// The configured windows, defaults filled in. Unset, the connection ceiling
/// scales with RAM (MemTotal / 16, 4..=24 MiB): a client reading slower than
/// the server sends parks a whole window in this process, and the stock
/// 20-24 MiB per connection is a third of a 64 MB router (meow-openwrt;
/// tunbench, 20 Mbit/s client: 29-33 MB anon). With `connections` parallel
/// connections that budget is shared between them (floor 2 MiB each).
/// Initials never exceed their ceilings.
pub(super) fn resolve_windows(
    cfg: super::ReceiveWindows,
    mem_total: Option<u64>,
    connections: usize,
) -> Windows {
    let auto_conn = mem_total.map_or(MAX_CONN_WINDOW, |m| {
        (m / 16).clamp(MIN_AUTO_CONN_WINDOW, MAX_CONN_WINDOW)
    });
    let auto_conn = (auto_conn / connections.max(1) as u64).max(MIN_AUTO_CONN_WINDOW / 2);
    let max_conn = cfg.max_conn.unwrap_or(auto_conn);
    let max_stream = cfg.max_stream.unwrap_or(MAX_STREAM_WINDOW.min(max_conn));
    Windows {
        initial_stream: cfg
            .initial_stream
            .unwrap_or(STREAM_RECEIVE_WINDOW)
            .min(max_stream),
        max_stream,
        initial_conn: cfg
            .initial_conn
            .unwrap_or(CONN_RECEIVE_WINDOW)
            .min(max_conn),
        max_conn,
    }
}

pub(super) fn parse_sha256_pin(raw: &str) -> Result<Option<[u8; 32]>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let without_prefix = raw
        .strip_prefix("sha256=")
        .or_else(|| raw.strip_prefix("SHA256="))
        .unwrap_or(raw);
    let normalized: String = without_prefix
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ':')
        .flat_map(char::to_lowercase)
        .collect();
    if normalized.len() != 64 || !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::config(
            "pin-sha256/fingerprint must be a SHA-256 hex digest",
        ));
    }
    let decoded = hex::decode(normalized)
        .map_err(|e| Error::config(format!("invalid SHA-256 fingerprint: {e}")))?;
    let mut pin = [0u8; 32];
    pin.copy_from_slice(&decoded);
    Ok(Some(pin))
}
