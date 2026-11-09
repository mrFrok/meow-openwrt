//! Crypto primitives for every crate that needs them outside a TLS stack
//! (QUIC Initial sniffing, the Snell/VMess/VLESS-encryption record layer,
//! REALITY, ShadowTLS).
//!
//! Two backends, picked at build time (meow-openwrt stage 7):
//!
//! - `cfg(boring)` (`tls` or `boring-crypto` on a little-endian target, see
//!   build.rs): BoringSSL, the library the TLS layer already links; its
//!   assembly keeps the record layers fast under the release profile's
//!   `opt-level = "z"` (issue #659).
//! - otherwise RustCrypto, for targets BoringSSL does not support
//!   (big-endian MIPS).

#[cfg(not(boring))]
use aes_gcm::aead::AeadInPlace as _;

/// AEAD algorithms used here; all take a 12-byte nonce and a 16-byte tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeadAlg {
    Aes128Gcm,
    Aes256Gcm,
    ChaCha20Poly1305,
}

impl AeadAlg {
    pub const fn key_len(self) -> usize {
        match self {
            Self::Aes128Gcm => 16,
            Self::Aes256Gcm | Self::ChaCha20Poly1305 => 32,
        }
    }
}

/// Nonce length of every algorithm here.
pub const NONCE_LEN: usize = 12;
/// Tag length of every algorithm here.
pub const TAG_LEN: usize = 16;

/// A tag that does not authenticate, a wrong-length key or a buffer the
/// cipher refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CryptoError;

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("crypto operation failed")
    }
}

impl std::error::Error for CryptoError {}

/// One key's AEAD context.
pub struct Aead(Inner);

#[cfg(boring)]
type Inner = boring::aead::AeadCtx;

#[cfg(not(boring))]
enum Inner {
    Aes128Gcm(Box<aes_gcm::Aes128Gcm>),
    Aes256Gcm(Box<aes_gcm::Aes256Gcm>),
    ChaCha20Poly1305(Box<chacha20poly1305::ChaCha20Poly1305>),
}

impl Aead {
    /// Errors on a key whose length does not match `alg`.
    pub fn new(alg: AeadAlg, key: &[u8]) -> Result<Self, CryptoError> {
        if key.len() != alg.key_len() {
            return Err(CryptoError);
        }
        #[cfg(boring)]
        {
            use boring::aead::Algorithm;
            let a = match alg {
                AeadAlg::Aes128Gcm => Algorithm::aes_128_gcm(),
                AeadAlg::Aes256Gcm => Algorithm::aes_256_gcm(),
                AeadAlg::ChaCha20Poly1305 => Algorithm::chacha20_poly1305(),
            };
            boring::aead::AeadCtx::new_default_tag(&a, key)
                .map(Self)
                .map_err(|_| CryptoError)
        }
        #[cfg(not(boring))]
        {
            use aes_gcm::KeyInit as _;
            Ok(Self(match alg {
                AeadAlg::Aes128Gcm => Inner::Aes128Gcm(Box::new(
                    aes_gcm::Aes128Gcm::new_from_slice(key).map_err(|_| CryptoError)?,
                )),
                AeadAlg::Aes256Gcm => Inner::Aes256Gcm(Box::new(
                    aes_gcm::Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError)?,
                )),
                AeadAlg::ChaCha20Poly1305 => Inner::ChaCha20Poly1305(Box::new(
                    chacha20poly1305::ChaCha20Poly1305::new_from_slice(key)
                        .map_err(|_| CryptoError)?,
                )),
            }))
        }
    }

    /// Encrypt `data` in place under `nonce` and `ad`; the tag goes to
    /// `tag` ([`TAG_LEN`] bytes).
    pub fn seal_in_place(
        &self,
        nonce: &[u8; NONCE_LEN],
        data: &mut [u8],
        tag: &mut [u8],
        ad: &[u8],
    ) -> Result<(), CryptoError> {
        if tag.len() != TAG_LEN {
            return Err(CryptoError);
        }
        #[cfg(boring)]
        {
            self.0
                .seal_in_place(nonce, data, tag, ad)
                .map(|_| ())
                .map_err(|_| CryptoError)
        }
        #[cfg(not(boring))]
        {
            let n = aes_gcm::Nonce::from_slice(nonce);
            let t = match &self.0 {
                Inner::Aes128Gcm(c) => c.encrypt_in_place_detached(n, ad, data),
                Inner::Aes256Gcm(c) => c.encrypt_in_place_detached(n, ad, data),
                Inner::ChaCha20Poly1305(c) => c.encrypt_in_place_detached(n, ad, data),
            }
            .map_err(|_| CryptoError)?;
            tag.copy_from_slice(&t);
            Ok(())
        }
    }

    /// Verify `tag` and decrypt `data` in place under `nonce` and `ad`.
    pub fn open_in_place(
        &self,
        nonce: &[u8; NONCE_LEN],
        data: &mut [u8],
        tag: &[u8],
        ad: &[u8],
    ) -> Result<(), CryptoError> {
        if tag.len() != TAG_LEN {
            return Err(CryptoError);
        }
        #[cfg(boring)]
        {
            self.0
                .open_in_place(nonce, data, tag, ad)
                .map_err(|_| CryptoError)
        }
        #[cfg(not(boring))]
        {
            let n = aes_gcm::Nonce::from_slice(nonce);
            let t = aes_gcm::Tag::from_slice(tag);
            match &self.0 {
                Inner::Aes128Gcm(c) => c.decrypt_in_place_detached(n, ad, data, t),
                Inner::Aes256Gcm(c) => c.decrypt_in_place_detached(n, ad, data, t),
                Inner::ChaCha20Poly1305(c) => c.decrypt_in_place_detached(n, ad, data, t),
            }
            .map_err(|_| CryptoError)
        }
    }
}

