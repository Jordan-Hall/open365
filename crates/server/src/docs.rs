//! Documents (opaque bytes), immutable versions, named-user ACLs and audit views.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::audit;
use crate::error::{ApiError, ApiResult};
use crate::perm::{Grant, Rights, Role, rights};
use crate::util::{clean_name, content_disposition, is_hex_id, new_id, now};
use crate::{AppState, Inner, TenantCtx};

#[derive(Debug, Clone)]
pub struct DocRow {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub current_version: i64,
}

/// Loads a document the caller may see. Missing, deleted, foreign-tenant and no-grant all give `NotFound`.
pub fn load_doc(conn: &Connection, ctx: &TenantCtx, id: &str) -> ApiResult<(DocRow, Role)> {
    if !is_hex_id(id, 32) {
        return Err(ApiError::NotFound);
    }
    let doc = conn
        .query_row(
            "SELECT id, name, owner_id, current_version FROM documents
             WHERE id = ?1 AND tenant_id = ?2 AND deleted_at IS NULL",
            params![id, ctx.tenant_id],
            |r| {
                Ok(DocRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    owner_id: r.get(2)?,
                    current_version: r.get(3)?,
                })
            },
        )
        .optional()?
        .ok_or(ApiError::NotFound)?;
    if doc.owner_id == ctx.user_id {
        return Ok((doc, Role::Owner));
    }
    let role: Option<String> = conn
        .query_row(
            "SELECT role FROM acl WHERE document_id = ?1 AND tenant_id = ?2 AND user_id = ?3",
            params![id, ctx.tenant_id, ctx.user_id],
            |r| r.get(0),
        )
        .optional()?;
    match role.as_deref() {
        Some("editor") => Ok((doc, Role::Editor)),
        Some("viewer") => Ok((doc, Role::Viewer)),
        _ => Err(ApiError::NotFound),
    }
}

/// Like [`load_doc`] but also requires a right; a refusal is audited as a denial (the document is known to exist).
fn authorize(
    inner: &Inner,
    conn: &Connection,
    ctx: &TenantCtx,
    id: &str,
    need: fn(&Rights) -> bool,
    what: &str,
) -> ApiResult<(DocRow, Role)> {
    let (doc, role) = load_doc(conn, ctx, id)?;
    if need(&rights(Grant::Role(role))) {
        return Ok((doc, role));
    }
    audit::append(
        conn,
        &inner.audit,
        &ctx.tenant_id,
        &ctx.user_id,
        "denied",
        id,
        what,
    )?;
    Err(ApiError::Forbidden("insufficient_role"))
}

/// Appends an immutable version. Quota and optimistic-concurrency checks happen here for users and links alike.
pub fn add_version(
    inner: &Inner,
    tx: &Transaction<'_>,
    tenant: &str,
    doc_id: &str,
    data: &[u8],
    created_by: &str,
    base_version: Option<i64>,
) -> ApiResult<i64> {
    let current: i64 = tx.query_row(
        "SELECT current_version FROM documents WHERE id = ?1 AND tenant_id = ?2",
        params![doc_id, tenant],
        |r| r.get(0),
    )?;
    if base_version.is_some_and(|b| b != current) {
        return Err(ApiError::Conflict("version_conflict"));
    }
    let used: i64 = tx.query_row(
        "SELECT COALESCE(SUM(size), 0) FROM versions WHERE tenant_id = ?1",
        [tenant],
        |r| r.get(0),
    )?;
    let size = i64::try_from(data.len()).map_err(|_| ApiError::TooLarge)?;
    let quota: i64 = tx.query_row(
        "SELECT quota_bytes FROM tenants WHERE id = ?1",
        [tenant],
        |r| r.get(0),
    )?;
    if used + size > quota {
        return Err(ApiError::TooLarge);
    }
    let blob = inner.blobs.put(tenant, data)?;
    let n = current + 1;
    tx.execute(
        "INSERT INTO versions (document_id, n, tenant_id, blob_id, size, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![doc_id, n, tenant, blob, size, created_by, now()],
    )?;
    tx.execute(
        "UPDATE documents SET current_version = ?1, updated_at = ?2 WHERE id = ?3 AND tenant_id = ?4",
        params![n, now(), doc_id, tenant],
    )?;
    Ok(n)
}

pub fn read_version(
    inner: &Inner,
    conn: &Connection,
    tenant: &str,
    doc_id: &str,
    n: i64,
) -> ApiResult<Vec<u8>> {
    let blob: String = conn
        .query_row(
            "SELECT blob_id FROM versions WHERE document_id = ?1 AND tenant_id = ?2 AND n = ?3",
            params![doc_id, tenant, n],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(ApiError::NotFound)?;
    Ok(inner.blobs.get(tenant, &blob)?)
}

/// Downloads are always opaque attachments: no content sniffing, no inline rendering (T13).
pub fn download_response(name: &str, data: Vec<u8>) -> ApiResult<Response> {
    let disp = HeaderValue::from_str(&content_disposition(name)).map_err(|_| ApiError::Internal)?;
    Ok((
        StatusCode::OK,
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (CONTENT_DISPOSITION, disp),
        ],
        data,
    )
        .into_response())
}

