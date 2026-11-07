#!/usr/bin/env bash
# meow-openwrt release build: one static meow for one OpenWrt arch and profile,
# with rustup's pinned rustc (build-std) and the C toolchain of a pinned
# OpenWrt SDK (only its staging_dir/toolchain-* is unpacked). Used by
# .github/workflows/openwrt.yml; runs the same on a workstation.
#
# usage: build.sh <arch> <profile>
#   arch:    aarch64 | armv7 | armv7-a9 | armv7-sf | mipsel | mips
#            armv7: Cortex-A7/A15 (NEON, VFPv4); armv7-a9: Cortex-A9 with
#            VFPv3-D16, no NEON (mvebu); armv7-sf: no FPU (bcm53xx)
#   profile: tiny | lightweight | full
# env:     TC_CACHE  toolchain dir cache (default ~/.cache/meow-owrt-tc)
#          TGT       cargo target dir (default target/owrt-<arch>)
#          OUT       where meow-<profile>-<arch> is copied (default dist)
#          EXTRA_FEATURES  extra cargo features (experiments only)
#          MEOW_PGO  0 = no profile-guided optimisation (lightweight/full)
#          OPENWRT_MIRROR  SDK mirror bases, space-separated, tried in turn
#                          (sha256 is checked either way)
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
armv7-a9)
	SDK_TGT=mvebu/cortexa9; SDK_SHA=fc59dddc696b88e32662f4b1611c880bed24a88115ae423b640c81d071bec8ab
	SDK_FILE=openwrt-sdk-$SDK_REL-mvebu-cortexa9_gcc-14.3.0_musl_eabi.Linux-x86_64.tar.zst
	RT=armv7-unknown-linux-musleabihf; GCC=arm-openwrt-linux-muslgnueabi
	# rustc 1.96 turns `d32` on for this target despite its vfp3d16 spec:
	# VFPv3-D16 parts (Armada 370/XP) SIGILL on d16-d31 without -d32.
	CFL="-Os -mcpu=cortex-a9 -mfpu=vfpv3-d16 -mfloat-abi=hard"; RFL="-Ctarget-feature=-d32" ;;
armv7-sf)
	SDK_TGT=bcm53xx/generic; SDK_SHA=291c0899a045a963aa5f610e885b9eb335cfeb5c8478c9c431770ba9cdbc98bf
	SDK_FILE=openwrt-sdk-$SDK_REL-bcm53xx-generic_gcc-14.3.0_musl_eabi.Linux-x86_64.tar.zst
	RT=armv7-unknown-linux-musleabi; GCC=arm-openwrt-linux-muslgnueabi
	# No target-cpu: cortex-a9 would let LLVM emit VFP code the BCM4708 lacks.
	CFL="-Os -mcpu=cortex-a9 -mfloat-abi=soft"; RFL="" ;;
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
	# downloads.openwrt.org drops long transfers and sometimes crawls at
	# ~100 KB/s (40 min per SDK on CI): take mirrors in turn, give up on one
	# below 1 MB/s for 20 s, and resume the partial file on the next.
	dl=$TC_CACHE/$SDK_FILE.part
	set -- ${OPENWRT_MIRROR:-https://ftp.halifax.rwth-aachen.de/openwrt https://downloads.openwrt.org}
	for try in 1 2 3 4 5 6 7 8; do
		m=$(eval echo "\${$(( (try - 1) % $# + 1 ))}")
		curl -fsSL --retry 2 --retry-all-errors --speed-limit 1000000 --speed-time 20 \
			-C - -o "$dl" "$m/releases/$SDK_REL/targets/$SDK_TGT/$SDK_FILE" && break
		echo "SDK download from $m interrupted (try $try), resuming" >&2
	done
	echo "$SDK_SHA  $dl" | sha256sum -c - || { rm -f "$dl"; exit 1; }
	# Release SDK compilers run through their own loader in host/lib.
	tar --zstd -xf "$dl" -C "$TCROOT" --strip-components=2 --wildcards \
		'*/staging_dir/toolchain-*' '*/staging_dir/host/lib/*'
	rm -f "$dl"
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
# PGO: lightweight and full use the profile trained on a router of the arch
# (openwrt-fork/pgo/README.md); tiny stays size-first. BPI-R4, 2026-11-08:
# release builds: hy2 TCP +2-4 % down / +3-15 % up, CPU per Mbit -5..-10 %,
# REALITY and DIRECT +1-2 %; +0.4-0.46 MB.
# MEOW_PGO=0 builds without it.
PGO_FILE=openwrt-fork/pgo/$ARCH.profdata.xz
if [ "${MEOW_PGO:-1}" = 1 ] && [ "$PROFILE" != tiny ] && [ -f "$PGO_FILE" ]; then
	mkdir -p "$TGT"
	xz -dc "$PGO_FILE" > "$TGT/pgo.profdata"
	FLAGS="$FLAGS -Cprofile-use=$PWD/$TGT/pgo.profdata -Cllvm-args=-no-pgo-warn-mismatch"
	echo "PGO: $PGO_FILE"
fi
export RUSTFLAGS="$FLAGS"

# restls and jls need BoringSSL, which big-endian targets do not build.
FEAT=$PROFILE; [ "$ARCH" = mips ] && [ "$PROFILE" = full ] && FEAT=full-be
# EXTRA_FEATURES: comma list added for experiments (e.g. mimalloc); EXTRA_CARGO:
# extra cargo args (e.g. --config profile overrides). Releases leave both empty.
FEAT=$FEAT${EXTRA_FEATURES:+,$EXTRA_FEATURES}

cargo +1.96.0 build -Zbuild-std=std,panic_abort -Zbuild-std-features=optimize_for_size \
	--release -p meow-app --bin meow --no-default-features --features "$FEAT" \
	--target "$RT" --target-dir "$TGT" ${EXTRA_CARGO:-}

OUT=${OUT:-dist}; mkdir -p "$OUT"
cp "$TGT/$RT/release/meow" "$OUT/meow-$PROFILE-$ARCH"
echo "$PROFILE $ARCH $(stat -c %s "$OUT/meow-$PROFILE-$ARCH") bytes"
