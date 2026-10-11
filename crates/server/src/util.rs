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

/// Creates a directory tree readable only by the server account (0700 on Unix; on Windows protect the data
/// directory with an ACL, see the README).
pub fn make_private_dir(p: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(p)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(p)
    }
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

#[cfg(test)]
mod prop {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn generated_tokens_always_have_the_expected_shape(_n in 0..16u8) {
            let t = new_token();
            prop_assert!(token_shape_ok(&t));
            prop_assert_eq!(t.len(), TOKEN_LEN);
        }

        #[test]
        fn only_43_url_safe_characters_pass_token_shape(s in r"\PC{0,60}") {
            if token_shape_ok(&s) {
                prop_assert_eq!(s.len(), TOKEN_LEN);
                prop_assert!(s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
            }
        }

        #[test]
        fn a_single_bad_character_breaks_a_token(pos in 0..TOKEN_LEN, bad in prop_oneof![Just('/'), Just('.'), Just(' '), Just('+'), Just('\n'), Just('%')]) {
            let mut t: Vec<char> = new_token().chars().collect();
            t[pos] = bad;
            prop_assert!(!token_shape_ok(&t.into_iter().collect::<String>()));
        }

        #[test]
        fn clean_names_never_carry_separators_or_controls(raw in r"\PC{0,260}") {
            if let Some(n) = clean_name(&raw) {
                prop_assert!(n.chars().count() <= 200 && !n.is_empty());
                prop_assert!(!n.chars().any(|c| c.is_control() || "/\\\"<>|?*:".contains(c)));
                prop_assert!(n != "." && n != "..");
            }
        }

        #[test]
        fn content_disposition_cannot_be_injected_through_a_name(raw in r"\PC{0,60}") {
            let d = content_disposition(&raw);
            prop_assert!(!d.contains('\n') && !d.contains('\r'));
            let ascii = d.split("filename=\"").nth(1).and_then(|r| r.split('"').next()).expect("quoted");
            prop_assert!(ascii.chars().all(|c| c.is_ascii_alphanumeric() || ".-_ ".contains(c)));
        }

        #[test]
        fn hex_round_trips_for_any_bytes(b in proptest::collection::vec(any::<u8>(), 0..64)) {
            prop_assert_eq!(unhex(&hex(&b)), Some(b));
        }

        #[test]
        fn hex_ids_are_exact(s in "[0-9a-f]{0,40}") {
            prop_assert_eq!(is_hex_id(&s, 32), s.len() == 32);
        }
    }
}

/// The short code that ties a native app's screen to the browser page of an OpenID Connect hand-off: the first eight
/// hex digits of the SHA-256 of the app's secret, as `ABCD-EF01`.
pub fn handoff_user_code(handoff_hash: &str) -> String {
    let h = handoff_hash.to_ascii_uppercase();
    format!("{}-{}", &h[..4], &h[4..8])
}
