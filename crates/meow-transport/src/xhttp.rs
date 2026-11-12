//! XHTTP (SplitHTTP) transport layer (`xhttp` feature).
//!
//! Tunnels bidirectional streams over HTTP/2 using the Xray-core / mihomo
//! `splithttp` (XHTTP) protocol in `stream-one` mode.
//!
//! upstream: transport/internet/splithttp/dialer.go

use std::time::Duration;

use async_trait::async_trait;
use rand::seq::IndexedRandom as _;
use rand::Rng as _;

use crate::h2_common::{H2Stream, RecvState, SharedH2, SharedH2Slot};
use crate::{Result, Stream, Transport, TransportError};

/// Timeout for acquiring send readiness (`h2.ready()`) during `connect`.
use crate::h2_common::OPEN_TIMEOUT;

/// Upper bound for `x-padding-bytes` — the padding becomes a `String` of that
/// length inside a Referer header on every connect, so an unbounded value is
/// a remotely-configurable allocation (a provider/subscription node could
/// request exabytes and abort the process at the first health check or dial).
/// 64 KiB is far beyond any real deployment (Xray ships 100–1000) while still
/// admitting every sane configuration (issue #648).
///
/// Enforced twice, deliberately: `parse_vless_xhttp_config` rejects at config
/// load (fail-fast, per-node vocabulary) and `validate_config` re-checks at
/// `connect()` (backstop for programmatic `XhttpConfig` construction).
pub const MAX_X_PADDING_BYTES: usize = 64 * 1024;

/// Same remotely-configurable memory concern as [`MAX_X_PADDING_BYTES`],
/// much smaller blast radius. Enforced at both layers like the padding cap
/// (issue #648). Defined at the crate root because `httpupgrade` shares the
/// bound and the two features compile independently.
pub use crate::MAX_EXTRA_HEADERS;

// ─── Public types ─────────────────────────────────────────────────────────────

/// Where `packet-up` carries each upload chunk.
///
/// upstream: `xhttp-opts.uplink-data-placement` (`auto` is `body` on a client).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UplinkData {
    #[default]
    Body,
    /// Base64url in `<key>-0`, `<key>-1`, … headers.
    Header,
    /// Base64url in `<key>_0`, `<key>_1`, … cookies.
    Cookie,
}

/// Where the session ID and the packet sequence number go.
///
/// upstream: `xhttp-opts.session-placement` / `seq-placement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetaPlacement {
    #[default]
    Path,
    Query,
    Header,
    Cookie,
}

/// Configuration for the XHTTP transport layer.
///
/// upstream: `xhttp-opts` YAML key block.
#[derive(Debug, Clone)]
pub struct XhttpConfig {
    /// The `:path` pseudo-header sent with every request.
    ///
    /// upstream: `xhttp-opts.path`; default `"/"`.
    pub path: String,

    /// Candidate `:authority` values. One is chosen uniformly at random per
    /// connection. The config layer resolves an omitted host from the effective
    /// TLS/REALITY server name and then the dial host.
    ///
    /// upstream: `xhttp-opts.host`.
    pub hosts: Vec<String>,

    /// HTTP scheme used for the `:scheme` pseudo-header and generated Referer.
    /// The config layer sets this to `https` when TLS/REALITY wraps XHTTP and
    /// to `http` for h2c.
    pub scheme: String,

    /// Extra custom HTTP headers sent with the request.
    /// Bounded to [`MAX_EXTRA_HEADERS`] entries — `connect()` rejects more.
    ///
    /// upstream: `xhttp-opts.headers`.
    pub extra_headers: Vec<(String, String)>,

    /// XHTTP mode: `stream-one`, `stream-up` or `packet-up` (the config
    /// layer resolves `auto`).
    ///
    /// upstream: `xhttp-opts.mode`.
    pub mode: String,

    /// If true, suppress setting the `Content-Type: application/grpc` header.
    /// Default is `false` (the header is set by default per Xray-core spec).
    ///
    /// upstream: `xhttp-opts.no-grpc-header`.
    pub no_grpc_header: bool,

    /// Range for random padding bytes `(min, max)`.
    /// When set and `max > 0`, a random padding string of length between `min`
    /// and `max` is generated and sent in the `Referer` header. The generated
    /// Referer replaces any request query with `x_padding=<padding>`, matching
    /// Xray's `queryInHeader` placement.
    ///
    /// upstream: `xhttp-opts.x-padding-bytes`; default `Some((100, 1000))`.
    /// `max` must not exceed [`MAX_X_PADDING_BYTES`].
    pub x_padding_bytes: Option<(usize, usize)>,

