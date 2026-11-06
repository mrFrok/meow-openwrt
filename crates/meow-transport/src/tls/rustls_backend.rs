//! rustls (+ `ring`) TLS backend — `cfg(not(boring))`: big-endian targets,
//! which BoringSSL does not support (meow-openwrt stage 7), or any target
//! with `MEOW_TLS_BACKEND=rustls` (build.rs) for host-side testing.
//!
//! Covers plain TLS 1.2/1.3, SNI (IP literals are verified, not sent), ALPN,
//! `skip-cert-verify`, version bounds, `verify_name`, certificate pinning,
//! mTLS, extra roots, `curves` (X25519 / P-256 / P-384) and the distrusted
//! intermediates of [`super::distrust`]. Not available here: uTLS
//! fingerprint shaping (`client-fingerprint` warns once and sends rustls's
//! own ClientHello), ECH (a config error) and XTLS-Vision raw passthrough
//! (rustls reads ahead, so a switch point cannot be kept).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest as _, Sha256};
use tracing::warn;

use super::{ConnectTypedError, TlsConfig, TlsVersion, TypedConnectResult};
use crate::{Result, Stream, TransportError};

/// The stream a handshake returns.
pub type TlsStream<S> = tokio_rustls::client::TlsStream<S>;

/// A failed [`super::TlsLayer::connect_typed`] handshake: the error and, when
/// rustls hands it back, the source stream.
pub struct HandshakeError<S> {
    pub error: std::io::Error,
    pub stream: Option<S>,
}

impl<S> HandshakeError<S> {
    pub fn as_io_error(&self) -> Option<&std::io::Error> {
        Some(&self.error)
    }

    pub fn into_source_stream(self) -> Option<S> {
        self.stream
    }
}

impl<S> std::fmt::Display for HandshakeError<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

/// Cap on the per-connection ciphertext send queue (rustls's default is
/// 64 KiB); a TLS record is at most 16 KiB, so throughput is unaffected.
const SEND_BUFFER_LIMIT: usize = 16 * 1024;

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    static P: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    Arc::clone(P.get_or_init(|| Arc::new(rustls::crypto::ring::default_provider())))
}

/// The Mozilla roots as a rustls store, parsed once.
fn mozilla_store() -> &'static rustls::RootCertStore {
    static S: OnceLock<rustls::RootCertStore> = OnceLock::new();
    S.get_or_init(|| {
        let mut s = rustls::RootCertStore::empty();
        let (_, bad) =
            s.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
        if bad > 0 {
            tracing::debug!("rustls: {bad} Mozilla roots unparsable, skipped");
        }
        s
    })
}

pub(crate) fn fingerprint_is_known(_fp: &str) -> bool {
    false
}

/// Warn once per profile name: rustls cannot shape a uTLS ClientHello.
fn warn_fingerprint_once(fp: &str) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let mut seen = SEEN
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if seen.insert(fp.to_owned()) {
        warn!(
            fingerprint = fp,
            "client-fingerprint: not supported by this build's TLS backend (rustls, \
             big-endian target); the default rustls ClientHello is sent"
        );
    }
}

pub(super) fn unbracket_ip_literal(name: &str) -> &str {
    name.strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .filter(|s| s.parse::<std::net::IpAddr>().is_ok())
        .unwrap_or(name)
}

/// Key-exchange groups for a BoringSSL-style `curves` list.
fn kx_groups(curves: &str) -> Result<Vec<&'static dyn rustls::crypto::SupportedKxGroup>> {
    use rustls::crypto::ring::kx_group;
    let mut out: Vec<&'static dyn rustls::crypto::SupportedKxGroup> = Vec::new();
    for name in curves.split(':').map(str::trim).filter(|s| !s.is_empty()) {
        match name.to_ascii_lowercase().as_str() {
            "x25519" => out.push(kx_group::X25519),
            "p-256" | "prime256v1" | "secp256r1" => out.push(kx_group::SECP256R1),
            "p-384" | "secp384r1" => out.push(kx_group::SECP384R1),
            other => warn!("curves: '{other}' is not supported by the rustls backend; skipped"),
        }
    }
    if out.is_empty() {
        return Err(TransportError::Config(format!(
            "curves '{curves}': no group the rustls backend supports"
        )));
    }
    Ok(out)
}

// ─── certificate verification ─────────────────────────────────────────────────

/// Verifies like webpki, then applies meow's policy: an optional verify
/// name, an optional SHA-256 pin, and the distrusted-intermediate list.
#[derive(Debug)]
struct MeowVerifier {
    webpki: Arc<WebPkiServerVerifier>,
    verify_name: Option<ServerName<'static>>,
    pin: Option<[u8; 32]>,
    skip: bool,
}

