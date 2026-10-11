//! OpenID Connect sign-in: Authorization Code with PKCE (S256), behind the `oidc` cargo feature.
//!
//! Security rules, in the order they matter:
//! - The issuer URL comes only from configuration, never from a request. It must be `https`, except for a loopback
//!   host (a test issuer). Discovery must report exactly that issuer, and every endpoint it names must be `https` on
//!   the issuer's host or on a host the operator listed. All fetches go through [`crate::client::Client`], which
//!   refuses private, loopback (unless the issuer itself is loopback) and link-local addresses and never follows
//!   redirects.
//! - `state` is random, single use and expires in ten minutes; only its hash is stored. The `nonce` and the PKCE
//!   verifier are bound to it.
//! - The ID token must be signed with RS256 or ES256 by a key in the issuer's JWKS (cached; refreshed at most every
//!   30 s on an unknown `kid`), and must carry the configured issuer, our client id as audience, a matching nonce
//!   and a valid time window. `none` and HMAC algorithms are never accepted.
//! - There is no just-in-time account creation. A user signs in if the `(issuer, sub)` pair is already linked, or
//!   if the IdP reports a verified email that matches an existing account in the tenant (which is then linked).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::header::{LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hyper::Method;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::audit;
use crate::client::{Client, is_loopback_host, pct_encode};
use crate::config::OidcProvider;
use crate::error::{ApiError, ApiResult};
use crate::util::{ct_eq, handoff_user_code, new_token, now, sha256_hex, token_shape_ok};
use crate::{AppState, ClientIp};

