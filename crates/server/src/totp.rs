//! TOTP (RFC 6238, SHA-1, 6 digits, 30 s steps) as a second factor for local accounts.
//!
//! A code is accepted for the current step and one step either side, and each step can be used once (replay
//! protection through `users.totp_last_step`). Attempts are counted before they are checked.
//!
//! The shared secret is stored encrypted under the server's secret key (`crate::secrets`), so a copy of the database
//! or a backup does not expose it. Ten single-use recovery codes (80 bits each, stored as SHA-256 only) are issued
//! when TOTP is confirmed and can be regenerated with a current code; a recovery code replaces the TOTP code at
//! sign-in.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use hmac::{Hmac, Mac};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::Sha1;

use crate::audit;
use crate::client::pct_encode;
use crate::error::{ApiError, ApiResult};
use crate::secrets::{SecretKeys, totp_aad};
use crate::util::{ct_eq, now, random_bytes, sha256_hex};
use crate::{AppState, TenantCtx};

const STEP_SECS: i64 = 30;
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const RECOVERY_CODES: usize = 10;

pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let (mut buf, mut bits) = (0u32, 0u32);
    let mut out = vec![];
    for c in s.bytes().filter(|c| *c != b'=' && *c != b' ') {
        let v = ALPHABET.iter().position(|a| *a == c.to_ascii_uppercase())?;
        buf = (buf << 5) | u32::try_from(v).ok()?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buf >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

pub fn hotp(key: &[u8], counter: u64) -> u32 {
    let mut m = Hmac::<Sha1>::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(&counter.to_be_bytes());
    let h = m.finalize().into_bytes();
    let off = usize::from(h[19] & 0x0f);
    let bin = (u32::from(h[off] & 0x7f) << 24)
        | (u32::from(h[off + 1]) << 16)
        | (u32::from(h[off + 2]) << 8)
        | u32::from(h[off + 3]);
    bin % 1_000_000
}

/// The code for the step that contains `unix`.
pub fn code_at(secret_b32: &str, unix: i64) -> Option<String> {
    let key = base32_decode(secret_b32)?;
    let step = u64::try_from(unix / STEP_SECS).ok()?;
    Some(format!("{:06}", hotp(&key, step)))
}

/// Checks `code` against the steps around `unix`. Returns the accepted step, which must be greater than
/// `last_step` (a step is single use).
pub fn verify(secret_b32: &str, code: &str, unix: i64, last_step: i64) -> Option<i64> {
    let code = code.trim();
    if code.len() != 6 || !code.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let key = base32_decode(secret_b32)?;
    let now_step = unix / STEP_SECS;
    let mut accepted = None;
    // Every candidate is compared (constant time), so timing does not reveal which step matched.
    for step in [now_step - 1, now_step, now_step + 1] {
        let Ok(counter) = u64::try_from(step) else {
            continue;
        };
        let want = format!("{:06}", hotp(&key, counter));
        if ct_eq(&want, code) && step > last_step {
            accepted = Some(accepted.map_or(step, |a: i64| a.max(step)));
        }
    }
    accepted
}

/// The server key, or a clear refusal: without it no TOTP secret can be stored or read.
pub fn keys(st: &AppState) -> ApiResult<&SecretKeys> {
    st.cfg
        .secret_keys
        .as_ref()
        .ok_or(ApiError::Unavailable("totp_unavailable"))
}

/// Decrypts a stored secret. A value that no configured key opens is an operator error, not a bad code.
pub fn open_secret(st: &AppState, user_id: &str, stored: &str) -> ApiResult<String> {
    keys(st)?.open(&totp_aad(user_id), stored).ok_or_else(|| {
        eprintln!("a TOTP secret cannot be decrypted with the configured secret keys");
        ApiError::Internal
    })
}

/// Upper case without separators, hashed: how a recovery code is stored and compared.
fn recovery_hash(code: &str) -> Option<String> {
    let c: String = code
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (c.len() == 16 && c.bytes().all(|b| ALPHABET.contains(&b)))
        .then(|| sha256_hex(format!("open365-recovery:{c}").as_bytes()))
}

fn new_recovery_code() -> String {
    let s = base32_encode(&random_bytes::<10>());
    format!("{}-{}-{}-{}", &s[..4], &s[4..8], &s[8..12], &s[12..16])
}

/// Replaces the user's recovery codes with fresh ones and returns them (the only time they are visible).
fn issue_recovery_codes(
    tx: &rusqlite::Transaction<'_>,
    tenant_id: &str,
    user_id: &str,
) -> ApiResult<Vec<String>> {
    tx.execute(
        "DELETE FROM totp_recovery_codes WHERE user_id = ?1 AND tenant_id = ?2",
        params![user_id, tenant_id],
    )?;
    let mut out = vec![];
    for _ in 0..RECOVERY_CODES {
        let code = new_recovery_code();
        let h = recovery_hash(&code).ok_or(ApiError::Internal)?;
        tx.execute(
            "INSERT INTO totp_recovery_codes (user_id, tenant_id, code_hash) VALUES (?1, ?2, ?3)",
            params![user_id, tenant_id, h],
        )?;
        out.push(code);
    }
    Ok(out)
}

/// Spends a recovery code. Atomic: two requests with the same code cannot both succeed.
pub fn consume_recovery(
    conn: &rusqlite::Connection,
    tenant_id: &str,
    user_id: &str,
    code: &str,
) -> ApiResult<bool> {
    let Some(h) = recovery_hash(code) else {
        return Ok(false);
    };
    Ok(conn.execute(
        "UPDATE totp_recovery_codes SET used_at = ?1
         WHERE user_id = ?2 AND tenant_id = ?3 AND code_hash = ?4 AND used_at IS NULL",
        params![now(), user_id, tenant_id, h],
    )? == 1)
}

#[derive(Deserialize, Debug)]
pub struct CodeReq {
    code: String,
}

async fn guard(st: &AppState, ctx: &TenantCtx) -> ApiResult<()> {
    st.limiter
        .hit_backoff(&format!("totp:{}", ctx.user_id), st.cfg.login_fail_rate)
        .await
        .into_result()
}

/// Starts enrolment: stores a fresh (encrypted) secret that is not active until a code proves the authenticator works.
pub async fn enroll(State(st): State<AppState>, ctx: TenantCtx) -> ApiResult<Json<Value>> {
    let secret = base32_encode(&random_bytes::<20>());
    let sealed = keys(&st)?
        .seal(&totp_aad(&ctx.user_id), &secret)
        .ok_or(ApiError::Internal)?;
    let email = ctx.email.clone();
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        let on: bool = tx.query_row(
            "SELECT totp_enabled FROM users WHERE id = ?1 AND tenant_id = ?2",
            params![ctx.user_id, ctx.tenant_id],
            |r| r.get(0),
        )?;
        if on {
            return Err(ApiError::Conflict("totp_already_enabled"));
        }
        tx.execute(
            "UPDATE users SET totp_secret = ?1, totp_last_step = 0 WHERE id = ?2 AND tenant_id = ?3",
            params![sealed, ctx.user_id, ctx.tenant_id],
        )?;
        audit::append(&tx, &inner.audit, &ctx.tenant_id, &ctx.user_id, "totp.enroll", &ctx.user_id, "")?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    let uri = format!(
        "otpauth://totp/Open365:{}?secret={secret}&issuer=Open365&algorithm=SHA1&digits=6&period=30",
        pct_encode(&email)
    );
    Ok(Json(json!({ "secret": secret, "otpauth_uri": uri })))
}

/// Activates TOTP and returns the recovery codes (shown once).
pub async fn confirm(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<CodeReq>,
) -> ApiResult<Json<Value>> {
    guard(&st, &ctx).await?;
    let k = keys(&st)?.clone();
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        let secret: Option<String> = tx.query_row(
            "SELECT totp_secret FROM users WHERE id = ?1 AND tenant_id = ?2 AND totp_enabled = 0",
            params![ctx.user_id, ctx.tenant_id],
            |r| r.get(0),
        ).optional()?.flatten();
        let secret = secret.ok_or(ApiError::BadRequest("totp_not_enrolling"))?;
        let secret = k.open(&totp_aad(&ctx.user_id), &secret).ok_or(ApiError::Internal)?;
        let step = verify(&secret, &req.code, now(), 0).ok_or(ApiError::BadRequest("invalid_code"))?;
        tx.execute(
            "UPDATE users SET totp_enabled = 1, totp_last_step = ?1 WHERE id = ?2 AND tenant_id = ?3",
            params![step, ctx.user_id, ctx.tenant_id],
        )?;
        let codes = issue_recovery_codes(&tx, &ctx.tenant_id, &ctx.user_id)?;
        audit::append(&tx, &inner.audit, &ctx.tenant_id, &ctx.user_id, "totp.enable", &ctx.user_id, "")?;
        tx.commit()?;
        Ok(Json(json!({ "recovery_codes": codes })))
    })
    .await
}