/// HMAC-SHA256.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    #[cfg(boring)]
    {
        boring::hash::hmac_sha256(key, data).expect("HMAC-SHA256 takes any key")
    }
    #[cfg(not(boring))]
    {
        use hmac::Mac as _;
        let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(key).expect("HMAC takes any key");
        m.update(data);
        m.finalize().into_bytes().into()
    }
}

/// One AES-128 block encryption (QUIC header protection).
pub fn aes128_encrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    #[cfg(boring)]
    {
        let out = boring::symm::encrypt(boring::symm::Cipher::aes_128_ecb(), key, None, block)
            .expect("AES-128-ECB of one block");
        let mut b = [0u8; 16];
        b.copy_from_slice(&out[..16]);
        b
    }
    #[cfg(not(boring))]
    {
        use aes_gcm::aes::cipher::{BlockEncrypt as _, KeyInit as _};
        let c = aes_gcm::aes::Aes128::new(key.into());
        let mut b = aes_gcm::aes::Block::clone_from_slice(block);
        c.encrypt_block(&mut b);
        b.into()
    }
}

/// Streaming SHA-1 (ShadowTLS v2/v3 HMAC framing); `Clone` copies the
/// state, so a rolling HMAC can be summed without allocating.
#[cfg(feature = "sha1-digest")]
#[derive(Clone)]
pub struct Sha1(
    #[cfg(boring)] boring::sha::Sha1,
    #[cfg(not(boring))] sha1::Sha1,
);

#[cfg(feature = "sha1-digest")]
impl Default for Sha1 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "sha1-digest")]
impl Sha1 {
    pub fn new() -> Self {
        #[cfg(boring)]
        {
            Self(boring::sha::Sha1::new())
        }
        #[cfg(not(boring))]
        {
            use sha1::Digest as _;
            Self(sha1::Sha1::new())
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        #[cfg(boring)]
        {
            self.0.update(data);
        }
        #[cfg(not(boring))]
        {
            use sha1::Digest as _;
            self.0.update(data);
        }
    }

    pub fn finish(self) -> [u8; 20] {
        #[cfg(boring)]
        {
            self.0.finish()
        }
        #[cfg(not(boring))]
        {
            use sha1::Digest as _;
            self.0.finalize().into()
        }
    }
}

/// Cryptographically secure random bytes.
pub fn rand_bytes(buf: &mut [u8]) -> Result<(), CryptoError> {
    #[cfg(boring)]
    {
        boring::rand::rand_bytes(buf).map_err(|_| CryptoError)
    }
    #[cfg(not(boring))]
    {
        getrandom::fill(buf).map_err(|_| CryptoError)
    }
}

/// An ML-KEM-768 decapsulation key and its encapsulation key (X25519MLKEM768
/// key shares).
#[cfg(feature = "mlkem")]
pub struct MlKem768 {
    #[cfg(boring)]
    private: boring::mlkem::MlKemPrivateKey,
    #[cfg(not(boring))]
    private: Box<ml_kem::DecapsulationKey<ml_kem::MlKem768>>,
    public: Vec<u8>,
}

#[cfg(feature = "mlkem")]
impl MlKem768 {
    pub fn generate() -> Result<Self, CryptoError> {
        #[cfg(boring)]
        {
            let (public, private) =
                boring::mlkem::MlKemPrivateKey::generate(boring::mlkem::Algorithm::MlKem768)
                    .map_err(|_| CryptoError)?;
            Ok(Self {
                private,
                public: public.as_bytes().to_vec(),
            })
        }
        #[cfg(not(boring))]
        {
            use ml_kem::kem::{Kem as _, KeyExport as _};
            let (dk, ek) = ml_kem::MlKem768::generate_keypair();
            Ok(Self {
                private: Box::new(dk),
                public: ek.to_bytes().as_slice().to_vec(),
            })
        }
    }

