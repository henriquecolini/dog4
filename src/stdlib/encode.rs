//! Encodings and hashes: the `url`, `base64` and `hex` modules, `sha256` and
//! `hmac_sha256`. Text is always UTF-8: decoding to bytes that aren't is an
//! error, since dog4 strings can't hold them.

use sha2::{Digest, Sha256};

use super::{Library, checked_str, join_display, text};
use crate::error::{Error, Result};
use crate::value::{Map, Value};
use crate::vm::Vm;

/// Charges fuel for a pass over `len` bytes.
fn charge(vm: &mut Vm, len: usize) -> Result<()> {
    vm.meter().tick(len as u64 / 64)
}

fn utf8(bytes: Vec<u8>, what: &str) -> Result<Value> {
    String::from_utf8(bytes)
        .map(Value::str)
        .map_err(|_| Error::value_error(format!("the decoded {what} is not UTF-8 text")))
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 15)] as char);
    }
    out
}

fn hex_digit(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

// ----- url --------------------------------------------------------------------------------------

/// Percent-encodes everything but the unreserved characters `A-Z a-z 0-9 - . _ ~`.
pub(crate) fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&to_hex(&[b]).to_uppercase());
        }
    }
    out
}

/// Decodes `%XX` escapes (and, in query strings, `+` as a space).
fn url_decode(s: &str, plus_is_space: bool) -> Result<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let byte = bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| Some(hex_digit(h[0])? * 16 + hex_digit(h[1])?))
                    .ok_or_else(|| {
                        Error::value_error(format!("invalid percent escape at byte {i} of {s:?}"))
                    })?;
                out.push(byte);
                i += 3;
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| Error::value_error("the decoded URL text is not UTF-8"))
}

/// `a=1&b=2` from a map; array values repeat the key and `null` values are left out.
pub(crate) fn query_string(map: &Map) -> String {
    let mut parts = Vec::new();
    for (key, value) in map {
        let values = match value {
            Value::Arr(items) => items.to_vec(),
            Value::Null => Vec::new(),
            other => vec![other.clone()],
        };
        for value in values {
            parts.push(format!(
                "{}={}",
                url_encode(key),
                url_encode(&join_display(&[value], ""))
            ));
        }
    }
    parts.join("&")
}

/// Parses `a=1&b=2&b=3` into `[a = 1, b = [2 3]]` (values stay text).
fn parse_query(query: &str) -> Result<Map> {
    let mut map = Map::default();
    for part in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        let key = url_decode(key, true)?;
        let value = Value::str(url_decode(value, true)?);
        match map.get_mut(key.as_str()) {
            Some(Value::Arr(values)) => values.make_mut().push(value),
            Some(first) => *first = Value::arr(vec![std::mem::take(first), value]),
            None => {
                map.insert(key.into(), value);
            }
        }
    }
    Ok(map)
}

/// `scheme://user:password@host:port/path?query#fragment` as a map.
fn parse_url(url: &str) -> Result<Value> {
    let invalid = |why: &str| Error::value_error(format!("{url:?} is not a URL: {why}"));
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| invalid("it has no `scheme://`"))?;
    let scheme_ok = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok {
        return Err(invalid("its scheme is not valid"));
    }
    let (rest, fragment) = match rest.split_once('#') {
        Some((rest, fragment)) => (rest, Value::str(fragment)),
        None => (rest, Value::Null),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((rest, query)) => (rest, parse_query(query)?),
        None => (rest, Map::default()),
    };
    let (authority, path) = match rest.find('/') {
        Some(slash) => (&rest[..slash], &rest[slash..]),
        None => (rest, "/"),
    };
    let (userinfo, host_port) = match authority.rsplit_once('@') {
        Some((userinfo, host_port)) => (Some(userinfo), host_port),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(info) => match info.split_once(':') {
            Some((user, password)) => (
                Value::str(url_decode(user, false)?),
                Value::str(url_decode(password, false)?),
            ),
            None => (Value::str(url_decode(info, false)?), Value::Null),
        },
        None => (Value::Null, Value::Null),
    };
    // `[::1]:8080` (IPv6) or `host:8080`.
    let port_at = match host_port.rfind(']') {
        Some(bracket) => host_port[bracket..].find(':').map(|i| bracket + i),
        None => host_port.rfind(':'),
    };
    let (host, port) = match port_at {
        Some(colon) => {
            let port: u16 = host_port[colon + 1..]
                .parse()
                .map_err(|_| invalid("its port is not a number from 0 to 65535"))?;
            (&host_port[..colon], Value::Int(port.into()))
        }
        None => (host_port, Value::Null),
    };
    if host.is_empty() {
        return Err(invalid("it has no host"));
    }
    let mut map = Map::default();
    for (key, value) in [
        ("scheme", Value::str(scheme.to_ascii_lowercase())),
        ("user", user),
        ("password", password),
        ("host", Value::str(host)),
        ("port", port),
        ("path", Value::str(path)),
        ("query", Value::map(query)),
        ("fragment", fragment),
    ] {
        map.insert(key.into(), value);
    }
    Ok(Value::map(map))
}