impl MeowVerifier {
    fn check_pin(
        &self,
        pin: &[u8; 32],
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let digest = |c: &CertificateDer<'_>| -> [u8; 32] { Sha256::digest(c.as_ref()).into() };
        if digest(end_entity) == *pin {
            return Ok(ServerCertVerified::assertion());
        }
        // A pinned non-leaf certificate becomes the only root.
        let Some(root) = intermediates.iter().find(|c| digest(c) == *pin) else {
            return Err(pin_error());
        };
        let mut store = rustls::RootCertStore::empty();
        store
            .add(root.clone().into_owned())
            .map_err(|_| pin_error())?;
        let v = WebPkiServerVerifier::builder_with_provider(Arc::new(store), provider())
            .build()
            .map_err(|_| pin_error())?;
        v.verify_server_cert(end_entity, intermediates, name, &[], now)
    }
}

/// A certificate-pin failure, reported as a certificate error so callers
/// (shadow-tls v3's cover check) can tell it from a transport failure.
fn pin_error() -> rustls::Error {
    rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
}

impl ServerCertVerifier for MeowVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if self.skip {
            return Ok(ServerCertVerified::assertion());
        }
        let name = self.verify_name.as_ref().unwrap_or(server_name);
        if let Some(pin) = &self.pin {
            // A pin replaces CA validation and is not filtered (distrust.rs).
            return self.check_pin(pin, end_entity, intermediates, name, now);
        }
        let ok = self
            .webpki
            .verify_server_cert(end_entity, intermediates, name, ocsp, now)?;
        if std::iter::once(end_entity)
            .chain(intermediates)
            .any(|c| super::distrust::der_is_distrusted(c.as_ref()))
        {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::Revoked,
            ));
        }
        Ok(ok)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        if self.skip {
            return Ok(HandshakeSignatureValid::assertion());
        }
        self.webpki.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        if self.skip {
            return Ok(HandshakeSignatureValid::assertion());
        }
        self.webpki.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.webpki.supported_verify_schemes()
    }
}

// ─── client config ────────────────────────────────────────────────────────────

/// The `TlsConfig` fields that shape a shared `ClientConfig`.
#[derive(PartialEq, Eq, Hash)]
struct ConfigKey {
    skip_cert_verify: bool,
    alpn: Vec<String>,
    curves: Option<String>,
    min_version: Option<TlsVersion>,
    max_version: Option<TlsVersion>,
    verify_name: Option<String>,
    cert_pin: Option<[u8; 32]>,
}

fn versions(
    min: Option<TlsVersion>,
    max: Option<TlsVersion>,
) -> Result<Vec<&'static rustls::SupportedProtocolVersion>> {
    let mut v = Vec::new();
    let ok = |t: TlsVersion| {
        min.is_none_or(|m| m as u8 <= t as u8) && max.is_none_or(|m| t as u8 <= m as u8)
    };
    if ok(TlsVersion::Tls13) {
        v.push(&rustls::version::TLS13);
    }
    if ok(TlsVersion::Tls12) {
        v.push(&rustls::version::TLS12);
    }
    if v.is_empty() {
        return Err(TransportError::Config(
            "TLS min_version is above max_version".into(),
        ));
    }
    Ok(v)
}

fn build_config(config: &TlsConfig) -> Result<rustls::ClientConfig> {
    let mut provider = (*provider()).clone();
    if let Some(curves) = &config.curves {
        provider.kx_groups = kx_groups(curves)?;
    }
    let provider = Arc::new(provider);
    let builder = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&versions(config.min_version, config.max_version)?)
        .map_err(|e| TransportError::Tls(format!("rustls protocol versions: {e}")))?;

    let roots = if config.additional_roots.is_empty() {
        Arc::new(mozilla_store().clone())
    } else {
        let mut s = mozilla_store().clone();
        for der in &config.additional_roots {
            s.add(CertificateDer::from(der.clone())).map_err(|e| {
                TransportError::Config(format!("additional_roots: invalid CA cert: {e}"))
            })?;
        }
        Arc::new(s)
    };
    let webpki = WebPkiServerVerifier::builder_with_provider(roots, provider)
        .build()
        .map_err(|e| TransportError::Tls(format!("rustls verifier: {e}")))?;
    let verify_name = config
        .verify_name
        .as_deref()
        .map(|n| {
            ServerName::try_from(unbracket_ip_literal(n).to_owned())
                .map_err(|e| TransportError::Config(format!("verify_name '{n}': {e}")))
        })
        .transpose()?;
    let builder = builder
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(MeowVerifier {
            webpki,
            verify_name,
            pin: config.cert_pin,
            skip: config.skip_cert_verify,
        }));

    let mut tls = match &config.client_cert {
        Some(cc) => {
            use rustls::pki_types::pem::PemObject as _;
            use rustls::pki_types::PrivateKeyDer;
            let chain = CertificateDer::pem_slice_iter(cc.cert_pem.as_slice())
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| {
                    TransportError::Config(format!("client_cert.cert_pem: PEM parse error: {e}"))
                })?;
            let key = PrivateKeyDer::from_pem_slice(cc.key_pem.as_slice()).map_err(|e| {
                TransportError::Config(format!("client_cert.key_pem: PEM parse error: {e}"))
            })?;
            builder
                .with_client_auth_cert(chain, key)
                .map_err(|e| TransportError::Tls(format!("client cert setup: {e}")))?
        }
        None => builder.with_no_client_auth(),
    };
    tls.alpn_protocols = config.alpn.iter().map(|p| p.as_bytes().to_vec()).collect();
    Ok(tls)
}

