#!/bin/sh
# Stripped binary size of the fork per arch and tag set.
# Usage: bench/size.sh [outdir]   (run from the repo root)
# Env: ARCHES="arm64 armv7 mipsle-sf mips-sf amd64v1", SETS="official lean"
set -eu
export LC_ALL=C
OUT=${1:-/tmp/mlw-size}; mkdir -p "$OUT"
ARCHES=${ARCHES:-"arm64 armv7 mipsle-sf mips-sf amd64v1"}
SETS=${SETS:-"official lean"}
tags() { case $1 in
	official) echo "with_gvisor" ;;
	lean) echo "with_gvisor no_tailscale no_zerotier no_easytier with_low_memory no_fake_tcp" ;;
	*) echo "$1" ;;
esac; }
envs() { case $1 in
	arm64) echo "GOARCH=arm64" ;;
	armv7) echo "GOARCH=arm GOARM=7" ;;
	mipsle-sf) echo "GOARCH=mipsle GOMIPS=softfloat" ;;
	mips-sf) echo "GOARCH=mips GOMIPS=softfloat" ;;
	amd64v1) echo "GOARCH=amd64 GOAMD64=v1" ;;
esac; }
printf '%-10s %-9s %10s %8s\n' arch set bytes MiB
for s in $SETS; do for a in $ARCHES; do
	b="$OUT/mihomo-$a-$s"
	env CGO_ENABLED=0 GOOS=linux $(envs "$a") go build -tags "$(tags "$s")" -trimpath \
		-ldflags "-s -w -buildid=" -o "$b" . 
	sz=$(stat -c%s "$b")
	printf '%-10s %-9s %10d %8.2f\n' "$a" "$s" "$sz" "$(awk "BEGIN{print $sz/1048576}")"
done; done
echo "go: $(go version)"