// ----- base64 -----------------------------------------------------------------------------------

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes base64, standard (`+/`) or URL-safe (`-_`), with or without padding.
fn base64_decode(s: &str) -> Result<Vec<u8>> {
    let invalid = || Error::value_error("invalid base64 text");
    let digits = s.trim_end_matches('=').as_bytes();
    if digits.len() % 4 == 1 {
        return Err(invalid());
    }
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' | b'-' => 62,
                b'/' | b'_' => 63,
                _ => return Err(invalid()),
            };
            n |= u32::from(v) << (18 - 6 * i);
        }
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Ok(out)
}

// ----- hashes -----------------------------------------------------------------------------------

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.map(|b| b ^ byte);
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

pub(super) fn install(lib: &mut Library) {
    let mut url = Library::new();
    url.def("encode", "text: str", |vm, args| {
        let s = text(&args[0]);
        charge(vm, s.len())?;
        checked_str(vm, url_encode(s))
    });
    url.def("decode", "text: str", |vm, args| {
        let s = text(&args[0]);
        charge(vm, s.len())?;
        Ok(Value::str(url_decode(s, false)?))
    });
    url.def("query", "params: map", |vm, args| {
        let Value::Map(map) = &args[0] else {
            return Ok(Value::str(""));
        };
        vm.meter().tick(map.len() as u64)?;
        checked_str(vm, query_string(map))
    });
    url.def("parse", "url: str", |vm, args| {
        let s = text(&args[0]);
        charge(vm, s.len())?;
        parse_url(s)
    });
    lib.module("url", url);

    let mut base64 = Library::new();
    base64.def("encode", "text: str", |vm, args| {
        let s = text(&args[0]);
        charge(vm, s.len())?;
        vm.meter().str_len(s.len().div_ceil(3) * 4)?;
        Ok(Value::str(base64_encode(s.as_bytes())))
    });
    base64.def("decode", "text: str", |vm, args| {
        let s = text(&args[0]);
        charge(vm, s.len())?;
        utf8(base64_decode(s.trim())?, "base64")
    });
    lib.module("base64", base64);

    let mut hex = Library::new();
    hex.def("encode", "text: str", |vm, args| {
        let s = text(&args[0]);
        charge(vm, s.len())?;
        vm.meter().str_len(s.len().saturating_mul(2))?;
        Ok(Value::str(to_hex(s.as_bytes())))
    });
    hex.def("decode", "text: str", |vm, args| {
        let s = text(&args[0]).trim().as_bytes();
        charge(vm, s.len())?;
        if s.len() % 2 != 0 {
            return Err(Error::value_error(
                "hex text needs an even number of digits",
            ));
        }
        let bytes = s
            .chunks(2)
            .map(|pair| Some(hex_digit(pair[0])? * 16 + hex_digit(pair[1])?))
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(|| Error::value_error("invalid hex digit"))?;
        utf8(bytes, "hex")
    });
    lib.module("hex", hex);

    lib.def("sha256", "text: str", |vm, args| {
        let s = text(&args[0]);
        charge(vm, s.len())?;
        Ok(Value::str(to_hex(&Sha256::digest(s.as_bytes()))))
    });
    lib.def("hmac_sha256", "key: str, text: str", |vm, args| {
        let (key, message) = (text(&args[0]), text(&args[1]));
        charge(vm, key.len() + message.len())?;
        Ok(Value::str(to_hex(&hmac_sha256(
            key.as_bytes(),
            message.as_bytes(),
        ))))
    });
}
