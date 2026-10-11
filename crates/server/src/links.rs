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
use crate::docs::{add_version, download_response, load_doc, read_version};
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
    let scope = LinkScope::parse(&req.scope).ok_or(ApiError::BadRequest("invalid_scope"))?;
    let audience = match req.audience.as_str() {
        "anyone" | "org" => req.audience.clone(),
        _ => return Err(ApiError::BadRequest("invalid_audience")),
    };
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
        let tx = c.transaction()?;
        tx.execute(
            "INSERT INTO share_links (id, tenant_id, document_id, token_hash, scope, audience, expires_at,
                                      password_hash, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![id2, ctx.tenant_id, d.id, th, scope.as_str(), audience, expires_at, pw_hash, ctx.user_id, now()],
        )?;
        audit::append(
            &tx,
            &inner.audit,
            &ctx.tenant_id,
            &ctx.user_id,
            "link.create",
            &d.id,
            &format!("link={id2} scope={} audience={audience} expires={expires_at:?} password={}", scope.as_str(), pw_hash.is_some()),
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
}

struct Candidate {
    r: Resolved,
    password_hash: Option<String>,
    token_hash: String,
}

const PASSWORD_HEADER: &str = "x-share-password";

/// Resolves and authorises a link request. `NotFound` is the single answer for every kind of unusable link.
async fn resolve(st: &AppState, token: &str, ip: &str, headers: &HeaderMap) -> ApiResult<Resolved> {
    let cfg = &st.cfg;
    let fail_key = format!("linkfail:{ip}");
    if !st.limiter.hit(&format!("linkip:{ip}"), cfg.link_ip_rate)
        || st.limiter.blocked(&fail_key, cfg.link_fail_rate)
    {
        return Err(ApiError::RateLimited);
    }
    if !token_shape_ok(token) {
        st.limiter.hit(&fail_key, cfg.link_fail_rate);
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
        Err(e) => {
            st.limiter.hit(&fail_key, cfg.link_fail_rate);
            return Err(e);
        }
    };
    if !st
        .limiter
        .hit(&format!("linktok:{}", cand.token_hash), cfg.link_token_rate)
    {
        return Err(ApiError::RateLimited);
    }
    if let Some(hash) = cand.password_hash {
        let pw_key = format!("linkpw:{}", cand.token_hash);
        if st.limiter.blocked(&pw_key, cfg.link_password_fail_rate) {
            return Err(ApiError::RateLimited);
        }
        let given = headers
            .get(PASSWORD_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if given.is_empty() || !verify_password(hash, given).await {
            st.limiter.hit(&pw_key, cfg.link_password_fail_rate);
            st.limiter.hit(&fail_key, cfg.link_fail_rate);
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
    }
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
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        let by = format!("link:{}", r.id);
        let n = add_version(inner, &tx, &r.tenant_id, &r.document_id, &body, &by, None)?;
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