/// A shared `ClientConfig` per shaping key (each owns a root-store clone);
/// configs with extra roots or a client certificate are built privately.
fn shared_config(config: &TlsConfig) -> Result<Arc<rustls::ClientConfig>> {
    static CACHE: OnceLock<Mutex<HashMap<ConfigKey, Arc<rustls::ClientConfig>>>> = OnceLock::new();
    if !config.additional_roots.is_empty() || config.client_cert.is_some() {
        return Ok(Arc::new(build_config(config)?));
    }
    let key = ConfigKey {
        skip_cert_verify: config.skip_cert_verify,
        alpn: config.alpn.clone(),
        curves: config.curves.clone(),
        min_version: config.min_version,
        max_version: config.max_version,
        verify_name: config.verify_name.clone(),
        cert_pin: config.cert_pin,
    };
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(c) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
    {
        return Ok(Arc::clone(c));
    }
    let built = Arc::new(build_config(config)?);
    let mut map = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(Arc::clone(map.entry(key).or_insert(built)))
}

/// The validated, shared rustls `ClientConfig` for `config` — what the
/// hysteria2 quinn driver hands to QUIC.
pub fn client_config(config: &TlsConfig) -> Result<Arc<rustls::ClientConfig>> {
    RustlsInner::validate(config)?;
    shared_config(config)
}

// ─── connector ────────────────────────────────────────────────────────────────

pub(super) struct RustlsInner {
    connector: tokio_rustls::TlsConnector,
    server_name: ServerName<'static>,
}

impl RustlsInner {
    pub(super) fn validate(config: &TlsConfig) -> Result<()> {
        if config.sni.is_none() {
            return Err(TransportError::Config(
                "TlsLayer requires sni to be Some; None is reserved for non-TLS paths.".into(),
            ));
        }
        if let Some(bad) = config
            .alpn
            .iter()
            .find(|p| p.is_empty() || p.len() > u8::MAX as usize)
        {
            return Err(TransportError::Config(format!(
                "alpn: protocol id {bad:?} must be 1–255 bytes (RFC 7301 §3.1)"
            )));
        }
        if config.ech.is_some() {
            return Err(TransportError::Config(
                "ech-opts: ECH is not supported by this build's TLS backend (rustls, \
                 big-endian target)"
                    .into(),
            ));
        }
        Ok(())
    }

    pub(super) fn new(config: &TlsConfig) -> Result<Self> {
        Self::validate(config)?;
        if let Some(fp) = &config.fingerprint {
            warn_fingerprint_once(fp);
        }
        let sni = config.sni.as_deref().unwrap_or_default();
        let server_name = ServerName::try_from(unbracket_ip_literal(sni).to_owned())
            .map_err(|e| TransportError::Config(format!("invalid SNI '{sni}': {e}")))?;
        Ok(Self {
            connector: tokio_rustls::TlsConnector::from(shared_config(config)?),
            server_name,
        })
    }

    pub(super) async fn connect(&self, inner: Box<dyn Stream>) -> Result<Box<dyn Stream>> {
        match self.connect_typed(inner).await {
            Ok(s) => Ok(Box::new(s)),
            Err(e) => Err(e.into_transport()),
        }
    }

    pub(super) async fn connect_typed<S>(&self, inner: S) -> TypedConnectResult<S>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        match self
            .connector
            .connect(self.server_name.clone(), inner)
            .into_fallible()
            .await
        {
            Ok(mut s) => {
                s.get_mut().1.set_buffer_limit(Some(SEND_BUFFER_LIMIT));
                Ok(s)
            }
            Err((error, stream)) => Err(ConnectTypedError::Handshake(HandshakeError {
                error,
                stream: Some(stream),
            })),
        }
    }
}

/// ALPN the server selected.
pub fn selected_alpn<S>(s: &TlsStream<S>) -> Option<&[u8]> {
    s.get_ref().1.alpn_protocol()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_follow_bounds() {
        assert_eq!(versions(None, None).unwrap().len(), 2);
        assert_eq!(
            versions(Some(TlsVersion::Tls13), None).unwrap()[0].version,
            rustls::ProtocolVersion::TLSv1_3
        );
        assert_eq!(
            versions(None, Some(TlsVersion::Tls12)).unwrap()[0].version,
            rustls::ProtocolVersion::TLSv1_2
        );
        assert!(versions(Some(TlsVersion::Tls13), Some(TlsVersion::Tls12)).is_err());
    }

    #[test]
    fn curves_map_to_groups() {
        assert_eq!(kx_groups("X25519:P-256").unwrap().len(), 2);
        assert!(kx_groups("X25519MLKEM768").is_err());
    }

    #[test]
    fn ech_is_a_config_error() {
        let mut c = TlsConfig::new("example.com");
        c.ech = Some(super::super::EchOpts::Config(vec![0]));
        assert!(RustlsInner::validate(&c).is_err());
    }
}
