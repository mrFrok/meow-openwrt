//! hysteria2 QUIC driver on quinn + rustls — `cfg(not(hy2_quiche))`: big-endian
//! targets, where quiche's BoringSSL does not build (meow-openwrt stage 7), or
//! any host with `MEOW_TLS_BACKEND=rustls` (build.rs).
//!
//! It serves the same [`Cmd`] / [`ReadItem`] protocol as the quiche driver, so
//! `tcp.rs`, `udp.rs` and `client.rs` are shared. quinn runs the connection
//! itself; this task routes commands, and each proxy stream gets a task of its
//! own. Salamander / Gecko obfuscation and port hopping wrap the UDP socket
//! ([`HySocket`]). Congestion control is BBR, as on the quiche path.

use super::obfs::{HopState, Obfs};
use super::params::{
    parse_sha256_pin, resolve_windows, ALPN_H3, DGRAM_QUEUE_LEN, MAX_CONCURRENT_STREAMS,
    MAX_IDLE_TIMEOUT, MAX_SEND_UDP_PAYLOAD,
};
use super::proto::{self, UdpMessage};
use super::tcp::{DuplexStream, WRITE_BUFFER_BYTES};
use super::{auth, Config, Error, Result};
use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, UdpPoller};
use std::collections::HashMap;
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::ReadBuf;
use tokio::net::UdpSocket;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot, Notify, OwnedSemaphorePermit, Semaphore};

/// App → driver commands.
pub(crate) enum Cmd {
    OpenTcp {
        first_frame: Vec<u8>,
        fast_open: bool,
        reply: oneshot::Sender<Result<DuplexStream>>,
    },
    Write {
        id: u64,
        data: Vec<u8>,
        permit: OwnedSemaphorePermit,
    },
    RegisterUdp {
        reply: oneshot::Sender<(u32, mpsc::Receiver<UdpMessage>)>,
    },
    UnregisterUdp {
        session_id: u32,
    },
    SendUdp {
        session_id: u32,
        addr: smol_str::SmolStr,
        data: Vec<u8>,
    },
}

/// Driver → `DuplexStream` read items.
pub(crate) enum ReadItem {
    Data(Vec<u8>),
    Eof,
    Err(std::io::ErrorKind),
}

/// Handle the app keeps to talk to a live connection.
pub(crate) struct ConnHandle {
    pub(crate) cmd_tx: mpsc::Sender<Cmd>,
    pub(crate) udp_enabled: bool,
    closed: Arc<AtomicBool>,
    _task: DriverTask,
}

impl ConnHandle {
    pub(crate) fn is_active(&self) -> bool {
        !self.closed.load(Ordering::Relaxed) && !self.cmd_tx.is_closed()
    }
}

/// Closes the connection and stops the driver when the handle goes away.
struct DriverTask {
    task: tokio::task::JoinHandle<()>,
    conn: quinn::Connection,
    _endpoint: quinn::Endpoint,
}

impl Drop for DriverTask {
    fn drop(&mut self) {
        self.conn.close(0u32.into(), b"done");
        self.task.abort();
    }
}

const CMD_CHANNEL_CAP: usize = 256;
const STREAM_READ_CHANNEL_CAP: usize = 16;
const UDP_SESSION_QUEUE: usize = 64;
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);
const READ_CHUNK_BYTES: usize = 16 * 1024;
/// Largest UDP datagram the socket wrapper reads.
const MAX_DATAGRAM: usize = 65535;

