//! OpenID Connect sign-in against an in-process mock issuer (plain HTTP on loopback, which is the one case the
//! server accepts without TLS). The mock enforces PKCE like a real issuer and signs ES256 ID tokens.
#![cfg(feature = "oidc")]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use common::*;
use open365_server::config::OidcProvider;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const CLIENT_ID: &str = "open365-test";
const REDIRECT: &str = "https://app.example/cb";

struct Knobs {
    discovery_issuer: Option<String>,
    token_endpoint: Option<String>,
    token_issuer: Option<String>,
    token_aud: String,
    nonce_override: Option<String>,
    exp_offset: i64,
    sub: String,
    email: String,
    email_verified: bool,
    sign_with_stranger: bool,
}

struct Keys {
    pair: EcdsaKeyPair,
    kid: String,
}

struct Mock {
    base: String,
    knobs: Mutex<Knobs>,
    keys: Mutex<Keys>,
    stranger: EcdsaKeyPair,
    /// code -> (nonce, code_challenge)
    codes: Mutex<HashMap<String, (String, String)>>,
    jwks_fetches: Mutex<usize>,
    pkce_checks: Mutex<usize>,
}

fn new_pair() -> EcdsaKeyPair {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).expect("gen");
    EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).expect("pair")
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).expect("hex"));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("utf8")
}

fn parse_pairs(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (pct_decode(k), pct_decode(v)))
        .collect()
}

fn jwt(pair: &EcdsaKeyPair, kid: &str, claims: &Value) -> String {
    let h = B64.encode(json!({"alg": "ES256", "kid": kid, "typ": "JWT"}).to_string());
    let p = B64.encode(claims.to_string());
    let msg = format!("{h}.{p}");
    let sig = pair
        .sign(&SystemRandom::new(), msg.as_bytes())
        .expect("sign");
    format!("{msg}.{}", B64.encode(sig.as_ref()))
}

async fn discovery(State(m): State<Arc<Mock>>) -> impl IntoResponse {
    let k = m.knobs.lock().expect("knobs");
    axum::Json(json!({
        "issuer": k.discovery_issuer.clone().unwrap_or_else(|| m.base.clone()),
        "authorization_endpoint": format!("{}/authorize", m.base),
        "token_endpoint": k.token_endpoint.clone().unwrap_or_else(|| format!("{}/token", m.base)),
        "jwks_uri": format!("{}/jwks", m.base),
    }))
}

async fn jwks(State(m): State<Arc<Mock>>) -> impl IntoResponse {
    *m.jwks_fetches.lock().expect("n") += 1;
    let keys = m.keys.lock().expect("keys");
    let pt = keys.pair.public_key().as_ref();
    axum::Json(json!({"keys": [{
        "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": keys.kid,
        "x": B64.encode(&pt[1..33]), "y": B64.encode(&pt[33..65]),
    }]}))
}

async fn token(State(m): State<Arc<Mock>>, body: String) -> axum::response::Response {
    let f = parse_pairs(&body);
    let refuse = || {
        (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": "invalid_grant"})),
        )
            .into_response()
    };
    if f.get("grant_type").map(String::as_str) != Some("authorization_code")
        || f.get("client_id").map(String::as_str) != Some(CLIENT_ID)
        || f.get("redirect_uri").map(String::as_str) != Some(REDIRECT)
        || f.get("client_secret").map(String::as_str) != Some("s3cret")
    {
        return refuse();
    }
    let Some((nonce, challenge)) = m
        .codes
        .lock()
        .expect("codes")
        .remove(f.get("code").map_or("", String::as_str))
    else {
        return refuse();
    };
    // PKCE: S256(code_verifier) must equal the challenge sent in the authorization request.
    *m.pkce_checks.lock().expect("n") += 1;
    let verifier = f.get("code_verifier").map_or("", String::as_str);
    if B64.encode(Sha256::digest(verifier.as_bytes())) != challenge {
        return refuse();
    }
    let k = m.knobs.lock().expect("knobs");
    let now = open365_server::util::now();
    let claims = json!({
        "iss": k.token_issuer.clone().unwrap_or_else(|| m.base.clone()),
        "aud": k.token_aud, "sub": k.sub, "email": k.email, "email_verified": k.email_verified,
        "nonce": k.nonce_override.clone().unwrap_or(nonce), "iat": now, "exp": now + k.exp_offset,
    });
    let keys = m.keys.lock().expect("keys");
    let id_token = if k.sign_with_stranger {
        jwt(&m.stranger, &keys.kid, &claims)
    } else {
        jwt(&keys.pair, &keys.kid, &claims)
    };
    axum::Json(json!({"id_token": id_token, "access_token": "unused", "token_type": "Bearer"}))
        .into_response()
}

