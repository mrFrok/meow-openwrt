mod auth;
#[cfg(hy2_quiche)]
mod bufpool;
mod client;
mod config;
#[cfg(hy2_quiche)]
mod driver;
#[cfg(not(hy2_quiche))]
#[path = "driver_quinn.rs"]
mod driver;
mod error;
mod obfs;
mod params;
mod proto;
mod qpack;
mod tcp;
#[cfg(hy2_quiche)]
mod tls;
mod udp;
#[cfg(hy2_quiche)]
mod udp_io;

/// The hysteria2 QUIC connection: quiche with recycled stream buffers.
#[cfg(hy2_quiche)]
pub(crate) type QuicConn = quiche::Connection<bufpool::PoolFactory>;

pub use client::ReconnectableClient;
pub use config::{Config, ObfsConfig, ReceiveWindows};
pub use error::{Error, Result};
pub(crate) use obfs::gecko_packet_sizes;
/// Adapter-side bound for `host:port` datagram addresses (issue #657).
pub use proto::MAX_ADDRESS_LENGTH;
pub use tcp::DuplexStream;
pub use udp::{UdpSender, UdpSession};
