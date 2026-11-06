#!/bin/sh
# meow-openwrt stage 7: a static big-endian MIPS (mips_24kc, ath79) meow,
# built with the OpenWrt SDK toolchain and build-std (the target is tier 3:
# no prebuilt std). BoringSSL does not support big-endian, so only feature
# sets without it build here.
# usage: SDK=~/mybuild/sdk-ath79 [TGT=<target dir>] [PROFILE=<features>] sh mips-be-build.sh
set -e
: "${SDK:?set SDK=<openwrt ath79 sdk dir>}"
cd "$(dirname "$0")/../.."
TC=$SDK/staging_dir/toolchain-mips_24kc_gcc-14.4.0_musl
export PATH=$TC/bin:$PATH STAGING_DIR=$SDK/staging_dir RUSTC_BOOTSTRAP=1
export CC_mips_unknown_linux_musl=mips-openwrt-linux-musl-gcc CXX_mips_unknown_linux_musl=mips-openwrt-linux-musl-g++ AR_mips_unknown_linux_musl=mips-openwrt-linux-musl-gcc-ar
export CFLAGS_mips_unknown_linux_musl="-Os -march=24kc -mno-branch-likely" CXXFLAGS_mips_unknown_linux_musl="-Os -march=24kc -mno-branch-likely"
export BINDGEN_EXTRA_CLANG_ARGS_mips_unknown_linux_musl="--sysroot=$TC"
export CARGO_TARGET_MIPS_UNKNOWN_LINUX_MUSL_LINKER=mips-openwrt-linux-musl-gcc
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_OPT_LEVEL=z CARGO_PROFILE_RELEASE_PANIC=abort CARGO_PROFILE_RELEASE_STRIP=true CARGO_PROFILE_RELEASE_DEBUG=false
# std's `unwind` links -lunwind even with panic=abort; the SDK only has
# libgcc_eh, so a one-line linker script stands in for libunwind.a.
SHIM=$(realpath -m "${TGT:-target/mips-be}")/unwind-shim
mkdir -p "$SHIM" && echo 'INPUT(-lgcc_eh)' > "$SHIM/libunwind.a"
export RUSTFLAGS="-Clink-self-contained=no -Lnative=$SHIM -Ctarget-cpu=mips32r2 -Ctarget-feature=+crt-static -Cforce-unwind-tables=no -Lnative=$TC/lib -Clink-arg=-lssp_nonshared -Zlocation-detail=none"
cargo +1.96.0 build -Zbuild-std=std,panic_abort \
	--release -p meow-app --bin meow --no-default-features --features "${PROFILE:-tiny}" \
	--target mips-unknown-linux-musl --target-dir "${TGT:-target/mips-be}"
