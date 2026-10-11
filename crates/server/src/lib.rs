//! Open365 sharing server (ADR-013 Stage 1): tenants, local accounts, versioned documents, ACLs, share links and an
//! HMAC-chained audit log. Uploads are opaque bytes: nothing here parses a document, and nothing here makes an
//! outbound network request (no SSRF surface).

pub mod api;
pub mod audit;
pub mod auth;
pub mod blob;
pub mod client;
pub mod config;
pub mod db;
pub mod docs;
pub mod error;
pub mod keys;
pub mod links;
#[cfg(feature = "oidc")]
pub mod oidc;
pub mod ops;
pub mod perm;
pub mod proxy;
pub mod ratelimit;
pub mod secrets;
pub mod serve;
pub mod totp;
pub mod util;

use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::Arc;

use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequestParts, Request, State};
use axum::handler::Handler as _;
use axum::http::Method;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, HeaderName, HeaderValue, REFERRER_POLICY,
    STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
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
    /// Caps concurrent argon2 computations so that sign-in floods cannot exhaust memory or CPU.
    pub argon_sem: tokio::sync::Semaphore,
    #[cfg(feature = "oidc")]
    pub oidc: oidc::Registry,
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
        util::make_private_dir(&cfg.data_dir)?;
        let db = Db::open(&cfg.data_dir.join("open365.db"))?;
        if let Some(keys) = &cfg.secret_keys {
            // Secrets stored before encryption existed are sealed now; nothing stays in the clear.
            secrets::reseal_all(&mut db.lock(), keys)?;
        }
        let limiter =
            ratelimit::Limiter::open(&cfg.data_dir.join("ratelimit.db"), audit.root_key())?;
        let blobs = Box::new(LocalDiskBlobStore::new(cfg.data_dir.join("blobs"))?);
        let dummy_hash = auth::hash_password_sync(&cfg, &util::hex(&util::random_bytes::<16>()))?;
        let permits = cfg.argon2_concurrency.max(1);
        #[cfg(feature = "oidc")]
        let oidc = oidc::Registry::new(&cfg.oidc).map_err(|e| {
            eprintln!("oidc configuration: {e}");
            ApiError::Internal
        })?;
        Ok(Self(Arc::new(Inner {
            db,
            blobs,
            cfg,
            audit,
            limiter,
            dummy_hash,
            argon_sem: tokio::sync::Semaphore::new(permits),
            #[cfg(feature = "oidc")]
            oidc,
        })))
    }

    /// Writes (and fsyncs) a blob off the executor and without the database lock. Returns `(blob id, size)`.
    pub async fn store_blob(
        &self,
        tenant: &str,
        data: axum::body::Bytes,
    ) -> ApiResult<(String, i64)> {
        let inner = self.0.clone();
        let tenant = tenant.to_string();
        tokio::task::spawn_blocking(move || {
            let size = i64::try_from(data.len()).map_err(|_| ApiError::TooLarge)?;
            Ok((inner.blobs.put(&tenant, &data)?, size))
        })
        .await
        .map_err(|_| ApiError::Internal)?
    }

    /// Runs work that takes the database lock itself, in short pieces (long verifications).
    pub async fn run_unlocked<R, F>(&self, f: F) -> ApiResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&Inner) -> ApiResult<R> + Send + 'static,
    {
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || f(&inner))
            .await
            .map_err(|_| ApiError::Internal)?
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

/// The client address: the TCP peer, or the address from `X-Forwarded-For` when the peer is a configured trusted
/// proxy. `unknown` when the server was not started with connect info (unit tests).
#[derive(Clone, Debug)]
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip());
        let fwd: Vec<&str> = parts
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        Ok(Self(proxy::client_ip(peer, &fwd, &st.cfg.trusted_proxies)))
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
    ("DELETE", "/api/v1/documents/{doc}"),
    ("POST", "/api/v1/documents/{doc}/restore"),
    ("POST", "/api/v1/documents/{doc}/purge"),
    ("POST", "/api/v1/documents/{doc}/transfer"),
    ("GET", "/api/v1/documents/trash"),
    ("POST", "/api/v1/me/totp/enroll"),
    ("POST", "/api/v1/me/totp/confirm"),
    ("POST", "/api/v1/me/totp/recovery-codes"),
    ("DELETE", "/api/v1/me/totp"),
    ("DELETE", "/api/v1/users/{user}/totp"),
    ("GET", "/api/v1/audit"),
    ("GET", "/api/v1/audit/verify"),
];

