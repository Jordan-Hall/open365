//! Local accounts (argon2id), opaque server-side sessions, tenant bootstrap and admin policy.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use axum::Json;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::json;

use crate::audit;
use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::util::{ct_eq, new_id, new_token, now, random_bytes, sha256_hex, token_shape_ok};
use crate::{AppState, ClientIp, TenantCtx};

const COOKIE_NAME: &str = "o365_session";
const MIN_PASSWORD: usize = 12;
const MAX_PASSWORD: usize = 256;

fn argon(cfg: &Config) -> ApiResult<Argon2<'static>> {
    let p = Params::new(cfg.argon2.m_kib, cfg.argon2.t, cfg.argon2.p, None)
        .map_err(|_| ApiError::Internal)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, p))
}

pub fn hash_password_sync(cfg: &Config, pw: &str) -> ApiResult<String> {
    let salt = SaltString::encode_b64(&random_bytes::<16>()).map_err(|_| ApiError::Internal)?;
    let h = argon(cfg)?
        .hash_password(pw.as_bytes(), &salt)
        .map_err(|_| ApiError::Internal)?;
    Ok(h.to_string())
}

pub fn verify_password_sync(hash: &str, pw: &str) -> bool {
    PasswordHash::new(hash)
        .is_ok_and(|p| Argon2::default().verify_password(pw.as_bytes(), &p).is_ok())
}

pub async fn hash_password(st: &AppState, pw: String) -> ApiResult<String> {
    let cfg = st.cfg.clone();
    tokio::task::spawn_blocking(move || hash_password_sync(&cfg, &pw))
        .await
        .map_err(|_| ApiError::Internal)?
}

pub async fn verify_password(hash: String, pw: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password_sync(&hash, &pw))
        .await
        .unwrap_or(false)
}

pub fn check_password_policy(pw: &str) -> ApiResult<()> {
    let n = pw.chars().count();
    if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&n) {
        return Err(ApiError::BadRequest("password_length"));
    }
    Ok(())
}

fn clean_email(e: &str) -> ApiResult<String> {
    let e = e.trim().to_lowercase();
    let ok = e.len() <= 254
        && e.contains('@')
        && !e.chars().any(|c| c.is_control() || c.is_whitespace());
    if ok {
        Ok(e)
    } else {
        Err(ApiError::BadRequest("invalid_email"))
    }
}

fn credential(headers: &HeaderMap) -> Option<String> {
    if let Some(t) = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        return Some(t.trim().to_string());
    }
    let cookies = headers.get(COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|c| {
        c.trim()
            .strip_prefix(&format!("{COOKIE_NAME}="))
            .map(str::to_string)
    })
}

/// Resolves the session credential to a [`TenantCtx`]. Revoked, expired and unknown sessions are all `401`.
pub async fn authenticate(st: &AppState, headers: &HeaderMap) -> ApiResult<TenantCtx> {
    let tok = credential(headers)
        .filter(|t| token_shape_ok(t))
        .ok_or(ApiError::Unauthorized("unauthenticated"))?;
    let th = sha256_hex(tok.as_bytes());
    st.run(move |_, c| {
        c.query_row(
            "SELECT s.tenant_id, s.user_id, u.email, u.is_admin
             FROM sessions s JOIN users u ON u.id = s.user_id AND u.tenant_id = s.tenant_id
             WHERE s.token_hash = ?1 AND s.revoked_at IS NULL AND s.expires_at > ?2 AND u.disabled = 0",
            params![th, now()],
            |r| {
                Ok(TenantCtx {
                    tenant_id: r.get(0)?,
                    user_id: r.get(1)?,
                    email: r.get(2)?,
                    is_admin: r.get::<_, i64>(3)? != 0,
                })
            },
        )
        .optional()?
        .ok_or(ApiError::Unauthorized("unauthenticated"))
    })
    .await
}

#[derive(Deserialize)]
// No Debug: carries a password.
#[allow(missing_debug_implementations)]
pub struct CreateTenant {
    slug: String,
    name: String,
    admin_email: String,
    admin_password: String,
}