    /// Method of the upload requests (`stream-up`, `packet-up`); `GET` only
    /// in `packet-up`. upstream: `xhttp-opts.uplink-http-method`.
    pub uplink_method: String,
    /// upstream: `xhttp-opts.uplink-data-placement`.
    pub uplink_data: UplinkData,
    /// Header / cookie name prefix for [`UplinkData::Header`] /
    /// [`UplinkData::Cookie`]; empty means `X-Data` / `x_data`.
    /// upstream: `xhttp-opts.uplink-data-key`.
    pub uplink_data_key: String,
    /// Base64 characters per header or cookie; `None` means 3000–4000 for
    /// headers and 2048–3072 for cookies. upstream: `uplink-chunk-size`.
    pub uplink_chunk_size: Option<(usize, usize)>,
    /// Payload bytes per `packet-up` request.
    /// upstream: `sc-max-each-post-bytes`; default 1 000 000.
    pub max_each_post_bytes: (usize, usize),
    /// Minimum gap between two `packet-up` requests.
    /// upstream: `sc-min-posts-interval-ms`; default 30.
    pub min_posts_interval_ms: (u64, u64),
    /// upstream: `session-placement` / `session-key` (empty: `X-Session` or
    /// `x_session`).
    pub session_placement: MetaPlacement,
    pub session_key: String,
    /// upstream: `seq-placement` / `seq-key` (empty: `X-Seq` or `x_seq`).
    pub seq_placement: MetaPlacement,
    pub seq_key: String,
}

impl Default for XhttpConfig {
    fn default() -> Self {
        Self {
            path: "/".into(),
            hosts: vec!["localhost".into()],
            scheme: "https".into(),
            extra_headers: Vec::new(),
            mode: "stream-one".into(),
            no_grpc_header: false,
            x_padding_bytes: Some((100, 1000)),
            uplink_method: "POST".into(),
            uplink_data: UplinkData::Body,
            uplink_data_key: String::new(),
            uplink_chunk_size: None,
            max_each_post_bytes: (1_000_000, 1_000_000),
            min_posts_interval_ms: (30, 30),
            session_placement: MetaPlacement::Path,
            session_key: String::new(),
            seq_placement: MetaPlacement::Path,
            seq_key: String::new(),
        }
    }
}

/// Largest `sc-max-each-post-bytes` accepted: each request's payload is held
/// in memory until sent (Xray's default is 1 000 000).
pub const MAX_EACH_POST_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    StreamOne,
    StreamUp,
    PacketUp,
}

fn parse_mode(mode: &str) -> Option<Mode> {
    match mode.to_ascii_lowercase().as_str() {
        "stream-one" => Some(Mode::StreamOne),
        "stream-up" => Some(Mode::StreamUp),
        "packet-up" => Some(Mode::PacketUp),
        _ => None,
    }
}

fn rand_in<T: rand::distr::uniform::SampleUniform + PartialOrd + Copy>((min, max): (T, T)) -> T {
    if min >= max {
        min
    } else {
        rand::rng().random_range(min..=max)
    }
}

/// Session ID: 16 random bytes in hex, as Xray/mihomo clients send by default.
fn session_id() -> String {
    use std::fmt::Write as _;
    let bytes: [u8; 16] = rand::random();
    let mut id = String::with_capacity(32);
    for b in bytes {
        let _ = write!(id, "{b:02x}");
    }
    id
}

/// What one request carries besides the configured headers.
#[derive(Default)]
struct Extra<'a> {
    session: Option<&'a str>,
    seq: Option<u64>,
    /// `Content-Type: application/grpc` (stream-one / stream-up uploads).
    grpc: bool,
    /// packet-up payload placed in headers or cookies.
    data: Option<&'a [u8]>,
}

// ─── XhttpLayer ───────────────────────────────────────────────────────────────

/// Transport layer that wraps an inner stream with an XHTTP tunnel.
pub struct XhttpLayer {
    config: std::sync::Arc<XhttpConfig>,
    /// Tunnels share one h2 connection, as Xray's xmux does
    /// (meow-openwrt): no new TCP + TLS/REALITY handshake per tunnel.
    session: SharedH2Slot,
}

impl XhttpLayer {
    /// Create an `XhttpLayer` from the given configuration.
    pub fn new(config: XhttpConfig) -> Self {
        Self {
            config: std::sync::Arc::new(config),
            session: SharedH2Slot::default(),
        }
    }

