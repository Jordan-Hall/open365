//! End-to-end tests through the real router (`tower::ServiceExt::oneshot`), SQLite on disk, local blob store.
//! Every secret is generated here; nothing is a real credential.

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use open365_server::config::{Argon2Cost, AuditKeys, Config, Rate};
use open365_server::util::{hex, random_bytes};
use open365_server::{AppState, ROUTES, router};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    dir: tempfile::TempDir,
    bootstrap: String,
}

fn env_with(tune: impl FnOnce(&mut Config)) -> Env {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = Config::new(dir.path());
    cfg.argon2 = Argon2Cost {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    let bootstrap = hex(&random_bytes::<16>());
    cfg.bootstrap_token = Some(bootstrap.clone());
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

fn env() -> Env {
    env_with(|_| {})
}

struct Resp {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

struct Req<'a> {
    method: Method,
    uri: String,
    token: Option<&'a str>,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    ip: Option<&'a str>,
}

fn req<'a>(method: Method, uri: impl Into<String>) -> Req<'a> {
    Req {
        method,
        uri: uri.into(),
        token: None,
        headers: vec![],
        body: vec![],
        ip: None,
    }
}

impl<'a> Req<'a> {
    fn auth(mut self, t: &'a str) -> Self {
        self.token = Some(t);
        self
    }
    fn hdr(mut self, k: &'static str, v: &str) -> Self {
        self.headers.push((k, v.to_string()));
        self
    }
    fn json(mut self, v: Value) -> Self {
        self.headers
            .push(("content-type", "application/json".into()));
        self.body = serde_json::to_vec(&v).expect("json");
        self
    }
    fn bytes(mut self, b: &[u8]) -> Self {
        self.body = b.to_vec();
        self
    }
    fn via_ip(mut self, ip: &'a str) -> Self {
        self.ip = Some(ip);
        self
    }
}

async fn send(e: &Env, r: Req<'_>) -> Resp {
    let mut b = Request::builder().method(r.method).uri(r.uri);
    if let Some(t) = r.token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    for (k, v) in &r.headers {
        b = b.header(*k, v);
    }
    let mut request = b.body(Body::from(r.body)).expect("request");
    if let Some(ip) = r.ip {
        let addr: SocketAddr = format!("{ip}:4000").parse().expect("addr");
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

const PW: &str = "correct horse battery staple";

async fn make_tenant(e: &Env, slug: &str, admin: &str) -> String {
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

async fn login(e: &Env, slug: &str, email: &str) -> String {
    let r = send(
        e,
        req(Method::POST, "/api/v1/login")
            .json(json!({"tenant": slug, "email": email, "password": PW, "bearer": true})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
    assert!(
        r.headers
            .get("set-cookie")
            .expect("cookie")
            .to_str()
            .expect("str")
            .contains("HttpOnly")
    );
    r.json()["token"].as_str().expect("token").to_string()
}

async fn add_user(e: &Env, admin_tok: &str, slug: &str, email: &str) -> String {
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

async fn upload(e: &Env, tok: &str, name: &str, data: &[u8]) -> String {
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

async fn new_link(e: &Env, tok: &str, doc: &str, body: Value) -> (String, String) {
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

/// A tenant with an admin who owns one uploaded document.
struct World {
    admin: String,
    doc: String,
}

async fn world(e: &Env) -> World {
    let admin = make_tenant(e, "acme", "admin@acme.test").await;
    let doc = upload(
        e,
        &admin,
        "Budget.xlsx",
        b"PK\x03\x04 pretend workbook bytes \x00\xff",
    )
    .await;
    World { admin, doc }
}

#[tokio::test]
async fn upload_download_and_versions_are_byte_exact() {
    let e = env();
    let w = world(&e).await;
    let r = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{}/content", w.doc)).auth(&w.admin),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body, b"PK\x03\x04 pretend workbook bytes \x00\xff");
    assert_eq!(r.headers["content-type"], "application/octet-stream");
    assert!(
        r.headers["content-disposition"]
            .to_str()
            .expect("s")
            .starts_with("attachment;")
    );
    assert_eq!(r.headers["x-content-type-options"], "nosniff");
    assert_eq!(r.headers["referrer-policy"], "no-referrer");
    assert!(
        r.headers["content-security-policy"]
            .to_str()
            .expect("s")
            .contains("sandbox")
    );

    let v2 = send(
        &e,
        req(
            Method::PUT,
            format!("/api/v1/documents/{}?base_version=1", w.doc),
        )
        .auth(&w.admin)
        .bytes(b"second"),
    )
    .await;
    assert_eq!(v2.status, StatusCode::OK);
    assert_eq!(v2.json()["version"], 2);
    let stale = send(
        &e,
        req(
            Method::PUT,
            format!("/api/v1/documents/{}?base_version=1", w.doc),
        )
        .auth(&w.admin)
        .bytes(b"third"),
    )
    .await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    let old = send(
        &e,
        req(
            Method::GET,
            format!("/api/v1/documents/{}/content?version=1", w.doc),
        )
        .auth(&w.admin),
    )
    .await;
    assert_eq!(
        old.body, b"PK\x03\x04 pretend workbook bytes \x00\xff",
        "versions are immutable"
    );
    let list = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{}/versions", w.doc)).auth(&w.admin),
    )
    .await;
    assert_eq!(list.json().as_array().expect("array").len(), 2);
}

#[tokio::test]
async fn uploads_are_size_limited_and_names_cannot_traverse() {
    let e = env_with(|c| c.max_upload_bytes = 1024);
    let w = world(&e).await;
    let big = vec![7u8; 4096];
    let r = send(
        &e,
        req(Method::POST, "/api/v1/documents?name=big.bin")
            .auth(&w.admin)
            .bytes(&big),
    )
    .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.json()["error"], "too_large");
    assert_eq!(r.headers["x-content-type-options"], "nosniff");
    for bad in ["..%2F..%2Fetc%2Fpasswd", "a%5Cb", "..", "x%00y"] {
        let r = send(
            &e,
            req(Method::POST, format!("/api/v1/documents?name={bad}"))
                .auth(&w.admin)
                .bytes(b"x"),
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
    }
    // The blob directory contains only tenant/xx/<sha256> files.
    for entry in walk(&e.dir.path().join("blobs")) {
        let n = entry
            .file_name()
            .expect("name")
            .to_string_lossy()
            .to_string();
        assert!(
            n.len() == 64 || n.len() == 32 || n.len() == 2,
            "unexpected file {n}"
        );
    }
}

fn walk(p: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                out.extend(walk(&path));
            }
            out.push(path);
        }
    }
    out
}

#[tokio::test]
async fn tenant_quota_is_enforced() {
    let e = env();
    let w = world(&e).await;
    e.state
        .db
        .lock()
        .execute("UPDATE tenants SET quota_bytes = 100", [])
        .expect("quota");
    let r = send(
        &e,
        req(Method::POST, "/api/v1/documents?name=q.bin")
            .auth(&w.admin)
            .bytes(&[1u8; 500]),
    )
    .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn acl_roles_are_enforced_and_listed() {
    let e = env();
    let w = world(&e).await;
    let viewer = add_user(&e, &w.admin, "acme", "v@acme.test").await;
    let editor = add_user(&e, &w.admin, "acme", "e@acme.test").await;
    let stranger = add_user(&e, &w.admin, "acme", "s@acme.test").await;
    let acl = |email: &str, role: &str| {
        req(Method::PUT, format!("/api/v1/documents/{}/acl", w.doc))
            .auth(&w.admin)
            .json(json!({"email": email, "role": role}))
    };
    assert_eq!(
        send(&e, acl("v@acme.test", "viewer")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&e, acl("e@acme.test", "editor")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&e, acl("nobody@acme.test", "editor")).await.status,
        StatusCode::BAD_REQUEST
    );

    let doc = &w.doc;
    let dl = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{doc}/content")).auth(&viewer),
    )
    .await;
    assert_eq!(dl.status, StatusCode::OK);
    let put_v = send(
        &e,
        req(Method::PUT, format!("/api/v1/documents/{doc}"))
            .auth(&viewer)
            .bytes(b"nope"),
    )
    .await;
    assert_eq!(put_v.status, StatusCode::FORBIDDEN);
    let put_e = send(
        &e,
        req(Method::PUT, format!("/api/v1/documents/{doc}"))
            .auth(&editor)
            .bytes(b"edited"),
    )
    .await;
    assert_eq!(put_e.status, StatusCode::OK);
    // Editors cannot manage sharing; viewers cannot create links.
    for t in [&editor, &viewer] {
        let r = send(
            &e,
            req(Method::POST, format!("/api/v1/documents/{doc}/links"))
                .auth(t)
                .json(json!({"scope": "view"})),
        )
        .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
    }
    // A user with no grant cannot even learn that the document exists.
    let none = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{doc}")).auth(&stranger),
    )
    .await;
    assert_eq!(none.status, StatusCode::NOT_FOUND);
    let list = send(&e, req(Method::GET, "/api/v1/documents").auth(&stranger)).await;
    assert_eq!(list.json().as_array().expect("arr").len(), 0);

    let shown = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{doc}/acl")).auth(&w.admin),
    )
    .await
    .json();
    let roles: Vec<(String, String)> = shown
        .as_array()
        .expect("arr")
        .iter()
        .map(|r| {
            (
                r["email"].as_str().expect("e").into(),
                r["role"].as_str().expect("r").into(),
            )
        })
        .collect();
    assert_eq!(
        roles,
        vec![
            ("admin@acme.test".into(), "owner".into()),
            ("e@acme.test".into(), "editor".into()),
            ("v@acme.test".into(), "viewer".into())
        ]
    );
    // Stop sharing.
    let uid = send(&e, req(Method::GET, "/api/v1/users").auth(&w.admin))
        .await
        .json();
    let vid = uid
        .as_array()
        .expect("arr")
        .iter()
        .find(|u| u["email"] == "v@acme.test")
        .expect("v")["user_id"]
        .as_str()
        .expect("id")
        .to_string();
    assert_eq!(
        send(
            &e,
            req(Method::DELETE, format!("/api/v1/documents/{doc}/acl/{vid}")).auth(&w.admin)
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &e,
            req(Method::GET, format!("/api/v1/documents/{doc}")).auth(&viewer)
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

fn fill(path: &str, doc: &str, link: &str, user: &str) -> String {
    path.replace("{doc}", doc)
        .replace("{link}", link)
        .replace("{user}", user)
}

#[tokio::test]
async fn cross_tenant_access_is_denied_on_every_route() {
    let e = env();
    let a = world(&e).await;
    let (link_id, token) = new_link(
        &e,
        &a.admin,
        &a.doc,
        json!({"scope": "edit", "audience": "org"}),
    )
    .await;
    let b_admin = make_tenant(&e, "globex", "admin@globex.test").await;
    let b_doc = upload(&e, &b_admin, "Mine.docx", b"b-bytes").await;
    let a_user = send(&e, req(Method::GET, "/api/v1/me").auth(&a.admin))
        .await
        .json()["user_id"]
        .as_str()
        .expect("id")
        .to_string();

    for (method, path) in ROUTES {
        // Unauthenticated callers never get in.
        let anon = send(
            &e,
            req(
                method.parse().expect("method"),
                fill(path, &a.doc, &link_id, &a_user),
            )
            .json(json!({})),
        )
        .await;
        assert_eq!(
            anon.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} without session"
        );

        // Tenant B's admin against tenant A's objects.
        let uri = fill(path, &a.doc, &link_id, &a_user);
        let mut r = req(method.parse().expect("method"), uri).auth(&b_admin);
        if *method == "POST" || *method == "PUT" {
            r = r.json(json!({"scope": "view", "email": "admin@acme.test", "role": "viewer", "allow_anyone_links": true}));
        }
        let res = send(&e, r).await;
        if path.contains("{doc}") || path.contains("{link}") {
            assert_eq!(
                res.status,
                StatusCode::NOT_FOUND,
                "{method} {path} leaked across tenants: {:?}",
                res.json()
            );
        }
        let text = String::from_utf8_lossy(&res.body).to_string();
        assert!(
            !text.contains(&a.doc) && !text.contains("Budget") && !text.contains("admin@acme"),
            "{method} {path} body leaked A data"
        );
    }
    // A's object is untouched by all of the above.
    let still = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{}/versions", a.doc)).auth(&a.admin),
    )
    .await;
    assert_eq!(still.json().as_array().expect("arr").len(), 1);

    // Naming another tenant's user in an ACL fails; B's session cannot use A's organisation link.
    let share = send(
        &e,
        req(Method::PUT, format!("/api/v1/documents/{}/acl", a.doc))
            .auth(&a.admin)
            .json(json!({"email": "admin@globex.test", "role": "viewer"})),
    )
    .await;
    assert_eq!(share.status, StatusCode::BAD_REQUEST);
    let org = send(&e, req(Method::GET, format!("/s/{token}")).auth(&b_admin)).await;
    assert_eq!(org.status, StatusCode::NOT_FOUND);
    // Tenant B's own document is not reachable with A's session either.
    let rev = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{b_doc}/content")).auth(&a.admin),
    )
    .await;
    assert_eq!(rev.status, StatusCode::NOT_FOUND);
    let audit_a = send(&e, req(Method::GET, "/api/v1/audit").auth(&a.admin))
        .await
        .json()
        .to_string();
    assert!(!audit_a.contains(&b_doc), "audit logs are per tenant");
}

#[tokio::test]
async fn link_scopes_are_enforced() {
    let e = env();
    let w = world(&e).await;
    let (_, view) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view", "audience": "anyone"}),
    )
    .await;
    let (_, edit) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "edit", "audience": "anyone"}),
    )
    .await;
    let (_, nodl) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view-no-download", "audience": "anyone"}),
    )
    .await;
    assert_ne!(view, edit);
    assert_eq!(view.len(), 43, "256 bits as unpadded base64url");

    let open = send(&e, req(Method::GET, format!("/s/{view}"))).await;
    assert_eq!(open.status, StatusCode::OK);
    assert_eq!(open.json()["name"], "Budget.xlsx");
    assert_eq!(open.json()["can_edit"], false);
    let dl = send(&e, req(Method::GET, format!("/s/{view}/content"))).await;
    assert_eq!(dl.status, StatusCode::OK);
    assert_eq!(dl.body, b"PK\x03\x04 pretend workbook bytes \x00\xff");
    assert_eq!(
        send(
            &e,
            req(Method::PUT, format!("/s/{view}/content")).bytes(b"x")
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );

    let n = send(&e, req(Method::GET, format!("/s/{nodl}"))).await;
    assert_eq!(n.status, StatusCode::OK);
    assert_eq!(n.json()["can_download"], false);
    assert_eq!(
        send(&e, req(Method::GET, format!("/s/{nodl}/content")))
            .await
            .json()["error"],
        "download_blocked"
    );
    assert_eq!(
        send(
            &e,
            req(Method::PUT, format!("/s/{nodl}/content")).bytes(b"x")
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );

    let up = send(
        &e,
        req(Method::PUT, format!("/s/{edit}/content")).bytes(b"via link"),
    )
    .await;
    assert_eq!(up.status, StatusCode::OK);
    assert_eq!(up.json()["version"], 2);
    let cur = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{}/content", w.doc)).auth(&w.admin),
    )
    .await;
    assert_eq!(cur.body, b"via link");
    let vs = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{}/versions", w.doc)).auth(&w.admin),
    )
    .await
    .json();
    assert!(
        vs[1]["created_by"]
            .as_str()
            .expect("s")
            .starts_with("link:")
    );
    // A link can never manage sharing.
    let mgmt = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{}/acl", w.doc)).auth(&edit),
    )
    .await;
    assert_eq!(mgmt.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_expired_and_revoked_links_look_identical_and_revocation_is_immediate() {
    let e = env();
    let w = world(&e).await;
    let (_, expired) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view", "audience": "anyone", "expires_in_secs": 3600}),
    )
    .await;
    let (rid, revoked) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view", "audience": "anyone"}),
    )
    .await;
    assert_eq!(
        send(&e, req(Method::GET, format!("/s/{revoked}")))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        send(&e, req(Method::GET, format!("/s/{expired}")))
            .await
            .status,
        StatusCode::OK
    );

    assert_eq!(
        send(
            &e,
            req(Method::DELETE, format!("/api/v1/links/{rid}")).auth(&w.admin)
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    e.state
        .db
        .lock()
        .execute(
            "UPDATE share_links SET expires_at = 1 WHERE expires_at IS NOT NULL",
            [],
        )
        .expect("expire");

    let unknown = hex(&random_bytes::<32>()); // wrong alphabet length on purpose is covered below
    let unknown = unknown[..43].to_string();
    let mut seen = vec![];
    for t in [&unknown, &expired, &revoked] {
        for suffix in ["", "/content"] {
            let r = send(&e, req(Method::GET, format!("/s/{t}{suffix}"))).await;
            seen.push((r.status, r.body.clone()));
        }
    }
    let uploads = send(
        &e,
        req(Method::PUT, format!("/s/{revoked}/content")).bytes(b"x"),
    )
    .await;
    assert_eq!((uploads.status, uploads.body), seen[0].clone());
    for pair in seen.chunks(2) {
        assert_eq!(pair[0].0, StatusCode::NOT_FOUND);
    }
    assert!(
        seen.iter().all(|s| *s == seen[0]),
        "unknown, expired and revoked must be indistinguishable"
    );
    // Malformed tokens get the same answer.
    let bad = send(&e, req(Method::GET, "/s/short")).await;
    assert_eq!((bad.status, bad.body), seen[0].clone());
}

#[tokio::test]
async fn wrong_password_is_rejected_and_locks_out() {
    let e = env_with(|c| {
        c.link_password_fail_rate = Rate {
            max: 3,
            window_secs: 900,
        }
    });
    let w = world(&e).await;
    let (_, tok) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view", "audience": "anyone", "password": "link-secret-1"}),
    )
    .await;
    let missing = send(&e, req(Method::GET, format!("/s/{tok}/content"))).await;
    assert_eq!(missing.status, StatusCode::UNAUTHORIZED);
    let wrong = send(
        &e,
        req(Method::GET, format!("/s/{tok}/content")).hdr("x-share-password", "guess"),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.body, missing.body);
    let right = send(
        &e,
        req(Method::GET, format!("/s/{tok}/content")).hdr("x-share-password", "link-secret-1"),
    )
    .await;
    assert_eq!(right.status, StatusCode::OK);
    assert_eq!(right.body, b"PK\x03\x04 pretend workbook bytes \x00\xff");
    // A missing password is a prompt, not a guess; a correct one clears the count. Three wrong guesses (the
    // limit) then lock the link for this address, even against the right password.
    for g in ["guess2", "guess3", "guess4"] {
        let r = send(
            &e,
            req(Method::GET, format!("/s/{tok}")).hdr("x-share-password", g),
        )
        .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    let locked = send(
        &e,
        req(Method::GET, format!("/s/{tok}")).hdr("x-share-password", "link-secret-1"),
    )
    .await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    // The stored value is an argon2id hash.
    let h: String = e
        .state
        .db
        .lock()
        .query_row(
            "SELECT password_hash FROM share_links WHERE password_hash IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("hash");
    assert!(h.starts_with("$argon2id$"));
}

#[tokio::test]
async fn link_resolution_is_rate_limited_per_address() {
    let e = env_with(|c| {
        c.link_ip_rate = Rate {
            max: 5,
            window_secs: 60,
        };
        c.link_fail_rate = Rate {
            max: 3,
            window_secs: 300,
        };
    });
    let w = world(&e).await;
    let (_, tok) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view", "audience": "anyone"}),
    )
    .await;
    // Guessing tokens from one address is cut off after a few misses.
    let mut statuses = vec![];
    for _ in 0..5 {
        let guess = hex(&random_bytes::<32>())[..43].to_string();
        statuses.push(
            send(
                &e,
                req(Method::GET, format!("/s/{guess}")).via_ip("203.0.113.9"),
            )
            .await
            .status,
        );
    }
    assert_eq!(statuses[0], StatusCode::NOT_FOUND);
    assert_eq!(
        *statuses.last().expect("last"),
        StatusCode::TOO_MANY_REQUESTS
    );
    // Another address is unaffected.
    assert_eq!(
        send(
            &e,
            req(Method::GET, format!("/s/{tok}")).via_ip("203.0.113.10")
        )
        .await
        .status,
        StatusCode::OK
    );
    // Plain request volume is capped too.
    let mut last = StatusCode::OK;
    for _ in 0..8 {
        last = send(
            &e,
            req(Method::GET, format!("/s/{tok}")).via_ip("198.51.100.1"),
        )
        .await
        .status;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn org_links_need_a_same_tenant_session_and_the_admin_kill_switch_works() {
    let e = env();
    let w = world(&e).await;
    let colleague = add_user(&e, &w.admin, "acme", "c@acme.test").await;
    let (_, org) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view", "audience": "org"}),
    )
    .await;
    let (_, any) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view", "audience": "anyone"}),
    )
    .await;
    assert_eq!(
        send(&e, req(Method::GET, format!("/s/{org}"))).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&e, req(Method::GET, format!("/s/{org}")).auth(&colleague))
            .await
            .status,
        StatusCode::OK
    );

    let pol = send(
        &e,
        req(Method::PUT, "/api/v1/admin/policy")
            .auth(&colleague)
            .json(json!({"allow_anyone_links": false})),
    )
    .await;
    assert_eq!(pol.status, StatusCode::FORBIDDEN, "only admins set policy");
    let pol = send(
        &e,
        req(Method::PUT, "/api/v1/admin/policy")
            .auth(&w.admin)
            .json(json!({"allow_anyone_links": false})),
    )
    .await;
    assert_eq!(pol.status, StatusCode::NO_CONTENT);
    assert_eq!(
        send(&e, req(Method::GET, format!("/s/{any}"))).await.status,
        StatusCode::NOT_FOUND
    );
    let again = send(
        &e,
        req(Method::POST, format!("/api/v1/documents/{}/links", w.doc))
            .auth(&w.admin)
            .json(json!({"scope": "view", "audience": "anyone"})),
    )
    .await;
    assert_eq!(again.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn no_plaintext_token_or_password_reaches_the_database_or_errors() {
    let e = env();
    let w = world(&e).await;
    let (_, tok) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view", "audience": "anyone", "password": "pw-for-link-1"}),
    )
    .await;
    let _ = send(
        &e,
        req(Method::GET, format!("/s/{tok}")).hdr("x-share-password", "pw-for-link-1"),
    )
    .await;
    let denied = send(&e, req(Method::GET, format!("/s/{tok}/content"))).await;
    assert!(!String::from_utf8_lossy(&denied.body).contains(&tok));
    // Includes the write-ahead log, where recent rows live.
    let mut hay = vec![];
    for f in ["open365.db", "open365.db-wal"] {
        if let Ok(b) = std::fs::read(e.dir.path().join(f)) {
            hay.extend(b);
        }
    }
    let contains = |needle: &str| hay.windows(needle.len()).any(|w| w == needle.as_bytes());
    assert!(!contains(&tok), "share token stored in plaintext");
    assert!(!contains(&w.admin), "session token stored in plaintext");
    assert!(
        !contains("pw-for-link-1") && !contains(PW),
        "password stored in plaintext"
    );
    assert!(
        contains(&open365_server::util::sha256_hex(tok.as_bytes())),
        "hash of the token is stored"
    );
}

#[tokio::test]
async fn audit_chain_verifies_records_events_and_detects_tampering() {
    let e = env();
    let w = world(&e).await;
    let viewer = add_user(&e, &w.admin, "acme", "v@acme.test").await;
    let (rid, tok) = new_link(
        &e,
        &w.admin,
        &w.doc,
        json!({"scope": "view", "audience": "anyone"}),
    )
    .await;
    let _ = send(&e, req(Method::GET, format!("/s/{tok}"))).await;
    let _ = send(&e, req(Method::GET, format!("/s/{tok}/content"))).await;
    let _ = send(
        &e,
        req(Method::PUT, format!("/api/v1/documents/{}", w.doc))
            .auth(&viewer)
            .bytes(b"x"),
    )
    .await; // no grant: 404, not audited as a known doc
    let _ = send(
        &e,
        req(Method::PUT, format!("/s/{tok}/content")).bytes(b"x"),
    )
    .await;
    let _ = send(
        &e,
        req(Method::DELETE, format!("/api/v1/links/{rid}")).auth(&w.admin),
    )
    .await;
    let _ = send(&e, req(Method::GET, format!("/s/{tok}"))).await;

    let v = send(&e, req(Method::GET, "/api/v1/audit/verify").auth(&w.admin))
        .await
        .json();
    assert_eq!(v["ok"], true, "{v}");
    let entries = send(
        &e,
        req(Method::GET, "/api/v1/audit?limit=200").auth(&w.admin),
    )
    .await
    .json();
    let actions: Vec<String> = entries
        .as_array()
        .expect("arr")
        .iter()
        .map(|x| x["action"].as_str().expect("a").to_string())
        .collect();
    for want in [
        "tenant.create",
        "login",
        "document.create",
        "user.create",
        "link.create",
        "link.open",
        "link.download",
        "link.denied",
        "link.revoke",
    ] {
        assert!(
            actions.iter().any(|a| a == want),
            "missing audit action {want}: {actions:?}"
        );
    }
    assert!(
        !entries.to_string().contains(&tok),
        "tokens never appear in the audit log"
    );
    // Non-admins cannot read the tenant log; the owner reads the log of their own document.
    assert_eq!(
        send(&e, req(Method::GET, "/api/v1/audit").auth(&viewer))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let per_doc = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{}/audit", w.doc)).auth(&w.admin),
    )
    .await;
    assert_eq!(per_doc.status, StatusCode::OK);
    assert!(per_doc.json().as_array().expect("arr").len() >= 4);

    // Tamper with one stored event: the chain breaks at exactly that row.
    let seq: i64 = e
        .state
        .db
        .lock()
        .query_row(
            "SELECT seq FROM audit WHERE action = 'document.create'",
            [],
            |r| r.get(0),
        )
        .expect("seq");
    e.state
        .db
        .lock()
        .execute(
            "UPDATE audit SET actor = 'mallory' WHERE action = 'document.create'",
            [],
        )
        .expect("tamper");
    let v = send(&e, req(Method::GET, "/api/v1/audit/verify").auth(&w.admin))
        .await
        .json();
    assert_eq!(v["ok"], false);
    assert_eq!(v["first_bad_seq"], seq);
}

#[tokio::test]
async fn sign_in_failures_are_uniform_rate_limited_and_logout_revokes() {
    let e = env_with(|c| {
        c.login_fail_rate = Rate {
            max: 3,
            window_secs: 900,
        }
    });
    let w = world(&e).await;
    let bad = |tenant: &str, email: &str, pw: &str| {
        req(Method::POST, "/api/v1/login")
            .json(json!({"tenant": tenant, "email": email, "password": pw}))
    };
    let a = send(&e, bad("acme", "admin@acme.test", "wrong-password-1")).await;
    let b = send(&e, bad("acme", "ghost@acme.test", "wrong-password-1")).await;
    let c = send(&e, bad("nope", "admin@acme.test", "wrong-password-1")).await;
    assert_eq!((a.status, &a.body), (StatusCode::UNAUTHORIZED, &b.body));
    assert_eq!(
        b.body, c.body,
        "unknown tenant, unknown user and wrong password look the same"
    );
    let _ = send(&e, bad("acme", "admin@acme.test", "wrong-password-2")).await;
    let _ = send(&e, bad("acme", "admin@acme.test", "wrong-password-3")).await;
    assert_eq!(
        send(&e, bad("acme", "admin@acme.test", PW)).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );

    assert_eq!(
        send(&e, req(Method::GET, "/api/v1/me").auth(&w.admin))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        send(&e, req(Method::POST, "/api/v1/logout").auth(&w.admin))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&e, req(Method::GET, "/api/v1/me").auth(&w.admin))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // A forged or malformed bearer value is rejected.
    assert_eq!(
        send(&e, req(Method::GET, "/api/v1/me").auth("not-a-session"))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn tenant_provisioning_needs_the_bootstrap_token() {
    let e = env();
    let body = json!({"slug": "x1", "name": "x", "admin_email": "a@x.test", "admin_password": PW});
    let none = send(
        &e,
        req(Method::POST, "/api/v1/admin/tenants").json(body.clone()),
    )
    .await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
    let wrong = send(
        &e,
        req(Method::POST, "/api/v1/admin/tenants")
            .hdr("x-bootstrap-token", "nope")
            .json(body.clone()),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let weak = send(&e, req(Method::POST, "/api/v1/admin/tenants").hdr("x-bootstrap-token", &e.bootstrap).json(json!({"slug": "x1", "name": "x", "admin_email": "a@x.test", "admin_password": "short"}))).await;
    assert_eq!(weak.status, StatusCode::BAD_REQUEST);
    let off = env_with(|c| c.bootstrap_token = None);
    let r = send(
        &off,
        req(Method::POST, "/api/v1/admin/tenants")
            .hdr("x-bootstrap-token", "x")
            .json(body),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::NOT_FOUND,
        "disabled when no bootstrap token is configured"
    );
}

#[tokio::test]
async fn data_survives_reopening_the_store() {
    let e = env();
    let w = world(&e).await;
    let dir = e.dir.path().to_path_buf();
    let keys = AuditKeys::single("test-key", vec![1u8; 32]).expect("keys");
    // Same data directory, fresh process state: migrations are idempotent and rows are still there.
    let mut cfg = Config::new(&dir);
    cfg.argon2 = Argon2Cost {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    let again = AppState::new(cfg, keys).expect("reopen");
    let n: i64 = again
        .db
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE id = ?1",
            [&w.doc],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 1);
}
