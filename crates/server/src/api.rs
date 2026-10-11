//! Typed Rust client for the sharing API: what the desktop Share dialog calls.
//!
//! One [`SharingClient`] per server and signed-in user. It covers sign-in (password with optional TOTP or recovery
//! code, and OpenID Connect through a browser with a device-style hand-off), uploading and updating documents,
//! creating, listing and revoking links, setting per-person access and listing who has access. Every method maps to
//! one route documented in `docs/api/sharing-api.md`.
//!
//! Transport: `https` to any host the user entered (including a private network), plain `http` only to loopback.
//! Redirects are never followed. The session token lives in memory and is never printed by `Debug`.
//!
//! OpenID Connect without a loopback listener: [`SharingClient::oidc_begin`] returns a URL for the system browser and
//! keeps a random hand-off secret; the server learns only its SHA-256. When the browser finishes,
//! [`SharingClient::oidc_poll`] (or [`oidc_wait`](SharingClient::oidc_wait)) redeems the secret once for a session.
//! Nothing secret is placed in a URL, and no local port is opened.

use std::time::Duration;

use hyper::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::{Client, ClientError, Response, pct_encode};
use crate::util::{handoff_user_code, new_token, sha256_hex};

#[derive(Debug)]
pub enum ApiClientError {
    /// The request did not complete (bad URL, refused address, network, timeout, oversized body).
    Transport(ClientError),
    /// The server answered with an error status; `code` is its fixed error code (for example `totp_required`).
    Status { status: u16, code: String },
    /// A success response that this client does not understand.
    Decode(&'static str),
    /// An operation that needs a session was called before signing in.
    NotSignedIn,
    /// The browser sign-in did not finish in time.
    TimedOut,
}

impl ApiClientError {
    /// The server's error code, when it sent one.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Status { code, .. } => Some(code),
            _ => None,
        }
    }

    pub fn needs_totp(&self) -> bool {
        self.code() == Some("totp_required")
    }

    /// An optimistic-concurrency conflict: the document changed since `base_version`.
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Status { status: 409, .. })
    }
}

impl std::fmt::Display for ApiClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "cannot reach the server: {e:?}"),
            Self::Status { status, code } => {
                write!(f, "the server refused the request ({status}: {code})")
            }
            Self::Decode(w) => write!(f, "unexpected response from the server: {w}"),
            Self::NotSignedIn => f.write_str("not signed in"),
            Self::TimedOut => f.write_str("sign-in was not completed in time"),
        }
    }
}

impl std::error::Error for ApiClientError {}

impl From<ClientError> for ApiClientError {
    fn from(e: ClientError) -> Self {
        Self::Transport(e)
    }
}

pub type ApiClientResult<T> = Result<T, ApiClientError>;

/// The second factor sent with a password sign-in.
#[derive(Clone, PartialEq, Eq)]
pub enum SecondFactor {
    None,
    /// The six-digit authenticator code.
    Totp(String),
    /// One of the single-use recovery codes.
    Recovery(String),
}