    /// Open one tunnel on `session`.
    async fn open(&self, session: &SharedH2, wait: bool) -> Result<Box<dyn Stream>> {
        validate_config(&self.config)?;
        let mode = parse_mode(&self.config.mode).unwrap_or(Mode::StreamOne);
        let open = async {
            match mode {
                Mode::StreamOne => self.open_stream_one(session, wait).await,
                Mode::StreamUp => self.open_stream_up(session, wait).await,
                Mode::PacketUp => self.open_packet_up(session, wait).await,
            }
        };
        if wait {
            tokio::time::timeout(OPEN_TIMEOUT, open)
                .await
                .map_err(|_| {
                    TransportError::Xhttp("timed out waiting for send readiness (open)".into())
                })?
        } else {
            open.await
        }
    }

    fn recv(response: h2::client::ResponseFuture) -> RecvState {
        RecvState::with_timeout(
            response,
            Duration::from_secs(15),
            crate::h2_common::StatusPolicy::Success,
            "xhttp",
        )
    }

    async fn open_stream_one(&self, session: &SharedH2, wait: bool) -> Result<Box<dyn Stream>> {
        let request = request(
            &self.config,
            http::Method::POST,
            &Extra {
                grpc: true,
                ..Extra::default()
            },
        )?;
        let (response, send) = session
            .send_request(request, wait, TransportError::Xhttp)
            .await?;
        // Do NOT await the response here: upstream's handler reads the
        // client's first DATA frame before it writes response headers.
        // The response is resolved lazily on the first read via RecvState.
        Ok(Box::new(
            H2Stream::new(send, Self::recv(response)).with_shared_driver(session.lease()),
        ))
    }

    /// The download half of stream-up / packet-up: `GET <path>/<session>`.
    async fn open_download(&self, session: &SharedH2, wait: bool, id: &str) -> Result<H2Stream> {
        let request = request(
            &self.config,
            http::Method::GET,
            &Extra {
                session: Some(id),
                ..Extra::default()
            },
        )?;
        let (response, send) = session
            .send_request_end(request, wait, true, TransportError::Xhttp)
            .await?;
        Ok(H2Stream::new(send, Self::recv(response)).with_shared_driver(session.lease()))
    }

    /// stream-up: the download GET plus one streaming upload request.
    async fn open_stream_up(&self, session: &SharedH2, wait: bool) -> Result<Box<dyn Stream>> {
        let id = session_id();
        let download = self.open_download(session, wait, &id).await?;
        let method = uplink_method(&self.config)?;
        let request = request(
            &self.config,
            method,
            &Extra {
                session: Some(&id),
                grpc: true,
                ..Extra::default()
            },
        )?;
        let (response, send) = session
            .send_request(request, wait, TransportError::Xhttp)
            .await?;
        // The upload's response only carries the server's keep-alive
        // padding; read it so its window keeps moving.
        let lease = session.lease();
        tokio::spawn(async move {
            RecvState::new(response).drain().await;
            drop(lease);
        });
        Ok(Box::new(SplitStream {
            download,
            upload: Upload::Stream(H2Stream::send_only(send)),
        }))
    }

    /// packet-up: the download GET; uploads are numbered requests sent by a
    /// task fed through a bounded buffer.
    async fn open_packet_up(&self, session: &SharedH2, wait: bool) -> Result<Box<dyn Stream>> {
        let id = session_id();
        let download = self.open_download(session, wait, &id).await?;
        let max = rand_in(self.config.max_each_post_bytes).max(1);
        let pipe = std::sync::Arc::new(Pipe::new(max));
        tokio::spawn(upload_packets(
            std::sync::Arc::clone(&self.config),
            session.clone(),
            session.lease(),
            id,
            std::sync::Arc::clone(&pipe),
            max,
        ));
        Ok(Box::new(SplitStream {
            download,
            upload: Upload::Packets(PipeWriter(pipe)),
        }))
    }
}

#[async_trait]
impl Transport for XhttpLayer {
    async fn reuse(&self) -> Option<Box<dyn Stream>> {
        let session = self.session.live().await?;
        match self.open(&session, false).await {
            Ok(stream) => Some(stream),
            Err(_) => {
                self.session.forget(&session);
                None
            }
        }
    }

    async fn connect(&self, inner: Box<dyn Stream>) -> Result<Box<dyn Stream>> {
        self.dial(inner, false).await
    }

    async fn connect_shared(&self, inner: Box<dyn Stream>) -> Result<Box<dyn Stream>> {
        // Single-flight on a slot miss: without the gate two concurrent
        // misses each dial and `set` last-writer-wins, orphaning a conn.
        // The second caller re-checks `reuse` behind the gate and opens a
        // stream on the winner's connection instead (`MuxClient::offer`
        // holds its pool lock across the dial the same way). Held across
        // the dial, so it is a tokio Mutex; plain `connect` never takes it.
        let _gate = self.session.dial.lock().await;
        if let Some(stream) = self.reuse().await {
            return Ok(stream);
        }
        self.dial(inner, true).await
    }