pub fn router(state: AppState) -> Router {
    let upload = state.cfg.max_upload_bytes;
    let json_limit = state.cfg.json_body_limit;
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/v1/admin/tenants", post(auth::create_tenant))
        .route("/api/v1/admin/policy", put(auth::set_policy))
        .route("/api/v1/login", post(auth::login))
        .route("/api/v1/logout", post(auth::logout))
        .route("/api/v1/me", get(auth::me))
        .route("/api/v1/me/totp/enroll", post(totp::enroll))
        .route("/api/v1/me/totp/confirm", post(totp::confirm))
        .route(
            "/api/v1/me/totp/recovery-codes",
            post(totp::new_recovery_codes),
        )
        .route("/api/v1/me/totp", delete(totp::disable))
        .route("/api/v1/users/{user}/totp", delete(totp::admin_reset))
        .route(
            "/api/v1/users",
            post(auth::create_user).get(auth::list_users),
        )
        .route(
            "/api/v1/documents",
            post(docs::create.layer(DefaultBodyLimit::max(upload))).get(docs::list),
        )
        .route("/api/v1/documents/trash", get(docs::trash))
        .route(
            "/api/v1/documents/{doc}",
            get(docs::meta)
                .put(docs::new_version.layer(DefaultBodyLimit::max(upload)))
                .delete(docs::delete),
        )
        .route("/api/v1/documents/{doc}/restore", post(docs::restore))
        .route("/api/v1/documents/{doc}/purge", post(docs::purge))
        .route("/api/v1/documents/{doc}/transfer", post(docs::transfer))
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
            get(links::download).put(links::upload.layer(DefaultBodyLimit::max(upload))),
        );
    #[cfg(feature = "oidc")]
    let app = app
        .route("/auth/oidc/{tenant}/start", get(oidc::start))
        .route("/auth/oidc/callback", get(oidc::callback))
        .route("/auth/oidc/poll", post(oidc::poll))
        .route("/auth/oidc/confirm", post(oidc::confirm));
    app.layer(DefaultBodyLimit::max(json_limit))
        .layer(middleware::from_fn_with_state(state.clone(), timeout))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        .with_state(state)
}

/// Routes that carry document bytes get the long time limit; everything else the short one.
fn is_upload_route(method: &Method, path: &str) -> bool {
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    matches!(
        (method, segs.as_slice()),
        (&Method::POST, ["api", "v1", "documents"])
            | (&Method::PUT, ["api", "v1", "documents", _])
            | (&Method::PUT, ["s", _, "content"])
    )
}

async fn timeout(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let secs = if is_upload_route(req.method(), req.uri().path()) {
        st.cfg.upload_timeout_secs
    } else {
        st.cfg.request_timeout_secs
    };
    match tokio::time::timeout(std::time::Duration::from_secs(secs), next.run(req)).await {
        Ok(res) => res,
        Err(_) => axum::response::IntoResponse::into_response(ApiError::Timeout),
    }
}

async fn security_headers(State(st): State<AppState>, req: Request, next: Next) -> Response {
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
    if !h.contains_key(CONTENT_SECURITY_POLICY) {
        h.insert(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'; sandbox"),
        );
    }
    h.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    if st.cfg.hsts {
        h.insert(
            STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    res
}

#[cfg(test)]
mod tests {
    #[test]
    fn upload_routes_are_recognised() {
        use axum::http::Method;
        assert!(super::is_upload_route(&Method::POST, "/api/v1/documents"));
        assert!(super::is_upload_route(
            &Method::PUT,
            "/api/v1/documents/abc"
        ));
        assert!(super::is_upload_route(&Method::PUT, "/s/tok/content"));
        assert!(!super::is_upload_route(&Method::GET, "/s/tok/content"));
        assert!(!super::is_upload_route(
            &Method::POST,
            "/api/v1/documents/abc/links"
        ));
        assert!(!super::is_upload_route(&Method::POST, "/api/v1/login"));
    }

    #[test]
    fn route_table_covers_every_tenant_route() {
        let src = include_str!("lib.rs");
        let body = src.split("pub fn router").nth(1).expect("router fn");
        let body = body.split("fn is_upload_route").next().expect("end");
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
                        .replace("{tenant}", "{x}")
                };
                canon(p) == canon(&norm)
            });
            assert!(listed, "route {path} missing from ROUTES");
        }
    }
}
