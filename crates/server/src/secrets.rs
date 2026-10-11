//! Encryption of secrets stored in the database (TOTP shared secrets) under a server key that is never in the data
//! directory, so a copy of the database or a backup does not hold them in the clear.
//!
//! Cipher: XChaCha20-Poly1305 (RustCrypto `chacha20poly1305`, MIT OR Apache-2.0), a fresh random 192-bit nonce per
//! value. The associated data names the owner (`totp:<user id>`), so a sealed value cannot be moved to another row.
//!
//! Stored form: `v1:<key id>:<base64url(nonce || ciphertext || tag)>`. The key id lets the server hold several keys
//! while a rotation is in progress: the newest key seals, any listed key opens, and `open365-server rotate-secrets`
//! re-seals every value under the newest key so that an old key can be retired.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rusqlite::{Connection, params};

use crate::util::random_bytes;

const PREFIX: &str = "v1:";
const NONCE_LEN: usize = 24;

#[derive(Clone)]
pub struct SecretKeys {
    /// `(key id, 32-byte key)`, oldest first; the last entry seals new values.
    keys: Vec<(String, [u8; 32])>,
}

impl std::fmt::Debug for SecretKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&String> = self.keys.iter().map(|(id, _)| id).collect();
        f.debug_struct("SecretKeys")
            .field("ids", &ids)
            .finish_non_exhaustive()
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

impl SecretKeys {
    /// Keys oldest first; ids unique; each key exactly 32 bytes.
    pub fn list(keys: Vec<(String, Vec<u8>)>) -> Result<Self, String> {
        if keys.is_empty() {
            return Err("at least one secret key is required".into());
        }
        let mut out: Vec<(String, [u8; 32])> = vec![];
        for (id, k) in keys {
            if !valid_id(&id) {
                return Err("secret key ids use letters, digits, '-' and '_' (at most 32)".into());
            }
            if out.iter().any(|(i, _)| *i == id) {
                return Err("secret key ids must be unique".into());
            }
            let k: [u8; 32] = k
                .try_into()
                .map_err(|_| "a secret key must be exactly 32 bytes (64 hex digits)".to_string())?;
            out.push((id, k));
        }
        Ok(Self { keys: out })
    }

    /// Keys oldest first with the current key named explicitly; it must exist and be the last entry.
    pub fn with_current(keys: Vec<(String, Vec<u8>)>, current: &str) -> Result<Self, String> {
        let k = Self::list(keys)?;
        if k.keys.iter().all(|(i, _)| i != current) {
            return Err("the named current secret key is not among the keys".into());
        }
        if k.current_id() != current {
            return Err("the current secret key must be the last (newest) entry".into());
        }
        Ok(k)
    }

    pub fn single(id: &str, key: Vec<u8>) -> Result<Self, String> {
        Self::list(vec![(id.to_string(), key)])
    }

    pub fn current_id(&self) -> &str {
        &self.keys.last().expect("SecretKeys is never empty").0
    }

    fn cipher(key: &[u8; 32]) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(&(*key).into())
    }

    /// Encrypts `plaintext` for the row named by `aad` under the newest key.
    pub fn seal(&self, aad: &str, plaintext: &str) -> Option<String> {
        let (id, key) = self.keys.last()?;
        let nonce = random_bytes::<NONCE_LEN>();
        let ct = Self::cipher(key)
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .ok()?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ct);
        Some(format!("{PREFIX}{id}:{}", URL_SAFE_NO_PAD.encode(blob)))
    }

    /// Decrypts a value made by [`seal`](Self::seal). `None` for plaintext, an unknown key id, a different `aad`
    /// or any tampering.
    pub fn open(&self, aad: &str, stored: &str) -> Option<String> {
        let (id, b64) = stored.strip_prefix(PREFIX)?.split_once(':')?;
        let key = &self.keys.iter().find(|(i, _)| i == id)?.1;
        let blob = URL_SAFE_NO_PAD.decode(b64).ok()?;
        if blob.len() < NONCE_LEN + 16 {
            return None;
        }
        let (nonce, ct) = blob.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;
        let pt = Self::cipher(key)
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ct,
                    aad: aad.as_bytes(),
                },
            )
            .ok()?;
        String::from_utf8(pt).ok()
    }

    /// True for a value that is plaintext or sealed under a key other than the newest.
    pub fn needs_rotation(&self, stored: &str) -> bool {
        match stored.strip_prefix(PREFIX).and_then(|r| r.split_once(':')) {
            Some((id, _)) => id != self.current_id(),
            None => true,
        }
    }
}

