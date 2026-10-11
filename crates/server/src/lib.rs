//! Open365 sharing server (ADR-013 Stage 1): tenants, local accounts, versioned documents, ACLs, share links and an
//! HMAC-chained audit log. Uploads are opaque bytes: nothing here parses a document, and nothing here makes an
//! outbound network request (no SSRF surface).

pub mod audit;
pub mod auth;
pub mod blob;
pub mod config;
pub mod db;
pub mod docs;
pub mod error;
pub mod links;
pub mod perm;
pub mod ratelimit;
pub mod util;

use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::Arc;

use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequestParts, Request};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, HeaderName, HeaderValue, REFERRER_POLICY,
    X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::request::Parts;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::{Router, http::StatusCode};
use rusqlite::Connection;

use crate::blob::{BlobStore, LocalDiskBlobStore};
use crate::config::{AuditKeys, Config};
use crate::db::Db;
use crate::error::{ApiError, ApiResult};

#[derive(Debug)]
pub struct Inner {
    pub db: Db,
    pub blobs: Box<dyn BlobStore>,
    pub cfg: Config,
    pub audit: AuditKeys,
    pub limiter: ratelimit::Limiter,
    /// Verified when a sign-in names an unknown account, so timing does not reveal which accounts exist.
    pub dummy_hash: String,
}

#[derive(Clone, Debug)]
pub struct AppState(Arc<Inner>);

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(cfg: Config, audit: AuditKeys) -> ApiResult<Self> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let db = Db::open(&cfg.data_dir.join("open365.db"))?;
        let blobs = Box::new(LocalDiskBlobStore::new(cfg.data_dir.join("blobs"))?);
        let dummy_hash = auth::hash_password_sync(&cfg, &util::hex(&util::random_bytes::<16>()))?;
        Ok(Self(Arc::new(Inner {
            db,
            blobs,
            cfg,
            audit,
            limiter: ratelimit::Limiter::default(),
            dummy_hash,
        })))
    }

    /// Runs blocking store work (SQLite, disk) off the async executor.
    pub async fn run<R, F>(&self, f: F) -> ApiResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&Inner, &mut Connection) -> ApiResult<R> + Send + 'static,
    {
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = inner.db.lock();
            f(&inner, &mut conn)
        })
        .await
        .map_err(|_| ApiError::Internal)?
    }
}

/// The caller's tenant and identity, derived only from the session (never from request fields).
#[derive(Clone, Debug)]
pub struct TenantCtx {
    pub tenant_id: String,
    pub user_id: String,
    pub email: String,
    pub is_admin: bool,
}

impl FromRequestParts<AppState> for TenantCtx {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        auth::authenticate(state, &parts.headers).await
    }
}

/// Peer address, or `unknown` when the server was not started with connect info (unit tests).
#[derive(Clone, Debug)]
pub struct ClientIp(pub String);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map_or_else(|| "unknown".to_string(), |c| c.0.ip().to_string());
        Ok(Self(ip))
    }
}

/// Every route that touches tenant data, as `(method, path)`. The cross-tenant test walks this table, and a
/// drift test fails if `router` registers a path that is not listed here.
pub const ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/v1/me"),
    ("POST", "/api/v1/users"),
    ("GET", "/api/v1/users"),
    ("PUT", "/api/v1/admin/policy"),
    ("POST", "/api/v1/documents"),
    ("GET", "/api/v1/documents"),
    ("GET", "/api/v1/documents/{doc}"),
    ("PUT", "/api/v1/documents/{doc}"),
    ("GET", "/api/v1/documents/{doc}/versions"),
    ("GET", "/api/v1/documents/{doc}/content"),
    ("GET", "/api/v1/documents/{doc}/acl"),
    ("PUT", "/api/v1/documents/{doc}/acl"),
    ("DELETE", "/api/v1/documents/{doc}/acl/{user}"),
    ("POST", "/api/v1/documents/{doc}/links"),
    ("GET", "/api/v1/documents/{doc}/links"),
    ("GET", "/api/v1/documents/{doc}/audit"),
    ("DELETE", "/api/v1/links/{link}"),
    ("GET", "/api/v1/audit"),
    ("GET", "/api/v1/audit/verify"),
];

pub fn router(state: AppState) -> Router {
    let body_limit = state.cfg.max_upload_bytes;
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/v1/admin/tenants", post(auth::create_tenant))
        .route("/api/v1/admin/policy", put(auth::set_policy))
        .route("/api/v1/login", post(auth::login))
        .route("/api/v1/logout", post(auth::logout))
        .route("/api/v1/me", get(auth::me))
        .route(
            "/api/v1/users",
            post(auth::create_user).get(auth::list_users),
        )
        .route("/api/v1/documents", post(docs::create).get(docs::list))
        .route(
            "/api/v1/documents/{doc}",
            get(docs::meta).put(docs::new_version),
        )
        .route("/api/v1/documents/{doc}/versions", get(docs::versions))
        .route("/api/v1/documents/{doc}/content", get(docs::content))
        .route(
            "/api/v1/documents/{doc}/acl",
            get(docs::acl_list).put(docs::acl_set),
        )
        .route(
            "/api/v1/documents/{doc}/acl/{user}",
            delete(docs::acl_remove),
        )
        .route(
            "/api/v1/documents/{doc}/links",
            post(links::create).get(links::list),
        )
        .route("/api/v1/documents/{doc}/audit", get(docs::audit_for_doc))
        .route("/api/v1/links/{link}", delete(links::revoke))
        .route("/api/v1/audit", get(docs::audit_list))
        .route("/api/v1/audit/verify", get(docs::audit_verify))
        .route("/s/{token}", get(links::open))
        .route(
            "/s/{token}/content",
            get(links::download).put(links::upload),
        )
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    if res.status() == StatusCode::PAYLOAD_TOO_LARGE {
        // axum's own limit rejection has a plain-text body; keep errors uniform.
        res = axum::response::IntoResponse::into_response(ApiError::TooLarge);
    }
    let h = res.headers_mut();
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'; sandbox"),
    );
    h.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    res
}

#[cfg(test)]
mod tests {
    #[test]
    fn route_table_covers_every_tenant_route() {
        let src = include_str!("lib.rs");
        let body = src.split("pub fn router").nth(1).expect("router fn");
        let body = body.split("async fn security_headers").next().expect("end");
        for line in body.lines().filter(|l| l.contains(".route(\"")) {
            let path = line.split('"').nth(1).expect("path");
            let tenant_route = path.starts_with("/api/v1/")
                && !path.contains("/admin/tenants")
                && path != "/api/v1/login"
                && path != "/api/v1/logout";
            if !tenant_route {
                continue;
            }
            let norm = path.replace("{token}", "{x}");
            let listed = super::ROUTES.iter().any(|(_, p)| {
                let canon = |s: &str| {
                    s.replace("{doc}", "{x}")
                        .replace("{user}", "{x}")
                        .replace("{link}", "{x}")
                };
                canon(p) == canon(&norm)
            });
            assert!(listed, "route {path} missing from ROUTES");
        }
    }
}