/// Connect, authenticate and start the driver.
pub(crate) async fn spawn(
    cfg: &Config,
    socket: UdpSocket,
    peer: SocketAddr,
    server_name: &str,
) -> Result<ConnHandle> {
    let sock = Arc::new(HySocket {
        socket,
        obfs: cfg.obfs.as_ref().map(|o| Mutex::new(Obfs::new(o))),
        hop: HopState::new(
            peer,
            &cfg.hop_ports,
            cfg.hop_interval_min_secs,
            cfg.hop_interval_max_secs,
        )?
        .map(Mutex::new),
        scratch: Mutex::new(Vec::new()),
    });
    let endpoint = quinn::Endpoint::new_with_abstract_socket(
        quinn::EndpointConfig::default(),
        None,
        sock,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(Error::Io)?;
    let conn = endpoint
        .connect_with(client_config(cfg)?, peer, server_name)
        .map_err(|e| Error::Quic(format!("connect start: {e}")))?
        .await
        .map_err(|e| Error::Quic(format!("handshake: {e}")))?;

    // The HTTP/3 control stream must stay open for the connection's life.
    let (control, udp_enabled) = match authenticate(&conn, &cfg.auth, cfg.rx_bps).await {
        Ok(v) => v,
        Err(e) => {
            conn.close(0u32.into(), b"auth");
            return Err(e);
        }
    };

    let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CHANNEL_CAP);
    let closed = Arc::new(AtomicBool::new(false));
    let driver = Driver {
        conn: conn.clone(),
        cmd_tx: cmd_tx.downgrade(),
        read_notify: Arc::new(Notify::new()),
        streams: HashMap::new(),
        next_stream_id: 0,
        udp_sessions: HashMap::new(),
        next_udp_session_id: 0,
        next_packet_id: 0,
        udp_out: Vec::new(),
        udp_dropped: 0,
    };
    let closed_task = Arc::clone(&closed);
    let task = tokio::spawn(async move {
        driver.run(cmd_rx).await;
        drop(control);
        closed_task.store(true, Ordering::Relaxed);
    });
    Ok(ConnHandle {
        cmd_tx,
        udp_enabled,
        closed,
        _task: DriverTask {
            task,
            conn,
            _endpoint: endpoint,
        },
    })
}

fn client_config(cfg: &Config) -> Result<quinn::ClientConfig> {
    use meow_transport::tls::{TlsConfig, TlsVersion};
    let pin = parse_sha256_pin(&cfg.pin_sha256)?;
    let mut tls = TlsConfig::new(cfg.server_name.clone());
    tls.alpn = vec![String::from_utf8_lossy(ALPN_H3).into_owned()];
    tls.min_version = Some(TlsVersion::Tls13);
    // A configured pin stays mandatory even with `insecure`.
    tls.skip_cert_verify = cfg.insecure && pin.is_none();
    tls.cert_pin = pin;
    let rustls = meow_transport::tls::rustls_backend::client_config(&tls)
        .map_err(|e| Error::tls(e.to_string()))?;
    let quic = quinn::crypto::rustls::QuicClientConfig::try_from(rustls)
        .map_err(|e| Error::tls(format!("QUIC TLS config: {e}")))?;
    let mut client = quinn::ClientConfig::new(Arc::new(quic));

    let w = resolve_windows(cfg.windows, meow_common::sysmem::mem_total());
    let varint = |v: u64| quinn::VarInt::from_u64(v).unwrap_or(quinn::VarInt::MAX);
    let mut t = quinn::TransportConfig::default();
    t.max_idle_timeout(MAX_IDLE_TIMEOUT.try_into().ok())
        .keep_alive_interval(Some(KEEP_ALIVE_INTERVAL))
        // quinn does not auto-tune: grant the ceilings outright.
        .stream_receive_window(varint(w.max_stream))
        .receive_window(varint(w.max_conn))
        .max_concurrent_bidi_streams(varint(MAX_CONCURRENT_STREAMS))
        .max_concurrent_uni_streams(varint(MAX_CONCURRENT_STREAMS))
        .datagram_receive_buffer_size(Some(DGRAM_QUEUE_LEN * MAX_SEND_UDP_PAYLOAD))
        .datagram_send_buffer_size(DGRAM_QUEUE_LEN * MAX_SEND_UDP_PAYLOAD)
        .congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    let mut mtu = quinn::MtuDiscoveryConfig::default();
    mtu.upper_bound(MAX_SEND_UDP_PAYLOAD as u16);
    t.mtu_discovery_config(Some(mtu));
    client.transport_config(Arc::new(t));
    Ok(client)
}

/// The HTTP/3 auth exchange (see `auth.rs`). Returns the open control stream
/// and whether the server relays UDP.
async fn authenticate(
    conn: &quinn::Connection,
    password: &str,
    rx_bps: u64,
) -> Result<(quinn::SendStream, bool)> {
    let h3 = |e: &dyn std::fmt::Display| Error::Http3(format!("auth: {e}"));
    let mut control = conn.open_uni().await.map_err(|e| h3(&e))?;
    control
        .write_all(auth::SETTINGS)
        .await
        .map_err(|e| h3(&e))?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| h3(&e))?;
    send.write_all(&auth::request(password, rx_bps)?)
        .await
        .map_err(|e| h3(&e))?;
    send.finish().map_err(|e| h3(&e))?;
    let response = recv
        .read_to_end(auth::MAX_AUTH_BYTES)
        .await
        .map_err(|e| h3(&e))?;
    Ok((control, auth::decode_response(&response)?))
}

