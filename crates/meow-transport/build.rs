//! Picks the TLS / crypto backend (meow-openwrt stage 7): `cfg(boring)` when
//! BoringSSL is linked — `tls` or `boring-crypto` on a little-endian target.
//! Big-endian targets (BoringSSL does not support them) get rustls and
//! RustCrypto instead; `MEOW_TLS_BACKEND=rustls` forces that on any target,
//! so the rustls path can be tested on the build host.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(boring)");
    println!("cargo::rerun-if-env-changed=MEOW_TLS_BACKEND");
    let wanted = std::env::var_os("CARGO_FEATURE_TLS").is_some()
        || std::env::var_os("CARGO_FEATURE_BORING_CRYPTO").is_some();
    let little = std::env::var("CARGO_CFG_TARGET_ENDIAN").as_deref() == Ok("little");
    let forced_rustls = std::env::var("MEOW_TLS_BACKEND").as_deref() == Ok("rustls");
    if wanted && little && !forced_rustls {
        println!("cargo::rustc-cfg=boring");
    }
}
