//! Small helpers: randomness, hex, hashing, time, name checks.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Bytes from the operating system CSPRNG. The process cannot run safely without it, so failure aborts.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("operating system CSPRNG unavailable");
    b
}

pub fn hex(b: &[u8]) -> String {
    use std::fmt::Write as _;
    b.iter()
        .fold(String::with_capacity(b.len() * 2), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

pub fn new_id() -> String {
    hex(&random_bytes::<16>())
}

pub fn sha256_hex(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

/// 256-bit token, base64url without padding (43 characters).
pub fn new_token() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

pub const TOKEN_LEN: usize = 43;

pub fn token_shape_ok(s: &str) -> bool {
    s.len() == TOKEN_LEN
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

pub fn ct_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

pub fn is_hex_id(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

/// A display name for a document. It is never used as a path. Rejects separators and control characters.
pub fn clean_name(raw: &str) -> Option<String> {
    let n = raw.trim();
    let ok = !n.is_empty()
        && n.chars().count() <= 200
        && n != "."
        && n != ".."
        && !n.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | '"' | '<' | '>' | '|' | '?' | '*' | ':')
        });
    ok.then(|| n.to_string())
}

/// `Content-Disposition` value: ASCII fallback plus an RFC 5987 `filename*`.
pub fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut enc = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
            enc.push(b as char);
        } else {
            enc.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{enc}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_256_bit_and_shaped() {
        let t = new_token();
        assert!(token_shape_ok(&t));
        let raw = URL_SAFE_NO_PAD.decode(&t).expect("decode");
        assert_eq!(raw.len(), 32);
        assert_ne!(t, new_token());
    }

    #[test]
    fn names_reject_traversal_and_controls() {
        for bad in ["", "..", "a/b", "a\\b", "a\0b", "x\ny", "C:evil"] {
            assert!(clean_name(bad).is_none(), "{bad:?}");
        }
        assert_eq!(
            clean_name(" Budget 2026.xlsx ").as_deref(),
            Some("Budget 2026.xlsx")
        );
    }

    #[test]
    fn hex_round_trips() {
        let b = random_bytes::<8>();
        assert_eq!(unhex(&hex(&b)).as_deref(), Some(&b[..]));
        assert!(unhex("zz").is_none());
    }
}