const DISCOVERY_TTL: Duration = Duration::from_secs(3600);
const JWKS_TTL: Duration = Duration::from_secs(600);
const JWKS_MIN_REFETCH: Duration = Duration::from_secs(30);
const PENDING_TTL_SECS: i64 = 600;
const PENDING_MAX: i64 = 10_000;
const MAX_KEYS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidcError {
    Config(&'static str),
    Fetch(&'static str),
    Token(&'static str),
}

#[derive(Debug, Clone)]
pub struct Discovery {
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

#[derive(Debug, Clone)]
pub enum KeyKind {
    Rsa { n: Vec<u8>, e: Vec<u8> },
    Ec { point: Vec<u8> },
}

#[derive(Debug, Clone)]
pub struct Jwk {
    pub kid: Option<String>,
    pub kind: KeyKind,
}

#[derive(Debug, Default)]
struct JwksCache {
    keys: Vec<Jwk>,
    fetched: Option<Instant>,
    last_forced: Option<Instant>,
}

#[derive(Debug)]
struct Provider {
    cfg: OidcProvider,
    client: Client,
    loopback_issuer: bool,
    disc: Mutex<Option<(Instant, Discovery)>>,
    jwks: Mutex<JwksCache>,
}

#[derive(Debug, Default)]
pub struct Registry {
    by_tenant: HashMap<String, Provider>,
}

fn parse_uri(s: &str) -> Result<hyper::Uri, OidcError> {
    s.parse().map_err(|_| OidcError::Config("unparsable url"))
}

/// `https`, or `http` to a loopback host; no credentials, query or fragment.
fn check_issuer(issuer: &str) -> Result<bool, OidcError> {
    let uri = parse_uri(issuer)?;
    let host = crate::client::host_of(&uri).ok_or(OidcError::Config("issuer has no host"))?;
    let loopback = is_loopback_host(&host);
    match uri.scheme_str() {
        Some("https") => {}
        Some("http") if loopback => {}
        _ => {
            return Err(OidcError::Config(
                "issuer must be https (http only for loopback)",
            ));
        }
    }
    if uri.authority().is_some_and(|a| a.as_str().contains('@'))
        || uri.query().is_some()
        || issuer.contains('#')
    {
        return Err(OidcError::Config(
            "issuer must not carry credentials, query or fragment",
        ));
    }
    Ok(loopback)
}

impl Registry {
    pub fn new(providers: &[OidcProvider]) -> Result<Self, String> {
        let mut by_tenant = HashMap::new();
        for p in providers {
            let loopback_issuer = check_issuer(&p.issuer).map_err(|e| format!("{e:?}"))?;
            if p.client_id.is_empty() {
                return Err("oidc client id is empty".into());
            }
            let redirect = parse_uri(&p.redirect_uri).map_err(|e| format!("{e:?}"))?;
            let rhost = crate::client::host_of(&redirect).unwrap_or_default();
            if !(redirect.scheme_str() == Some("https")
                || (redirect.scheme_str() == Some("http") && is_loopback_host(&rhost)))
            {
                return Err("oidc redirect uri must be https (http only for loopback)".into());
            }
            let client = Client::new(loopback_issuer, &[])?;
            by_tenant.insert(
                p.tenant_slug.to_lowercase(),
                Provider {
                    cfg: p.clone(),
                    client,
                    loopback_issuer,
                    disc: Mutex::new(None),
                    jwks: Mutex::new(JwksCache::default()),
                },
            );
        }
        Ok(Self { by_tenant })
    }

    pub fn is_empty(&self) -> bool {
        self.by_tenant.is_empty()
    }
}

impl Provider {
    /// An endpoint named by discovery is acceptable only on the issuer's own host or an operator-listed host.
    fn check_endpoint(&self, url: &str) -> Result<(), OidcError> {
        let uri = parse_uri(url)?;
        let host = crate::client::host_of(&uri).ok_or(OidcError::Config("endpoint has no host"))?;
        let scheme_ok = uri.scheme_str() == Some("https")
            || (uri.scheme_str() == Some("http")
                && self.loopback_issuer
                && is_loopback_host(&host));
        let issuer_host = crate::client::host_of(&parse_uri(&self.cfg.issuer)?).unwrap_or_default();
        let host_ok = host.eq_ignore_ascii_case(&issuer_host)
            || self
                .cfg
                .extra_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(&host));
        let creds = uri.authority().is_some_and(|a| a.as_str().contains('@'));
        if scheme_ok && host_ok && !creds {
            Ok(())
        } else {
            Err(OidcError::Fetch(
                "discovery named an endpoint outside the allowed hosts",
            ))
        }
    }

    async fn discovery(&self) -> Result<Discovery, OidcError> {
        let mut slot = self.disc.lock().await;
        if let Some((at, d)) = slot.as_ref()
            && at.elapsed() < DISCOVERY_TTL
        {
            return Ok(d.clone());
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.cfg.issuer.trim_end_matches('/')
        );
        let res = self
            .client
            .get(&url)
            .await
            .map_err(|_| OidcError::Fetch("discovery request failed"))?;
        if res.status != 200 {
            return Err(OidcError::Fetch("discovery status"));
        }
        let j = res.json();
        // The document must describe exactly the issuer we were configured with.
        if j["issuer"].as_str() != Some(self.cfg.issuer.as_str()) {
            return Err(OidcError::Fetch("discovery issuer mismatch"));
        }
        let get = |k: &str| -> Result<String, OidcError> {
            let v = j[k]
                .as_str()
                .ok_or(OidcError::Fetch("discovery field missing"))?;
            self.check_endpoint(v)?;
            Ok(v.to_string())
        };
        let d = Discovery {
            authorization_endpoint: get("authorization_endpoint")?,
            token_endpoint: get("token_endpoint")?,
            jwks_uri: get("jwks_uri")?,
        };
        *slot = Some((Instant::now(), d.clone()));
        Ok(d)
    }

    /// Keys that may have signed a token with this `kid`. Cached; an unknown `kid` forces a refresh, but not more
    /// often than every 30 seconds.
    async fn keys_for(&self, kid: Option<&str>) -> Result<Vec<Jwk>, OidcError> {
        let disc = self.discovery().await?;
        let mut cache = self.jwks.lock().await;
        let pick = |keys: &[Jwk]| -> Vec<Jwk> {
            keys.iter()
                .filter(|k| kid.is_none() || k.kid.as_deref() == kid)
                .cloned()
                .collect()
        };
        let fresh = cache.fetched.is_some_and(|t| t.elapsed() < JWKS_TTL);
        if fresh {
            let hit = pick(&cache.keys);
            if !hit.is_empty() {
                return Ok(hit);
            }
            if cache
                .last_forced
                .is_some_and(|t| t.elapsed() < JWKS_MIN_REFETCH)
            {
                return Ok(vec![]);
            }
            cache.last_forced = Some(Instant::now());
        }
        let res = self
            .client
            .get(&disc.jwks_uri)
            .await
            .map_err(|_| OidcError::Fetch("jwks request failed"))?;
        if res.status != 200 {
            return Err(OidcError::Fetch("jwks status"));
        }
        cache.keys = parse_jwks(&res.json());
        cache.fetched = Some(Instant::now());
        Ok(pick(&cache.keys))
    }
}

fn b64(v: &Value) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(v.as_str()?).ok()
}

/// Reads the usable signature keys out of a JWKS document (RSA and P-256), ignoring everything else.
pub fn parse_jwks(j: &Value) -> Vec<Jwk> {
    let mut out = vec![];
    for k in j["keys"].as_array().into_iter().flatten().take(MAX_KEYS) {
        if k["use"].as_str().is_some_and(|u| u != "sig") {
            continue;
        }
        let kid = k["kid"].as_str().map(str::to_string);
        match k["kty"].as_str() {
            Some("RSA") => {
                if let (Some(n), Some(e)) = (b64(&k["n"]), b64(&k["e"])) {
                    out.push(Jwk {
                        kid,
                        kind: KeyKind::Rsa { n, e },
                    });
                }
            }
            Some("EC") if k["crv"].as_str() == Some("P-256") => {
                if let (Some(x), Some(y)) = (b64(&k["x"]), b64(&k["y"]))
                    && x.len() == 32
                    && y.len() == 32
                {
                    let mut point = vec![4u8];
                    point.extend(x);
                    point.extend(y);
                    out.push(Jwk {
                        kid,
                        kind: KeyKind::Ec { point },
                    });
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Expect<'a> {
    pub issuer: &'a str,
    pub client_id: &'a str,
    pub nonce: &'a str,
    pub now: i64,
}

fn decode_part(s: &str) -> Result<Value, OidcError> {
    let raw = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| OidcError::Token("not base64url"))?;
    serde_json::from_slice(&raw).map_err(|_| OidcError::Token("not json"))
}

/// The `kid` in a token's header, without trusting anything else in it.
pub fn peek_kid(token: &str) -> Option<String> {
    let h = decode_part(token.split('.').next()?).ok()?;
    h["kid"].as_str().map(str::to_string)
}

fn signature_ok(alg: &str, keys: &[Jwk], msg: &[u8], sig: &[u8]) -> bool {
    keys.iter().any(|k| match (alg, &k.kind) {
        ("RS256", KeyKind::Rsa { n, e }) => ring::signature::RsaPublicKeyComponents {
            n: n.as_slice(),
            e: e.as_slice(),
        }
        .verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, msg, sig)
        .is_ok(),
        ("ES256", KeyKind::Ec { point }) => ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_FIXED,
            point.as_slice(),
        )
        .verify(msg, sig)
        .is_ok(),
        _ => false,
    })
}

/// Validates an ID token: algorithm allow-list, signature, issuer, audience, nonce and time window.
pub fn verify_id_token(
    token: &str,
    keys: &[Jwk],
    expect: &Expect<'_>,
) -> Result<Claims, OidcError> {
    if token.len() > 16 * 1024 {
        return Err(OidcError::Token("too long"));
    }
    let parts: Vec<&str> = token.split('.').collect();
    let [h, p, s] = parts.as_slice() else {
        return Err(OidcError::Token("not a compact jws"));
    };
    let header = decode_part(h)?;
    let alg = header["alg"].as_str().unwrap_or_default();
    if !matches!(alg, "RS256" | "ES256") {
        return Err(OidcError::Token("algorithm not allowed"));
    }
    let sig = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| OidcError::Token("bad signature encoding"))?;
    if !signature_ok(alg, keys, format!("{h}.{p}").as_bytes(), &sig) {
        return Err(OidcError::Token("bad signature"));
    }
    let c = decode_part(p)?;
    if c["iss"].as_str() != Some(expect.issuer) {
        return Err(OidcError::Token("wrong issuer"));
    }
    let aud_ok = match &c["aud"] {
        Value::String(a) => a == expect.client_id,
        Value::Array(a) => {
            a.iter().any(|v| v.as_str() == Some(expect.client_id))
                && (a.len() == 1 || c["azp"].as_str() == Some(expect.client_id))
        }
        _ => false,
    };
    if !aud_ok {
        return Err(OidcError::Token("wrong audience"));
    }
    let exp = c["exp"].as_i64().ok_or(OidcError::Token("no exp"))?;
    if exp + 60 <= expect.now {
        return Err(OidcError::Token("expired"));
    }
    if c["iat"].as_i64().is_some_and(|i| i > expect.now + 300)
        || c["nbf"].as_i64().is_some_and(|n| n > expect.now + 60)
    {
        return Err(OidcError::Token("not yet valid"));
    }
    if !c["nonce"].as_str().is_some_and(|n| ct_eq(n, expect.nonce)) {
        return Err(OidcError::Token("wrong nonce"));
    }
    let subject = c["sub"].as_str().unwrap_or_default();
    if subject.is_empty() || subject.len() > 255 {
        return Err(OidcError::Token("bad subject"));
    }
    let email_verified = match &c["email_verified"] {
        Value::Bool(b) => *b,
        Value::String(s) => s == "true",
        _ => false,
    };
    Ok(Claims {
        subject: subject.to_string(),
        email: c["email"]
            .as_str()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| e.len() <= 254),
        email_verified,
    })
}

pub fn pkce_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[derive(Deserialize, Debug)]
pub struct StartQ {
    #[serde(default)]
    bearer: bool,
    /// SHA-256 (64 hex digits) of a secret the native app keeps; see [`poll`].
    handoff: Option<String>,
}

fn provider<'a>(st: &'a AppState, slug: &str) -> ApiResult<&'a Provider> {
    st.oidc
        .by_tenant
        .get(&slug.to_lowercase())
        .ok_or(ApiError::NotFound)
}