#[derive(Deserialize, Debug)]
pub struct NameQ {
    name: String,
}

pub async fn create(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Query(q): Query<NameQ>,
    body: Bytes,
) -> ApiResult<Response> {
    let name = clean_name(&q.name).ok_or(ApiError::BadRequest("invalid_name"))?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty_body"));
    }
    let id = new_id();
    let id2 = id.clone();
    st.run(move |inner, c| {
        let tx = c.transaction()?;
        tx.execute(
            "INSERT INTO documents (id, tenant_id, name, owner_id, current_version, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
            params![id2, ctx.tenant_id, name, ctx.user_id, now()],
        )?;
        let n = add_version(inner, &tx, &ctx.tenant_id, &id2, &body, &ctx.user_id, None)?;
        audit::append(&tx, &inner.audit, &ctx.tenant_id, &ctx.user_id, "document.create", &id2, &format!("v{n}"))?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "version": 1 }))).into_response())
}

pub async fn list(State(st): State<AppState>, ctx: TenantCtx) -> ApiResult<Json<Value>> {
    st.run(move |_, c| {
        let mut s = c.prepare(
            "SELECT d.id, d.name, d.current_version, d.updated_at,
                    CASE WHEN d.owner_id = ?2 THEN 'owner' ELSE a.role END AS role
             FROM documents d LEFT JOIN acl a ON a.document_id = d.id AND a.user_id = ?2 AND a.tenant_id = d.tenant_id
             WHERE d.tenant_id = ?1 AND d.deleted_at IS NULL AND (d.owner_id = ?2 OR a.user_id IS NOT NULL)
             ORDER BY d.updated_at DESC, d.id",
        )?;
        let rows = s
            .query_map(params![ctx.tenant_id, ctx.user_id], |r| {
                Ok(json!({ "id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?,
                           "version": r.get::<_, i64>(2)?, "updated_at": r.get::<_, i64>(3)?,
                           "role": r.get::<_, String>(4)? }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Json(json!(rows)))
    })
    .await
}

pub async fn meta(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(doc): Path<String>,
) -> ApiResult<Json<Value>> {
    st.run(move |_, c| {
        let (d, role) = load_doc(c, &ctx, &doc)?;
        Ok(Json(
            json!({ "id": d.id, "name": d.name, "version": d.current_version, "role": role.as_str(),
                        "owner_id": d.owner_id }),
        ))
    })
    .await
}

#[derive(Deserialize, Debug)]
pub struct VersionQ {
    base_version: Option<i64>,
}

pub async fn new_version(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(doc): Path<String>,
    Query(q): Query<VersionQ>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty_body"));
    }
    st.run(move |inner, c| {
        authorize(inner, c, &ctx, &doc, |r| r.write, "upload")?;
        let tx = c.transaction()?;
        let n = add_version(
            inner,
            &tx,
            &ctx.tenant_id,
            &doc,
            &body,
            &ctx.user_id,
            q.base_version,
        )?;
        audit::append(
            &tx,
            &inner.audit,
            &ctx.tenant_id,
            &ctx.user_id,
            "document.version",
            &doc,
            &format!("v{n}"),
        )?;
        tx.commit()?;
        Ok(Json(json!({ "id": doc, "version": n })))
    })
    .await
}

pub async fn versions(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(doc): Path<String>,
) -> ApiResult<Json<Value>> {
    st.run(move |_, c| {
        load_doc(c, &ctx, &doc)?;
        let mut s = c.prepare(
            "SELECT n, size, blob_id, created_by, created_at FROM versions
             WHERE document_id = ?1 AND tenant_id = ?2 ORDER BY n",
        )?;
        let rows = s
            .query_map(params![doc, ctx.tenant_id], |r| {
                Ok(json!({ "n": r.get::<_, i64>(0)?, "size": r.get::<_, i64>(1)?, "sha256": r.get::<_, String>(2)?,
                           "created_by": r.get::<_, String>(3)?, "created_at": r.get::<_, i64>(4)? }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Json(json!(rows)))
    })
    .await
}

#[derive(Deserialize, Debug)]
pub struct ContentQ {
    version: Option<i64>,
}

pub async fn content(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(doc): Path<String>,
    Query(q): Query<ContentQ>,
) -> ApiResult<Response> {
    st.run(move |inner, c| {
        let (d, _) = authorize(inner, c, &ctx, &doc, |r| r.download, "download")?;
        let n = q.version.unwrap_or(d.current_version);
        let data = read_version(inner, c, &ctx.tenant_id, &d.id, n)?;
        audit::append(
            c,
            &inner.audit,
            &ctx.tenant_id,
            &ctx.user_id,
            "document.download",
            &d.id,
            &format!("v{n}"),
        )?;
        download_response(&d.name, data)
    })
    .await
}

#[derive(Deserialize, Debug)]
pub struct AclReq {
    email: String,
    role: String,
}

pub async fn acl_set(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(doc): Path<String>,
    Json(req): Json<AclReq>,
) -> ApiResult<StatusCode> {
    let role = match req.role.as_str() {
        "viewer" | "editor" => req.role.clone(),
        _ => return Err(ApiError::BadRequest("invalid_role")),
    };
    let email = req.email.trim().to_lowercase();
    st.run(move |inner, c| {
        let (d, _) = authorize(inner, c, &ctx, &doc, |r| r.manage, "acl.set")?;
        // The principal is looked up inside the caller's tenant only: other tenants' users cannot be named.
        let uid: String = c
            .query_row(
                "SELECT id FROM users WHERE tenant_id = ?1 AND email = ?2 AND disabled = 0",
                params![ctx.tenant_id, email],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(ApiError::BadRequest("unknown_user"))?;
        if uid == d.owner_id {
            return Err(ApiError::BadRequest("already_owner"));
        }
        let tx = c.transaction()?;
        tx.execute(
            "INSERT INTO acl (document_id, tenant_id, user_id, role, granted_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (document_id, user_id) DO UPDATE SET role = excluded.role, granted_by = excluded.granted_by",
            params![d.id, ctx.tenant_id, uid, role, ctx.user_id, now()],
        )?;
        audit::append(&tx, &inner.audit, &ctx.tenant_id, &ctx.user_id, "acl.set", &d.id, &format!("{uid} {role}"))?;
        tx.commit()?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

pub async fn acl_remove(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path((doc, user)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    st.run(move |inner, c| {
        let (d, _) = authorize(inner, c, &ctx, &doc, |r| r.manage, "acl.remove")?;
        let tx = c.transaction()?;
        let n = tx.execute(
            "DELETE FROM acl WHERE document_id = ?1 AND tenant_id = ?2 AND user_id = ?3",
            params![d.id, ctx.tenant_id, user],
        )?;
        if n == 0 {
            return Err(ApiError::NotFound);
        }
        audit::append(
            &tx,
            &inner.audit,
            &ctx.tenant_id,
            &ctx.user_id,
            "acl.remove",
            &d.id,
            &user,
        )?;
        tx.commit()?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

/// Manage access: owner first, then named users, matching the stored ACL.
pub async fn acl_list(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(doc): Path<String>,
) -> ApiResult<Json<Value>> {
    st.run(move |inner, c| {
        let (d, _) = authorize(inner, c, &ctx, &doc, |r| r.manage, "acl.list")?;
        let mut out = vec![];
        let owner: (String, String) = c.query_row(
            "SELECT id, email FROM users WHERE id = ?1 AND tenant_id = ?2",
            params![d.owner_id, ctx.tenant_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        out.push(json!({ "user_id": owner.0, "email": owner.1, "role": "owner" }));
        let mut s = c.prepare(
            "SELECT u.id, u.email, a.role FROM acl a JOIN users u ON u.id = a.user_id AND u.tenant_id = a.tenant_id
             WHERE a.document_id = ?1 AND a.tenant_id = ?2 ORDER BY u.email",
        )?;
        for r in s.query_map(params![d.id, ctx.tenant_id], |r| {
            Ok(json!({ "user_id": r.get::<_, String>(0)?, "email": r.get::<_, String>(1)?, "role": r.get::<_, String>(2)? }))
        })? {
            out.push(r?);
        }
        Ok(Json(json!(out)))
    })
    .await
}

fn entry_json(e: &audit::Entry) -> Value {
    json!({ "seq": e.seq, "ts": e.ts, "actor": e.actor, "action": e.action, "target": e.target,
            "detail": e.detail, "key_id": e.key_id, "mac": e.mac })
}

#[derive(Deserialize, Debug)]
pub struct LimitQ {
    limit: Option<u32>,
}

pub async fn audit_for_doc(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Path(doc): Path<String>,
    Query(q): Query<LimitQ>,
) -> ApiResult<Json<Value>> {
    st.run(move |inner, c| {
        let (d, _) = authorize(inner, c, &ctx, &doc, |r| r.manage, "audit.view")?;
        let rows = audit::list(c, &ctx.tenant_id, Some(&d.id), q.limit.unwrap_or(100))?;
        Ok(Json(json!(rows.iter().map(entry_json).collect::<Vec<_>>())))
    })
    .await
}

pub async fn audit_list(
    State(st): State<AppState>,
    ctx: TenantCtx,
    Query(q): Query<LimitQ>,
) -> ApiResult<Json<Value>> {
    if !ctx.is_admin {
        return Err(ApiError::Forbidden("admin_required"));
    }
    st.run(move |_, c| {
        let rows = audit::list(c, &ctx.tenant_id, None, q.limit.unwrap_or(100))?;
        Ok(Json(json!(rows.iter().map(entry_json).collect::<Vec<_>>())))
    })
    .await
}

pub async fn audit_verify(State(st): State<AppState>, ctx: TenantCtx) -> ApiResult<Json<Value>> {
    if !ctx.is_admin {
        return Err(ApiError::Forbidden("admin_required"));
    }
    st.run(move |inner, c| {
        let v = audit::verify(c, &inner.audit, &ctx.tenant_id)?;
        Ok(Json(json!({ "ok": v.ok, "entries": v.entries, "first_bad_seq": v.first_bad_seq, "head_mac": v.head_mac })))
    })
    .await
}