    /// The encapsulation key (1184 bytes).
    pub fn public(&self) -> &[u8] {
        &self.public
    }

    /// The shared secret a peer's ciphertext carries.
    pub fn decapsulate(&self, ciphertext: &[u8]) -> Result<[u8; 32], CryptoError> {
        #[cfg(boring)]
        {
            let s = self
                .private
                .decapsulate(ciphertext)
                .map_err(|_| CryptoError)?;
            s.as_ref().try_into().map_err(|_| CryptoError)
        }
        #[cfg(not(boring))]
        {
            use ml_kem::kem::Decapsulate as _;
            let s = self
                .private
                .decapsulate_slice(ciphertext)
                .map_err(|_| CryptoError)?;
            s.as_slice().try_into().map_err(|_| CryptoError)
        }
    }
}

/// Encapsulate to `public` (an ML-KEM-768 encapsulation key): the
/// ciphertext and the shared secret. The server side; tests use it.
#[cfg(feature = "mlkem")]
pub fn mlkem768_encapsulate(public: &[u8]) -> Result<(Vec<u8>, [u8; 32]), CryptoError> {
    #[cfg(boring)]
    {
        let ek =
            boring::mlkem::MlKemPublicKey::from_slice(boring::mlkem::Algorithm::MlKem768, public)
                .map_err(|_| CryptoError)?;
        let (ct, s) = ek.encapsulate().map_err(|_| CryptoError)?;
        Ok((ct, s.as_ref().try_into().map_err(|_| CryptoError)?))
    }
    #[cfg(not(boring))]
    {
        use ml_kem::kem::{Encapsulate as _, TryKeyInit as _};
        let ek = ml_kem::EncapsulationKey::<ml_kem::MlKem768>::new_from_slice(public)
            .map_err(|_| CryptoError)?;
        let (ct, s) = ek.encapsulate();
        Ok((
            ct.as_slice().to_vec(),
            s.as_slice().try_into().map_err(|_| CryptoError)?,
        ))
    }
}

/// A fresh P-256 public key as an uncompressed point (65 bytes); the
/// private half is dropped (REALITY's decoy key share).
#[cfg(feature = "p256-share")]
pub fn p256_public_uncompressed() -> Result<Vec<u8>, CryptoError> {
    #[cfg(boring)]
    {
        use boring::{
            bn::BigNumContext,
            ec::{EcGroup, EcKey, PointConversionForm},
            nid::Nid,
        };
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).map_err(|_| CryptoError)?;
        let key = EcKey::generate(&group).map_err(|_| CryptoError)?;
        let mut ctx = BigNumContext::new().map_err(|_| CryptoError)?;
        key.public_key()
            .to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut ctx)
            .map_err(|_| CryptoError)
    }
    #[cfg(not(boring))]
    {
        use p256::elliptic_curve::sec1::ToEncodedPoint as _;
        // A random 32-byte string is a valid scalar with overwhelming
        // probability; retry the rare one that is not.
        for _ in 0..8 {
            let mut d = [0u8; 32];
            rand_bytes(&mut d)?;
            if let Ok(sk) = p256::SecretKey::from_slice(&d) {
                return Ok(sk.public_key().to_encoded_point(false).as_bytes().to_vec());
            }
        }
        Err(CryptoError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cost probe for qemu-user instruction counting (`openwrt-fork/bench/
    /// aead-cost.sh`): seals `MEOW_AEAD_KIB` KiB in 16 KiB records under
    /// `MEOW_AEAD` (aes128 | aes256 | chacha). Run twice with different
    /// sizes; the instruction delta over the byte delta is the per-byte cost.
    #[test]
    #[ignore = "cost probe, run by aead-cost.sh"]
    fn aead_cost_probe() {
        let alg = match std::env::var("MEOW_AEAD").as_deref() {
            Ok("aes128") => AeadAlg::Aes128Gcm,
            Ok("aes256") => AeadAlg::Aes256Gcm,
            _ => AeadAlg::ChaCha20Poly1305,
        };
        let kib: usize = std::env::var("MEOW_AEAD_KIB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let aead = Aead::new(alg, &[7u8; 32][..alg.key_len()]).unwrap();
        let mut buf = vec![0x5au8; 16 * 1024];
        let mut tag = [0u8; TAG_LEN];
        for i in 0..kib / 16 {
            let mut nonce = [0u8; NONCE_LEN];
            nonce[..8].copy_from_slice(&(i as u64).to_be_bytes());
            aead.seal_in_place(&nonce, &mut buf, &mut tag, b"ad")
                .unwrap();
        }
        std::hint::black_box((&buf, &tag));
    }

    /// RFC 9001 appendix A.2 / A.3 vectors, through every primitive the
    /// QUIC sniffer uses.
    #[test]
    fn quic_initial_vectors() {
        let salt: [u8; 20] = [
            0x38, 0x76, 0x2c, 0xf7, 0xf5, 0x59, 0x34, 0xb3, 0x4d, 0x17, 0x9a, 0xe6, 0xa4, 0xc8,
            0x0c, 0xad, 0xcc, 0xbb, 0x7f, 0x0a,
        ];
        let dcid = [0x83, 0x94, 0xc8, 0xf0, 0x3e, 0x51, 0x57, 0x08];
        let initial = hmac_sha256(&salt, &dcid);
        assert_eq!(
            initial[..4],
            [0x7d, 0xb5, 0xdf, 0x06],
            "initial_secret prefix (RFC 9001 A.1)"
        );
        // Header protection: hp key and sample from RFC 9001 A.2.
        let hp = [
            0x9f, 0x50, 0x44, 0x9e, 0x04, 0xa0, 0xe8, 0x10, 0x28, 0x3a, 0x1e, 0x99, 0x33, 0xad,
            0xed, 0xd2,
        ];
        let sample = [
            0xd1, 0xb1, 0xc9, 0x8d, 0xd7, 0x68, 0x9f, 0xb8, 0xec, 0x11, 0xd2, 0x42, 0xb1, 0x23,
            0xdc, 0x9b,
        ];
        assert_eq!(
            aes128_encrypt_block(&hp, &sample)[..5],
            [0x43, 0x7b, 0x9a, 0xec, 0x36]
        );
    }

    #[test]
    fn aead_round_trip_and_tamper() {
        for alg in [
            AeadAlg::Aes128Gcm,
            AeadAlg::Aes256Gcm,
            AeadAlg::ChaCha20Poly1305,
        ] {
            let key = vec![7u8; alg.key_len()];
            let a = Aead::new(alg, &key).unwrap();
            let nonce = [1u8; NONCE_LEN];
            let mut data = *b"meow-openwrt";
            let mut tag = [0u8; TAG_LEN];
            a.seal_in_place(&nonce, &mut data, &mut tag, b"ad").unwrap();
            assert_ne!(&data, b"meow-openwrt");
            let mut bad = data;
            assert!(a.open_in_place(&nonce, &mut bad, &tag, b"AD").is_err());
            a.open_in_place(&nonce, &mut data, &tag, b"ad").unwrap();
            assert_eq!(&data, b"meow-openwrt");
            assert!(Aead::new(alg, &key[1..]).is_err());
        }
    }

    #[cfg(feature = "sha1-digest")]
    #[test]
    fn sha1_abc() {
        let mut h = Sha1::new();
        h.update(b"a");
        let fork = h.clone();
        h.update(b"bc");
        assert_eq!(
            h.finish()[..4],
            [0xa9, 0x99, 0x3e, 0x36],
            "SHA-1(\"abc\") (FIPS 180)"
        );
        assert_eq!(fork.finish()[..4], [0x86, 0xf7, 0xe4, 0x37], "SHA-1(\"a\")");
    }

    #[cfg(feature = "mlkem")]
    #[test]
    fn mlkem768_round_trip() {
        let k = MlKem768::generate().unwrap();
        assert_eq!(k.public().len(), 1184);
        let (ct, s) = mlkem768_encapsulate(k.public()).unwrap();
        assert_eq!(ct.len(), 1088);
        assert_eq!(k.decapsulate(&ct).unwrap(), s);
    }

    #[cfg(feature = "p256-share")]
    #[test]
    fn p256_share_is_an_uncompressed_point() {
        let p = p256_public_uncompressed().unwrap();
        assert_eq!((p.len(), p[0]), (65, 4));
        assert_ne!(p, p256_public_uncompressed().unwrap());
    }

    /// AES-128-GCM known answer (NIST GCM test case 3 shape, all-zero
    /// key/nonce, empty input): the tag is fixed, so both backends agree.
    #[test]
    fn aes128_gcm_empty_tag() {
        let a = Aead::new(AeadAlg::Aes128Gcm, &[0u8; 16]).unwrap();
        let mut tag = [0u8; TAG_LEN];
        a.seal_in_place(&[0u8; 12], &mut [], &mut tag, &[]).unwrap();
        assert_eq!(
            tag,
            [
                0x58, 0xe2, 0xfc, 0xce, 0xfa, 0x7e, 0x30, 0x61, 0x36, 0x7f, 0x1d, 0x57, 0xa4, 0xe7,
                0x45, 0x5a
            ]
        );
    }
}