    fn reset(&self) {
        self.session.clear();
    }
}

impl XhttpLayer {
    /// Open the h2 connection and its first tunnel; `share` keeps the
    /// connection for [`Transport::reuse`].
    async fn dial(&self, inner: Box<dyn Stream>, share: bool) -> Result<Box<dyn Stream>> {
        // Configuration errors surface before the h2 handshake.
        validate_config(&self.config)?;
        request(&self.config, http::Method::POST, &Extra::default())?;

        // A `reset` landing mid-dial invalidates the connection being set
        // up — its socket may predate a new outbound binding (issue #695)
        // — so `set` discards it when the generation captured here has
        // moved on (`MuxClient::offer_new_locked` drops a straddling
        // session the same way).
        let generation = self.session.generation();

        // Proxy-sized receive windows — see `h2_common::client_builder`.
        // Bounded: `connect_shared` callers hold the slot's dial gate, so
        // the handshake needs its own ceiling independent of the caller's
        // dial deadline.
        let (h2, conn) = tokio::time::timeout(
            OPEN_TIMEOUT,
            crate::h2_common::client_builder().handshake::<_, bytes::Bytes>(inner),
        )
        .await
        .map_err(|_| TransportError::Xhttp("timed out in h2 handshake".into()))?
        .map_err(|e| crate::h2_common::h2_to_transport(e, TransportError::Xhttp))?;

        // A failed first stream drops `session`, whose driver aborts
        // (issue #669); only a working connection is offered for reuse.
        let session = SharedH2::spawn(h2, conn);
        let stream = self.open(&session, true).await?;
        if share {
            if let Some(generation) = generation {
                self.session.set(session, generation);
            }
        }
        Ok(stream)
    }
}

fn uplink_method(config: &XhttpConfig) -> Result<http::Method> {
    http::Method::from_bytes(config.uplink_method.to_ascii_uppercase().as_bytes()).map_err(|_| {
        TransportError::Config(format!(
            "xhttp: invalid uplink-http-method {:?}",
            config.uplink_method
        ))
    })
}

/// Build one request: path, session/seq placement, padding Referer, and the
/// packet-up payload when it rides in headers or cookies.
fn request(
    config: &XhttpConfig,
    method: http::Method,
    extra: &Extra<'_>,
) -> Result<http::Request<()>> {
    let host = config
        .hosts
        .choose(&mut rand::rng())
        .cloned()
        .unwrap_or_else(|| "localhost".to_string());
    let authority = format_authority(&host);
    let normalized = normalize_path(&config.path);
    let (base, query) = normalized
        .split_once('?')
        .map_or((normalized.as_str(), None), |(p, q)| (p, Some(q)));

    let mut path = base.to_string();
    let mut query: Vec<String> = query.map(|q| vec![q.to_string()]).unwrap_or_default();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut cookies: Vec<String> = Vec::new();
    let mut place =
        |placement: MetaPlacement, key: &str, header: &str, cookie: &str, value: &str| {
            match placement {
                MetaPlacement::Path => {
                    if !path.ends_with('/') {
                        path.push('/');
                    }
                    path.push_str(value);
                }
                MetaPlacement::Query => query.push(format!("{}={value}", or(key, cookie))),
                MetaPlacement::Header => {
                    headers.push((or(key, header).to_string(), value.to_string()));
                }
                MetaPlacement::Cookie => cookies.push(format!("{}={value}", or(key, cookie))),
            }
        };
    if let Some(id) = extra.session {
        place(
            config.session_placement,
            &config.session_key,
            "X-Session",
            "x_session",
            id,
        );
    }
    if let Some(seq) = extra.seq {
        place(
            config.seq_placement,
            &config.seq_key,
            "X-Seq",
            "x_seq",
            &seq.to_string(),
        );
    }
    if let Some(data) = extra.data {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data);
        let (key, default_chunk) = match config.uplink_data {
            UplinkData::Cookie => (or(&config.uplink_data_key, "x_data"), (2048, 3072)),
            _ => (or(&config.uplink_data_key, "X-Data"), (3000, 4000)),
        };
        let chunk = config
            .uplink_chunk_size
            .filter(|&(_, max)| max > 0)
            .map_or(default_chunk, |(min, max)| (min.max(64), max.max(64)));
        let mut rest = encoded.as_str();
        let mut i = 0;
        while !rest.is_empty() {
            let (part, tail) = rest.split_at(rand_in(chunk).min(rest.len()));
            rest = tail;
            if config.uplink_data == UplinkData::Cookie {
                cookies.push(format!("{key}_{i}={part}"));
            } else {
                headers.push((format!("{key}-{i}"), part.to_string()));
            }
            i += 1;
        }
    }

    let mut uri = format!("{}://{authority}{path}", config.scheme);
    if !query.is_empty() {
        uri.push('?');
        uri.push_str(&query.join("&"));
    }
    let mut builder = http::Request::builder().method(method).uri(uri);

    let has = |name: &str| {
        config
            .extra_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(name))
    };
    if extra.grpc && !config.no_grpc_header && !has("content-type") {
        builder = builder.header("content-type", "application/grpc");
    }
    for (k, v) in &config.extra_headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    // Xray places default padding in a Referer query so it is not exposed
    // as an unusual request-URI query parameter.
    if !has("referer") {
        if let Some(range) = config.x_padding_bytes.filter(|&(_, max)| max > 0) {
            let pad_len = rand_in(range);
            if pad_len > 0 {
                let padding = "X".repeat(pad_len);
                let referer = format!("{}://{authority}{base}?x_padding={padding}", config.scheme);
                builder = builder.header("referer", referer);
            }
        }
    }
    for (k, v) in headers {
        builder = builder.header(k, v);
    }
    if !cookies.is_empty() {
        builder = builder.header("cookie", cookies.join("; "));
    }

    builder
        .body(())
        .map_err(|e| TransportError::Config(format!("xhttp: invalid request config: {e}")))
}

