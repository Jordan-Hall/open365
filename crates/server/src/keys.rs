//! Key management: where the audit HMAC keys and the TOTP encryption keys come from.
//!
//! A [`KeyProvider`] returns the keys for one [`KeyPurpose`] as `(key id, key bytes)`, oldest first; the last key is
//! the current one. New audit rows are signed, and new secrets sealed, with the current key only; every listed key
//! still verifies or opens old records, found by the key id stored with each record. Rotation is therefore:
//! add a new key as the last entry, restart, run `open365-server rotate-secrets` (re-wrap TOTP secrets), then retire
//! an old key once `open365-server key-status` shows no record still uses it. Audit rows are never re-signed:
//! rewriting them would erase the tamper evidence, so an audit key id stays listed for as long as its rows exist.
//!
//! Providers (`OPEN365_KEY_PROVIDER`, default `file`):
//! - `file`: key files with mode 0600 (`OPEN365_AUDIT_KEY_FILE(S)`, `OPEN365_SECRET_KEY_FILE(S)`), plus the
//!   single-key environment variables the server has always accepted.
//! - `env`: `OPEN365_AUDIT_KEYS` / `OPEN365_SECRET_KEYS` = `id=hex,id=hex` (oldest first), for a secret manager
//!   that injects environment variables. Environment variables are visible to same-user processes and crash dumps;
//!   prefer `command` or `file`.
//! - `command`: runs the argv in `OPEN365_KEY_COMMAND` (a JSON array, no shell), with `OPEN365_KEY_PURPOSE` set to
//!   `audit` or `secret`. It prints one `id=hex` per line, oldest first. Use it to fetch keys from a KMS or a vault
//!   at start-up. The key never touches disk here. Exit status must be 0, output at most 64 KiB, and the run is
//!   killed after 10 s. Standard error is discarded and nothing the command prints is ever logged.
//!
//! The keys are read once at start-up (and by operator commands), never from a request.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::config::{AuditKeys, check_mode};
use crate::secrets::SecretKeys;
use crate::util::unhex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPurpose {
    /// HMAC-SHA-256 key for the audit chain (at least 32 bytes).
    Audit,
    /// XChaCha20-Poly1305 key for TOTP secrets (exactly 32 bytes).
    Secret,
}

impl KeyPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audit => "audit",
            Self::Secret => "secret",
        }
    }
}

/// A source of keys. Errors are fixed text and never contain key material.
pub trait KeyProvider: Send + Sync {
    fn name(&self) -> &'static str;
    /// `(id, key bytes)`, oldest first; `Ok(vec![])` when nothing is configured for this purpose.
    fn load(&self, purpose: KeyPurpose) -> Result<Vec<(String, Vec<u8>)>, String>;
}

/// Parses `id=hex` entries separated by commas or newlines. Errors name the entry number, never its value.
pub fn parse_entries(text: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = vec![];
    for (n, item) in text
        .split([',', '\n'])
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .enumerate()
    {
        let (id, hex) = item
            .split_once('=')
            .ok_or_else(|| format!("key entry {} is not of the form id=hex", n + 1))?;
        let key = unhex(hex.trim()).ok_or_else(|| format!("key entry {} is not hex", n + 1))?;
        out.push((id.trim().to_string(), key));
    }
    Ok(out)
}

/// Key files (the original behaviour).
#[derive(Debug)]
pub struct FileProvider;

fn read_key_file(label: &str, id: &str, path: &str) -> Result<Vec<u8>, String> {
    check_mode(Path::new(path))?;
    let hex =
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {label} key {id}: {e}"))?;
    unhex(hex.trim()).ok_or_else(|| format!("{label} key {id} must be hex"))
}