// ─── driver ───────────────────────────────────────────────────────────────────

enum StreamMsg {
    Data(Vec<u8>, OwnedSemaphorePermit),
    Fin,
}

struct StreamEntry {
    tx: mpsc::UnboundedSender<StreamMsg>,
    shutdown: Arc<AtomicBool>,
    fin_sent: bool,
}

struct Driver {
    conn: quinn::Connection,
    cmd_tx: mpsc::WeakSender<Cmd>,
    /// Woken by `DuplexStream` on shutdown (and on reads, harmlessly).
    read_notify: Arc<Notify>,
    streams: HashMap<u64, StreamEntry>,
    next_stream_id: u64,
    udp_sessions: HashMap<u32, mpsc::Sender<UdpMessage>>,
    next_udp_session_id: u32,
    next_packet_id: u16,
    udp_out: Vec<u8>,
    udp_dropped: u64,
}

impl Driver {
    async fn run(mut self, mut cmd_rx: mpsc::Receiver<Cmd>) {
        let notify = Arc::clone(&self.read_notify);
        let conn = self.conn.clone();
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(cmd) => self.handle(cmd),
                    None => {
                        conn.close(0u32.into(), b"done");
                        break;
                    }
                },
                dgram = conn.read_datagram() => match dgram {
                    Ok(d) => self.on_datagram(&d),
                    Err(_) => break,
                },
                () = notify.notified() => {}
            }
            self.sweep();
        }
        // Dropping the senders ends every stream task (writes reset).
        self.streams.clear();
        self.udp_sessions.clear();
    }

    /// Send FIN for streams the app shut down; forget finished ones.
    fn sweep(&mut self) {
        self.streams.retain(|_, s| {
            if !s.fin_sent && s.shutdown.load(Ordering::Acquire) {
                s.fin_sent = true;
                let _ = s.tx.send(StreamMsg::Fin);
            }
            !s.tx.is_closed()
        });
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::OpenTcp {
                first_frame,
                fast_open,
                reply,
            } => {
                let Some(cmd_tx) = self.cmd_tx.upgrade() else {
                    let _ = reply.send(Err(Error::Closed));
                    return;
                };
                let id = self.next_stream_id;
                self.next_stream_id += 1;
                let (read_tx, read_rx) = mpsc::channel(STREAM_READ_CHANNEL_CAP);
                let (connected_tx, connected_rx) = oneshot::channel();
                let (tx, rx) = mpsc::unbounded_channel();
                let capacity = Arc::new(Semaphore::new(WRITE_BUFFER_BYTES as usize));
                let shutdown = Arc::new(AtomicBool::new(false));
                self.streams.insert(
                    id,
                    StreamEntry {
                        tx,
                        shutdown: Arc::clone(&shutdown),
                        fin_sent: false,
                    },
                );
                tokio::spawn(stream_task(
                    self.conn.clone(),
                    first_frame,
                    fast_open,
                    read_tx,
                    rx,
                    connected_tx,
                    Arc::clone(&capacity),
                ));
                let _ = reply.send(Ok(DuplexStream::new(
                    id,
                    read_rx,
                    Arc::clone(&self.read_notify),
                    cmd_tx,
                    capacity,
                    connected_rx,
                    shutdown,
                )));
            }
            Cmd::Write { id, data, permit } => {
                if let Some(s) = self.streams.get(&id) {
                    let _ = s.tx.send(StreamMsg::Data(data, permit));
                }
            }
            Cmd::RegisterUdp { reply } => {
                let session_id = self.next_udp_session_id;
                self.next_udp_session_id = session_id.wrapping_add(1);
                let (tx, rx) = mpsc::channel(UDP_SESSION_QUEUE);
                self.udp_sessions.insert(session_id, tx);
                let _ = reply.send((session_id, rx));
            }
            Cmd::UnregisterUdp { session_id } => {
                self.udp_sessions.remove(&session_id);
            }
            Cmd::SendUdp {
                session_id,
                addr,
                data,
            } => self.send_udp(session_id, &addr, &data),
        }
    }

    fn on_datagram(&mut self, d: &[u8]) {
        match proto::decode_udp_message(d) {
            Ok(msg) => {
                if let Some(tx) = self.udp_sessions.get(&msg.session_id) {
                    if let Err(TrySendError::Full(_)) = tx.try_send(msg) {
                        self.udp_dropped += 1;
                        if self.udp_dropped.is_power_of_two() {
                            tracing::info!(
                                dropped = self.udp_dropped,
                                "hysteria2: UDP session queue full, datagrams dropped"
                            );
                        }
                    }
                }
            }
            Err(e) => tracing::debug!("dropping malformed hysteria2 UDP datagram: {e}"),
        }
    }

    fn send_udp(&mut self, session_id: u32, addr: &str, data: &[u8]) {
        let max = self.conn.max_datagram_size().unwrap_or(0);
        let Ok(header) = proto::udp_header_len(addr) else {
            return;
        };
        let limit = max.saturating_sub(header);
        if limit == 0 {
            return;
        }
        let packet_id = self.next_packet_id;
        self.next_packet_id = packet_id.wrapping_add(1);
        let frag_count = data.len().div_ceil(limit).max(1);
        if frag_count > u8::MAX as usize {
            return;
        }
        let chunks: Box<dyn Iterator<Item = &[u8]>> = if data.is_empty() {
            Box::new(std::iter::once(data))
        } else {
            Box::new(data.chunks(limit))
        };
        for (i, chunk) in chunks.enumerate() {
            let h = proto::UdpHeader {
                session_id,
                packet_id,
                frag_id: i as u8,
                frag_count: frag_count as u8,
            };
            if proto::encode_udp_into(&mut self.udp_out, h, addr, chunk).is_ok() {
                if let Err(e) = self
                    .conn
                    .send_datagram(bytes::Bytes::copy_from_slice(&self.udp_out))
                {
                    tracing::debug!("hysteria2 could not queue UDP datagram: {e}");
                }
            }
        }
    }
}