fn or<'a>(value: &'a str, default: &'a str) -> &'a str {
    if value.is_empty() {
        default
    } else {
        value
    }
}

// ─── stream-up / packet-up streams ───────────────────────────────────────────

enum Upload {
    Stream(H2Stream),
    Packets(PipeWriter),
}

/// A tunnel whose two directions are separate requests.
struct SplitStream {
    download: H2Stream,
    upload: Upload,
}

impl tokio::io::AsyncRead for SplitStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.download).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for SplitStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match &mut self.upload {
            Upload::Stream(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            Upload::Packets(p) => p.poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut self.upload {
            Upload::Stream(s) => std::pin::Pin::new(s).poll_flush(cx),
            Upload::Packets(_) => std::task::Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut self.upload {
            Upload::Stream(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            Upload::Packets(p) => {
                p.0.close();
                std::task::Poll::Ready(Ok(()))
            }
        }
    }
}

#[derive(Default)]
struct PipeState {
    buf: bytes::BytesMut,
    closed: bool,
    failed: bool,
    writer: Option<std::task::Waker>,
}

/// Bounded byte buffer between a packet-up tunnel and its upload task.
struct Pipe {
    state: std::sync::Mutex<PipeState>,
    data: tokio::sync::Notify,
    cap: usize,
}

impl Pipe {
    fn new(cap: usize) -> Self {
        Self {
            state: std::sync::Mutex::default(),
            data: tokio::sync::Notify::new(),
            cap,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PipeState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn close(&self) {
        self.lock().closed = true;
        self.data.notify_one();
    }

    fn fail(&self) {
        let mut state = self.lock();
        state.failed = true;
        if let Some(waker) = state.writer.take() {
            waker.wake();
        }
    }

    /// Wait for data, then take up to `max` bytes; `None` once closed and
    /// empty.
    async fn take(&self, max: usize) -> Option<bytes::Bytes> {
        loop {
            if let Some(chunk) = take_now(self, max) {
                return Some(chunk);
            }
            {
                let state = self.lock();
                if state.buf.is_empty() && (state.closed || state.failed) {
                    return None;
                }
            }
            self.data.notified().await;
        }
    }
}

struct PipeWriter(std::sync::Arc<Pipe>);

impl PipeWriter {
    fn poll_write(
        &self,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if buf.is_empty() {
            return std::task::Poll::Ready(Ok(0));
        }
        let mut state = self.0.lock();
        if state.failed || state.closed {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "xhttp: packet-up upload failed",
            )));
        }
        let room = self.0.cap.saturating_sub(state.buf.len());
        if room == 0 {
            state.writer = Some(cx.waker().clone());
            return std::task::Poll::Pending;
        }
        let n = room.min(buf.len());
        state.buf.extend_from_slice(&buf[..n]);
        drop(state);
        self.0.data.notify_one();
        std::task::Poll::Ready(Ok(n))
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// Upper bound on the payload of packet-up requests in flight at once.
const PACKETS_IN_FLIGHT_BYTES: usize = 256 * 1024;

/// The packet-up upload task: numbered requests, at most one per
/// `sc-min-posts-interval-ms`, several in flight (the server reorders them
/// by sequence number).
async fn upload_packets(
    config: std::sync::Arc<XhttpConfig>,
    session: SharedH2,
    _lease: crate::h2_common::StreamLease,
    id: String,
    pipe: std::sync::Arc<Pipe>,
    max: usize,
) {
    let slots = (PACKETS_IN_FLIGHT_BYTES / max).clamp(1, 16);
    let in_flight = std::sync::Arc::new(tokio::sync::Semaphore::new(slots));
    let Ok(method) = uplink_method(&config) else {
        pipe.fail();
        return;
    };
    let mut last: Option<tokio::time::Instant> = None;
    for seq in 0u64.. {
        // Wait for data, keep the minimum gap, then send whatever is
        // buffered by then, up to `max` (Xray's dialer does the same).
        let Some(first) = pipe.take(max).await else {
            break;
        };
        last = wait_gap(&config, last).await;
        let chunk = match take_now(&pipe, max - first.len()) {
            Some(more) => {
                let mut joined = bytes::BytesMut::with_capacity(first.len() + more.len());
                joined.extend_from_slice(&first);
                joined.extend_from_slice(&more);
                joined.freeze()
            }
            None => first,
        };
        if !send_packet(
            &config, &session, &method, &id, seq, chunk, &in_flight, &pipe,
        )
        .await
        {
            return;
        }
    }
    // Let the last requests finish before the lease goes.
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        in_flight.acquire_many(u32::try_from(slots).unwrap_or(1)),
    )
    .await;
}

/// Sleep out the rest of `sc-min-posts-interval-ms` since the last request;
/// returns the new "last request" time.
async fn wait_gap(
    config: &XhttpConfig,
    last: Option<tokio::time::Instant>,
) -> Option<tokio::time::Instant> {
    if let Some(last) = last {
        let gap = Duration::from_millis(rand_in(config.min_posts_interval_ms));
        tokio::time::sleep_until(last + gap).await;
    }
    Some(tokio::time::Instant::now())
}

fn take_now(pipe: &Pipe, max: usize) -> Option<bytes::Bytes> {
    let mut state = pipe.lock();
    if state.buf.is_empty() || max == 0 {
        return None;
    }
    let n = state.buf.len().min(max);
    let chunk = state.buf.split_to(n).freeze();
    if let Some(waker) = state.writer.take() {
        waker.wake();
    }
    Some(chunk)
}

#[allow(
    clippy::too_many_arguments,
    reason = "one call site, all per-tunnel state"
)]
async fn send_packet(
    config: &std::sync::Arc<XhttpConfig>,
    session: &SharedH2,
    method: &http::Method,
    id: &str,
    seq: u64,
    chunk: bytes::Bytes,
    in_flight: &std::sync::Arc<tokio::sync::Semaphore>,
    pipe: &std::sync::Arc<Pipe>,
) -> bool {
    let Ok(permit) = std::sync::Arc::clone(in_flight).acquire_owned().await else {
        return false;
    };
    let in_body = config.uplink_data == UplinkData::Body;
    let extra = Extra {
        session: Some(id),
        seq: Some(seq),
        data: (!in_body).then_some(chunk.as_ref()),
        ..Extra::default()
    };
    let Ok(request) = request(config, method.clone(), &extra) else {
        pipe.fail();
        return false;
    };
    let sent = session
        .send_request_end(request, true, !in_body, TransportError::Xhttp)
        .await;
    let Ok((response, mut send)) = sent else {
        pipe.fail();
        return false;
    };
    if in_body && send.send_data(chunk, true).is_err() {
        pipe.fail();
        return false;
    }
    let pipe = std::sync::Arc::clone(pipe);
    tokio::spawn(async move {
        let ok = match tokio::time::timeout(Duration::from_secs(30), response).await {
            Ok(Ok(response)) if response.status() == http::StatusCode::OK => {
                let mut body = response.into_body();
                let _ = tokio::time::timeout(Duration::from_secs(10), async {
                    while let Some(Ok(data)) = body.data().await {
                        let _ = body.flow_control().release_capacity(data.len());
                    }
                })
                .await;
                true
            }
            _ => false,
        };
        if !ok {
            pipe.fail();
        }
        drop(permit);
    });
    true
}

