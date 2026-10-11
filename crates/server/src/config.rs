//! Server configuration and secret loading (T16: secrets come from a file or the environment, never the database).

use std::path::{Path, PathBuf};

/// Argon2id cost. Tests lower it; production uses [`Argon2Cost::default`].
#[derive(Clone, Copy, Debug)]
pub struct Argon2Cost {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl Default for Argon2Cost {
    fn default() -> Self {
        // OWASP minimum for Argon2id: 19 MiB, t=2, p=1.
        Self {
            m_kib: 19 * 1024,
            t: 2,
            p: 1,
        }
    }
}

/// A limit of `max` events per `window_secs`.
#[derive(Clone, Copy, Debug)]
pub struct Rate {
    pub max: u32,
    pub window_secs: i64,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    pub max_upload_bytes: usize,
    pub tenant_quota_bytes: i64,
    pub session_ttl_secs: i64,
    pub max_link_ttl_secs: i64,
    pub argon2: Argon2Cost,
    /// Enables `POST /api/v1/admin/tenants` when set. Compared in constant time.
    pub bootstrap_token: Option<String>,
    /// Requests to `/s/*` per client address.
    pub link_ip_rate: Rate,
    /// Requests to one link.
    pub link_token_rate: Rate,
    /// Failed link lookups or password tries per client address.
    pub link_fail_rate: Rate,
    /// Wrong passwords per link, across all addresses.
    pub link_password_fail_rate: Rate,
    /// Failed sign-ins per tenant, address and email (counted before the password check).
    pub login_fail_rate: Rate,
    /// Every sign-in attempt per client address, successful or not.
    pub login_ip_rate: Rate,
    /// Wrong passwords per link and client address (the looser per-link limit is `link_password_token_rate`).
    pub link_password_token_rate: Rate,
    /// Uploads through one edit link.
    pub link_upload_rate: Rate,
    /// Failed sign-ins per tenant and account across ALL addresses. Applies only to addresses that have not signed in
    /// to that account before, so an attacker cannot lock the owner out (they only lock themselves).
    pub login_account_rate: Rate,
    /// Share links one signed-in user may create.
    pub link_create_rate: Rate,
    /// Bytes one edit link may add per hour.
    pub link_upload_bytes_per_hour: i64,
    /// Whole-request time limit for ordinary routes.
    pub request_timeout_secs: u64,
    /// Whole-request time limit for routes that carry document bytes.
    pub upload_timeout_secs: u64,
    /// Body limit for JSON routes (uploads use `max_upload_bytes`).
    pub json_body_limit: usize,
    /// Peers allowed to set `X-Forwarded-For`. Empty (the default) trusts none.
    pub trusted_proxies: Vec<crate::proxy::Cidr>,
    /// Concurrent argon2 computations.
    pub argon2_concurrency: usize,
    /// Send `Strict-Transport-Security` (set when TLS is served here or terminated by a proxy).
    pub hsts: bool,
    /// OpenID Connect providers, one per tenant (used only with the `oidc` feature).
    pub oidc: Vec<OidcProvider>,
    /// Encrypts TOTP secrets at rest. Without it TOTP enrolment and sign-in with TOTP are refused (fail closed).
    pub secret_keys: Option<crate::secrets::SecretKeys>,
    /// Time a client has to send a complete request head (slowloris defence).
    pub header_read_timeout_secs: u64,
    /// Open connections in total and per peer address; 0 = unlimited.
    pub max_connections: usize,
    pub max_connections_per_ip: usize,
}

/// One tenant's OpenID Connect provider. The issuer comes only from here, never from a request.
#[derive(Clone)]
pub struct OidcProvider {
    pub tenant_slug: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    /// Hosts besides the issuer's that discovery may point at (for example Google's token host).
    pub extra_hosts: Vec<String>,
}

impl std::fmt::Debug for OidcProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcProvider")
            .field("tenant_slug", &self.tenant_slug)
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl Config {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            max_upload_bytes: 64 * 1024 * 1024,
            tenant_quota_bytes: 10 * 1024 * 1024 * 1024,
            session_ttl_secs: 12 * 3600,
            max_link_ttl_secs: 365 * 24 * 3600,
            argon2: Argon2Cost::default(),
            bootstrap_token: None,
            link_ip_rate: Rate {
                max: 300,
                window_secs: 60,
            },
            link_token_rate: Rate {
                max: 120,
                window_secs: 60,
            },
            link_fail_rate: Rate {
                max: 20,
                window_secs: 300,
            },
            link_password_fail_rate: Rate {
                max: 10,
                window_secs: 900,
            },
            login_fail_rate: Rate {
                max: 8,
                window_secs: 900,
            },
            login_ip_rate: Rate {
                max: 60,
                window_secs: 60,
            },
            link_password_token_rate: Rate {
                max: 100,
                window_secs: 900,
            },
            link_upload_rate: Rate {
                max: 60,
                window_secs: 3600,
            },
            login_account_rate: Rate {
                max: 40,
                window_secs: 3600,
            },
            link_create_rate: Rate {
                max: 120,
                window_secs: 3600,
            },
            link_upload_bytes_per_hour: 512 * 1024 * 1024,
            request_timeout_secs: 30,
            upload_timeout_secs: 300,
            json_body_limit: 64 * 1024,
            trusted_proxies: vec![],
            argon2_concurrency: std::thread::available_parallelism().map_or(2, usize::from),
            hsts: false,
            oidc: vec![],
            secret_keys: None,
            header_read_timeout_secs: 10,
            max_connections: 4096,
            max_connections_per_ip: 64,
        }
    }
}