/// One proxy stream: writes from the app, reads (after the TCP response) to it.
async fn stream_task(
    conn: quinn::Connection,
    first_frame: Vec<u8>,
    fast_open: bool,
    read_tx: mpsc::Sender<ReadItem>,
    mut write_rx: mpsc::UnboundedReceiver<StreamMsg>,
    connected: oneshot::Sender<Result<()>>,
    capacity: Arc<Semaphore>,
) {
    let (mut send, mut recv) = match conn.open_bi().await {
        Ok(v) => v,
        Err(e) => {
            capacity.close();
            let _ = connected.send(Err(Error::Quic(format!("open stream: {e}"))));
            return;
        }
    };
    if let Err(e) = send.write_all(&first_frame).await {
        capacity.close();
        let _ = connected.send(Err(Error::Quic(format!("stream send: {e}"))));
        return;
    }
    let mut connected = Some(connected);
    if fast_open {
        if let Some(c) = connected.take() {
            let _ = c.send(Ok(()));
        }
    }

    let closed_tx = read_tx.clone();
    let writer = async {
        loop {
            tokio::select! {
                msg = write_rx.recv() => match msg {
                    Some(StreamMsg::Data(data, _permit)) => {
                        if send.write_all(&data).await.is_err() {
                            capacity.close();
                            return;
                        }
                    }
                    Some(StreamMsg::Fin) => {
                        let _ = send.finish();
                        return;
                    }
                    None => break,
                },
                // The app dropped the stream without shutting it down.
                () = closed_tx.closed() => break,
            }
        }
        capacity.close();
        let _ = send.reset(0u32.into());
    };

    let reader = async {
        let fail = |connected: &mut Option<oneshot::Sender<Result<()>>>,
                    kind: io::ErrorKind,
                    err: Error| {
            match connected.take() {
                Some(c) => {
                    let _ = c.send(Err(err));
                    None
                }
                None => Some(ReadItem::Err(kind)),
            }
        };
        let mut resp = Vec::new();
        let leftover = loop {
            match recv.read_chunk(READ_CHUNK_BYTES, true).await {
                Ok(Some(chunk)) => {
                    resp.extend_from_slice(&chunk.bytes);
                    match proto::parse_tcp_response(&resp) {
                        Ok(Some(n)) => break resp.split_off(n),
                        Ok(None) => {}
                        Err(e) => {
                            if let Some(item) = fail(&mut connected, io::ErrorKind::InvalidData, e)
                            {
                                let _ = read_tx.send(item).await;
                            }
                            return;
                        }
                    }
                }
                Ok(None) => {
                    let e = Error::protocol("truncated TCP response");
                    if let Some(item) = fail(&mut connected, io::ErrorKind::UnexpectedEof, e) {
                        let _ = read_tx.send(item).await;
                    }
                    return;
                }
                Err(e) => {
                    let e = Error::Quic(format!("stream receive: {e}"));
                    if let Some(item) = fail(&mut connected, io::ErrorKind::ConnectionReset, e) {
                        let _ = read_tx.send(item).await;
                    }
                    return;
                }
            }
        };
        if let Some(c) = connected.take() {
            let _ = c.send(Ok(()));
        }
        if !leftover.is_empty() && read_tx.send(ReadItem::Data(leftover)).await.is_err() {
            let _ = recv.stop(0u32.into());
            return;
        }
        loop {
            let item = match recv.read_chunk(READ_CHUNK_BYTES, true).await {
                Ok(Some(chunk)) => ReadItem::Data(chunk.bytes.to_vec()),
                Ok(None) => ReadItem::Eof,
                Err(_) => ReadItem::Err(io::ErrorKind::ConnectionReset),
            };
            let last = !matches!(item, ReadItem::Data(_));
            if read_tx.send(item).await.is_err() {
                let _ = recv.stop(0u32.into());
                return;
            }
            if last {
                return;
            }
        }
    };
    tokio::join!(writer, reader);
}

