#!/usr/bin/env bash
# meow-openwrt release build: one static meow for one OpenWrt arch and profile,
# with rustup's pinned rustc (build-std) and the C toolchain of a pinned
# OpenWrt SDK (only its staging_dir/toolchain-* is unpacked). Used by
# .github/workflows/openwrt.yml; runs the same on a workstation.
#
# usage: build.sh <arch> <profile>
#   arch:    aarch64 | armv7 | mipsel | mips
#   profile: tiny | lightweight | full
# env:     TC_CACHE  toolchain dir cache (default ~/.cache/meow-owrt-tc)
#          TGT       cargo target dir (default target/owrt-<arch>)
#          OUT       where meow-<profile>-<arch> is copied (default dist)
set -euo pipefail
ARCH=${1:?arch}; PROFILE=${2:?profile}
cd "$(dirname "$0")/../.."

SDK_REL=25.12.6
case $ARCH in
aarch64)
	SDK_TGT=mediatek/filogic; SDK_SHA=1c04ac47bdfc361d6aab299eb520d570e368013dedcb8d29cf39dc7168ae642e
	SDK_FILE=openwrt-sdk-$SDK_REL-mediatek-filogic_gcc-14.3.0_musl.Linux-x86_64.tar.zst
	RT=aarch64-unknown-linux-musl; GCC=aarch64-openwrt-linux-musl
	CFL="-Os -mcpu=cortex-a53 -mno-outline-atomics"; RFL="" ;;
armv7)
	SDK_TGT=ipq40xx/generic; SDK_SHA=72b2a064190a46b8026e192e29b8ff357dc040850eb3309573124509acbb8a05
	SDK_FILE=openwrt-sdk-$SDK_REL-ipq40xx-generic_gcc-14.3.0_musl_eabi.Linux-x86_64.tar.zst
	RT=armv7-unknown-linux-musleabihf; GCC=arm-openwrt-linux-muslgnueabi
	CFL="-Os -mcpu=cortex-a7 -mfpu=neon-vfpv4 -mfloat-abi=hard"; RFL="-Ctarget-cpu=cortex-a7" ;;
mipsel)
	SDK_TGT=ramips/mt7621; SDK_SHA=e4e3f25962ddf3ed55207fed456ecd1d6e55f3482c976b3183a640df6ef0adf0
	SDK_FILE=openwrt-sdk-$SDK_REL-ramips-mt7621_gcc-14.3.0_musl.Linux-x86_64.tar.zst
	RT=mipsel-unknown-linux-musl; GCC=mipsel-openwrt-linux-musl
	CFL="-Os -march=24kc -mno-branch-likely"; RFL="-Ctarget-cpu=mips32r2" ;;
mips)
	SDK_TGT=ath79/generic; SDK_SHA=941c19d4787944e569bc0ca7fb4b6d0f4387ba24d188900bf0ffb6787fee31c6
	SDK_FILE=openwrt-sdk-$SDK_REL-ath79-generic_gcc-14.3.0_musl.Linux-x86_64.tar.zst
	RT=mips-unknown-linux-musl; GCC=mips-openwrt-linux-musl
	CFL="-Os -march=24kc -mno-branch-likely"; RFL="-Ctarget-cpu=mips32r2" ;;
*) echo "unknown arch $ARCH" >&2; exit 2 ;;
esac
case $PROFILE in tiny|lightweight|full) ;; *) echo "unknown profile $PROFILE" >&2; exit 2 ;; esac

# --- toolchain (cached) ---
TC_CACHE=${TC_CACHE:-$HOME/.cache/meow-owrt-tc}
TCROOT=$TC_CACHE/$ARCH-$SDK_SHA
if ! { [ -d "$TCROOT/host/lib" ] && ls -d "$TCROOT"/toolchain-* >/dev/null 2>&1; }; then
	mkdir -p "$TCROOT"
	dl=$(mktemp -d)
	curl -fsSL --retry 3 -o "$dl/sdk.tar.zst" \
		"https://downloads.openwrt.org/releases/$SDK_REL/targets/$SDK_TGT/$SDK_FILE"
	echo "$SDK_SHA  $dl/sdk.tar.zst" | sha256sum -c -
	# Release SDK compilers run through their own loader in host/lib.
	tar --zstd -xf "$dl/sdk.tar.zst" -C "$TCROOT" --strip-components=2 --wildcards \
		'*/staging_dir/toolchain-*' '*/staging_dir/host/lib/*'
	rm -rf "$dl"