// ─── Validation Helpers ───────────────────────────────────────────────────────

// Xray normalizes the base path with a trailing slash before matching it,
// including stream-one requests that carry no session ID in the path.
fn normalize_path(path_and_query: &str) -> String {
    let (path, query) = path_and_query
        .split_once('?')
        .map_or((path_and_query, None), |(path, query)| (path, Some(query)));
    let mut normalized = path.to_string();
    if !normalized.ends_with('/') {
        normalized.push('/');
    }
    if let Some(query) = query {
        normalized.push('?');
        normalized.push_str(query);
    }
    normalized
}

fn validate_config(config: &XhttpConfig) -> Result<()> {
    validate_path(&config.path)?;
    if config.hosts.is_empty() {
        return Err(TransportError::Config(
            "xhttp: hosts must not be empty".into(),
        ));
    }
    for host in &config.hosts {
        validate_host(host)?;
    }
    // Structural bound before the per-entry scan — an oversized list bails
    // without paying the O(n) byte checks first.
    if config.extra_headers.len() > MAX_EXTRA_HEADERS {
        return Err(TransportError::Config(format!(
            "xhttp: too many extra headers ({}, max {MAX_EXTRA_HEADERS})",
            config.extra_headers.len()
        )));
    }
    for (name, value) in &config.extra_headers {
        validate_header_name(name)?;
        validate_header_value(name, value)?;
    }
    let Some(mode) = parse_mode(&config.mode) else {
        return Err(TransportError::Config(format!(
            "xhttp: unsupported mode {:?}; expected stream-one, stream-up or packet-up",
            config.mode
        )));
    };
    let method = uplink_method(config)?;
    if mode != Mode::PacketUp {
        if method == http::Method::GET {
            return Err(TransportError::Config(
                "xhttp: uplink-http-method GET works only in packet-up".into(),
            ));
        }
        if config.uplink_data != UplinkData::Body {
            return Err(TransportError::Config(
                "xhttp: uplink-data-placement header/cookie works only in packet-up".into(),
            ));
        }
    }
    let (min, max) = config.max_each_post_bytes;
    if max == 0 || min > max || max > MAX_EACH_POST_BYTES {
        return Err(TransportError::Config(format!(
            "xhttp: sc-max-each-post-bytes must be 1..={MAX_EACH_POST_BYTES} (min <= max)"
        )));
    }
    let (min, max) = config.min_posts_interval_ms;
    if min > max || max > 60_000 {
        return Err(TransportError::Config(
            "xhttp: sc-min-posts-interval-ms must be 0..=60000 (min <= max)".into(),
        ));
    }
    if let Some((min, max)) = config.uplink_chunk_size {
        if min > max {
            return Err(TransportError::Config(
                "xhttp: uplink-chunk-size min exceeds max".into(),
            ));
        }
    }
    for key in [
        &config.uplink_data_key,
        &config.session_key,
        &config.seq_key,
    ] {
        if !key.bytes().all(is_header_token_byte) {
            return Err(TransportError::Config(format!(
                "xhttp: invalid key {key:?}"
            )));
        }
    }
    validate_scheme(&config.scheme)?;
    if let Some((min, max)) = config.x_padding_bytes {
        if min > max {
            return Err(TransportError::Config(format!(
                "xhttp: invalid x_padding_bytes: min ({min}) cannot exceed max ({max})"
            )));
        }
        if max > MAX_X_PADDING_BYTES {
            return Err(TransportError::Config(format!(
                "xhttp: x_padding_bytes max ({max}) exceeds {MAX_X_PADDING_BYTES}"
            )));
        }
    }
    Ok(())
}