impl std::fmt::Debug for SecondFactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::None => "None",
            Self::Totp(_) => "Totp(..)",
            Self::Recovery(_) => "Recovery(..)",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Session {
    pub tenant_id: String,
    pub user_id: String,
    pub expires_in: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Me {
    pub tenant_id: String,
    pub user_id: String,
    pub email: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkScope {
    View,
    Edit,
    ViewNoDownload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    Anyone,
    Org,
    Specific,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Saved {
    pub id: String,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DocumentInfo {
    pub id: String,
    pub name: String,
    pub version: i64,
    /// `owner`, `editor` or `viewer`.
    pub role: String,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VersionInfo {
    pub n: i64,
    pub size: i64,
    pub sha256: String,
    pub created_by: String,
    pub created_at: i64,
}

/// What to create. `emails` is required for [`Audience::Specific`] and must be empty otherwise.
#[derive(Clone, Serialize)]
// No Debug: may carry a password.
#[allow(missing_debug_implementations)]
pub struct NewLink {
    pub scope: LinkScope,
    pub audience: Audience,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emails: Option<Vec<String>>,
}

impl NewLink {
    pub fn new(scope: LinkScope, audience: Audience) -> Self {
        Self {
            scope,
            audience,
            expires_in_secs: None,
            password: None,
            emails: None,
        }
    }
}

/// A link as returned at creation: the only time the token is visible.
#[derive(Clone, Deserialize)]
pub struct CreatedLink {
    pub id: String,
    pub token: String,
    pub url_path: String,
    pub scope: LinkScope,
    pub audience: Audience,
    pub expires_at: Option<i64>,
}

impl std::fmt::Debug for CreatedLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatedLink")
            .field("id", &self.id)
            .field("scope", &self.scope)
            .field("audience", &self.audience)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LinkInfo {
    pub id: String,
    pub scope: LinkScope,
    pub audience: Audience,
    pub expires_at: Option<i64>,
    pub has_password: bool,
    pub created_by: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
    #[serde(default)]
    pub emails: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AccessEntry {
    pub user_id: String,
    pub email: String,
    /// `owner`, `editor` or `viewer`.
    pub role: String,
}

/// An OpenID Connect sign-in in progress; hand `browser_url` to the system browser.
#[derive(Clone)]
// No Debug: holds the hand-off secret.
#[allow(missing_debug_implementations)]
pub struct OidcSignIn {
    pub browser_url: String,
    /// Show this to the person; the browser page shows the same code and asks them to confirm.
    pub user_code: String,
    secret: String,
}

#[derive(Clone)]
pub struct SharingClient {
    http: Client,
    base: String,
    token: Option<String>,
}

impl std::fmt::Debug for SharingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharingClient")
            .field("base", &self.base)
            .field("signed_in", &self.token.is_some())
            .finish_non_exhaustive()
    }
}

fn decode<T: for<'de> Deserialize<'de>>(r: &Response, what: &'static str) -> ApiClientResult<T> {
    serde_json::from_slice(&r.body).map_err(|_| ApiClientError::Decode(what))
}

impl SharingClient {
    /// `base_url` is the server's origin, for example `https://share.example.org`. `extra_roots` are DER
    /// certificates trusted besides the system store (a private CA).
    pub fn new(base_url: &str, extra_roots: &[Vec<u8>]) -> Result<Self, String> {
        let http = Client::for_user_server(extra_roots)?
            .with_limits(512 * 1024 * 1024, Duration::from_secs(300));
        Ok(Self::with_client(http, base_url))
    }

    pub fn with_client(http: Client, base_url: &str) -> Self {
        Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
            token: None,
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.token.is_some()
    }

    /// Resumes with a token kept by the caller (for example in the OS keychain).
    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// The link a person opens: the server origin plus the token path. Use it only for a [`CreatedLink`].
    pub fn share_url(&self, link: &CreatedLink) -> String {
        format!("{}{}", self.base, link.url_path)
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        content_type: &str,
        authed: bool,
    ) -> ApiClientResult<Response> {
        let bearer;
        let mut headers: Vec<(&str, &str)> = vec![];
        if authed {
            let t = self.token.as_deref().ok_or(ApiClientError::NotSignedIn)?;
            bearer = format!("Bearer {t}");
            headers.push(("authorization", &bearer));
        }
        if body.is_some() {
            headers.push(("content-type", content_type));
        }
        let url = format!("{}{path}", self.base);
        let r = self
            .http
            .request(method, &url, &headers, body.unwrap_or_default())
            .await?;
        if (200..300).contains(&r.status) {
            Ok(r)
        } else {
            Err(ApiClientError::Status {
                status: r.status,
                code: r.json()["error"].as_str().unwrap_or("unknown").to_string(),
            })
        }
    }

    async fn json_call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> ApiClientResult<Response> {
        let bytes = body.map(|v| serde_json::to_vec(&v).unwrap_or_default());
        self.call(method, path, bytes, "application/json", true)
            .await
    }

    // ---- sign-in ----

    /// Signs in with a local account. A `totp_required` error ([`ApiClientError::needs_totp`]) means: ask for a code
    /// and call again with [`SecondFactor::Totp`] or [`SecondFactor::Recovery`].
    pub async fn sign_in_password(
        &mut self,
        tenant: &str,
        email: &str,
        password: &str,
        second: SecondFactor,
    ) -> ApiClientResult<Session> {
        let mut body =
            json!({ "tenant": tenant, "email": email, "password": password, "bearer": true });
        match second {
            SecondFactor::None => {}
            SecondFactor::Totp(c) => body["totp"] = json!(c),
            SecondFactor::Recovery(c) => body["recovery_code"] = json!(c),
        }
        let r = self
            .call(
                Method::POST,
                "/api/v1/login",
                Some(serde_json::to_vec(&body).map_err(|_| ApiClientError::Decode("request"))?),
                "application/json",
                false,
            )
            .await?;
        self.take_session(&r)
    }

    fn take_session(&mut self, r: &Response) -> ApiClientResult<Session> {
        let v = r.json();
        let token = v["token"]
            .as_str()
            .ok_or(ApiClientError::Decode("no token"))?
            .to_string();
        let session = decode(r, "session")?;
        self.token = Some(token);
        Ok(session)
    }

    /// Starts an OpenID Connect sign-in for `tenant`. Open `browser_url` in the system browser, then call
    /// [`oidc_wait`](Self::oidc_wait).
    pub fn oidc_begin(&self, tenant: &str) -> OidcSignIn {
        let secret = new_token();
        let handoff = sha256_hex(secret.as_bytes());
        OidcSignIn {
            browser_url: format!(
                "{}/auth/oidc/{}/start?handoff={handoff}",
                self.base,
                pct_encode(tenant)
            ),
            user_code: handoff_user_code(&handoff),
            secret,
        }
    }

    /// One redemption attempt: `Ok(None)` while the browser step is not finished.
    pub async fn oidc_poll(&mut self, signin: &OidcSignIn) -> ApiClientResult<Option<Session>> {
        let body = serde_json::to_vec(&json!({ "handoff": signin.secret }))
            .map_err(|_| ApiClientError::Decode("request"))?;
        let r = self
            .call(
                Method::POST,
                "/auth/oidc/poll",
                Some(body),
                "application/json",
                false,
            )
            .await?;
        if r.status == 202 {
            return Ok(None);
        }
        self.take_session(&r).map(Some)
    }

    /// Polls every `interval` until the browser step finishes or `timeout` passes.
    pub async fn oidc_wait(
        &mut self,
        signin: &OidcSignIn,
        interval: Duration,
        timeout: Duration,
    ) -> ApiClientResult<Session> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(s) = self.oidc_poll(signin).await? {
                return Ok(s);
            }
            if tokio::time::Instant::now() + interval > deadline {
                return Err(ApiClientError::TimedOut);
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Ends the session on the server and forgets the token.
    pub async fn sign_out(&mut self) -> ApiClientResult<()> {
        self.json_call(Method::POST, "/api/v1/logout", None).await?;
        self.token = None;
        Ok(())
    }

    pub async fn me(&self) -> ApiClientResult<Me> {
        let r = self.json_call(Method::GET, "/api/v1/me", None).await?;
        decode(&r, "me")
    }

    // ---- two-factor ----

    /// Starts TOTP enrolment; returns the `otpauth://` URI to show as a QR code. Confirm with a code next.
    pub async fn totp_enroll(&self) -> ApiClientResult<String> {
        let r = self
            .json_call(Method::POST, "/api/v1/me/totp/enroll", Some(json!({})))
            .await?;
        r.json()["otpauth_uri"]
            .as_str()
            .map(str::to_string)
            .ok_or(ApiClientError::Decode("otpauth_uri"))
    }

    /// Activates TOTP and returns the single-use recovery codes (shown once).
    pub async fn totp_confirm(&self, code: &str) -> ApiClientResult<Vec<String>> {
        let r = self
            .json_call(
                Method::POST,
                "/api/v1/me/totp/confirm",
                Some(json!({ "code": code })),
            )
            .await?;
        recovery_codes(&r)
    }

    /// Replaces the recovery codes; needs a current authenticator code.
    pub async fn new_recovery_codes(&self, code: &str) -> ApiClientResult<Vec<String>> {
        let r = self
            .json_call(
                Method::POST,
                "/api/v1/me/totp/recovery-codes",
                Some(json!({ "code": code })),
            )
            .await?;
        recovery_codes(&r)
    }

    // ---- documents ----

    /// Uploads a new document (opaque bytes).
    pub async fn upload(&self, name: &str, bytes: Vec<u8>) -> ApiClientResult<Saved> {
        let r = self
            .call(
                Method::POST,
                &format!("/api/v1/documents?name={}", pct_encode(name)),
                Some(bytes),
                "application/octet-stream",
                true,
            )
            .await?;
        decode(&r, "saved")
    }

    /// Uploads a new version. With `base_version`, a document that changed since then is refused
    /// ([`ApiClientError::is_conflict`]) instead of being overwritten.
    pub async fn update(
        &self,
        doc: &str,
        bytes: Vec<u8>,
        base_version: Option<i64>,
    ) -> ApiClientResult<Saved> {
        let q = base_version.map_or_else(String::new, |v| format!("?base_version={v}"));
        let r = self
            .call(
                Method::PUT,
                &format!("/api/v1/documents/{}{q}", pct_encode(doc)),
                Some(bytes),
                "application/octet-stream",
                true,
            )
            .await?;
        decode(&r, "saved")
    }

    pub async fn list_documents(&self) -> ApiClientResult<Vec<DocumentInfo>> {
        decode(
            &self
                .json_call(Method::GET, "/api/v1/documents", None)
                .await?,
            "documents",
        )
    }

    pub async fn document(&self, doc: &str) -> ApiClientResult<DocumentInfo> {
        let r = self
            .json_call(
                Method::GET,
                &format!("/api/v1/documents/{}", pct_encode(doc)),
                None,
            )
            .await?;
        decode(&r, "document")
    }

    pub async fn versions(&self, doc: &str) -> ApiClientResult<Vec<VersionInfo>> {
        let r = self
            .json_call(
                Method::GET,
                &format!("/api/v1/documents/{}/versions", pct_encode(doc)),
                None,
            )
            .await?;
        decode(&r, "versions")
    }

    /// The bytes of a version (the current one when `version` is `None`).
    pub async fn download(&self, doc: &str, version: Option<i64>) -> ApiClientResult<Vec<u8>> {
        let q = version.map_or_else(String::new, |v| format!("?version={v}"));
        let r = self
            .json_call(
                Method::GET,
                &format!("/api/v1/documents/{}/content{q}", pct_encode(doc)),
                None,
            )
            .await?;
        Ok(r.body)
    }

    // ---- links ----

    pub async fn create_link(&self, doc: &str, link: &NewLink) -> ApiClientResult<CreatedLink> {
        let body = serde_json::to_value(link).map_err(|_| ApiClientError::Decode("request"))?;
        let r = self
            .json_call(
                Method::POST,
                &format!("/api/v1/documents/{}/links", pct_encode(doc)),
                Some(body),
            )
            .await?;
        decode(&r, "link")
    }

    pub async fn list_links(&self, doc: &str) -> ApiClientResult<Vec<LinkInfo>> {
        let r = self
            .json_call(
                Method::GET,
                &format!("/api/v1/documents/{}/links", pct_encode(doc)),
                None,
            )
            .await?;
        decode(&r, "links")
    }

    pub async fn revoke_link(&self, link_id: &str) -> ApiClientResult<()> {
        self.json_call(
            Method::DELETE,
            &format!("/api/v1/links/{}", pct_encode(link_id)),
            None,
        )
        .await
        .map(drop)
    }

    // ---- access ----

    /// Gives `email` (a person in the same organisation) the role, or changes it.
    pub async fn set_access(&self, doc: &str, email: &str, role: Role) -> ApiClientResult<()> {
        self.json_call(
            Method::PUT,
            &format!("/api/v1/documents/{}/acl", pct_encode(doc)),
            Some(json!({ "email": email, "role": role })),
        )
        .await
        .map(drop)
    }

    pub async fn remove_access(&self, doc: &str, user_id: &str) -> ApiClientResult<()> {
        self.json_call(
            Method::DELETE,
            &format!(
                "/api/v1/documents/{}/acl/{}",
                pct_encode(doc),
                pct_encode(user_id)
            ),
            None,
        )
        .await
        .map(drop)
    }

    /// The owner first, then everyone with a grant.
    pub async fn list_access(&self, doc: &str) -> ApiClientResult<Vec<AccessEntry>> {
        let r = self
            .json_call(
                Method::GET,
                &format!("/api/v1/documents/{}/acl", pct_encode(doc)),
                None,
            )
            .await?;
        decode(&r, "access")
    }
}

fn recovery_codes(r: &Response) -> ApiClientResult<Vec<String>> {
    r.json()["recovery_codes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .ok_or(ApiClientError::Decode("recovery_codes"))
}