/// Turns TOTP off for the caller; needs a current code.
pub async fn disable(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<CodeReq>,
) -> ApiResult<StatusCode> {
    guard(&st, &ctx).await?;
    let k = keys(&st)?.clone();
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        let (secret, last): (Option<String>, i64) = tx.query_row(
            "SELECT totp_secret, totp_last_step FROM users WHERE id = ?1 AND tenant_id = ?2 AND totp_enabled = 1",
            params![ctx.user_id, ctx.tenant_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?.ok_or(ApiError::BadRequest("totp_not_enabled"))?;
        let secret = k
            .open(&totp_aad(&ctx.user_id), &secret.unwrap_or_default())
            .ok_or(ApiError::Internal)?;
        verify(&secret, &req.code, now(), last).ok_or(ApiError::BadRequest("invalid_code"))?;
        tx.execute(
            "UPDATE users SET totp_enabled = 0, totp_secret = NULL, totp_last_step = 0 WHERE id = ?1 AND tenant_id = ?2",
            params![ctx.user_id, ctx.tenant_id],
        )?;
        tx.execute(
            "DELETE FROM totp_recovery_codes WHERE user_id = ?1 AND tenant_id = ?2",
            params![ctx.user_id, ctx.tenant_id],
        )?;
        audit::append(&tx, &inner.audit, &ctx.tenant_id, &ctx.user_id, "totp.disable", &ctx.user_id, "self")?;
        tx.commit()?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

/// Replaces the caller's recovery codes; needs a current TOTP code (single use, like at sign-in).
pub async fn new_recovery_codes(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<CodeReq>,
) -> ApiResult<Json<Value>> {
    guard(&st, &ctx).await?;
    let k = keys(&st)?.clone();
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        let (secret, last): (Option<String>, i64) = tx.query_row(
            "SELECT totp_secret, totp_last_step FROM users WHERE id = ?1 AND tenant_id = ?2 AND totp_enabled = 1",
            params![ctx.user_id, ctx.tenant_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?.ok_or(ApiError::BadRequest("totp_not_enabled"))?;
        let secret = k
            .open(&totp_aad(&ctx.user_id), &secret.unwrap_or_default())
            .ok_or(ApiError::Internal)?;
        let step = verify(&secret, &req.code, now(), last).ok_or(ApiError::BadRequest("invalid_code"))?;
        tx.execute(
            "UPDATE users SET totp_last_step = ?1 WHERE id = ?2 AND tenant_id = ?3",
            params![step, ctx.user_id, ctx.tenant_id],
        )?;
        let codes = issue_recovery_codes(&tx, &ctx.tenant_id, &ctx.user_id)?;
        audit::append(&tx, &inner.audit, &ctx.tenant_id, &ctx.user_id, "totp.recovery_codes", &ctx.user_id, "")?;
        tx.commit()?;
        Ok(Json(json!({ "recovery_codes": codes })))
    })
    .await
}

/// Admin recovery for a lost authenticator (and its recovery codes).
pub async fn admin_reset(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(user): Path<String>,
) -> ApiResult<StatusCode> {
    if !ctx.is_admin {
        return Err(ApiError::Forbidden("admin_required"));
    }
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        let n = tx.execute(
            "UPDATE users SET totp_enabled = 0, totp_secret = NULL, totp_last_step = 0 WHERE id = ?1 AND tenant_id = ?2",
            params![user, ctx.tenant_id],
        )?;
        if n == 0 {
            return Err(ApiError::NotFound);
        }
        tx.execute(
            "DELETE FROM totp_recovery_codes WHERE user_id = ?1 AND tenant_id = ?2",
            params![user, ctx.tenant_id],
        )?;
        audit::append(&tx, &inner.audit, &ctx.tenant_id, &ctx.user_id, "totp.reset", &user, "admin")?;
        tx.commit()?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 appendix B secret "12345678901234567890", 6-digit truncation of the published 8-digit values.
    const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn matches_rfc6238_vectors() {
        assert_eq!(code_at(RFC_SECRET, 59).as_deref(), Some("287082"));
        assert_eq!(
            code_at(RFC_SECRET, 1_111_111_109).as_deref(),
            Some("081804")
        );
        assert_eq!(
            code_at(RFC_SECRET, 1_234_567_890).as_deref(),
            Some("005924")
        );
    }

    #[test]
    fn base32_round_trips() {
        let b = random_bytes::<20>();
        assert_eq!(base32_decode(&base32_encode(&b)).as_deref(), Some(&b[..]));
        assert_eq!(
            base32_decode(RFC_SECRET).as_deref(),
            Some(&b"12345678901234567890"[..])
        );
        assert!(base32_decode("1!").is_none());
    }

    #[test]
    fn window_and_replay() {
        let t = 1_234_567_890;
        let code = code_at(RFC_SECRET, t).expect("code");
        let step = t / 30;
        assert_eq!(verify(RFC_SECRET, &code, t, 0), Some(step));
        assert_eq!(
            verify(RFC_SECRET, &code, t + 30, 0),
            Some(step),
            "one step of drift"
        );
        assert_eq!(verify(RFC_SECRET, &code, t + 90, 0), None, "too old");
        assert_eq!(
            verify(RFC_SECRET, &code, t, step),
            None,
            "a step is single use"
        );
        assert_eq!(verify(RFC_SECRET, "12345", t, 0), None);
        assert_eq!(verify(RFC_SECRET, "abcdef", t, 0), None);
    }
}

#[cfg(test)]
mod prop {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn base32_round_trips(b in proptest::collection::vec(any::<u8>(), 0..48)) {
            prop_assert_eq!(base32_decode(&base32_encode(&b)), Some(b));
        }

        #[test]
        fn a_code_is_accepted_only_inside_the_window_and_only_once(t in 1_000_000i64..4_000_000_000, drift in -3i64..=3) {
            let secret = base32_encode(&[7u8; 20]);
            let code = code_at(&secret, t).expect("code");
            let probe = t + drift * STEP_SECS;
            let got = verify(&secret, &code, probe, 0);
            if drift.abs() <= 1 {
                prop_assert_eq!(got, Some(t / STEP_SECS));
                prop_assert_eq!(verify(&secret, &code, probe, t / STEP_SECS), None);
            } else {
                // Outside the window it can only match by a 1 in 10^6 coincidence of another step's code.
                prop_assert!(got.is_none() || got != Some(t / STEP_SECS));
            }
        }
    }
}