/// The associated data for a user's TOTP secret.
pub fn totp_aad(user_id: &str) -> String {
    format!("totp:{user_id}")
}

/// Counts of values changed by [`reseal_all`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resealed {
    pub sealed_plaintext: usize,
    pub rotated: usize,
    pub unreadable: usize,
}

/// Seals every plaintext TOTP secret (rows from before encryption existed) and moves values sealed under an older
/// key to the newest. Values no listed key can open are counted and left alone. One transaction.
pub fn reseal_all(conn: &mut Connection, keys: &SecretKeys) -> rusqlite::Result<Resealed> {
    let tx = conn.transaction()?;
    let rows: Vec<(String, String)> = {
        let mut s =
            tx.prepare("SELECT id, totp_secret FROM users WHERE totp_secret IS NOT NULL")?;
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    let mut out = Resealed::default();
    for (id, stored) in rows {
        if !keys.needs_rotation(&stored) {
            continue;
        }
        let aad = totp_aad(&id);
        let (plain, was_plain) = if stored.starts_with(PREFIX) {
            (keys.open(&aad, &stored), false)
        } else {
            (Some(stored), true)
        };
        let Some(plain) = plain else {
            out.unreadable += 1;
            continue;
        };
        let Some(sealed) = keys.seal(&aad, &plain) else {
            out.unreadable += 1;
            continue;
        };
        tx.execute(
            "UPDATE users SET totp_secret = ?1 WHERE id = ?2",
            params![sealed, id],
        )?;
        if was_plain {
            out.sealed_plaintext += 1;
        } else {
            out.rotated += 1;
        }
    }
    tx.commit()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(ids: &[(&str, u8)]) -> SecretKeys {
        SecretKeys::list(
            ids.iter()
                .map(|(i, b)| (i.to_string(), vec![*b; 32]))
                .collect(),
        )
        .expect("keys")
    }

    #[test]
    fn round_trip_and_nonces_differ() {
        let k = keys(&[("s1", 1)]);
        let a = k.seal("totp:u1", "JBSWY3DPEHPK3PXP").expect("seal");
        let b = k.seal("totp:u1", "JBSWY3DPEHPK3PXP").expect("seal");
        assert_ne!(a, b, "fresh nonce each time");
        assert!(a.starts_with("v1:s1:") && !a.contains("JBSWY3DPEHPK3PXP"));
        assert_eq!(k.open("totp:u1", &a).as_deref(), Some("JBSWY3DPEHPK3PXP"));
    }

    #[test]
    fn rejects_other_row_tampering_plaintext_and_wrong_key() {
        let k = keys(&[("s1", 1)]);
        let a = k.seal("totp:u1", "SECRET").expect("seal");
        assert!(k.open("totp:u2", &a).is_none(), "bound to the row");
        let mut t = a.clone();
        t.pop();
        t.push(if a.ends_with('A') { 'B' } else { 'A' });
        assert!(k.open("totp:u1", &t).is_none(), "tag check");
        assert!(
            k.open("totp:u1", "SECRET").is_none(),
            "no plaintext fallback"
        );
        assert!(keys(&[("s1", 2)]).open("totp:u1", &a).is_none());
        assert!(
            keys(&[("s9", 1)]).open("totp:u1", &a).is_none(),
            "unknown id"
        );
    }

    #[test]
    fn rotation_keeps_old_values_readable_until_resealed() {
        let old = keys(&[("s1", 1)]);
        let sealed_old = old.seal("totp:u1", "SECRET").expect("seal");
        let both = keys(&[("s1", 1), ("s2", 2)]);
        assert_eq!(both.open("totp:u1", &sealed_old).as_deref(), Some("SECRET"));
        assert!(both.needs_rotation(&sealed_old));
        let fresh = both.seal("totp:u1", "SECRET").expect("seal");
        assert!(fresh.starts_with("v1:s2:") && !both.needs_rotation(&fresh));
        assert!(
            keys(&[("s2", 2)]).open("totp:u1", &fresh).is_some(),
            "old key can be retired"
        );
    }

    #[test]
    fn key_validation() {
        assert!(SecretKeys::single("s1", vec![0; 31]).is_err());
        assert!(SecretKeys::single("a:b", vec![0; 32]).is_err());
        assert!(SecretKeys::list(vec![]).is_err());
        assert!(
            SecretKeys::list(vec![("a".into(), vec![0; 32]), ("a".into(), vec![1; 32])]).is_err()
        );
        assert!(format!("{:?}", keys(&[("s1", 7)])).find("07").is_none());
    }
}
