# PGO profiles

`<arch>.profdata.xz` is the merged LLVM profile `ci/build.sh` applies to the
lightweight and full builds of that arch (`-Cprofile-use`). tiny is built
without it. Functions changed since the profile was taken just lose their
profile, so refresh it before larger releases.

Refresh (aarch64, BPI-R4, about 20 minutes):

1. `SDK=~/mybuild/sdk-filogic TGT=target/pgo PROFILE=full sh openwrt-fork/bench/pgo-gen.sh`
   builds an instrumented full meow (rustup 1.96, the release rustc).
2. On the router, the tunbench stand (`bench/tunbench.sh up`, mihomo server), then
   `PAR=1 SECS=8 sh tunbench.sh run train <instrumented meow> hy2tcp-dl hy2tcp-ul hy2-dl hy2-ul vrtcp-dl vrtcp-ul tcp-dl tcp-ul`;
   the profile is written to `/tmp/pgo/*.profraw` when meow exits.
3. `llvm-profdata merge` (rustup 1.96 `llvm-tools`) the `.profraw` files and
   `xz -9` the result here.

Training uses one stream for 8 s per scenario; the measurements in PLAN.md use
four streams and longer runs, so the profile is not fitted to the benchmark.
