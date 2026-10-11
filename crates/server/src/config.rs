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
    /// Failed sign-ins per tenant, address and email.
    pub login_fail_rate: Rate,
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
fn check_mode(p: &Path) -> Result<(), String> {
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
fn check_mode(_: &Path) -> Result<(), String> {
    Ok(())
}

pub fn audit_keys_from_env() -> Result<AuditKeys, String> {
    let hex = load_secret("OPEN365_AUDIT_KEY_FILE", "OPEN365_AUDIT_KEY")?
        .ok_or("set OPEN365_AUDIT_KEY_FILE (see `open365-server gen-key`) or OPEN365_AUDIT_KEY")?;
    let key = crate::util::unhex(&hex).ok_or("audit key must be hex")?;
    let id = std::env::var("OPEN365_AUDIT_KEY_ID").unwrap_or_else(|_| "k1".into());
    AuditKeys::single(&id, key)
}
