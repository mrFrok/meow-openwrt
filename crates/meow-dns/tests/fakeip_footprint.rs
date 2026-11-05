//! Memory per entry of the fake-ip pool and the DNS cache at full capacity
//! (meow-openwrt stage 3: sizing cache limits from MemTotal).
//!
//!     cargo test -p meow-dns --test fakeip_footprint --release -- --ignored --nocapture
//!
//! Prints RSS growth per entry. Hosts look like browsing traffic
//! (`<n>.cdn-<k>.example-site.com`, ~28 bytes).

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use meow_dns::{DnsCache, MemoryStore, Pool};

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.trim().trim_end_matches(" kB").trim().parse().ok())
        })
        .unwrap_or(0)
}

fn host(i: usize) -> String {
    format!("{i}.cdn-{}.example-site.com", i % 97)
}

#[test]
#[ignore = "footprint measurement; opt in with --ignored"]
fn fakeip_pool_and_dns_cache_bytes_per_entry() {
    let n = 65_534;
    let base = rss_kb();
    let pool = Pool::new(
        "198.18.0.0/16".parse().unwrap(),
        Arc::new(MemoryStore::new(n)),
    )
    .unwrap();
    for i in 0..n {
        std::hint::black_box(pool.lookup(&host(i)));
    }
    let pool_kb = rss_kb().saturating_sub(base);
    println!(
        "fake-ip pool: {n} entries, +{pool_kb} KiB, {} B/entry",
        pool_kb * 1024 / n as u64
    );

    let base = rss_kb();
    let cap = 4096;
    let cache = DnsCache::new(cap);
    for i in 0..cap {
        let ips = [
            IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i as u32 * 2)),
            IpAddr::V4(Ipv4Addr::from(0x0a00_0001 + i as u32 * 2)),
        ];
        cache.put(&host(i), &ips, Duration::from_secs(600));
    }
    let cache_kb = rss_kb().saturating_sub(base);
    println!(
        "dns cache: {cap} entries x2 IPs, +{cache_kb} KiB, {} B/entry",
        cache_kb * 1024 / cap as u64
    );
    drop((pool, cache));
}