/// Begins sign-in: stores state, nonce and verifier, and redirects the browser to the issuer.
pub async fn start(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(slug): Path<String>,
    Query(q): Query<StartQ>,
) -> ApiResult<Response> {
    if slug.len() > 64 {
        return Err(ApiError::NotFound);
    }
    let prov = provider(&st, &slug)?;
    if let Some(h) = &q.handoff
        && !crate::util::is_hex_id(h, 64)
    {
        return Err(ApiError::BadRequest("invalid_handoff"));
    }
    st.limiter
        .hit(
            &format!("loginip:{}", crate::ratelimit::ip_key(&ip)),
            st.cfg.login_ip_rate,
        )
        .await
        .into_result()?;
    let disc = prov.discovery().await.map_err(|e| {
        eprintln!("oidc discovery failed: {e:?}");
        ApiError::Internal
    })?;
    let (state, nonce, verifier) = (new_token(), new_token(), new_token());
    let handoff = q.handoff.clone();
    let browser = new_token();
    let browser_hash = sha256_hex(browser.as_bytes());
    let (sh, n2, v2, s2) = (
        sha256_hex(state.as_bytes()),
        nonce.clone(),
        verifier.clone(),
        prov.cfg.tenant_slug.to_lowercase(),
    );
    st.run(move |_, c| {
        let tenant: String = c
            .query_row("SELECT id FROM tenants WHERE slug = ?1", [&s2], |r| r.get(0))
            .optional()?
            .ok_or(ApiError::NotFound)?;
        c.execute("DELETE FROM oidc_pending WHERE expires_at <= ?1", [now()])?;
        let open: i64 = c.query_row("SELECT COUNT(*) FROM oidc_pending", [], |r| r.get(0))?;
        if open >= PENDING_MAX {
            return Err(ApiError::RateLimited(60));
        }
        c.execute(
            "INSERT INTO oidc_pending (state_hash, tenant_id, nonce, code_verifier, bearer, expires_at, handoff_hash, browser_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![sh, tenant, n2, v2, q.bearer, now() + PENDING_TTL_SECS, handoff, browser_hash],
        )?;
        Ok(())
    })
    .await?;
    let sep = if disc.authorization_endpoint.contains('?') {
        '&'
    } else {
        '?'
    };
    let url = format!(
        "{}{sep}response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
        disc.authorization_endpoint,
        pct_encode(&prov.cfg.client_id),
        pct_encode(&prov.cfg.redirect_uri),
        pct_encode("openid email"),
        state,
        nonce,
        pkce_challenge(&verifier),
    );
    let loc = axum::http::HeaderValue::from_str(&url).map_err(|_| ApiError::Internal)?;
    let cookie = format!(
        "{}={browser}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={PENDING_TTL_SECS}",
        cookie_name(&st)
    );
    let cookie = axum::http::HeaderValue::from_str(&cookie).map_err(|_| ApiError::Internal)?;
    Ok((StatusCode::FOUND, [(LOCATION, loc), (SET_COOKIE, cookie)]).into_response())
}