async fn start_mock() -> Arc<Mock> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let mock = Arc::new(Mock {
        base: base.clone(),
        knobs: Mutex::new(Knobs {
            discovery_issuer: None,
            token_endpoint: None,
            token_issuer: None,
            token_aud: CLIENT_ID.into(),
            nonce_override: None,
            exp_offset: 3600,
            sub: "sub-1".into(),
            email: "bob@acme.test".into(),
            email_verified: true,
            sign_with_stranger: false,
        }),
        keys: Mutex::new(Keys {
            pair: new_pair(),
            kid: "k1".into(),
        }),
        stranger: new_pair(),
        codes: Mutex::new(HashMap::new()),
        jwks_fetches: Mutex::new(0),
        pkce_checks: Mutex::new(0),
    });
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .with_state(mock.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    mock
}

struct World {
    e: Env,
    mock: Arc<Mock>,
    /// The browser's cookie jar (the cookie set by the last `start`).
    cookie: std::sync::Mutex<String>,
}

async fn world() -> World {
    world_with(false).await
}

async fn world_with(tls: bool) -> World {
    let mock = start_mock().await;
    let issuer = mock.base.clone();
    let e = env_with(move |c| {
        c.hsts = tls;
        c.oidc = vec![OidcProvider {
            tenant_slug: "acme".into(),
            issuer,
            client_id: CLIENT_ID.into(),
            client_secret: Some("s3cret".into()),
            redirect_uri: REDIRECT.into(),
            extra_hosts: vec![],
        }]
    });
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    add_user(&e, &admin, "acme", "bob@acme.test").await;
    World {
        e,
        mock,
        cookie: std::sync::Mutex::new(String::new()),
    }
}

/// The browser's part: start, follow the redirect (the mock issues a code bound to nonce and challenge).
async fn begin(w: &World) -> (String, String) {
    begin_at(w, "/auth/oidc/acme/start?bearer=true").await
}

fn cookie_of(r: &Resp) -> String {
    r.headers["set-cookie"]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("pair")
        .to_string()
}

async fn begin_at(w: &World, path: &str) -> (String, String) {
    let r = send(&w.e, req(Method::GET, path)).await;
    let set = r.headers["set-cookie"]
        .to_str()
        .expect("cookie")
        .to_string();
    assert!(
        set.contains("HttpOnly") && set.contains("Secure") && set.contains("SameSite=Lax"),
        "{set}"
    );
    *w.cookie.lock().expect("jar") = cookie_of(&r);
    assert_eq!(r.status, StatusCode::FOUND, "{:?}", r.json());
    let loc = r.headers["location"]
        .to_str()
        .expect("location")
        .to_string();
    assert!(loc.starts_with(&format!("{}/authorize?", w.mock.base)));
    let q = parse_pairs(loc.split_once('?').expect("query").1);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["redirect_uri"], REDIRECT);
    assert!(q["scope"].contains("openid"));
    let code = open365_server::util::hex(&open365_server::util::random_bytes::<8>());
    w.mock.codes.lock().expect("codes").insert(
        code.clone(),
        (q["nonce"].clone(), q["code_challenge"].clone()),
    );
    (code, q["state"].clone())
}

async fn callback(w: &World, code: &str, state: &str) -> Resp {
    let jar = w.cookie.lock().expect("jar").clone();
    callback_with(w, code, state, Some(&jar)).await
}