impl KeyProvider for FileProvider {
    fn name(&self) -> &'static str {
        "file"
    }

    fn load(&self, purpose: KeyPurpose) -> Result<Vec<(String, Vec<u8>)>, String> {
        let (label, list_var, file_var, value_var, id_var, default_id) = match purpose {
            KeyPurpose::Audit => (
                "audit",
                "OPEN365_AUDIT_KEY_FILES",
                "OPEN365_AUDIT_KEY_FILE",
                "OPEN365_AUDIT_KEY",
                "OPEN365_AUDIT_KEY_ID",
                "k1",
            ),
            KeyPurpose::Secret => (
                "secret",
                "OPEN365_SECRET_KEY_FILES",
                "OPEN365_SECRET_KEY_FILE",
                "OPEN365_SECRET_KEY",
                "OPEN365_SECRET_KEY_ID",
                "s1",
            ),
        };
        if let Ok(list) = std::env::var(list_var) {
            let mut keys = vec![];
            for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
                let (id, path) = item
                    .split_once('=')
                    .ok_or_else(|| format!("{list_var} entries look like id=path"))?;
                keys.push((id.to_string(), read_key_file(label, id, path)?));
            }
            return Ok(keys);
        }
        let Some(hex) = crate::config::load_secret(file_var, value_var)? else {
            return Ok(vec![]);
        };
        let key = unhex(&hex).ok_or_else(|| format!("{label} key must be hex"))?;
        let id = std::env::var(id_var).unwrap_or_else(|_| default_id.into());
        Ok(vec![(id, key)])
    }
}

/// Keys held in environment variables.
#[derive(Debug)]
pub struct EnvProvider;

impl KeyProvider for EnvProvider {
    fn name(&self) -> &'static str {
        "env"
    }

    fn load(&self, purpose: KeyPurpose) -> Result<Vec<(String, Vec<u8>)>, String> {
        let var = match purpose {
            KeyPurpose::Audit => "OPEN365_AUDIT_KEYS",
            KeyPurpose::Secret => "OPEN365_SECRET_KEYS",
        };
        match std::env::var(var) {
            Ok(v) => parse_entries(&v).map_err(|e| format!("{var}: {e}")),
            Err(_) => Ok(vec![]),
        }
    }
}

/// Keys fetched by an external program (KMS client, vault agent, HSM tool).
pub struct CommandProvider {
    argv: Vec<String>,
    timeout: Duration,
    pass_env: Vec<String>,
}

impl std::fmt::Debug for CommandProvider {
    /// Names the program only: arguments may carry a token for the KMS.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandProvider")
            .field("program", &self.argv[0])
            .finish_non_exhaustive()
    }
}

const MAX_OUTPUT: usize = 64 * 1024;

impl CommandProvider {
    pub fn new(argv: Vec<String>, timeout: Duration) -> Result<Self, String> {
        if argv.is_empty() || argv[0].is_empty() {
            return Err("the key command is empty".into());
        }
        Ok(Self {
            argv,
            timeout,
            pass_env: vec![],
        })
    }

    /// Environment variables to pass through besides the fixed allowlist (for example a KMS credential).
    #[must_use]
    pub fn pass_env(mut self, names: Vec<String>) -> Self {
        self.pass_env = names;
        self
    }

    /// From `OPEN365_KEY_COMMAND`, a JSON array of strings (program first). It is run directly, never by a shell.
    /// `OPEN365_KEY_COMMAND_ENV` (comma-separated names) lists extra variables the command may inherit.
    pub fn from_env() -> Result<Self, String> {
        let raw = std::env::var("OPEN365_KEY_COMMAND").map_err(
            |_| "set OPEN365_KEY_COMMAND (a JSON array) for OPEN365_KEY_PROVIDER=command",
        )?;
        let argv: Vec<String> = serde_json::from_str(&raw)
            .map_err(|_| "OPEN365_KEY_COMMAND must be a JSON array of strings".to_string())?;
        let extra = std::env::var("OPEN365_KEY_COMMAND_ENV")
            .unwrap_or_default()
            .split(',')
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect();
        Ok(Self::new(argv, Duration::from_secs(10))?.pass_env(extra))
    }

