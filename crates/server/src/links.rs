//! Share links (capability URLs): `/s/<token>`. The token is 256 bits of CSPRNG output shown once; only its SHA-256
//! is stored. Unknown, expired, revoked and policy-blocked links all return the same `404` (T15).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::audit;
use crate::auth::{authenticate, hash_password, verify_password};
use crate::docs::{add_version, download_response, load_doc, quota_precheck, read_version};
use crate::error::{ApiError, ApiResult};
use crate::perm::{Grant, LinkScope, Rights, rights};
use crate::util::{ct_eq, new_id, new_token, now, sha256_hex, token_shape_ok};
use crate::{AppState, ClientIp, TenantCtx};

#[derive(Deserialize)]
// No Debug: carries a password.
#[allow(missing_debug_implementations)]
pub struct CreateLink {
    scope: String,
    #[serde(default = "default_audience")]
    audience: String,
    expires_in_secs: Option<i64>,
    password: Option<String>,
    /// For the `specific` audience: the people (by email, in this tenant) who may open the link.
    emails: Option<Vec<String>>,
}

fn default_audience() -> String {
    "org".into()
}

pub async fn create(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(doc): Path<String>,
    Json(req): Json<CreateLink>,
) -> ApiResult<Response> {
    // Counted before any work: a stolen session cannot mint links without bound.
    st.limiter
        .hit(
            &format!("linkcreate:{}:{}", ctx.tenant_id, ctx.user_id),
            st.cfg.link_create_rate,
        )
        .await
        .into_result()?;
    let scope = LinkScope::parse(&req.scope).ok_or(ApiError::BadRequest("invalid_scope"))?;
    let audience = match req.audience.as_str() {
        "anyone" | "org" | "specific" => req.audience.clone(),
        _ => return Err(ApiError::BadRequest("invalid_audience")),
    };
    let emails: Vec<String> = req
        .emails
        .iter()
        .flatten()
        .map(|e| e.trim().to_lowercase())
        .collect();
    if (audience == "specific") == emails.is_empty()
        || emails.len() > 50
        || emails.iter().any(|e| e.len() > 254)
    {
        return Err(ApiError::BadRequest("invalid_emails"));
    }
    let expires_at = match req.expires_in_secs {
        None => None,
        Some(s) if (1..=st.cfg.max_link_ttl_secs).contains(&s) => Some(now() + s),
        Some(_) => return Err(ApiError::BadRequest("invalid_expiry")),
    };
    let pw_hash = match req.password {
        Some(p) => {
            if p.chars().count() < 8 || p.chars().count() > 256 {
                return Err(ApiError::BadRequest("password_length"));
            }
            Some(hash_password(&st, p).await?)
        }
        None => None,
    };
    let (id, token) = (new_id(), new_token());
    let (id2, th) = (id.clone(), sha256_hex(token.as_bytes()));
    st.run(move |inner, c| {
        let (d, role) = load_doc(c, &ctx, &doc)?;
        if !rights(Grant::Role(role)).manage {
            audit::append(c, &inner.audit, &ctx.tenant_id, &ctx.user_id, "denied", &d.id, "link.create")?;
            return Err(ApiError::Forbidden("insufficient_role"));
        }
        if audience == "anyone" {
            let allowed: bool =
                c.query_row("SELECT allow_anyone_links FROM tenants WHERE id = ?1", [&ctx.tenant_id], |r| r.get(0))?;
            if !allowed {
                audit::append(c, &inner.audit, &ctx.tenant_id, &ctx.user_id, "denied", &d.id, "link.create anyone blocked by policy")?;
                return Err(ApiError::Forbidden("anyone_links_disabled"));
            }
        }
        // Named people must be accounts in the caller's own tenant.
        let mut people = vec![];
        for e in &emails {
            let uid: String = c
                .query_row(
                    "SELECT id FROM users WHERE tenant_id = ?1 AND email = ?2 AND disabled = 0",
                    params![ctx.tenant_id, e],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(ApiError::BadRequest("unknown_user"))?;
            people.push(uid);
        }
        let tx = c.transaction()?;
        tx.execute(
            "INSERT INTO share_links (id, tenant_id, document_id, token_hash, scope, audience, expires_at,
                                      password_hash, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![id2, ctx.tenant_id, d.id, th, scope.as_str(), audience, expires_at, pw_hash, ctx.user_id, now()],
        )?;
        for uid in &people {
            tx.execute(
                "INSERT OR IGNORE INTO share_link_users (link_id, tenant_id, user_id) VALUES (?1, ?2, ?3)",
                params![id2, ctx.tenant_id, uid],
            )?;
        }
        audit::append(
            &tx,
            &inner.audit,
            &ctx.tenant_id,
            &ctx.user_id,
            "link.create",
            &d.id,
            &format!("link={id2} scope={} audience={audience} expires={expires_at:?} password={} people={}", scope.as_str(), pw_hash.is_some(), people.len()),
        )?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    // The only time the token exists in plaintext on the server.
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "token": token, "url_path": format!("/s/{token}"), "scope": scope.as_str(),
                     "audience": req.audience, "expires_at": expires_at })),
    )
        .into_response())
}

