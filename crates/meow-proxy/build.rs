//! hysteria2 QUIC stack (meow-openwrt stage 7): `cfg(hy2_quiche)` — quiche on
//! BoringSSL — on little-endian targets; big-endian ones (no BoringSSL) take
//! quinn + rustls. `MEOW_TLS_BACKEND=rustls` forces the quinn path anywhere,
//! matching meow-transport's TLS backend switch.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(hy2_quiche)");
    println!("cargo::rerun-if-env-changed=MEOW_TLS_BACKEND");
    let little = std::env::var("CARGO_CFG_TARGET_ENDIAN").as_deref() == Ok("little");
    let forced_rustls = std::env::var("MEOW_TLS_BACKEND").as_deref() == Ok("rustls");
    if std::env::var_os("CARGO_FEATURE_HYSTERIA2").is_some() && little && !forced_rustls {
        println!("cargo::rustc-cfg=hy2_quiche");
    }
}