/// First-run provisioning, enabled only when `OPEN365_BOOTSTRAP_TOKEN(_FILE)` is configured.
pub async fn create_tenant(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateTenant>,
) -> ApiResult<Response> {
    let Some(expected) = st.cfg.bootstrap_token.clone() else {
        return Err(ApiError::NotFound);
    };
    let given = headers
        .get("x-bootstrap-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ct_eq(
        &sha256_hex(given.as_bytes()),
        &sha256_hex(expected.as_bytes()),
    ) {
        return Err(ApiError::Unauthorized("bad_bootstrap_token"));
    }
    let slug = req.slug.trim().to_lowercase();
    if !(2..=40).contains(&slug.len())
        || !slug
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err(ApiError::BadRequest("invalid_slug"));
    }
    let name = crate::util::clean_name(&req.name).ok_or(ApiError::BadRequest("invalid_name"))?;
    let email = clean_email(&req.admin_email)?;
    check_password_policy(&req.admin_password)?;
    let hash = hash_password(&st, req.admin_password).await?;
    let (tid, uid) = (new_id(), new_id());
    let (t2, u2) = (tid.clone(), uid.clone());
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM tenants WHERE slug = ?1)", [&slug], |r| r.get(0))?;
        if exists {
            return Err(ApiError::Conflict("slug_taken"));
        }
        tx.execute(
            "INSERT INTO tenants (id, slug, name, quota_bytes, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![t2, slug, name, inner.cfg.tenant_quota_bytes, now()],
        )?;
        tx.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, password_hash, is_admin, created_at)
             VALUES (?1, ?2, ?3, ?3, ?4, 1, ?5)",
            params![u2, t2, email, hash, now()],
        )?;
        audit::append(&tx, &inner.audit, &t2, "bootstrap", "tenant.create", &t2, &slug)?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "tenant_id": tid, "admin_user_id": uid })),
    )
        .into_response())
}

#[derive(Deserialize)]
// No Debug: carries a password.
#[allow(missing_debug_implementations)]
pub struct LoginReq {
    tenant: String,
    email: String,
    password: String,
}

pub async fn login(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(req): Json<LoginReq>,
) -> ApiResult<Response> {
    let email = req.email.trim().to_lowercase();
    let slug = req.tenant.trim().to_lowercase();
    let key = format!("login:{ip}:{slug}:{email}");
    if st.limiter.blocked(&key, st.cfg.login_fail_rate) {
        return Err(ApiError::RateLimited);
    }
    let (s2, e2) = (slug.clone(), email.clone());
    let found: Option<(String, String, String)> = st
        .run(move |_, c| {
            Ok(c.query_row(
                "SELECT t.id, u.id, u.password_hash FROM users u JOIN tenants t ON t.id = u.tenant_id
                 WHERE t.slug = ?1 AND u.email = ?2 AND u.disabled = 0",
                params![s2, e2],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
        })
        .await?;
    let hash = found
        .as_ref()
        .map_or_else(|| st.dummy_hash.clone(), |f| f.2.clone());
    let ok = verify_password(hash, req.password).await && found.is_some();
    let Some((tenant_id, user_id, _)) = found.filter(|_| ok) else {
        st.limiter.hit(&key, st.cfg.login_fail_rate);
        let detail = format!("tenant={slug}");
        st.run(move |inner, c| {
            audit::append(
                c,
                &inner.audit,
                audit::SYSTEM_TENANT,
                &email,
                "login.failed",
                &slug,
                &detail,
            )
        })
        .await?;
        return Err(ApiError::Unauthorized("invalid_credentials"));
    };
    let token = new_token();
    let th = sha256_hex(token.as_bytes());
    let ttl = st.cfg.session_ttl_secs;
    let (t2, u2) = (tenant_id.clone(), user_id.clone());
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        let t = now();
        tx.execute(
            "INSERT INTO sessions (token_hash, tenant_id, user_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![th, t2, u2, t, t + ttl],
        )?;
        audit::append(&tx, &inner.audit, &t2, &u2, "login", &u2, "local")?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    let cookie =
        format!("{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={ttl}");
    let mut res = Json(
        json!({ "token": token, "tenant_id": tenant_id, "user_id": user_id, "expires_in": ttl }),
    )
    .into_response();
    res.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| ApiError::Internal)?,
    );
    Ok(res)
}