fn validate_scheme(scheme: &str) -> Result<()> {
    if matches!(scheme, "http" | "https") {
        Ok(())
    } else {
        Err(TransportError::Config(format!(
            "xhttp: unsupported scheme {scheme:?}; expected 'http' or 'https'"
        )))
    }
}

fn format_authority(host: &str) -> String {
    if host.starts_with('[') || !host.contains(':') {
        host.to_string()
    } else if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn validate_path(path: &str) -> Result<()> {
    if !path.starts_with('/') {
        return Err(TransportError::Config(
            "xhttp: path must start with '/'".into(),
        ));
    }
    if path.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(TransportError::Config(
            "xhttp: path contains whitespace or control bytes".into(),
        ));
    }
    Ok(())
}

fn validate_host(host: &str) -> Result<()> {
    if host.is_empty() || host.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(TransportError::Config(
            "xhttp: host contains whitespace or control bytes".into(),
        ));
    }
    Ok(())
}

fn validate_header_name(name: &str) -> Result<()> {
    if name.is_empty() || !name.bytes().all(is_header_token_byte) {
        return Err(TransportError::Config(format!(
            "xhttp: invalid extra header name {name:?}"
        )));
    }
    Ok(())
}

fn validate_header_value(name: &str, value: &str) -> Result<()> {
    if value.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0)) {
        return Err(TransportError::Config(format!(
            "xhttp: invalid value for extra header {name:?}"
        )));
    }
    Ok(())
}