async fn callback_with(w: &World, code: &str, state: &str, cookie: Option<&str>) -> Resp {
    let mut r = req(
        Method::GET,
        format!("/auth/oidc/callback?code={code}&state={state}"),
    );
    if let Some(c) = cookie {
        r = r.hdr("cookie", c);
    }
    send(&w.e, r).await
}

async fn sign_in(w: &World) -> Resp {
    let (code, state) = begin(w).await;
    callback(w, &code, &state).await
}

#[tokio::test]
async fn code_flow_with_pkce_signs_in_and_links_the_verified_email() {
    let w = world().await;
    let r = sign_in(&w).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
    assert_eq!(*w.mock.pkce_checks.lock().expect("n"), 1);
    let tok = r.json()["token"]
        .as_str()
        .expect("bearer token")
        .to_string();
    let me = send(&w.e, req(Method::GET, "/api/v1/me").auth(&tok))
        .await
        .json();
    assert_eq!(me["email"], "bob@acme.test");
    // A returning user is matched by (issuer, sub), even if the email claim changed.
    w.mock.knobs.lock().expect("k").email = "renamed@elsewhere.test".into();
    let again = sign_in(&w).await;
    assert_eq!(again.status, StatusCode::OK);
    let me2 = send(
        &w.e,
        req(Method::GET, "/api/v1/me").auth(again.json()["token"].as_str().expect("t")),
    )
    .await
    .json();
    assert_eq!(me2["user_id"], me["user_id"]);
    let admin = login(&w.e, "acme", "admin@acme.test").await;
    let actions = audit_actions(&w.e, &admin).await;
    assert!(actions.iter().any(|a| a == "oidc.link"));
    assert!(actions.iter().any(|a| a == "login"));
}

#[tokio::test]
async fn unverified_or_unknown_identities_never_create_or_take_over_accounts() {
    let w = world().await;
    w.mock.knobs.lock().expect("k").email_verified = false;
    let r = sign_in(&w).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.json()["error"], "oidc_no_account");
    {
        let mut k = w.mock.knobs.lock().expect("k");
        k.email_verified = true;
        k.email = "stranger@acme.test".into();
    }
    assert_eq!(
        sign_in(&w).await.status,
        StatusCode::FORBIDDEN,
        "no just-in-time accounts"
    );
    let users: i64 =
        w.e.state
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
            .expect("n");
    assert_eq!(users, 2);
    let admin = login(&w.e, "acme", "admin@acme.test").await;
    assert!(
        audit_actions(&w.e, &admin)
            .await
            .iter()
            .filter(|a| *a == "login.failed")
            .count()
            >= 2
    );
}

#[tokio::test]
async fn forged_or_mismatched_id_tokens_are_rejected() {
    let w = world().await;
    type Tweak = fn(&mut Knobs);
    let cases: [(&str, Tweak); 6] = [
        ("wrong issuer", |k| {
            k.token_issuer = Some("https://evil.example".into())
        }),
        ("wrong audience", |k| k.token_aud = "someone-else".into()),
        ("wrong nonce", |k| {
            k.nonce_override = Some("not-the-nonce".into())
        }),
        ("expired", |k| k.exp_offset = -3600),
        ("signed by another key", |k| k.sign_with_stranger = true),
        ("empty subject", |k| k.sub = String::new()),
    ];
    for (name, tweak) in cases {
        {
            let mut k = w.mock.knobs.lock().expect("k");
            *k = Knobs {
                discovery_issuer: None,
                token_endpoint: None,
                token_issuer: None,
                token_aud: CLIENT_ID.into(),
                nonce_override: None,
                exp_offset: 3600,
                sub: "sub-1".into(),
                email: "bob@acme.test".into(),
                email_verified: true,
                sign_with_stranger: false,
            };
            tweak(&mut k);
        }
        let r = sign_in(&w).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{name}: {:?}", r.json());
        assert_eq!(r.json()["error"], "oidc_failed", "{name}");
    }
    let linked: i64 =
        w.e.state
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM oidc_identities", [], |r| r.get(0))
            .expect("n");
    assert_eq!(linked, 0, "a rejected token links nothing");
}

