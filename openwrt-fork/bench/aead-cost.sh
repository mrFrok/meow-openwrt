#!/bin/sh
# meow-openwrt — AEAD cost per byte on a router arch, counted under qemu-user.
#
# qemu's timing says nothing about a 24Kc, but an in-order single-issue core
# runs roughly one instruction per cycle, so instructions per byte (counted
# by the TCG plugin in qemu-insn/) is a fair proxy. Builds the
# `crypto::tests::aead_cost_probe` test for <arch> in release flags
# (build.sh MEOW_TEST) and runs it at 0 and 4 MiB per cipher.
#
# Usage: sh aead-cost.sh <arch> [features]   e.g. mipsel crypto,boring-crypto
#        (mips: crypto — RustCrypto; others: crypto,boring-crypto)
set -e
ARCH=${1:?arch}; FEAT=${2:-crypto,boring-crypto}
D=$(dirname "$0"); W=${TMPDIR:-/tmp}/aead-cost.$$; mkdir -p "$W"; trap 'rm -rf "$W"' EXIT
gcc -shared -fPIC -O2 $(pkg-config --cflags glib-2.0) -o "$W/libinsn.so" "$D/qemu-insn/insn.c"
MEOW_TEST="-p meow-transport --features $FEAT --release --lib --no-run" \
	bash "$D/../ci/build.sh" "$ARCH" full > "$W/build.log" 2>&1 || { tail -20 "$W/build.log"; exit 1; }
BIN=$(sed -n 's/.*Executable unittests src\/lib.rs (\(.*\))$/\1/p' "$W/build.log" | tail -1)
QEMU=qemu-$(file -b "$BIN" | grep -q MSB && echo mips || case $ARCH in mipsel) echo mipsel;; armv7*) echo arm;; aarch64) echo aarch64;; *) echo "$ARCH";; esac)
cnt() { MEOW_AEAD=$1 MEOW_AEAD_KIB=$2 $QEMU -plugin "$W/libinsn.so" "$BIN" --exact \
	crypto::tests::aead_cost_probe --ignored --test-threads=1 2>&1 | sed -n 's/^INSNS //p'; }
for a in chacha aes128 aes256; do
	z=$(cnt $a 0); n=$(cnt $a 4096)
	echo "$ARCH $FEAT $a: $(( (n - z) / 4194304 )).$(( (n - z) * 10 / 4194304 % 10 )) insn/byte"
done