pub async fn logout(
    State(st): State<AppState>,
    headers: HeaderMap,
    ctx: TenantCtx,
) -> ApiResult<Response> {
    let th = sha256_hex(credential(&headers).unwrap_or_default().as_bytes());
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        tx.execute(
            "UPDATE sessions SET revoked_at = ?1 WHERE token_hash = ?2 AND tenant_id = ?3",
            params![now(), th, ctx.tenant_id],
        )?;
        audit::append(
            &tx,
            &inner.audit,
            &ctx.tenant_id,
            &ctx.user_id,
            "logout",
            &ctx.user_id,
            "",
        )?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn me(ctx: TenantCtx) -> Json<serde_json::Value> {
    Json(
        json!({ "tenant_id": ctx.tenant_id, "user_id": ctx.user_id, "email": ctx.email, "is_admin": ctx.is_admin }),
    )
}

#[derive(Deserialize)]
// No Debug: carries a password.
#[allow(missing_debug_implementations)]
pub struct CreateUser {
    email: String,
    display_name: Option<String>,
    password: String,
    #[serde(default)]
    is_admin: bool,
}

fn require_admin(ctx: &TenantCtx) -> ApiResult<()> {
    if ctx.is_admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden("admin_required"))
    }
}

pub async fn create_user(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<CreateUser>,
) -> ApiResult<Response> {
    require_admin(&ctx)?;
    let email = clean_email(&req.email)?;
    check_password_policy(&req.password)?;
    let display = crate::util::clean_name(req.display_name.as_deref().unwrap_or(&email))
        .ok_or(ApiError::BadRequest("invalid_name"))?;
    let hash = hash_password(&st, req.password).await?;
    let uid = new_id();
    let u2 = uid.clone();
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        let dup: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE tenant_id = ?1 AND email = ?2)",
            params![ctx.tenant_id, email],
            |r| r.get(0),
        )?;
        if dup {
            return Err(ApiError::Conflict("email_taken"));
        }
        tx.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, password_hash, is_admin, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![u2, ctx.tenant_id, email, display, hash, req.is_admin, now()],
        )?;
        audit::append(&tx, &inner.audit, &ctx.tenant_id, &ctx.user_id, "user.create", &u2, &email)?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "user_id": uid }))).into_response())
}

pub async fn list_users(
    State(st): State<AppState>,
    ctx: TenantCtx,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&ctx)?;
    st.run(move |_, c| {
        let mut s = c.prepare("SELECT id, email, display_name, is_admin FROM users WHERE tenant_id = ?1 ORDER BY email")?;
        let rows = s
            .query_map([&ctx.tenant_id], |r| {
                Ok(json!({ "user_id": r.get::<_, String>(0)?, "email": r.get::<_, String>(1)?,
                           "display_name": r.get::<_, String>(2)?, "is_admin": r.get::<_, i64>(3)? != 0 }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Json(json!(rows)))
    })
    .await
}

#[derive(Deserialize, Debug)]
pub struct Policy {
    allow_anyone_links: bool,
}

/// Admin kill switch for "anyone" links. Existing anyone-links stop resolving immediately.
pub async fn set_policy(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<Policy>,
) -> ApiResult<StatusCode> {
    require_admin(&ctx)?;
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        tx.execute(
            "UPDATE tenants SET allow_anyone_links = ?1 WHERE id = ?2",
            params![req.allow_anyone_links, ctx.tenant_id],
        )?;
        audit::append(
            &tx,
            &inner.audit,
            &ctx.tenant_id,
            &ctx.user_id,
            "policy.set",
            &ctx.tenant_id,
            &format!("allow_anyone_links={}", req.allow_anyone_links),
        )?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_is_argon2id_and_verifies() {
        let mut cfg = Config::new(".");
        cfg.argon2 = crate::config::Argon2Cost {
            m_kib: 64,
            t: 1,
            p: 1,
        };
        let h = hash_password_sync(&cfg, "correct horse battery").expect("hash");
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password_sync(&h, "correct horse battery"));
        assert!(!verify_password_sync(&h, "wrong"));
        assert!(!verify_password_sync("not a hash", "x"));
    }

    #[test]
    fn credential_reads_bearer_and_cookie() {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, HeaderValue::from_static("Bearer abc"));
        assert_eq!(credential(&h).as_deref(), Some("abc"));
        let mut h = HeaderMap::new();
        h.insert(COOKIE, HeaderValue::from_static("x=1; o365_session=tok"));
        assert_eq!(credential(&h).as_deref(), Some("tok"));
    }
}