/// Keys that sign the audit chain, with ids so that a key can be rotated while old rows still verify.
#[derive(Clone)]
pub struct AuditKeys {
    /// `(key id, key bytes)`; the last entry signs new rows.
    pub keys: Vec<(String, Vec<u8>)>,
}

impl std::fmt::Debug for AuditKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&String> = self.keys.iter().map(|(id, _)| id).collect();
        f.debug_struct("AuditKeys")
            .field("ids", &ids)
            .finish_non_exhaustive()
    }
}

impl AuditKeys {
    pub fn single(id: &str, key: Vec<u8>) -> Result<Self, String> {
        if key.len() < 32 {
            return Err("audit key must be at least 32 bytes".into());
        }
        Ok(Self {
            keys: vec![(id.to_string(), key)],
        })
    }

    /// Several keys, oldest first, with the current key named explicitly. The named key must exist and must be the
    /// last entry, so that a provider returning keys in a different order is refused, not silently trusted.
    pub fn with_current(keys: Vec<(String, Vec<u8>)>, current: &str) -> Result<Self, String> {
        let k = Self::list(keys)?;
        match k.index_of(current) {
            None => Err("the named current audit key is not among the keys".into()),
            Some(i) if i + 1 != k.keys.len() => {
                Err("the current audit key must be the last (newest) entry".into())
            }
            Some(_) => Ok(k),
        }
    }

    /// Position of `id` in the list (older keys have lower positions).
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.keys.iter().position(|(k, _)| k == id)
    }

    /// The oldest key, used as the root for server-side derived keys (stable across rotations).
    pub fn root_key(&self) -> &[u8] {
        &self.keys[0].1
    }

    pub fn current(&self) -> (&str, &[u8]) {
        let (id, k) = self.keys.last().expect("AuditKeys is never empty");
        (id, k)
    }

    pub fn get(&self, id: &str) -> Option<&[u8]> {
        self.keys
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, v)| v.as_slice())
    }
}

/// Reads a secret from `file_var` (a path) or else `value_var`. A key file must not be readable by group or others
/// on Unix; Windows relies on the ACL of the data directory (documented in the README).
pub fn load_secret(file_var: &str, value_var: &str) -> Result<Option<String>, String> {
    if let Ok(path) = std::env::var(file_var) {
        check_mode(Path::new(&path))?;
        let s =
            std::fs::read_to_string(&path).map_err(|e| format!("cannot read {file_var}: {e}"))?;
        return Ok(Some(s.trim().to_string()));
    }
    Ok(std::env::var(value_var).ok().map(|s| s.trim().to_string()))
}

#[cfg(unix)]
pub(crate) fn check_mode(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(p)
        .map_err(|e| format!("cannot stat key file: {e}"))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err(format!(
            "key file {} must have mode 0600 (found {:o})",
            p.display(),
            mode & 0o777
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn check_mode(_: &Path) -> Result<(), String> {
    Ok(())
}

impl AuditKeys {
    /// Several keys, oldest first. Only the newest signs new rows; the others still verify old rows.
    pub fn list(keys: Vec<(String, Vec<u8>)>) -> Result<Self, String> {
        if keys.is_empty() {
            return Err("at least one audit key is required".into());
        }
        if keys.iter().any(|(_, k)| k.len() < 32) {
            return Err("audit key must be at least 32 bytes".into());
        }
        let mut ids: Vec<&String> = keys.iter().map(|(i, _)| i).collect();
        ids.sort();
        ids.dedup();
        if ids.len() != keys.len() {
            return Err("audit key ids must be unique".into());
        }
        Ok(Self { keys })
    }
}

/// Audit keys from the file provider (`OPEN365_AUDIT_KEY_FILES=k1=/path/old.key,k2=/path/new.key`, oldest first,
/// newest signs; else `OPEN365_AUDIT_KEY_FILE` / `OPEN365_AUDIT_KEY`). See [`crate::keys`] for the other providers.
pub fn audit_keys_from_env() -> Result<AuditKeys, String> {
    crate::keys::audit_keys(&crate::keys::FileProvider)
}

/// Secret keys from the file provider (`OPEN365_SECRET_KEY_FILES=s1=/path/old.key,s2=/path/new.key`, oldest first,
/// newest seals; else `OPEN365_SECRET_KEY_FILE` / `OPEN365_SECRET_KEY`). `None` when nothing is set.
pub fn secret_keys_from_env() -> Result<Option<crate::secrets::SecretKeys>, String> {
    crate::keys::secret_keys(&crate::keys::FileProvider)
}
