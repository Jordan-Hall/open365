//! Shared harness for the feature tests: the real router over SQLite on disk, driven with `tower::ServiceExt`.
//! Every secret is generated here; nothing is a real credential.

#![allow(dead_code)]

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use open365_server::config::{Argon2Cost, AuditKeys, Config};
use open365_server::util::{hex, random_bytes};
use open365_server::{AppState, router};
use serde_json::{Value, json};
use tower::ServiceExt;

pub const PW: &str = "correct horse battery staple";

pub struct Env {
    pub app: Router,
    pub state: AppState,
    pub dir: tempfile::TempDir,
    pub bootstrap: String,
}

pub fn env_with(tune: impl FnOnce(&mut Config)) -> Env {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = Config::new(dir.path());
    cfg.argon2 = Argon2Cost {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    let bootstrap = hex(&random_bytes::<16>());
    cfg.bootstrap_token = Some(bootstrap.clone());
    cfg.secret_keys = Some(
        open365_server::secrets::SecretKeys::single("test-s1", random_bytes::<32>().to_vec())
            .expect("secret key"),
    );
    tune(&mut cfg);
    let keys = AuditKeys::single("test-key", random_bytes::<32>().to_vec()).expect("keys");
    let state = AppState::new(cfg, keys).expect("state");
    Env {
        app: router(state.clone()),
        state,
        dir,
        bootstrap,
    }
}

pub fn env() -> Env {
    env_with(|_| {})
}

pub struct Resp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

pub struct Req {
    pub method: Method,
    pub uri: String,
    pub token: Option<String>,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
    pub ip: Option<String>,
}

pub fn req(method: Method, uri: impl Into<String>) -> Req {
    Req {
        method,
        uri: uri.into(),
        token: None,
        headers: vec![],
        body: vec![],
        ip: None,
    }
}

impl Req {
    pub fn auth(mut self, t: &str) -> Self {
        self.token = Some(t.to_string());
        self
    }
    pub fn hdr(mut self, k: &'static str, v: &str) -> Self {
        self.headers.push((k, v.to_string()));
        self
    }
    pub fn json(mut self, v: Value) -> Self {
        self.headers
            .push(("content-type", "application/json".into()));
        self.body = serde_json::to_vec(&v).expect("json");
        self
    }
    pub fn bytes(mut self, b: &[u8]) -> Self {
        self.body = b.to_vec();
        self
    }
    pub fn via_ip(mut self, ip: &str) -> Self {
        self.ip = Some(ip.to_string());
        self
    }
}

pub async fn send(e: &Env, r: Req) -> Resp {
    send_body(e, r, None).await
}

/// Like [`send`], with an explicit body (for streams that never finish).
pub async fn send_body(e: &Env, r: Req, body: Option<Body>) -> Resp {
    let mut b = Request::builder().method(r.method).uri(r.uri);
    if let Some(t) = &r.token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    for (k, v) in &r.headers {
        b = b.header(*k, v);
    }
    let mut request = b
        .body(body.unwrap_or_else(|| Body::from(r.body)))
        .expect("request");
    if let Some(ip) = &r.ip {
        let addr: SocketAddr = if ip.contains(':') {
            format!("[{ip}]:4000")
        } else {
            format!("{ip}:4000")
        }
        .parse()
        .expect("addr");
        request.extensions_mut().insert(ConnectInfo(addr));
    }
    let res = e.app.clone().oneshot(request).await.expect("response");
    let (parts, body) = res.into_parts();
    let body = body.collect().await.expect("body").to_bytes().to_vec();
    Resp {
        status: parts.status,
        headers: parts.headers,
        body,
    }
}

pub async fn make_tenant(e: &Env, slug: &str, admin: &str) -> String {
    let r = send(
        e,
        req(Method::POST, "/api/v1/admin/tenants")
            .hdr("x-bootstrap-token", &e.bootstrap)
            .json(json!({"slug": slug, "name": slug, "admin_email": admin, "admin_password": PW})),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json());
    login(e, slug, admin).await
}

pub async fn login(e: &Env, slug: &str, email: &str) -> String {
    let r = send(
        e,
        req(Method::POST, "/api/v1/login")
            .json(json!({"tenant": slug, "email": email, "password": PW, "bearer": true})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
    r.json()["token"].as_str().expect("token").to_string()
}

pub async fn add_user(e: &Env, admin_tok: &str, slug: &str, email: &str) -> String {
    let r = send(
        e,
        req(Method::POST, "/api/v1/users")
            .auth(admin_tok)
            .json(json!({"email": email, "password": PW, "display_name": email})),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json());
    login(e, slug, email).await
}

pub async fn upload(e: &Env, tok: &str, name: &str, data: &[u8]) -> String {
    let r = send(
        e,
        req(Method::POST, format!("/api/v1/documents?name={name}"))
            .auth(tok)
            .bytes(data),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json());
    r.json()["id"].as_str().expect("id").to_string()
}

pub async fn new_link(e: &Env, tok: &str, doc: &str, body: Value) -> (String, String) {
    let r = send(
        e,
        req(Method::POST, format!("/api/v1/documents/{doc}/links"))
            .auth(tok)
            .json(body),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json());
    let j = r.json();
    (
        j["id"].as_str().expect("id").to_string(),
        j["token"].as_str().expect("token").to_string(),
    )
}

pub async fn audit_actions(e: &Env, admin: &str) -> Vec<String> {
    let r = send(e, req(Method::GET, "/api/v1/audit?limit=1000").auth(admin)).await;
    r.json()
        .as_array()
        .expect("array")
        .iter()
        .map(|x| x["action"].as_str().expect("action").to_string())
        .collect()
}

/// Serves the environment's router over a real loopback socket (limits applied) and returns its base URL.
pub async fn serve_env(e: &Env) -> (String, axum_server::Handle<SocketAddr>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = axum_server::Handle::new();
    let limits = open365_server::serve::ServeLimits::from_config(&e.state.cfg);
    tokio::spawn(open365_server::serve::serve(
        listener,
        router(e.state.clone()),
        limits,
        None,
        handle.clone(),
    ));
    (format!("http://{addr}"), handle)
}
