#!/bin/sh
# PGO step 1 (meow-openwrt): an instrumented aarch64 tiny meow, built with rustup
# 1.96 (its prebuilt std has the profiler runtime; the SDK std and build-std do not)
# and the SDK toolchain. Run it on the router (writes /tmp/pgo/*.profraw on exit),
# merge with the SDK llvm-profdata, then build the package with MEOW_PGO_PROFILE=.
# usage: SDK=~/mybuild/sdk-filogic sh pgo-gen.sh  (needs a prior SDK build for the cmake file)
set -e
: "${SDK:?set SDK=<openwrt sdk dir>}"
cd "$(dirname "$0")/../.."
export PATH=$SDK/staging_dir/toolchain-aarch64_cortex-a53_gcc-14.4.0_musl/bin:$PATH STAGING_DIR=$SDK/staging_dir RUSTC_BOOTSTRAP=1
export CC_aarch64_unknown_linux_musl=aarch64-openwrt-linux-musl-gcc CXX_aarch64_unknown_linux_musl=aarch64-openwrt-linux-musl-g++ AR_aarch64_unknown_linux_musl=aarch64-openwrt-linux-musl-gcc-ar
export CFLAGS_aarch64_unknown_linux_musl="-Os -mcpu=cortex-a53 -mno-outline-atomics" CXXFLAGS_aarch64_unknown_linux_musl="-Os -mcpu=cortex-a53 -mno-outline-atomics"
export CMAKE_TOOLCHAIN_FILE_aarch64_unknown_linux_musl=$SDK/build_dir/target-aarch64_cortex-a53_musl/meow-tiny/meow-0.28.0/openwrt-toolchain.cmake
export BORING_BSSL_RUST_CPPLIB_aarch64_unknown_linux_musl=static=stdc++
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_musl="--sysroot=$SDK/staging_dir/toolchain-aarch64_cortex-a53_gcc-14.4.0_musl"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-openwrt-linux-musl-gcc
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_OPT_LEVEL=z CARGO_PROFILE_RELEASE_PANIC=abort CARGO_PROFILE_RELEASE_STRIP=true CARGO_PROFILE_RELEASE_DEBUG=false
export RUSTFLAGS="-lssp_nonshared -Clink-arg=-fuse-ld=bfd -Ctarget-feature=+crt-static -Cforce-unwind-tables=no -Lnative=$SDK/staging_dir/toolchain-aarch64_cortex-a53_gcc-14.4.0_musl/lib -Zlocation-detail=none -Cprofile-generate=/tmp/pgo"
cargo +1.96.0 build --release -p meow-app --bin meow --no-default-features --features tiny --target aarch64-unknown-linux-musl --target-dir ${TGT:-/tmp/meow-pgo-target}