    /// Runs the command and returns its standard output. The environment is cleared first: only `PATH`,
    /// `SystemRoot`, `HOME`, the listed extras and `OPEN365_KEY_PURPOSE` are passed on, so the server's own secrets
    /// (bootstrap token, other key variables) never reach the program. The whole run, including `wait`, is bounded
    /// by the timeout, and on a timeout the program's whole process tree is killed.
    pub(crate) fn run_raw(&self, purpose: KeyPurpose) -> Result<String, String> {
        let mut cmd = Command::new(&self.argv[0]);
        cmd.args(&self.argv[1..]).env_clear();
        for name in ["PATH", "SystemRoot", "HOME"]
            .into_iter()
            .map(String::from)
            .chain(self.pass_env.iter().cloned())
        {
            if let Some(v) = std::env::var_os(&name) {
                cmd.env(&name, v);
            }
        }
        cmd.env("OPEN365_KEY_PURPOSE", purpose.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|_| "cannot start the key command".to_string())?;
        let deadline = Instant::now() + self.timeout;
        let mut out = child
            .stdout
            .take()
            .ok_or("cannot read the key command output")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = vec![];
            let r = out
                .by_ref()
                .take(u64::try_from(MAX_OUTPUT + 1).unwrap_or(u64::MAX))
                .read_to_end(&mut buf)
                .map(|_| buf);
            let _ = tx.send(r);
        });
        let Ok(Ok(buf)) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        else {
            kill_tree(&mut child);
            return Err("the key command timed out or failed to produce output".into());
        };
        if buf.len() > MAX_OUTPUT {
            kill_tree(&mut child);
            return Err("the key command printed too much".into());
        }
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => {
                    kill_tree(&mut child);
                    return Err("the key command timed out".into());
                }
            }
        };
        if !status.success() {
            return Err("the key command exited with an error".into());
        }
        String::from_utf8(buf).map_err(|_| "the key command output is not text".to_string())
    }
}

/// Kills `child` and everything it started. Without `unsafe` there is no direct Job Object or `killpg` call, so this
/// uses the platform tool (`taskkill /T` on Windows, `kill` on the child's process group on Unix, which the child was
/// started in), then kills the direct child as well in case the tool is missing.
fn kill_tree(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/PID", &pid, "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    #[cfg(unix)]
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

impl KeyProvider for CommandProvider {
    fn name(&self) -> &'static str {
        "command"
    }

    fn load(&self, purpose: KeyPurpose) -> Result<Vec<(String, Vec<u8>)>, String> {
        let text = self.run_raw(purpose)?;
        parse_entries(&text).map_err(|e| format!("key command output: {e}"))
    }
}

/// The provider named by `OPEN365_KEY_PROVIDER` (`file` when unset).
pub fn provider_from_env() -> Result<Box<dyn KeyProvider>, String> {
    match std::env::var("OPEN365_KEY_PROVIDER").as_deref() {
        Err(_) | Ok("file") => Ok(Box::new(FileProvider)),
        Ok("env") => Ok(Box::new(EnvProvider)),
        Ok("command") => Ok(Box::new(CommandProvider::from_env()?)),
        Ok(_) => Err("OPEN365_KEY_PROVIDER must be file, env or command".into()),
    }
}

/// The current key id for `purpose`: `OPEN365_AUDIT_KEY_CURRENT` / `OPEN365_SECRET_KEY_CURRENT`. It must be set
/// whenever more than one key is listed (the order of a provider's output is not trusted to decide it); with a
/// single key it defaults to that key.
fn current_from_env(purpose: KeyPurpose, ids: &[String]) -> Result<String, String> {
    let var = match purpose {
        KeyPurpose::Audit => "OPEN365_AUDIT_KEY_CURRENT",
        KeyPurpose::Secret => "OPEN365_SECRET_KEY_CURRENT",
    };
    match std::env::var(var) {
        Ok(v) => Ok(v.trim().to_string()),
        Err(_) if ids.len() == 1 => Ok(ids[0].clone()),
        Err(_) => Err(format!("set {var} to the id of the current (newest) key")),
    }
}

/// The audit keys from `p`, current key from the environment; an error when none are configured.
pub fn audit_keys(p: &dyn KeyProvider) -> Result<AuditKeys, String> {
    audit_keys_with(p, None)
}

/// As [`audit_keys`], with the current key id given (tests) instead of read from the environment.
pub fn audit_keys_with(p: &dyn KeyProvider, current: Option<&str>) -> Result<AuditKeys, String> {
    let keys = p.load(KeyPurpose::Audit)?;
    if keys.is_empty() {
        return Err(
            "no audit key: set OPEN365_AUDIT_KEY_FILE (see `open365-server gen-key`) or configure OPEN365_KEY_PROVIDER"
                .into(),
        );
    }
    let ids: Vec<String> = keys.iter().map(|(i, _)| i.clone()).collect();
    let cur = match current {
        Some(c) => c.to_string(),
        None => current_from_env(KeyPurpose::Audit, &ids)?,
    };
    AuditKeys::with_current(keys, &cur)
}

