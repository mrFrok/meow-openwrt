use crate::error::Result;
use bytes::Bytes;
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite};

pub trait ProxyConn: AsyncRead + AsyncWrite + Unpin + Send + Sync {
    fn remote_destination(&self) -> String {
        String::new()
    }

    /// The kernel TCP socket this connection reads and writes byte for
    /// byte, if there is one. The relay then moves bulk traffic with
    /// `splice(2)` instead of copying it through userspace, so a wrapper
    /// that frames, encrypts, buffers or counts bytes must keep the
    /// default `None`.
    fn tcp_stream(&self) -> Option<&tokio::net::TcpStream> {
        None
    }
}

// Blanket impl for TcpStream etc.
impl ProxyConn for tokio::net::TcpStream {
    fn tcp_stream(&self) -> Option<&tokio::net::TcpStream> {
        Some(self)
    }
}

// Impl for Box<dyn ProxyConn>
impl<T: ProxyConn + ?Sized> ProxyConn for Box<T> {
    fn tcp_stream(&self) -> Option<&tokio::net::TcpStream> {
        (**self).tcp_stream()
    }
}

#[async_trait::async_trait]
pub trait ProxyPacketConn: Send + Sync {
    /// Per-packet conns return the real wire source; bound conns may
    /// report the bound remote or an unspecified address when the
    /// underlying transport carries no source information. Consumers
    /// filtering on the source must skip the check for unspecified
    /// addresses or they would wedge bound conns.
    async fn read_packet(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)>;
    /// [`Self::read_packet`] for a caller that needs the datagram owned:
    /// at most `scratch.len()` bytes. The default reads into `scratch` and
    /// copies out; a conn whose datagrams already arrive as owned buffers
    /// (hysteria2) hands them over without the copy.
    async fn read_packet_owned(&self, scratch: &mut [u8]) -> Result<(Vec<u8>, SocketAddr)> {
        let (n, from) = self.read_packet(scratch).await?;
        Ok((scratch[..n].to_vec(), from))
    }
    async fn write_packet(&self, buf: &[u8], addr: &SocketAddr) -> Result<usize>;
    fn local_addr(&self) -> Result<SocketAddr>;
    fn close(&self) -> Result<()>;
}

pub struct UdpPacket {
    pub data: Bytes,
    pub addr: SocketAddr,
}