fn is_header_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_valid() {
        let config = XhttpConfig::default();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn invalid_path() {
        let config = XhttpConfig {
            path: "no_slash".into(),
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());

        let config = XhttpConfig {
            path: "/has space".into(),
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn empty_hosts_rejected() {
        let config = XhttpConfig {
            hosts: vec![],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn invalid_mode() {
        for mode in ["auto", "", "packet-down"] {
            let config = XhttpConfig {
                mode: mode.into(),
                ..Default::default()
            };
            assert!(validate_config(&config).is_err(), "mode {mode:?}");
        }

        let config = XhttpConfig {
            mode: "stream-one".into(),
            ..Default::default()
        };
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn validates_scheme_and_formats_ipv6_authority() {
        let config = XhttpConfig {
            scheme: "ftp".into(),
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
        assert_eq!(format_authority("2001:db8::1"), "[2001:db8::1]");
        assert_eq!(format_authority("[2001:db8::1]"), "[2001:db8::1]");
        assert_eq!(format_authority("example.com"), "example.com");
    }

    #[test]
    fn invalid_padding_range() {
        let config = XhttpConfig {
            x_padding_bytes: Some((500, 100)),
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());

        let config = XhttpConfig {
            x_padding_bytes: Some((100, 500)),
            ..Default::default()
        };
        assert!(validate_config(&config).is_ok());
    }
    #[test]
    fn padding_and_header_count_bounded() {
        // An over-cap padding range is a remotely-configurable allocation
        // (issue #648) — reject at validation, before `connect` allocates.
        let config = XhttpConfig {
            x_padding_bytes: Some((0, MAX_X_PADDING_BYTES + 1)),
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());

        let config = XhttpConfig {
            x_padding_bytes: Some((0, MAX_X_PADDING_BYTES)),
            ..Default::default()
        };
        assert!(validate_config(&config).is_ok());

        // Inverted and disable ranges.
        let config = XhttpConfig {
            x_padding_bytes: Some((200, 100)),
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
        let config = XhttpConfig {
            x_padding_bytes: Some((0, 0)),
            ..Default::default()
        };
        assert!(validate_config(&config).is_ok());

        let config = XhttpConfig {
            extra_headers: (0..=MAX_EXTRA_HEADERS)
                .map(|i| (format!("X-H{i}"), "v".into()))
                .collect(),
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn invalid_headers() {
        let mut config = XhttpConfig::default();
        config.extra_headers.push(("Bad:Name".into(), "val".into()));
        assert!(validate_config(&config).is_err());

        config.extra_headers.clear();
        config
            .extra_headers
            .push(("Good-Name".into(), "val\r\nbad".into()));
        assert!(validate_config(&config).is_err());

        config.extra_headers.clear();
        config
            .extra_headers
            .push(("Good-Name".into(), "good-value".into()));
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn packet_request_places_session_seq_and_data() {
        let config = XhttpConfig {
            path: "/xh?ed=1".into(),
            hosts: vec!["h.test".into()],
            mode: "packet-up".into(),
            uplink_method: "GET".into(),
            uplink_data: UplinkData::Header,
            uplink_chunk_size: Some((64, 64)),
            session_placement: MetaPlacement::Query,
            seq_placement: MetaPlacement::Header,
            ..Default::default()
        };
        let data = [7u8; 100];
        let req = request(
            &config,
            http::Method::GET,
            &Extra {
                session: Some("abc"),
                seq: Some(5),
                data: Some(&data),
                ..Extra::default()
            },
        )
        .expect("request");
        assert_eq!(
            req.uri().to_string(),
            "https://h.test/xh/?ed=1&x_session=abc"
        );
        assert_eq!(req.headers()["x-seq"], "5");
        assert_eq!(req.headers()["x-data-0"].len(), 64);
        assert_eq!(req.headers()["x-data-2"].len(), 134 - 128);
        assert!(req.headers().get("content-type").is_none());

        let path = XhttpConfig {
            path: "/xh".into(),
            ..Default::default()
        };
        let req = request(
            &path,
            http::Method::POST,
            &Extra {
                session: Some("abc"),
                seq: Some(0),
                ..Extra::default()
            },
        )
        .expect("request");
        assert_eq!(req.uri().path(), "/xh/abc/0");
    }
}
