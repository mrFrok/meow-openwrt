#!/usr/bin/env bash
# meow-openwrt: wrap one static meow binary into an OpenWrt .ipk (opkg) and
# .apk (apk-tools 3, OpenWrt 25+), the same packages openwrt/package/meow
# makes: meow-tiny / meow / meow-full, mutually exclusive, /usr/bin/meow only.
#
# usage: package.sh <binary> <profile> <openwrt-arch> <version> <outdir>
# The binary is fully static: no libc or libgcc dependency, so one package
# installs on any OpenWrt release of that arch.
#   version: X.Y.Z-N (ipk "X.Y.Z-N", apk "X.Y.Z-rN"), or X.Y.Z_gitN-N
# env: APK=<apk binary> (default: apk on PATH); without it only the ipk is made.
set -euo pipefail
BIN=${1:?binary}; PROFILE=${2:?profile}; OARCH=${3:?arch}; VER=${4:?version}; OUT=${5:?outdir}
case $PROFILE in
tiny) NAME=meow-tiny; CONFL="meow meow-full"; WHAT="tiny profile" ;;
lightweight) NAME=meow; CONFL="meow-tiny meow-full"; WHAT="lightweight profile" ;;
full) NAME=meow-full; CONFL="meow-tiny meow"; WHAT="all protocols" ;;
*) echo "unknown profile $PROFILE" >&2; exit 2 ;;
esac
DESC="Clash Meta compatible proxy kernel (meow-openwrt, $WHAT)"
URL=https://github.com/mrFrok/meow-rs
mkdir -p "$OUT"; OUT=$(realpath "$OUT")
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-0}
TAR=(tar --format=ustar --owner=0 --group=0 --numeric-owner --sort=name --mtime=@"$SOURCE_DATE_EPOCH")

mkdir -p "$W/data/usr/bin" "$W/ctl" "$W/ipk"
install -m 0755 "$BIN" "$W/data/usr/bin/meow"
SIZE=$(stat -c %s "$BIN")

# --- ipk ---
cat > "$W/ctl/control" <<EOF
Package: $NAME
Version: $VER
Conflicts: ${CONFL// /, }
Provides: meow-core
License: GPL-3.0-only
Section: net
URL: $URL
Maintainer: mrFrok
Architecture: $OARCH
Installed-Size: $SIZE
Description: $DESC
EOF
echo 2.0 > "$W/ipk/debian-binary"
(cd "$W/ctl" && "${TAR[@]}" -czf "$W/ipk/control.tar.gz" ./control)
(cd "$W/data" && "${TAR[@]}" -czf "$W/ipk/data.tar.gz" ./usr)
IPK=$OUT/${NAME}_${VER}_${OARCH}.ipk
(cd "$W/ipk" && "${TAR[@]}" -czf "$IPK" ./debian-binary ./control.tar.gz ./data.tar.gz)
echo "built $IPK"

# --- apk ---
APK=${APK:-$(command -v apk || true)}
[ -n "$APK" ] || { echo "no apk-tools: skipping .apk" >&2; exit 0; }
AVER=${VER%-*}-r${VER##*-}
mkdir -p "$W/data/lib/apk/packages"
echo /usr/bin/meow > "$W/data/lib/apk/packages/$NAME.list"
dep=""; for c in $CONFL; do dep="$dep !$c"; done; dep=${dep# }
"$APK" mkpkg \
	--info "name:$NAME" --info "version:$AVER" --info "description:$DESC" \
	--info "arch:$OARCH" --info "license:GPL-3.0-only" --info "origin:meow" \
	--info "url:$URL" --info "maintainer:mrFrok" --info "depends:$dep" \
	--info "provides:meow-core" \
	--files "$W/data" --output "$OUT/${NAME}-${AVER}_${OARCH}.apk"
echo "built $OUT/${NAME}-${AVER}_${OARCH}.apk"
