//! Batched UDP I/O for the Hysteria 2 driver on Linux (meow-openwrt): UDP
//! GSO (`UDP_SEGMENT`) sends a run of equal-sized QUIC packets to one peer
//! in a single `sendmsg`, and UDP GRO hands back a run of received ones in
//! a single `recvmsg`. Per-packet syscalls dominated the driver's CPU on a
//! router; elsewhere (and on a kernel without either) the driver keeps one
//! syscall per packet.

use std::io;
use std::net::SocketAddr;

use tokio::io::Interest;
use tokio::net::UdpSocket;

/// Kernel cap on segments per GSO send (`UDP_MAX_SEGMENTS`).
pub(super) const MAX_SEGMENTS: usize = 64;
/// One GSO send carries at most one IP datagram's worth of payload.
pub(super) const MAX_BATCH_BYTES: usize = 65_000;

#[cfg(target_os = "linux")]
mod sys {
    use super::{io, Interest, SocketAddr, UdpSocket};
    use std::os::fd::AsRawFd;

    // Defined here: libc gates them off some musl targets.
    const UDP_SEGMENT: libc::c_int = 103;
    const UDP_GRO: libc::c_int = 104;

    /// Turn on GRO; `false` when the kernel lacks it (pre-5.0).
    pub(in super::super) fn enable_gro(socket: &UdpSocket) -> bool {
        let one: libc::c_int = 1;
        // SAFETY: a valid fd and a correctly sized int option value.
        let rc = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_UDP,
                UDP_GRO,
                std::ptr::from_ref(&one).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        rc == 0
    }

    /// Send `buf` to `dest` as `seg`-byte datagrams (the last may be
    /// shorter) in one `sendmsg`.
    pub(in super::super) async fn send_segments(
        socket: &UdpSocket,
        buf: &[u8],
        seg: usize,
        dest: SocketAddr,
    ) -> io::Result<()> {
        let addr = socket2::SockAddr::from(dest);
        let seg = u16::try_from(seg).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        socket
            .async_io(Interest::WRITABLE, || {
                sendmsg_gso(socket.as_raw_fd(), buf, seg, &addr)
            })
            .await
    }

    fn sendmsg_gso(fd: i32, buf: &[u8], seg: u16, addr: &socket2::SockAddr) -> io::Result<()> {
        // u64-aligned room for one cmsg carrying a u16.
        let mut control = [0u64; 4];
        let mut iov = libc::iovec {
            iov_base: buf.as_ptr().cast_mut().cast(),
            iov_len: buf.len(),
        };
        // SAFETY: zeroed msghdr is a valid empty header; every pointer set
        // below outlives the sendmsg call.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_name = addr.as_ptr().cast_mut().cast();
        msg.msg_namelen = addr.len();
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        // SAFETY: CMSG_SPACE is a pure size computation.
        let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<u16>() as u32) };
        msg.msg_controllen = space as _;
        // SAFETY: msg_control points at `control`, large enough for one
        // cmsg of CMSG_SPACE(2) bytes.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
            (*cmsg).cmsg_level = libc::SOL_UDP;
            (*cmsg).cmsg_type = UDP_SEGMENT;
            (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<u16>() as u32) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<u16>(), seg);
        }
        // SAFETY: `msg` is fully initialised above.
        let n = unsafe { libc::sendmsg(fd, &raw const msg, 0) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Non-blocking receive of one (possibly GRO-coalesced) read: bytes,
    /// source, and the segment size when the read holds several datagrams.
    pub(in super::super) fn try_recv_gro(
        socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<(usize, SocketAddr, Option<usize>)> {
        socket.try_io(Interest::READABLE, || recvmsg_gro(socket.as_raw_fd(), buf))
    }

    fn recvmsg_gro(fd: i32, buf: &mut [u8]) -> io::Result<(usize, SocketAddr, Option<usize>)> {
        let mut control = [0u64; 8];
        let mut seg = None;
        let mut n = 0usize;
        // SAFETY: try_init hands a zeroed sockaddr_storage and its length
        // for recvmsg to fill; the closure only writes within it.
        let ((), addr) = unsafe {
            socket2::SockAddr::try_init(|storage, len| {
                let mut iov = libc::iovec {
                    iov_base: buf.as_mut_ptr().cast(),
                    iov_len: buf.len(),
                };
                let mut msg: libc::msghdr = std::mem::zeroed();
                msg.msg_name = storage.cast();
                msg.msg_namelen = *len;
                msg.msg_iov = &raw mut iov;
                msg.msg_iovlen = 1;
                msg.msg_control = control.as_mut_ptr().cast();
                msg.msg_controllen = std::mem::size_of_val(&control) as _;
                let r = libc::recvmsg(fd, &raw mut msg, 0);
                if r < 0 {
                    return Err(io::Error::last_os_error());
                }
                n = r as usize;
                *len = msg.msg_namelen;
                let mut cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
                while !cmsg.is_null() {
                    if (*cmsg).cmsg_level == libc::SOL_UDP && (*cmsg).cmsg_type == UDP_GRO {
                        let size =
                            std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<libc::c_int>());
                        seg = usize::try_from(size).ok().filter(|&s| s > 0);
                    }
                    cmsg = libc::CMSG_NXTHDR(&raw const msg, cmsg);
                }
                Ok(())
            })?
        };
        let from = addr
            .as_socket()
            .ok_or_else(|| io::Error::other("recvmsg: non-IP source address"))?;
        Ok((n, from, seg))
    }
}

#[cfg(not(target_os = "linux"))]
mod sys {
    use super::{io, SocketAddr, UdpSocket};

    pub(in super::super) fn enable_gro(_socket: &UdpSocket) -> bool {
        false
    }

    pub(in super::super) async fn send_segments(
        _socket: &UdpSocket,
        _buf: &[u8],
        _seg: usize,
        _dest: SocketAddr,
    ) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub(in super::super) fn try_recv_gro(
        socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<(usize, SocketAddr, Option<usize>)> {
        socket.try_recv_from(buf).map(|(n, from)| (n, from, None))
    }
}

pub(super) use sys::{enable_gro, send_segments, try_recv_gro};

/// Whether GSO sends are possible at all on this target.
pub(super) const GSO_SUPPORTED: bool = cfg!(target_os = "linux");

/// Whether a failed GSO send means the path cannot take GSO (the driver
/// then falls back to one send per packet for the connection's life).
pub(super) fn gso_unsupported(e: &io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::EIO | libc::EINVAL | libc::ENOPROTOOPT)
    ) || e.kind() == io::ErrorKind::Unsupported
}