#[tokio::test]
async fn state_is_single_use_bound_and_expires() {
    let w = world().await;
    let (code, state) = begin(&w).await;
    assert_eq!(
        callback(&w, &code, &"a".repeat(43)).await.status,
        StatusCode::BAD_REQUEST,
        "unknown state"
    );
    assert_eq!(
        callback(&w, &code, "short").await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(callback(&w, &code, &state).await.status, StatusCode::OK);
    assert_eq!(
        callback(&w, &code, &state).await.status,
        StatusCode::BAD_REQUEST,
        "replayed state"
    );
    // The stored value is a hash, and an expired state is refused.
    let (code, state) = begin(&w).await;
    let plain: i64 =
        w.e.state
            .db
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM oidc_pending WHERE state_hash = ?1",
                [&state],
                |r| r.get(0),
            )
            .expect("n");
    assert_eq!(plain, 0, "state is stored hashed");
    w.e.state
        .db
        .lock()
        .execute("UPDATE oidc_pending SET expires_at = 1", [])
        .expect("age");
    assert_eq!(
        callback(&w, &code, &state).await.status,
        StatusCode::BAD_REQUEST,
        "expired"
    );
    // A code without a started flow cannot be exchanged.
    let r = send(&w.e, req(Method::GET, "/auth/oidc/callback?code=x")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pkce_verifier_mismatch_is_refused_by_the_issuer() {
    let w = world().await;
    let (code, state) = begin(&w).await;
    // The attacker who steals the code but not the verifier: the mock holds a different challenge.
    w.mock
        .codes
        .lock()
        .expect("c")
        .get_mut(&code)
        .expect("code")
        .1 = B64.encode(Sha256::digest(b"another verifier"));
    let r = callback(&w, &code, &state).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn discovery_cannot_redirect_the_server_elsewhere() {
    let w = world().await;
    // Wrong issuer in the discovery document.
    w.mock.knobs.lock().expect("k").discovery_issuer = Some("https://evil.example".into());
    let r = send(&w.e, req(Method::GET, "/auth/oidc/acme/start")).await;
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(r.headers.get("location").is_none());

    // Endpoints on other hosts (metadata service, another loopback address) are refused before any request.
    for bad in [
        "http://169.254.169.254/token",
        "http://127.0.0.2:1/token",
        "https://evil.example/token",
    ] {
        let w = world().await;
        w.mock.knobs.lock().expect("k").token_endpoint = Some(bad.into());
        let r = send(&w.e, req(Method::GET, "/auth/oidc/acme/start")).await;
        assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR, "{bad}");
    }
    // No provider for the tenant, or no such tenant.
    let admin = make_tenant(&w.e, "globex", "root@globex.test").await;
    let _ = admin;
    assert_eq!(
        send(&w.e, req(Method::GET, "/auth/oidc/globex/start"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&w.e, req(Method::GET, "/auth/oidc/nope/start"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn jwks_is_cached_and_refreshed_once_on_an_unknown_key_id() {
    let w = world().await;
    assert_eq!(sign_in(&w).await.status, StatusCode::OK);
    assert_eq!(sign_in(&w).await.status, StatusCode::OK);
    assert_eq!(
        *w.mock.jwks_fetches.lock().expect("n"),
        1,
        "second sign-in used the cache"
    );

    // The issuer rotates its key: the unknown kid triggers one refresh and sign-in works.
    {
        let mut k = w.mock.keys.lock().expect("keys");
        *k = Keys {
            pair: new_pair(),
            kid: "k2".into(),
        };
    }
    assert_eq!(sign_in(&w).await.status, StatusCode::OK);
    assert_eq!(*w.mock.jwks_fetches.lock().expect("n"), 2);

    // A token with an unknown kid cannot make the server refetch again within the minimum interval.
    w.mock.keys.lock().expect("keys").kid = "k3".into();
    w.mock.knobs.lock().expect("k").sign_with_stranger = true;
    for _ in 0..3 {
        assert_eq!(sign_in(&w).await.status, StatusCode::UNAUTHORIZED);
    }
    assert!(
        *w.mock.jwks_fetches.lock().expect("n") <= 3,
        "at most one forced refresh"
    );
}

#[tokio::test]
async fn non_loopback_http_issuers_are_refused_at_startup() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut cfg = open365_server::config::Config::new(dir.path());
    cfg.oidc = vec![OidcProvider {
        tenant_slug: "acme".into(),
        issuer: "http://idp.example.com".into(),
        client_id: CLIENT_ID.into(),
        client_secret: None,
        redirect_uri: REDIRECT.into(),
        extra_hosts: vec![],
    }];
    let keys = open365_server::config::AuditKeys::single("k", vec![1u8; 32]).expect("keys");
    assert!(open365_server::AppState::new(cfg, keys).is_err());
}

// ---- login CSRF: the flow must finish in the browser that started it ----

#[tokio::test]
async fn callback_without_the_start_cookie_is_rejected_and_spends_the_state() {
    let w = world().await;
    let (code, state) = begin(&w).await;
    let r = callback_with(&w, &code, &state, None).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], "invalid_state");
    let jar = w.cookie.lock().expect("jar").clone();
    assert_eq!(
        callback_with(&w, &code, &state, Some(&jar)).await.status,
        StatusCode::BAD_REQUEST,
        "the state was spent"
    );
    assert_eq!(
        *w.mock.pkce_checks.lock().expect("n"),
        0,
        "no code was exchanged"
    );
}

#[tokio::test]
async fn a_cookie_from_another_browser_is_rejected_with_the_same_error() {
    let w = world().await;
    // The attacker starts a flow; the victim's browser has its own, different cookie.
    let (code, attacker_state) = begin(&w).await;
    let victim = send(&w.e, req(Method::GET, "/auth/oidc/acme/start")).await;
    let victim_cookie = cookie_of(&victim);
    let r = callback_with(&w, &code, &attacker_state, Some(&victim_cookie)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], "invalid_state");
    let forged = callback_with(
        &w,
        &code,
        &attacker_state,
        Some("o365_oidc=not-a-real-value"),
    )
    .await;
    assert_eq!(forged.status, StatusCode::BAD_REQUEST);
    assert_eq!(*w.mock.pkce_checks.lock().expect("n"), 0);
}

fn handoff_pair() -> (String, String) {
    let secret = open365_server::util::new_token();
    let hash = open365_server::util::sha256_hex(secret.as_bytes());
    (secret, hash)
}

async fn poll(w: &World, secret: &str) -> Resp {
    send(
        &w.e,
        req(Method::POST, "/auth/oidc/poll").json(json!({ "handoff": secret })),
    )
    .await
}

fn confirm_token(page: &Resp) -> String {
    let html = String::from_utf8_lossy(&page.body).into_owned();
    html.split("name=\"token\" value=\"")
        .nth(1)
        .expect("token field")
        .split('"')
        .next()
        .expect("value")
        .to_string()
}

#[tokio::test]
async fn a_handoff_does_not_complete_without_confirmation() {
    let w = world().await;
    let (secret, hash) = handoff_pair();
    let (code, state) = begin_at(&w, &format!("/auth/oidc/acme/start?handoff={hash}")).await;
    let page = callback(&w, &code, &state).await;
    assert_eq!(page.status, StatusCode::OK);
    let html = String::from_utf8_lossy(&page.body).into_owned();
    assert!(
        html.contains(&open365_server::util::handoff_user_code(&hash)),
        "page shows the app's code"
    );
    assert!(!html.contains(&secret), "the page never carries the secret");
    assert!(
        page.headers["content-security-policy"]
            .to_str()
            .expect("csp")
            .contains("allow-forms")
    );
    // Signed in at the issuer, but nothing can be redeemed yet.
    assert_eq!(poll(&w, &secret).await.status, StatusCode::ACCEPTED);
    let jar = w.cookie.lock().expect("jar").clone();
    let token = confirm_token(&page);
    // Confirming from another browser (no cookie, or someone else's) or with a wrong token does nothing.
    let post = |cookie: Option<&str>, tok: &str| {
        let mut r =
            req(Method::POST, "/auth/oidc/confirm").bytes(format!("token={tok}").as_bytes());
        if let Some(c) = cookie {
            r = r.hdr("cookie", c);
        }
        send(&w.e, r)
    };
    assert_eq!(post(None, &token).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        post(
            Some("o365_oidc=zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
            &token
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(Some(&jar), &"a".repeat(43)).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        poll(&w, &secret).await.status,
        StatusCode::ACCEPTED,
        "still unconfirmed"
    );
    // Happy path: the same browser confirms, then the app redeems once.
    assert_eq!(post(Some(&jar), &token).await.status, StatusCode::OK);
    let done = poll(&w, &secret).await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.json());
    let tok = done.json()["token"].as_str().expect("token").to_string();
    let me = send(&w.e, req(Method::GET, "/api/v1/me").auth(&tok))
        .await
        .json();
    assert_eq!(me["email"], "bob@acme.test");
    assert_eq!(
        poll(&w, &secret).await.status,
        StatusCode::ACCEPTED,
        "single use"
    );
}

#[tokio::test]
async fn a_handoff_started_by_someone_else_cannot_be_redeemed_with_another_secret() {
    let w = world().await;
    let (_secret, hash) = handoff_pair();
    let (code, state) = begin_at(&w, &format!("/auth/oidc/acme/start?handoff={hash}")).await;
    let _ = callback(&w, &code, &state).await;
    let (other, _) = handoff_pair();
    assert_eq!(poll(&w, &other).await.status, StatusCode::ACCEPTED);
    let bad = send(&w.e, req(Method::GET, "/auth/oidc/acme/start?handoff=zz")).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_client_signs_in_through_the_browser_handoff() {
    use open365_server::api::SharingClient;
    let w = world().await;
    let (base, handle) = serve_env(&w.e).await;
    let mut client = SharingClient::new(&base, &[]).expect("client");
    let signin = client.oidc_begin("acme");
    assert!(!format!("{:?}", signin.user_code).is_empty());
    // The person's browser opens the URL (the secret is not in it), signs in at the issuer and sees the code.
    let path = signin
        .browser_url
        .strip_prefix(&base)
        .expect("same origin")
        .to_string();
    let (code, state) = begin_at(&w, &path).await;
    let page = callback(&w, &code, &state).await;
    let html = String::from_utf8_lossy(&page.body).into_owned();
    assert!(
        html.contains(&signin.user_code),
        "browser shows the app's code"
    );
    assert!(
        client.oidc_poll(&signin).await.expect("poll").is_none(),
        "not confirmed yet"
    );
    let jar = w.cookie.lock().expect("jar").clone();
    let ok = send(
        &w.e,
        req(Method::POST, "/auth/oidc/confirm")
            .hdr("cookie", &jar)
            .bytes(format!("token={}", confirm_token(&page)).as_bytes()),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK);
    let session = client
        .oidc_wait(
            &signin,
            std::time::Duration::from_millis(50),
            std::time::Duration::from_secs(5),
        )
        .await
        .expect("session");
    assert!(session.expires_in > 0);
    assert_eq!(client.me().await.expect("me").email, "bob@acme.test");
    handle.shutdown();
}

#[tokio::test]
async fn with_tls_only_the_host_prefixed_cookie_counts() {
    let w = world_with(true).await;
    let (code, state) = begin(&w).await;
    let jar = w.cookie.lock().expect("jar").clone();
    assert!(jar.starts_with("__Host-o365_oidc="), "{jar}");
    // A sibling subdomain can set an unprefixed cookie with a value it chose; it must not be accepted.
    let value = jar.split_once('=').expect("pair").1;
    let planted = format!("o365_oidc={value}");
    let r = callback_with(&w, &code, &state, Some(&planted)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], "invalid_state");
    // Without TLS the prefixed name is the one that is ignored.
    let w = world_with(false).await;
    let (code, state) = begin(&w).await;
    let jar = w.cookie.lock().expect("jar").clone();
    assert!(jar.starts_with("o365_oidc="));
    let prefixed = format!("__Host-{jar}");
    assert_eq!(
        callback_with(&w, &code, &state, Some(&prefixed))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let (code, state) = begin(&w).await;
    assert_eq!(callback(&w, &code, &state).await.status, StatusCode::OK);
}
