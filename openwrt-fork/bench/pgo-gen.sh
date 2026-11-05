#!/bin/sh
# PGO step 1 (meow-openwrt): an instrumented aarch64 tiny meow, built with rustup
# 1.96 (its prebuilt std has the profiler runtime; the SDK std and build-std do not)
# and the SDK toolchain. Run it on the router (writes /tmp/pgo/*.profraw on exit),
# merge with the SDK llvm-profdata, then build the package with MEOW_PGO_PROFILE=.
# usage: SDK=~/mybuild/sdk-filogic [TGT=<target dir>] sh pgo-gen.sh
#   PGO_USE=<merged .profdata>: build the optimised binary instead (same rustc,
#   so the profile always matches; merge with rustup 1.96's llvm-tools
#   llvm-profdata). PGO_USE=none: the plain baseline, same flags.
set -e
: "${SDK:?set SDK=<openwrt sdk dir>}"
cd "$(dirname "$0")/../.."
export PATH=$SDK/staging_dir/toolchain-aarch64_cortex-a53_gcc-14.4.0_musl/bin:$PATH STAGING_DIR=$SDK/staging_dir RUSTC_BOOTSTRAP=1
export CC_aarch64_unknown_linux_musl=aarch64-openwrt-linux-musl-gcc CXX_aarch64_unknown_linux_musl=aarch64-openwrt-linux-musl-g++ AR_aarch64_unknown_linux_musl=aarch64-openwrt-linux-musl-gcc-ar
export CFLAGS_aarch64_unknown_linux_musl="-Os -mcpu=cortex-a53 -mno-outline-atomics" CXXFLAGS_aarch64_unknown_linux_musl="-Os -mcpu=cortex-a53 -mno-outline-atomics"
# A minimal cmake toolchain for boring-sys (no prior SDK package build needed).
TC=$SDK/staging_dir/toolchain-aarch64_cortex-a53_gcc-14.4.0_musl
CMAKE_TC=${TGT:-/tmp/meow-pgo-target}/openwrt-aarch64.cmake
mkdir -p "$(dirname "$CMAKE_TC")"
cat > "$CMAKE_TC" <<EOF
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_C_COMPILER $TC/bin/aarch64-openwrt-linux-musl-gcc)
set(CMAKE_CXX_COMPILER $TC/bin/aarch64-openwrt-linux-musl-g++)
set(CMAKE_ASM_COMPILER $TC/bin/aarch64-openwrt-linux-musl-gcc)
set(CMAKE_AR $TC/bin/aarch64-openwrt-linux-musl-gcc-ar)
set(CMAKE_RANLIB $TC/bin/aarch64-openwrt-linux-musl-gcc-ranlib)
set(CMAKE_FIND_ROOT_PATH $TC)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(BUILD_TESTING OFF CACHE BOOL "" FORCE)
EOF
export CMAKE_TOOLCHAIN_FILE_aarch64_unknown_linux_musl=$CMAKE_TC
export BORING_BSSL_RUST_CPPLIB_aarch64_unknown_linux_musl=static=stdc++
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_musl="--sysroot=$SDK/staging_dir/toolchain-aarch64_cortex-a53_gcc-14.4.0_musl"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-openwrt-linux-musl-gcc
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_OPT_LEVEL=z CARGO_PROFILE_RELEASE_PANIC=abort CARGO_PROFILE_RELEASE_STRIP=true CARGO_PROFILE_RELEASE_DEBUG=false
case "${PGO_USE:-}" in
"") PGO_FLAG=-Cprofile-generate=/tmp/pgo ;;
none) PGO_FLAG= ;;
*) PGO_FLAG=-Cprofile-use=$PGO_USE ;;
esac
export RUSTFLAGS="-lssp_nonshared -Clink-arg=-fuse-ld=bfd -Ctarget-feature=+crt-static -Cforce-unwind-tables=no -Lnative=$SDK/staging_dir/toolchain-aarch64_cortex-a53_gcc-14.4.0_musl/lib -Zlocation-detail=none $PGO_FLAG"
cargo +1.96.0 build --release -p meow-app --bin meow --no-default-features --features tiny --target aarch64-unknown-linux-musl --target-dir ${TGT:-/tmp/meow-pgo-target}