/// `__Host-` (no Domain, Secure, Path=/) when this deployment serves or fronts TLS.
fn cookie_name(st: &AppState) -> &'static str {
    if st.cfg.hsts {
        "__Host-o365_oidc"
    } else {
        "o365_oidc"
    }
}

/// SHA-256 of the browser-binding cookie, if the request carries a well-formed one under the name this deployment
/// issues. The other name is ignored: a sibling subdomain could set an unprefixed cookie and defeat `__Host-`.
fn browser_cookie_hash(st: &AppState, headers: &HeaderMap) -> Option<String> {
    let name = cookie_name(st);
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, v)| *k == name && token_shape_ok(v))
        .map(|(_, v)| sha256_hex(v.as_bytes()))
}

fn clear_cookie(st: &AppState, mut res: Response) -> Response {
    let c = format!(
        "{}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0",
        cookie_name(st)
    );
    if let Ok(v) = axum::http::HeaderValue::from_str(&c) {
        res.headers_mut().append(SET_COOKIE, v);
    }
    res
}

fn page(body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Open365 sign-in</title></head><body>{body}</body></html>"
    );
    let mut res = (StatusCode::OK, html).into_response();
    let h = res.headers_mut();
    h.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    // Forms need `allow-forms`, and the cookie is sent only from the page's own origin.
    h.insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        axum::http::HeaderValue::from_static(
            "default-src 'none'; frame-ancestors 'none'; form-action 'self'; sandbox allow-forms allow-same-origin",
        ),
    );
    res
}

