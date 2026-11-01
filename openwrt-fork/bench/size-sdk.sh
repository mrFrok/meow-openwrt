#!/bin/sh
# Build one meow package variant in the OpenWrt SDK and print its sizes.
# usage: size-sdk.sh <label> <variant: tiny|lightweight|full> [make VAR=value ...]
# (SDK=~/mybuild/sdk-mt7621). Extra VARs: MEOW_FEATURES, MEOW_EXTRA_RUSTFLAGS, MEOW_BUILD_STD...
set -e
SDK=${SDK:-$HOME/mybuild/sdk-mt7621}
label=$1 variant=$2; shift 2
cd "$SDK"
B=$(ls -d build_dir/target-*/meow-"$variant"/meow-* 2>/dev/null || true)
[ -n "$B" ] && rm -f "$B"/.built* "$B"/.configured* "$B"/.prepared*
make package/meow/compile BUILD_VARIANT="$variant" -j"$(nproc)" V=s "$@" > "meow-$label.log" 2>&1 \
  || { grep -nE '^error|cargo::error' "meow-$label.log" | head; exit 1; }
B=$(ls -d build_dir/target-*/meow-"$variant"/meow-*)
bin=$(ls "$B"/.pkgdir/*/usr/bin/meow)
# jffs2 (lzma, as OpenWrt kernels use) estimate of the installed binary
fs=$(mktemp -d); mkdir -p "$fs/d"; cp "$bin" "$fs/d/"
staging_dir/host/bin/mkfs.jffs2 -l -e 64KiB -q -x zlib -d "$fs/d" -o "$fs/j"
printf '%-24s bin=%s xz=%s jffs2=%s needed=%s\n' "$label" "$(stat -c %s "$bin")" \
  "$(xz -9e -c "$bin" | wc -c)" "$(stat -c %s "$fs/j")" \
  "$(readelf -d "$bin" | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p' | tr '\n' ,)"
rm -rf "$fs"