// ─── UDP socket with obfuscation and port hopping ─────────────────────────────

struct HySocket {
    socket: UdpSocket,
    obfs: Option<Mutex<Obfs>>,
    hop: Option<Mutex<HopState>>,
    /// Receive scratch for obfuscated datagrams.
    scratch: Mutex<Vec<u8>>,
}

impl std::fmt::Debug for HySocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HySocket")
            .field("obfs", &self.obfs.is_some())
            .field("hop", &self.hop.is_some())
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug)]
struct Poller(Arc<HySocket>);

impl UdpPoller for Poller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.socket.poll_send_ready(cx)
    }
}

impl AsyncUdpSocket for HySocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(Poller(self))
    }

    fn try_send(&self, t: &Transmit<'_>) -> io::Result<()> {
        let dest = match &self.hop {
            Some(hop) => lock(hop).outgoing(),
            None => t.destination,
        };
        let seg = t.segment_size.unwrap_or(t.contents.len()).max(1);
        for packet in t.contents.chunks(seg) {
            match &self.obfs {
                Some(obfs) => {
                    let wire = lock(obfs).encode(packet);
                    for d in wire.datagrams() {
                        self.socket.try_send_to(d, dest)?;
                    }
                }
                None => {
                    self.socket.try_send_to(packet, dest)?;
                }
            }
        }
        Ok(())
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let (Some(out), Some(m)) = (bufs.first_mut(), meta.first_mut()) else {
            return Poll::Ready(Ok(0));
        };
        loop {
            let (len, from) = match &self.obfs {
                None => {
                    let mut rb = ReadBuf::new(out);
                    let from = std::task::ready!(self.socket.poll_recv_from(cx, &mut rb))?;
                    (rb.filled().len(), from)
                }
                Some(obfs) => {
                    let mut scratch = lock(&self.scratch);
                    scratch.resize(MAX_DATAGRAM, 0);
                    let mut rb = ReadBuf::new(&mut scratch);
                    let from = std::task::ready!(self.socket.poll_recv_from(cx, &mut rb))?;
                    let n = rb.filled().len();
                    let from = self.normalize(from);
                    let Some(plain) = lock(obfs).decode(&scratch[..n], from) else {
                        continue;
                    };
                    if plain.len() > out.len() {
                        continue;
                    }
                    out[..plain.len()].copy_from_slice(&plain);
                    (plain.len(), from)
                }
            };
            *m = RecvMeta {
                addr: self.normalize(from),
                len,
                stride: len,
                ecn: None,
                dst_ip: None,
            };
            return Poll::Ready(Ok(1));
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn may_fragment(&self) -> bool {
        false
    }
}

impl HySocket {
    fn normalize(&self, from: SocketAddr) -> SocketAddr {
        self.hop
            .as_ref()
            .map_or(from, |h| lock(h).normalize_source(from))
    }
}