/// The TOTP encryption keys from `p`; `None` when none are configured (TOTP then fails closed).
pub fn secret_keys(p: &dyn KeyProvider) -> Result<Option<SecretKeys>, String> {
    secret_keys_with(p, None)
}

pub fn secret_keys_with(
    p: &dyn KeyProvider,
    current: Option<&str>,
) -> Result<Option<SecretKeys>, String> {
    let keys = p.load(KeyPurpose::Secret)?;
    if keys.is_empty() {
        return Ok(None);
    }
    let ids: Vec<String> = keys.iter().map(|(i, _)| i.clone()).collect();
    let cur = match current {
        Some(c) => c.to_string(),
        None => current_from_env(KeyPurpose::Secret, &ids)?,
    };
    SecretKeys::with_current(keys, &cur).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    const K1: &str = "0101010101010101010101010101010101010101010101010101010101010101";
    const K2: &str = "0202020202020202020202020202020202020202020202020202020202020202";

    #[test]
    fn entries_parse_in_order_and_errors_do_not_echo_values() {
        let v = parse_entries(&format!("a={K1}, b={K2}\n")).expect("parse");
        assert_eq!(
            v.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(v[1].1, vec![2; 32]);
        let e = parse_entries("a=zzTOPSECRETzz").expect_err("not hex");
        assert!(!e.contains("TOPSECRET"), "{e}");
        assert!(parse_entries("novalue").is_err());
        assert!(parse_entries("").expect("empty").is_empty());
    }

    #[test]
    fn provider_output_is_validated_like_any_other_keys() {
        struct Fixed(Vec<(String, Vec<u8>)>);
        impl KeyProvider for Fixed {
            fn name(&self) -> &'static str {
                "fixed"
            }
            fn load(&self, _: KeyPurpose) -> Result<Vec<(String, Vec<u8>)>, String> {
                Ok(self.0.clone())
            }
        }
        let one = |len: usize, id: &str| Fixed(vec![(id.into(), vec![1; len])]);
        assert!(
            audit_keys_with(&Fixed(vec![]), Some("a")).is_err(),
            "no audit key is an error"
        );
        assert!(
            secret_keys_with(&Fixed(vec![]), Some("a"))
                .expect("ok")
                .is_none()
        );
        assert!(
            audit_keys_with(&one(16, "a"), Some("a")).is_err(),
            "short audit key"
        );
        assert!(
            secret_keys_with(&one(16, "a"), Some("a")).is_err(),
            "short secret key"
        );
        assert!(
            secret_keys_with(&one(32, "a:b"), Some("a:b")).is_err(),
            "bad id"
        );
        let two = || Fixed(vec![("a".into(), vec![1; 32]), ("b".into(), vec![2; 32])]);
        let k = audit_keys_with(&two(), Some("b")).expect("keys");
        assert_eq!(k.current().0, "b");
        assert!(k.get("a").is_some(), "older keys still verify");
        // The current key is named, not inferred from output order.
        assert!(
            audit_keys_with(&two(), Some("a")).is_err(),
            "named key is not the newest entry"
        );
        assert!(
            audit_keys_with(&two(), Some("zzz")).is_err(),
            "unknown current key"
        );
        assert!(
            secret_keys_with(&two(), Some("a")).is_err(),
            "named key is not the newest entry"
        );
        assert!(
            secret_keys_with(&two(), Some("zzz")).is_err(),
            "unknown current key"
        );
        assert_eq!(
            secret_keys_with(&two(), Some("b"))
                .expect("ok")
                .expect("some")
                .current_id(),
            "b"
        );
        let ids = vec!["a".to_string(), "b".to_string()];
        assert!(
            current_from_env(KeyPurpose::Audit, &ids).is_err(),
            "several keys need an explicit current"
        );
        assert_eq!(
            current_from_env(KeyPurpose::Audit, &ids[..1]).expect("single"),
            "a"
        );
    }

    #[cfg(windows)]
    fn echo(text: &str) -> Vec<String> {
        vec!["cmd".into(), "/C".into(), format!("echo {text}")]
    }
    #[cfg(unix)]
    fn echo(text: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), format!("printf '%s\\n' '{text}'")]
    }

    #[test]
    fn command_provider_reads_keys_from_stdout() {
        let p = CommandProvider::new(echo(&format!("old={K1},new={K2}")), Duration::from_secs(10))
            .expect("p");
        let keys = p.load(KeyPurpose::Secret).expect("load");
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[1].0, "new");
        assert_eq!(keys[1].1, vec![2; 32]);
    }

    #[test]
    fn debug_output_never_shows_arguments() {
        let p = CommandProvider::new(
            vec!["kms-get".into(), "--token=HUNTER2".into()],
            Duration::from_secs(1),
        )
        .expect("p");
        let d = format!("{p:?}");
        assert!(d.contains("kms-get") && !d.contains("HUNTER2"), "{d}");
    }

    #[test]
    fn command_failure_is_an_error_without_output() {
        #[cfg(windows)]
        let argv: Vec<String> = vec!["cmd".into(), "/C".into(), "echo s1=LEAKME& exit 3".into()];
        #[cfg(unix)]
        let argv: Vec<String> = vec!["sh".into(), "-c".into(), "echo s1=LEAKME; exit 3".into()];
        let e = CommandProvider::new(argv, Duration::from_secs(10))
            .expect("p")
            .load(KeyPurpose::Audit)
            .expect_err("non-zero exit");
        assert!(!e.contains("LEAKME"), "{e}");
        let e = CommandProvider::new(
            vec!["definitely-not-a-program-xyz".into()],
            Duration::from_secs(1),
        )
        .expect("p")
        .load(KeyPurpose::Audit)
        .expect_err("missing");
        assert!(e.contains("cannot start"));
        assert!(CommandProvider::new(vec![], Duration::from_secs(1)).is_err());
    }

    #[test]
    fn command_is_killed_after_the_timeout() {
        #[cfg(windows)]
        let argv: Vec<String> = vec!["ping".into(), "-n".into(), "20".into(), "127.0.0.1".into()];
        #[cfg(unix)]
        let argv: Vec<String> = vec!["sleep".into(), "20".into()];
        let t = std::time::Instant::now();
        let e = CommandProvider::new(argv, Duration::from_millis(300))
            .expect("p")
            .load(KeyPurpose::Audit)
            .expect_err("timeout");
        assert!(e.contains("timed out"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn command_receives_the_purpose_and_a_cleared_environment() {
        // CARGO_PKG_NAME is set in this test process by cargo; the command must not see it.
        #[cfg(windows)]
        let argv: Vec<String> = vec![
            "cmd".into(),
            "/C".into(),
            "echo p=%OPEN365_KEY_PURPOSE% v=%CARGO_PKG_NAME%".into(),
        ];
        #[cfg(unix)]
        let argv: Vec<String> = vec![
            "sh".into(),
            "-c".into(),
            "echo p=$OPEN365_KEY_PURPOSE v=${CARGO_PKG_NAME-absent}".into(),
        ];
        assert!(
            std::env::var("CARGO_PKG_NAME").is_ok(),
            "precondition: run by cargo"
        );
        let out = CommandProvider::new(argv.clone(), Duration::from_secs(10))
            .expect("p")
            .run_raw(KeyPurpose::Secret)
            .expect("run");
        assert!(out.contains("p=secret"), "{out}");
        assert!(
            !out.contains("open365-server"),
            "inherited the parent environment: {out}"
        );
        // Listed extras are passed.
        let out = CommandProvider::new(argv, Duration::from_secs(10))
            .expect("p")
            .pass_env(vec!["CARGO_PKG_NAME".into()])
            .run_raw(KeyPurpose::Audit)
            .expect("run");
        assert!(out.contains("open365-server"), "{out}");
    }

    #[test]
    fn a_command_that_closes_stdout_but_keeps_running_is_still_bounded() {
        #[cfg(windows)]
        let argv: Vec<String> = vec![
            "cmd".into(),
            "/C".into(),
            "echo a=01 & ping -n 30 127.0.0.1 >nul".into(),
        ];
        #[cfg(unix)]
        let argv: Vec<String> = vec![
            "sh".into(),
            "-c".into(),
            "echo a=01; exec sleep 30 >&-".into(),
        ];
        let t = Instant::now();
        let r = CommandProvider::new(argv, Duration::from_millis(500))
            .expect("p")
            .run_raw(KeyPurpose::Audit);
        assert!(r.is_err(), "must time out");
        assert!(
            t.elapsed() < Duration::from_secs(15),
            "took {:?}",
            t.elapsed()
        );
    }
}
