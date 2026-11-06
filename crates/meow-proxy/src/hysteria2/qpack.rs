//! A static-table-only QPACK codec (RFC 9204) for the one HTTP/3 request
//! hysteria2 makes: the auth exchange runs with no dynamic table (empty
//! SETTINGS), so only static references and literals occur. Values are sent
//! without Huffman coding and decoded with it (`octets`).
//!
//! The static table below is taken from quiche (src/h3/qpack/static_table.rs):
//! Copyright (C) 2019, Cloudflare, Inc. All rights reserved. Redistribution
//! and use in source and binary forms, with or without modification, are
//! permitted under the BSD 2-Clause license (see quiche's COPYING).

use super::{Error, Result};

/// A decoded header list.
pub(super) type Headers = Vec<(Vec<u8>, Vec<u8>)>;

/// Encode `headers` as a field section (prefix: no dynamic table).
pub(super) fn encode(headers: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut out = vec![0, 0];
    for &(name, value) in headers {
        let exact = STATIC_TABLE
            .iter()
            .position(|&(n, v)| n == name && v == value);
        let by_name = STATIC_TABLE.iter().position(|&(n, _)| n == name);
        if let Some(i) = exact {
            put_int(&mut out, 0xc0, 6, i as u64); // indexed, static
        } else if let Some(i) = by_name {
            put_int(&mut out, 0x50, 4, i as u64); // literal, static name ref
            put_int(&mut out, 0, 7, value.len() as u64);
            out.extend_from_slice(value);
        } else {
            put_int(&mut out, 0x20, 3, name.len() as u64); // literal name
            out.extend_from_slice(name);
            put_int(&mut out, 0, 7, value.len() as u64);
            out.extend_from_slice(value);
        }
    }
    out
}

/// Decode a field section that references no dynamic table, at most `max`
/// bytes of names and values in all.
pub(super) fn decode(block: &[u8], max: usize) -> Result<Headers> {
    let err = |m: &str| Error::Http3(format!("QPACK: {m}"));
    let mut b = octets::Octets::with_slice(block);
    if get_int(&mut b, 8)? != 0 {
        return Err(err("dynamic table reference"));
    }
    get_int(&mut b, 7)?; // delta base, meaningless without a dynamic table
    let mut out = Vec::new();
    let mut total = 0usize;
    while b.cap() > 0 {
        let first = b.peek_u8().map_err(|_| err("truncated"))?;
        let (name, value) = if first & 0x80 != 0 {
            if first & 0x40 == 0 {
                return Err(err("dynamic table reference"));
            }
            let (n, v) = static_entry(get_int(&mut b, 6)?)?;
            (n.to_vec(), v.to_vec())
        } else if first & 0x40 != 0 {
            if first & 0x10 == 0 {
                return Err(err("dynamic table reference"));
            }
            let (n, _) = static_entry(get_int(&mut b, 4)?)?;
            (n.to_vec(), get_str(&mut b, 7, max)?)
        } else if first & 0x20 != 0 {
            let name = get_str(&mut b, 3, max)?;
            (name, get_str(&mut b, 7, max)?)
        } else {
            return Err(err("post-base reference"));
        };
        total += name.len() + value.len();
        if total > max {
            return Err(err("header list too large"));
        }
        out.push((name, value));
    }
    Ok(out)
}

fn static_entry(i: u64) -> Result<(&'static [u8], &'static [u8])> {
    usize::try_from(i)
        .ok()
        .and_then(|i| STATIC_TABLE.get(i).copied())
        .ok_or_else(|| Error::Http3("QPACK: bad static index".into()))
}

/// RFC 7541 §5.1 integer with an `n`-bit prefix, flags in `first`.
fn put_int(out: &mut Vec<u8>, first: u8, n: u32, v: u64) {
    let max = (1u64 << n) - 1;
    if v < max {
        out.push(first | v as u8);
        return;
    }
    out.push(first | max as u8);
    let mut v = v - max;
    while v >= 128 {
        out.push((v % 128) as u8 | 0x80);
        v /= 128;
    }
    out.push(v as u8);
}