#[derive(Deserialize, Debug)]
pub struct CallbackQ {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

struct Pending {
    tenant_id: String,
    slug: String,
    nonce: String,
    verifier: String,
    bearer: bool,
    handoff: Option<String>,
    browser_hash: Option<String>,
}

/// Completes sign-in: consumes the state, exchanges the code (with the PKCE verifier), validates the ID token and
/// maps it to an existing account.
pub async fn callback(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Query(q): Query<CallbackQ>,
) -> ApiResult<Response> {
    st.limiter
        .hit(
            &format!("loginip:{}", crate::ratelimit::ip_key(&ip)),
            st.cfg.login_ip_rate,
        )
        .await
        .into_result()?;
    let state = q
        .state
        .filter(|s| token_shape_ok(s))
        .ok_or(ApiError::BadRequest("invalid_state"))?;
    let sh = sha256_hex(state.as_bytes());
    // Single use: the row is deleted as it is read.
    let pending: Option<Pending> = st
        .run(move |_, c| {
            Ok(c.query_row(
                "DELETE FROM oidc_pending WHERE state_hash = ?1 AND expires_at > ?2
                 RETURNING tenant_id, nonce, code_verifier, bearer, handoff_hash, browser_hash",
                params![sh, now()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, bool>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?)
        })
        .await?
        .map(
            |(tenant_id, nonce, verifier, bearer, handoff, browser_hash)| Pending {
                tenant_id,
                slug: String::new(),
                nonce,
                verifier,
                bearer,
                handoff,
                browser_hash,
            },
        );
    let Some(mut pending) = pending else {
        return Err(ApiError::BadRequest("invalid_state"));
    };
    // The flow must finish in the browser that started it. A missing or different cookie is refused exactly like an
    // unknown state (the state is already spent).
    let bound = match (&pending.browser_hash, browser_cookie_hash(&st, &headers)) {
        (Some(want), Some(got)) => ct_eq(want, &got),
        _ => false,
    };
    if !bound {
        return Err(ApiError::BadRequest("invalid_state"));
    }
    let tid = pending.tenant_id.clone();
    pending.slug = st
        .run(move |_, c| {
            Ok(
                c.query_row("SELECT slug FROM tenants WHERE id = ?1", [&tid], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?
                .unwrap_or_default(),
            )
        })
        .await?;
    let prov = provider(&st, &pending.slug)?;
    let outcome = exchange_and_verify(prov, &q.code, q.error.as_deref(), &pending).await;
    let claims = match outcome {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oidc sign-in failed: {e:?}");
            crate::auth::login_failed(
                &st,
                Some(pending.tenant_id),
                "oidc",
                "oidc".into(),
                pending.slug,
                ip,
            )
            .await?;
            return Err(ApiError::Unauthorized("oidc_failed"));
        }
    };
    let (tenant_id, issuer, bearer) = (
        pending.tenant_id.clone(),
        prov.cfg.issuer.clone(),
        pending.bearer,
    );
    let t2 = tenant_id.clone();
    let user: Option<String> = st
        .run(move |inner, c| {
            let tx = c.transaction()?;
            let linked: Option<String> = tx
                .query_row(
                    "SELECT i.user_id FROM oidc_identities i JOIN users u ON u.id = i.user_id AND u.tenant_id = i.tenant_id
                     WHERE i.tenant_id = ?1 AND i.issuer = ?2 AND i.subject = ?3 AND u.disabled = 0",
                    params![t2, issuer, claims.subject],
                    |r| r.get(0),
                )
                .optional()?;
            if linked.is_some() {
                return Ok(linked);
            }
            // First sign-in: link to the account whose email the IdP vouches for. Never create accounts here.
            let Some(email) = claims.email.as_ref().filter(|_| claims.email_verified) else {
                return Ok(None);
            };
            let uid: Option<String> = tx
                .query_row(
                    "SELECT id FROM users WHERE tenant_id = ?1 AND email = ?2 AND disabled = 0",
                    params![t2, email],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(uid) = &uid {
                tx.execute(
                    "INSERT INTO oidc_identities (tenant_id, issuer, subject, user_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![t2, issuer, claims.subject, uid, now()],
                )?;
                audit::append(&tx, &inner.audit, &t2, uid, "oidc.link", uid, &issuer)?;
            }
            tx.commit()?;
            Ok(uid)
        })
        .await?;
    let Some(user_id) = user else {
        crate::auth::login_failed(
            &st,
            Some(tenant_id),
            "oidc-no-account",
            "oidc".into(),
            pending.slug,
            ip,
        )
        .await?;
        return Err(ApiError::Forbidden("oidc_no_account"));
    };
    if let Some(h) = pending.handoff {
        // A native app is waiting. Nothing can be redeemed until the person confirms, in this browser, the code that
        // the app shows: a flow started by someone else will show a code that is not on the person's own screen.
        let browser_hash = pending.browser_hash.clone().unwrap_or_default();
        let confirm = new_token();
        let ch = sha256_hex(confirm.as_bytes());
        let code = handoff_user_code(&h);
        st.run(move |_, c| {
            c.execute("DELETE FROM oidc_handoff WHERE expires_at <= ?1", [now()])?;
            let open: i64 = c.query_row("SELECT COUNT(*) FROM oidc_handoff", [], |r| r.get(0))?;
            if open >= PENDING_MAX {
                return Err(ApiError::RateLimited(60));
            }
            c.execute(
                "INSERT OR REPLACE INTO oidc_handoff (handoff_hash, tenant_id, user_id, browser_hash, confirm_hash, confirmed, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                params![h, tenant_id, user_id, browser_hash, ch, now() + HANDOFF_TTL_SECS],
            )?;
            Ok(())
        })
        .await?;
        let body = format!(
            "<h1>Sign in to the Open365 app?</h1><p>Your app should be showing this code:</p><p><strong>{code}</strong></p>\
             <p>Confirm only if the codes match and you just asked the app to sign in.</p>\
             <form method=\"post\" action=\"/auth/oidc/confirm\"><input type=\"hidden\" name=\"token\" value=\"{confirm}\">\
             <button type=\"submit\">The codes match, sign in</button></form>"
        );
        return Ok(page(&body));
    }
    let res = crate::auth::issue_session(&st, tenant_id, user_id, "oidc", bearer).await?;
    Ok(clear_cookie(&st, res))
}

const HANDOFF_TTL_SECS: i64 = 120;

#[derive(Deserialize)]
// No Debug: carries the hand-off secret.
#[allow(missing_debug_implementations)]
pub struct PollReq {
    handoff: String,
}

/// The native app redeems its hand-off secret. `202` until the browser has finished signing in; then one `200` with
/// a session token (single use, valid for two minutes after the browser step).
pub async fn poll(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    axum::Json(req): axum::Json<PollReq>,
) -> ApiResult<Response> {
    if !token_shape_ok(&req.handoff) {
        return Err(ApiError::BadRequest("invalid_handoff"));
    }
    st.limiter
        .hit(
            &format!("oidcpoll:{}", crate::ratelimit::ip_key(&ip)),
            st.cfg.login_ip_rate,
        )
        .await
        .into_result()?;
    let h = sha256_hex(req.handoff.as_bytes());
    let claim: Option<(String, String)> = st
        .run(move |_, c| {
            Ok(c.query_row(
                "DELETE FROM oidc_handoff WHERE handoff_hash = ?1 AND confirmed = 1 AND expires_at > ?2 RETURNING tenant_id, user_id",
                params![h, now()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
        })
        .await?;
    match claim {
        Some((tenant_id, user_id)) => {
            crate::auth::issue_session(&st, tenant_id, user_id, "oidc", true).await
        }
        None => Ok((
            StatusCode::ACCEPTED,
            axum::Json(serde_json::json!({ "status": "pending" })),
        )
            .into_response()),
    }
}

async fn exchange_and_verify(
    prov: &Provider,
    code: &Option<String>,
    idp_error: Option<&str>,
    pending: &Pending,
) -> Result<Claims, OidcError> {
    if idp_error.is_some() {
        return Err(OidcError::Token("idp reported an error"));
    }
    let code = code
        .as_deref()
        .filter(|c| !c.is_empty() && c.len() <= 2048)
        .ok_or(OidcError::Token("no code"))?;
    let disc = prov.discovery().await?;
    let mut form = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        pct_encode(code),
        pct_encode(&prov.cfg.redirect_uri),
        pct_encode(&prov.cfg.client_id),
        pending.verifier,
    );
    if let Some(secret) = &prov.cfg.client_secret {
        form.push_str(&format!("&client_secret={}", pct_encode(secret)));
    }
    let res = prov
        .client
        .request(
            Method::POST,
            &disc.token_endpoint,
            &[
                ("content-type", "application/x-www-form-urlencoded"),
                ("accept", "application/json"),
            ],
            form.into_bytes(),
        )
        .await
        .map_err(|_| OidcError::Fetch("token request failed"))?;
    if res.status != 200 {
        return Err(OidcError::Fetch("token endpoint refused the code"));
    }
    let id_token = res.json()["id_token"]
        .as_str()
        .map(str::to_string)
        .ok_or(OidcError::Token("no id_token"))?;
    let keys = prov.keys_for(peek_kid(&id_token).as_deref()).await?;
    verify_id_token(
        &id_token,
        &keys,
        &Expect {
            issuer: &prov.cfg.issuer,
            client_id: &prov.cfg.client_id,
            nonce: &pending.nonce,
            now: now(),
        },
    )
}

/// The person confirms the hand-off in the browser that signed in (same cookie, and the one-time token from the page).
pub async fn confirm(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    st.limiter
        .hit(
            &format!("loginip:{}", crate::ratelimit::ip_key(&ip)),
            st.cfg.login_ip_rate,
        )
        .await
        .into_result()?;
    let token = std::str::from_utf8(&body)
        .ok()
        .and_then(|b| b.split('&').find_map(|kv| kv.strip_prefix("token=")))
        .filter(|t| token_shape_ok(t))
        .ok_or(ApiError::BadRequest("invalid_state"))?;
    let ch = sha256_hex(token.as_bytes());
    let browser =
        browser_cookie_hash(&st, &headers).ok_or(ApiError::BadRequest("invalid_state"))?;
    st.run(move |_, c| {
        let row: Option<(String, String)> = c
            .query_row(
                "SELECT handoff_hash, browser_hash FROM oidc_handoff WHERE confirm_hash = ?1 AND confirmed = 0 AND expires_at > ?2",
                params![ch, now()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((h, want)) = row.filter(|(_, want)| ct_eq(want, &browser)) else {
            return Err(ApiError::BadRequest("invalid_state"));
        };
        c.execute(
            "UPDATE oidc_handoff SET confirmed = 1 WHERE handoff_hash = ?1 AND browser_hash = ?2",
            params![h, want],
        )?;
        Ok(())
    })
    .await?;
    Ok(clear_cookie(
        &st,
        page("<h1>Signed in</h1><p>You can close this window and return to the Open365 app.</p>"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(issuer: &str) -> OidcProvider {
        OidcProvider {
            tenant_slug: "acme".into(),
            issuer: issuer.into(),
            client_id: "client".into(),
            client_secret: None,
            redirect_uri: "https://app.example/cb".into(),
            extra_hosts: vec![],
        }
    }

    #[test]
    fn issuer_rules() {
        assert!(Registry::new(&[cfg("https://idp.example.com")]).is_ok());
        assert!(Registry::new(&[cfg("http://127.0.0.1:9000")]).is_ok());
        assert!(Registry::new(&[cfg("http://localhost:9000/realm")]).is_ok());
        for bad in [
            "http://idp.example.com",
            "http://10.0.0.1",
            "https://user:pw@idp.example.com",
            "https://idp.example.com/?x=1",
            "ftp://idp.example.com",
            "idp.example.com",
        ] {
            assert!(Registry::new(&[cfg(bad)]).is_err(), "{bad}");
        }
    }

    #[test]
    fn pkce_matches_rfc7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn algorithms_none_and_hmac_are_rejected() {
        let hdr = |alg: &str| URL_SAFE_NO_PAD.encode(format!("{{\"alg\":\"{alg}\"}}"));
        let body = URL_SAFE_NO_PAD.encode("{}");
        let e = Expect {
            issuer: "i",
            client_id: "c",
            nonce: "n",
            now: 0,
        };
        for alg in ["none", "HS256", "RS512", ""] {
            let t = format!("{}.{body}.AAAA", hdr(alg));
            assert_eq!(
                verify_id_token(&t, &[], &e),
                Err(OidcError::Token("algorithm not allowed")),
                "{alg}"
            );
        }
        assert!(verify_id_token("a.b", &[], &e).is_err());
        assert!(verify_id_token("", &[], &e).is_err());
    }

    // RS256 test vector: a throwaway 2048-bit key made for this test (the private key was discarded); the token was
    // signed with RSASSA-PKCS1-v1_5 and SHA-256 by an independent implementation (Python `cryptography`).
    const RS_N: &str = "ugzFk5mtkrGvt3Jdb53g-qGP1RcCslwvVla1A0bMTJr4-ybaGA9W2b9jHFJCY5Oh03k7XAPzgY0iIW1Xj_wvRsJp9-WRM8FT7ZtyWktLs84lldmnVbRytkgI-6UPipMqrO1QB1GQ28g_kfwTxz1sThk5Y04N7C-G6aZpzCuEzJ6ma-gFPlz0ISv5XDyZHztefQ31MDVp0thx9JOopX6khBTHLARdIrc6QM2l5g5Y0KGg5WAnl3ofGGDCroIQTn9AVgacSi_bsngyLuIl6iAI9sj35K_8VXrrEbrMzPvdmmHTk-G_3yJYIJpa8QkZhN4Ia3GohtQk3SzzxR626dgdLQ";
    const RS_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6InJzLXRlc3QtMSJ9.eyJpc3MiOiJodHRwczovL2lkcC5leGFtcGxlLnRlc3QiLCJzdWIiOiJzdWJqZWN0LXJzMjU2IiwiYXVkIjoiY2xpZW50LXJzMjU2IiwiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3MDAwMDAwMDAsIm5vbmNlIjoibm9uY2UtcnMyNTYiLCJlbWFpbCI6IkFsaWNlQEV4YW1wbGUudGVzdCIsImVtYWlsX3ZlcmlmaWVkIjp0cnVlfQ.FfWc8hv4Eoql0R63cR2_YPPTb4FM-zseyIIQQbkGYvd5BKP6uPRgroxyxveJZhBBIyvoUUn0Fn18-fl4fcgzQAzojGma61hWnRh0tCgKkYH6PmuW1Ou_SxKgpT6DEmL1TSBe2mYPSaqxADmmIaG4puoyN_cK4UVWnwh7gjXCPEmWsJwXcfnmp0dYWmfts08MtdHhJ__dJNpr85YFz_aZbSYp5Woj9SGR94RUdy8mf7KHEqRpgghTNEqkc2GNYkLlzeyuK1yItm_1Yod97iZ-l3na67ahyMKxLgcPewKuuBYw2eNNUiCgVlTZX0j2A_Z2UobmG2eylFhL7J3Ht6h4zQ";

    fn rs_keys(n: &str) -> Vec<Jwk> {
        parse_jwks(&serde_json::json!({ "keys": [
            { "kty": "RSA", "use": "sig", "kid": "rs-test-1", "alg": "RS256", "n": n, "e": "AQAB" }
        ]}))
    }

    fn rs_expect(now: i64) -> Expect<'static> {
        Expect {
            issuer: "https://idp.example.test",
            client_id: "client-rs256",
            nonce: "nonce-rs256",
            now,
        }
    }

    #[test]
    fn rs256_known_good_token_verifies() {
        let keys = rs_keys(RS_N);
        assert_eq!(keys.len(), 1);
        assert_eq!(peek_kid(RS_TOKEN).as_deref(), Some("rs-test-1"));
        let c = verify_id_token(RS_TOKEN, &keys, &rs_expect(1_800_000_000)).expect("valid");
        assert_eq!(c.subject, "subject-rs256");
        assert_eq!(c.email.as_deref(), Some("alice@example.test"));
        assert!(c.email_verified);
    }

    #[test]
    fn rs256_rejects_forgeries_and_wrong_context() {
        let keys = rs_keys(RS_N);
        let now = 1_800_000_000;
        // Flipped signature bit, payload swapped under the old signature, a different key, and an HS256 header.
        let mut bad_sig = RS_TOKEN.to_string();
        bad_sig.pop();
        bad_sig.push(if RS_TOKEN.ends_with('A') { 'B' } else { 'A' });
        assert!(verify_id_token(&bad_sig, &keys, &rs_expect(now)).is_err());
        let parts: Vec<&str> = RS_TOKEN.split('.').collect();
        let other = URL_SAFE_NO_PAD.encode(br#"{"iss":"https://idp.example.test","sub":"evil","aud":"client-rs256","exp":4102444800,"nonce":"nonce-rs256"}"#);
        let swapped = format!("{}.{other}.{}", parts[0], parts[2]);
        assert!(verify_id_token(&swapped, &keys, &rs_expect(now)).is_err());
        let mut n = URL_SAFE_NO_PAD.decode(RS_N).expect("n");
        n[10] ^= 1;
        assert!(
            verify_id_token(
                RS_TOKEN,
                &rs_keys(&URL_SAFE_NO_PAD.encode(n)),
                &rs_expect(now)
            )
            .is_err()
        );
        let hs = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let hs_token = format!("{hs}.{}.{}", parts[1], parts[2]);
        assert!(verify_id_token(&hs_token, &keys, &rs_expect(now)).is_err());
        // Right signature, wrong context.
        let mut e = rs_expect(now);
        e.nonce = "other";
        assert!(verify_id_token(RS_TOKEN, &keys, &e).is_err());
        let mut e = rs_expect(now);
        e.client_id = "other";
        assert!(verify_id_token(RS_TOKEN, &keys, &e).is_err());
        assert!(
            verify_id_token(RS_TOKEN, &keys, &rs_expect(4_102_444_800 + 61)).is_err(),
            "expired"
        );
    }
}