pub async fn list(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(doc): Path<String>,
) -> ApiResult<Json<Value>> {
    st.run(move |inner, c| {
        let (d, role) = load_doc(c, &ctx, &doc)?;
        if !rights(Grant::Role(role)).manage {
            audit::append(c, &inner.audit, &ctx.tenant_id, &ctx.user_id, "denied", &d.id, "link.list")?;
            return Err(ApiError::Forbidden("insufficient_role"));
        }
        let mut s = c.prepare(
            "SELECT id, scope, audience, expires_at, password_hash IS NOT NULL, created_by, created_at, revoked_at
             FROM share_links WHERE document_id = ?1 AND tenant_id = ?2 ORDER BY created_at, id",
        )?;
        let rows = s
            .query_map(params![d.id, ctx.tenant_id], |r| {
                Ok(json!({ "id": r.get::<_, String>(0)?, "scope": r.get::<_, String>(1)?,
                           "audience": r.get::<_, String>(2)?, "expires_at": r.get::<_, Option<i64>>(3)?,
                           "has_password": r.get::<_, bool>(4)?, "created_by": r.get::<_, String>(5)?,
                           "created_at": r.get::<_, i64>(6)?, "revoked_at": r.get::<_, Option<i64>>(7)? }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut rows = rows;
        for row in &mut rows {
            let id = row["id"].as_str().unwrap_or_default().to_string();
            let mut s = c.prepare(
                "SELECT u.email FROM share_link_users l JOIN users u ON u.id = l.user_id AND u.tenant_id = l.tenant_id
                 WHERE l.link_id = ?1 AND l.tenant_id = ?2 ORDER BY u.email",
            )?;
            let emails: Vec<String> = s
                .query_map(params![id, ctx.tenant_id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            if row["audience"] == "specific" {
                row["emails"] = json!(emails);
            }
        }
        Ok(Json(json!(rows)))
    })
    .await
}

pub async fn revoke(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(link): Path<String>,
) -> ApiResult<StatusCode> {
    st.run(move |inner, c| {
        let doc_id: String = c
            .query_row(
                "SELECT document_id FROM share_links WHERE id = ?1 AND tenant_id = ?2",
                params![link, ctx.tenant_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(ApiError::NotFound)?;
        let (d, role) = load_doc(c, &ctx, &doc_id)?;
        if !rights(Grant::Role(role)).manage {
            audit::append(c, &inner.audit, &ctx.tenant_id, &ctx.user_id, "denied", &d.id, "link.revoke")?;
            return Err(ApiError::Forbidden("insufficient_role"));
        }
        let tx = c.transaction()?;
        tx.execute(
            "UPDATE share_links SET revoked_at = ?1 WHERE id = ?2 AND tenant_id = ?3 AND revoked_at IS NULL",
            params![now(), link, ctx.tenant_id],
        )?;
        audit::append(&tx, &inner.audit, &ctx.tenant_id, &ctx.user_id, "link.revoke", &d.id, &format!("link={link}"))?;
        tx.commit()?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

/// A link that passed every check, including the password.
struct Resolved {
    id: String,
    tenant_id: String,
    document_id: String,
    doc_name: String,
    version: i64,
    scope: LinkScope,
    token_hash: String,
}

struct Candidate {
    r: Resolved,
    password_hash: Option<String>,
    token_hash: String,
}

const PASSWORD_HEADER: &str = "x-share-password";

/// True when the signed-in caller is one of the people a `specific` link names.
fn is_named(
    c: &rusqlite::Connection,
    res: &Resolved,
    session: Option<&TenantCtx>,
) -> ApiResult<bool> {
    let Some(s) = session.filter(|s| s.tenant_id == res.tenant_id) else {
        return Ok(false);
    };
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM share_link_users WHERE link_id = ?1 AND tenant_id = ?2 AND user_id = ?3)",
        params![res.id, res.tenant_id, s.user_id],
        |r| r.get(0),
    )?)
}

/// Resolves and authorises a link request. `NotFound` is the single answer for every kind of unusable link.
async fn resolve(st: &AppState, token: &str, ip: &str, headers: &HeaderMap) -> ApiResult<Resolved> {
    let cfg = &st.cfg;
    let ipk = crate::ratelimit::ip_key(ip);
    let fail_key = format!("linkfail:{ipk}");
    st.limiter
        .hit(&format!("linkip:{ipk}"), cfg.link_ip_rate)
        .await
        .into_result()?;
    // Every lookup is counted BEFORE it is resolved, so concurrent probes cannot overshoot the limit; a lookup that
    // reaches a usable link (below) clears the count, so only failures accumulate.
    st.limiter
        .hit_backoff(&fail_key, cfg.link_fail_rate)
        .await
        .into_result()?;
    if !token_shape_ok(token) {
        return Err(ApiError::NotFound);
    }
    let th = sha256_hex(token.as_bytes());
    // An optional session, used only for organisation-audience links.
    let session = authenticate(st, headers).await.ok();

    let th2 = th.clone();
    let cand: Result<Candidate, ApiError> = st
        .run(move |inner, c| {
            let row = c
                .query_row(
                    "SELECT l.id, l.tenant_id, l.document_id, d.name, d.current_version, l.scope, l.audience,
                            l.expires_at, l.password_hash, l.revoked_at, d.deleted_at, t.allow_anyone_links, l.token_hash
                     FROM share_links l
                     JOIN documents d ON d.id = l.document_id AND d.tenant_id = l.tenant_id
                     JOIN tenants t ON t.id = l.tenant_id
                     WHERE l.token_hash = ?1",
                    [&th2],
                    |r| {
                        Ok((
                            Resolved {
                                id: r.get(0)?,
                                tenant_id: r.get(1)?,
                                document_id: r.get(2)?,
                                doc_name: r.get(3)?,
                                version: r.get(4)?,
                                scope: LinkScope::parse(&r.get::<_, String>(5)?).unwrap_or(LinkScope::ViewNoDownload),
                                token_hash: th2.clone(),
                            },
                            r.get::<_, String>(6)?,
                            r.get::<_, Option<i64>>(7)?,
                            r.get::<_, Option<String>>(8)?,
                            r.get::<_, Option<i64>>(9)?.is_some(),
                            r.get::<_, Option<i64>>(10)?.is_some(),
                            r.get::<_, bool>(11)?,
                            r.get::<_, String>(12)?,
                        ))
                    },
                )
                .optional()?;
            let Some((res, audience, expires, pw, revoked, deleted, anyone_ok, stored)) = row else {
                audit::append(c, &inner.audit, audit::SYSTEM_TENANT, "anonymous", "link.unknown", "-", "")?;
                return Ok(Err(ApiError::NotFound));
            };
            if !ct_eq(&stored, &th2) {
                return Ok(Err(ApiError::NotFound));
            }
            let denial = if revoked {
                Some("revoked")
            } else if expires.is_some_and(|e| e <= now()) {
                Some("expired")
            } else if deleted {
                Some("document_deleted")
            } else if audience == "anyone" && !anyone_ok {
                Some("anyone_links_disabled")
            } else if audience == "org" && session.as_ref().is_none_or(|s| s.tenant_id != res.tenant_id) {
                Some("org_session_required")
            } else if audience == "specific" && !is_named(c, &res, session.as_ref())? {
                Some("not_a_named_person")
            } else {
                None
            };
            if let Some(why) = denial {
                audit::append(c, &inner.audit, &res.tenant_id, "anonymous", "link.denied", &res.document_id, &format!("link={} {why}", res.id))?;
                return Ok(Err(ApiError::NotFound));
            }
            Ok(Ok(Candidate { r: res, password_hash: pw, token_hash: stored }))
        })
        .await?;
    let cand = match cand {
        Ok(c) => c,
        Err(e) => return Err(e),
    };
    st.limiter
        .hit(&format!("linktok:{}", cand.token_hash), cfg.link_token_rate)
        .await
        .into_result()?;
    if let Some(hash) = cand.password_hash {
        let given = headers
            .get(PASSWORD_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // A missing password is a prompt for one, not a failed guess: it costs nothing and is not counted.
        if given.is_empty() {
            st.limiter.reset(&fail_key).await;
            return Err(ApiError::Unauthorized("password_required"));
        }
        // Per link and address, plus a looser per link limit. Counted BEFORE the verification, so concurrent
        // guesses cannot all pass a check-then-act gap; both are cleared by a correct password.
        let pw_key = format!("linkpw:{}:{ipk}", cand.token_hash);
        let tok_key = format!("linkpwtok:{}", cand.token_hash);
        st.limiter
            .hit_backoff(&pw_key, cfg.link_password_fail_rate)
            .await
            .into_result()?;
        st.limiter
            .hit(&tok_key, cfg.link_password_token_rate)
            .await
            .into_result()?;
        if !verify_password(st, hash, given).await {
            let (t, d, id) = (
                cand.r.tenant_id.clone(),
                cand.r.document_id.clone(),
                cand.r.id.clone(),
            );
            st.run(move |inner, c| {
                audit::append(
                    c,
                    &inner.audit,
                    &t,
                    "anonymous",
                    "link.password_failed",
                    &d,
                    &format!("link={id}"),
                )
            })
            .await?;
            return Err(ApiError::Unauthorized("password_required"));
        }
        st.limiter.reset(&pw_key).await;
        st.limiter.reset(&tok_key).await;
    }
    st.limiter.reset(&fail_key).await;
    Ok(cand.r)
}

fn link_rights(r: &Resolved) -> Rights {
    rights(Grant::Link(r.scope))
}

fn audit_use(
    st: &AppState,
    r: &Resolved,
    action: &'static str,
    detail: String,
) -> impl Future<Output = ApiResult<()>> {
    let (t, d, id) = (r.tenant_id.clone(), r.document_id.clone(), r.id.clone());
    let st = st.clone();
    async move {
        st.run(move |inner, c| {
            audit::append(
                c,
                &inner.audit,
                &t,
                &format!("link:{id}"),
                action,
                &d,
                &detail,
            )
        })
        .await
    }
}

pub async fn open(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let r = resolve(&st, &token, &ip, &headers).await?;
    let rt = link_rights(&r);
    audit_use(&st, &r, "link.open", format!("link={}", r.id)).await?;
    Ok(Json(
        json!({ "name": r.doc_name, "version": r.version, "scope": r.scope.as_str(),
                    "can_download": rt.download, "can_edit": rt.write }),
    ))
}

pub async fn download(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let r = resolve(&st, &token, &ip, &headers).await?;
    if !link_rights(&r).download {
        audit_use(
            &st,
            &r,
            "link.denied",
            format!("link={} download blocked", r.id),
        )
        .await?;
        return Err(ApiError::Forbidden("download_blocked"));
    }
    let (t, d, v, id) = (
        r.tenant_id.clone(),
        r.document_id.clone(),
        r.version,
        r.id.clone(),
    );
    st.run(move |inner, c| {
        let data = read_version(inner, c, &t, &d, v)?;
        audit::append(
            c,
            &inner.audit,
            &t,
            &format!("link:{id}"),
            "link.download",
            &d,
            &format!("v{v}"),
        )?;
        download_response(&r.doc_name, data)
    })
    .await
}

pub async fn upload(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let r = resolve(&st, &token, &ip, &headers).await?;
    if !link_rights(&r).write {
        audit_use(
            &st,
            &r,
            "link.denied",
            format!("link={} upload not permitted", r.id),
        )
        .await?;
        return Err(ApiError::Forbidden("read_only_link"));
    }
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty_body"));
    }
    // Anonymous writers get a per-link rate and an hourly byte budget, so a leaked edit link cannot fill the quota.
    st.limiter
        .hit(&format!("linkup:{}", r.token_hash), st.cfg.link_upload_rate)
        .await
        .into_result()?;
    let by = format!("link:{}", r.id);
    let (t, by2, size, limit) = (
        r.tenant_id.clone(),
        by.clone(),
        body.len(),
        st.cfg.link_upload_bytes_per_hour,
    );
    st.run(move |_, c| {
        quota_precheck(c, &t, size)?;
        link_budget(c, &t, &by2, size, limit)
    })
    .await?;
    // Bytes reach the disk (fsynced) before the database lock is taken.
    let blob = st.store_blob(&r.tenant_id, body).await?;
    st.run(move |inner, c| {
        link_budget(
            c,
            &r.tenant_id,
            &by,
            usize::try_from(blob.1).unwrap_or(usize::MAX),
            inner.cfg.link_upload_bytes_per_hour,
        )?;
        let tx = c.transaction()?;
        let n = add_version(&tx, &r.tenant_id, &r.document_id, &blob, &by, None)?;
        audit::append(
            &tx,
            &inner.audit,
            &r.tenant_id,
            &by,
            "link.upload",
            &r.document_id,
            &format!("v{n}"),
        )?;
        tx.commit()?;
        Ok(Json(json!({ "version": n })))
    })
    .await
}

/// Bytes one link may add in an hour.
fn link_budget(
    c: &rusqlite::Connection,
    tenant: &str,
    actor: &str,
    size: usize,
    limit: i64,
) -> ApiResult<()> {
    let used: i64 = c.query_row(
        "SELECT COALESCE(SUM(size), 0) FROM versions WHERE tenant_id = ?1 AND created_by = ?2 AND created_at > ?3",
        params![tenant, actor, now() - 3600],
        |r| r.get(0),
    )?;
    if used.saturating_add(i64::try_from(size).unwrap_or(i64::MAX)) > limit {
        return Err(ApiError::RateLimited(60));
    }
    Ok(())
}