fn get_int(b: &mut octets::Octets<'_>, n: u32) -> Result<u64> {
    let bad = || Error::Http3("QPACK: bad integer".into());
    let max = (1u64 << n) - 1;
    let mut v = u64::from(b.get_u8().map_err(|_| bad())?) & max;
    if v < max {
        return Ok(v);
    }
    let mut shift = 0;
    loop {
        let byte = b.get_u8().map_err(|_| bad())?;
        if shift > 56 {
            return Err(bad());
        }
        v += u64::from(byte & 0x7f) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
}

/// A string literal whose Huffman flag sits just above the `n`-bit length.
fn get_str(b: &mut octets::Octets<'_>, n: u32, max: usize) -> Result<Vec<u8>> {
    let bad = |m: &str| Error::Http3(format!("QPACK: {m}"));
    let huffman = b.peek_u8().map_err(|_| bad("truncated"))? & (1 << n) != 0;
    let len = usize::try_from(get_int(b, n)?).map_err(|_| bad("string too long"))?;
    if len > max {
        return Err(bad("string too long"));
    }
    let mut s = b.get_bytes(len).map_err(|_| bad("truncated string"))?;
    if huffman {
        s.get_huffman_decoded_with_max_length(max)
            .map_err(|_| bad("bad Huffman string"))
    } else {
        Ok(s.to_vec())
    }
}

const STATIC_TABLE: [(&[u8], &[u8]); 99] = [
    (b":authority", b""),
    (b":path", b"/"),
    (b"age", b"0"),
    (b"content-disposition", b""),
    (b"content-length", b"0"),
    (b"cookie", b""),
    (b"date", b""),
    (b"etag", b""),
    (b"if-modified-since", b""),
    (b"if-none-match", b""),
    (b"last-modified", b""),
    (b"link", b""),
    (b"location", b""),
    (b"referer", b""),
    (b"set-cookie", b""),
    (b":method", b"CONNECT"),
    (b":method", b"DELETE"),
    (b":method", b"GET"),
    (b":method", b"HEAD"),
    (b":method", b"OPTIONS"),
    (b":method", b"POST"),
    (b":method", b"PUT"),
    (b":scheme", b"http"),
    (b":scheme", b"https"),
    (b":status", b"103"),
    (b":status", b"200"),
    (b":status", b"304"),
    (b":status", b"404"),
    (b":status", b"503"),
    (b"accept", b"*/*"),
    (b"accept", b"application/dns-message"),
    (b"accept-encoding", b"gzip, deflate, br"),
    (b"accept-ranges", b"bytes"),
    (b"access-control-allow-headers", b"cache-control"),
    (b"access-control-allow-headers", b"content-type"),
    (b"access-control-allow-origin", b"*"),
    (b"cache-control", b"max-age=0"),
    (b"cache-control", b"max-age=2592000"),
    (b"cache-control", b"max-age=604800"),
    (b"cache-control", b"no-cache"),
    (b"cache-control", b"no-store"),
    (b"cache-control", b"public, max-age=31536000"),
    (b"content-encoding", b"br"),
    (b"content-encoding", b"gzip"),
    (b"content-type", b"application/dns-message"),
    (b"content-type", b"application/javascript"),
    (b"content-type", b"application/json"),
    (b"content-type", b"application/x-www-form-urlencoded"),
    (b"content-type", b"image/gif"),
    (b"content-type", b"image/jpeg"),
    (b"content-type", b"image/png"),
    (b"content-type", b"text/css"),
    (b"content-type", b"text/html; charset=utf-8"),
    (b"content-type", b"text/plain"),
    (b"content-type", b"text/plain;charset=utf-8"),
    (b"range", b"bytes=0-"),
    (b"strict-transport-security", b"max-age=31536000"),
    (
        b"strict-transport-security",
        b"max-age=31536000; includesubdomains",
    ),
    (
        b"strict-transport-security",
        b"max-age=31536000; includesubdomains; preload",
    ),
    (b"vary", b"accept-encoding"),
    (b"vary", b"origin"),
    (b"x-content-type-options", b"nosniff"),
    (b"x-xss-protection", b"1; mode=block"),
    (b":status", b"100"),
    (b":status", b"204"),
    (b":status", b"206"),
    (b":status", b"302"),
    (b":status", b"400"),
    (b":status", b"403"),
    (b":status", b"421"),
    (b":status", b"425"),
    (b":status", b"500"),
    (b"accept-language", b""),
    (b"access-control-allow-credentials", b"FALSE"),
    (b"access-control-allow-credentials", b"TRUE"),
    (b"access-control-allow-headers", b"*"),
    (b"access-control-allow-methods", b"get"),
    (b"access-control-allow-methods", b"get, post, options"),
    (b"access-control-allow-methods", b"options"),
    (b"access-control-expose-headers", b"content-length"),
    (b"access-control-request-headers", b"content-type"),
    (b"access-control-request-method", b"get"),
    (b"access-control-request-method", b"post"),
    (b"alt-svc", b"clear"),
    (b"authorization", b""),
    (
        b"content-security-policy",
        b"script-src 'none'; object-src 'none'; base-uri 'none'",
    ),
    (b"early-data", b"1"),
    (b"expect-ct", b""),
    (b"forwarded", b""),
    (b"if-range", b""),
    (b"origin", b""),
    (b"purpose", b"prefetch"),
    (b"server", b""),
    (b"timing-allow-origin", b"*"),
    (b"upgrade-insecure-requests", b"1"),
    (b"user-agent", b""),
    (b"x-forwarded-for", b""),
    (b"x-frame-options", b"deny"),
    (b"x-frame-options", b"sameorigin"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_static_and_literal_fields() {
        let headers: &[(&[u8], &[u8])] = &[
            (b":method", b"POST"),
            (b":path", b"/auth"),
            (b":authority", b"hysteria"),
            (b"hysteria-auth", b"secret"),
            (b"hysteria-padding", &[b'x'; 300]),
        ];
        let got = decode(&encode(headers), 4096).unwrap();
        assert_eq!(got.len(), headers.len());
        for ((n, v), (en, ev)) in got.iter().zip(headers) {
            assert_eq!((n.as_slice(), v.as_slice()), (*en, *ev));
        }
    }

    #[test]
    fn decodes_huffman_values() {
        // RFC 7541 C.4.1: "www.example.com", Huffman-coded, as a literal
        // with a static name ref to :authority (index 0).
        let mut block = vec![0, 0, 0x50, 0x8c];
        block.extend_from_slice(&[
            0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab, 0x90, 0xf4, 0xff,
        ]);
        let got = decode(&block, 4096).unwrap();
        assert_eq!(got[0].1, b"www.example.com");
    }

    #[test]
    fn rejects_dynamic_references_and_oversize() {
        assert!(decode(&[1, 0], 4096).is_err());
        assert!(decode(&[0, 0, 0x80], 4096).is_err());
        assert!(decode(&[0, 0, 0x10], 4096).is_err());
        assert!(decode(&encode(&[(b"x-a", &[b'v'; 100])]), 50).is_err());
    }
}