fi
TC=$(ls -d "$TCROOT"/toolchain-* | head -1)
export PATH=$TC/bin:$PATH STAGING_DIR=$TCROOT RUSTC_BOOTSTRAP=1

T=${RT//-/_}; TU=${T^^}
export "CC_$T=$GCC-gcc" "CXX_$T=$GCC-g++" "AR_$T=$GCC-gcc-ar"
export "CFLAGS_$T=$CFL" "CXXFLAGS_$T=$CFL"
export "BINDGEN_EXTRA_CLANG_ARGS_$T=--sysroot=$TC"
export "CARGO_TARGET_${TU}_LINKER=$GCC-gcc"
export "BORING_BSSL_RUST_CPPLIB_$T=static=stdc++"
TGT=${TGT:-target/owrt-$ARCH}
mkdir -p "$TGT"
CMAKE_TC=$(realpath "$TGT")/openwrt.cmake
cat > "$CMAKE_TC" <<EOF
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR ${RT%%-*})
set(CMAKE_C_COMPILER $TC/bin/$GCC-gcc)
set(CMAKE_CXX_COMPILER $TC/bin/$GCC-g++)
set(CMAKE_ASM_COMPILER $TC/bin/$GCC-gcc)
set(CMAKE_AR $TC/bin/$GCC-gcc-ar)
set(CMAKE_RANLIB $TC/bin/$GCC-gcc-ranlib)
set(CMAKE_FIND_ROOT_PATH $TC)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(BUILD_TESTING OFF CACHE BOOL "" FORCE)
EOF
export "CMAKE_TOOLCHAIN_FILE_$T=$CMAKE_TC"
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_LTO=true \
	CARGO_PROFILE_RELEASE_OPT_LEVEL=z CARGO_PROFILE_RELEASE_PANIC=abort \
	CARGO_PROFILE_RELEASE_STRIP=true CARGO_PROFILE_RELEASE_DEBUG=false

# std's `unwind` links -lunwind even with panic=abort; OpenWrt toolchains
# only have libgcc_eh, so a one-line linker script stands in for it.
SHIM=$(realpath "$TGT")/unwind-shim
mkdir -p "$SHIM"; echo 'INPUT(-lgcc_eh)' > "$SHIM/libunwind.a"

FLAGS="-Clink-self-contained=no -Lnative=$SHIM -Ctarget-feature=+crt-static -Cforce-unwind-tables=no"
FLAGS="$FLAGS -Lnative=$TC/lib -Clink-arg=-lssp_nonshared -Zlocation-detail=none $RFL"
# OpenWrt's musl has a 64-bit time_t on 32-bit targets; tell the libc crate.
[ "$ARCH" != aarch64 ] && FLAGS="$FLAGS --cfg=libc_unstable_musl_v1_2"
# MIPS PIC goes through the GOT for every call: non-PIE is ~14% smaller.
# BoringSSL's static libstdc++ needs __sync_synchronize from libgcc there.
case $ARCH in mips|mipsel) FLAGS="$FLAGS -Crelocation-model=static -Clink-arg=-lgcc" ;; esac
# Size profiles drop panic messages (-Cpanic=immediate-abort); full keeps them.
[ "$PROFILE" != full ] && FLAGS="$FLAGS -Zunstable-options -Cpanic=immediate-abort"
export RUSTFLAGS="$FLAGS"

cargo +1.96.0 build -Zbuild-std=std,panic_abort -Zbuild-std-features=optimize_for_size \
	--release -p meow-app --bin meow --no-default-features --features "$PROFILE" \
	--target "$RT" --target-dir "$TGT"

OUT=${OUT:-dist}; mkdir -p "$OUT"
cp "$TGT/$RT/release/meow" "$OUT/meow-$PROFILE-$ARCH"
echo "$PROFILE $ARCH $(stat -c %s "$OUT/meow-$PROFILE-$ARCH") bytes"
