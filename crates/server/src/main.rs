//! `open365-server`: single binary, SQLite, local-disk blobs, TLS via rustls.
//!
//! Configuration is by environment variable (see the README). Operator commands:
//! `gen-key <path>`, `backup <dest>`, `restore <backup> [--no-audit-check]`, `audit-verify`,
//! `rotate-secrets` (re-wrap), `key-status`, `gc-blobs [--dry-run] [--min-age-secs N]`.

use std::net::SocketAddr;
use std::path::PathBuf;

use open365_server::config::{Config, OidcProvider, load_secret};
use open365_server::keys;
use open365_server::proxy::Cidr;
use open365_server::serve::{ServeLimits, serve};
use open365_server::util::{hex, random_bytes};
use open365_server::{AppState, ops, router};

fn fail(msg: &str) -> ! {
    eprintln!("open365-server: {msg}");
    std::process::exit(2);
}

fn gen_key(path: &str) {
    let key = hex(&random_bytes::<32>());
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    match opts.open(path) {
        Ok(mut f) => {
            use std::io::Write as _;
            if let Err(e) = writeln!(f, "{key}") {
                fail(&format!("cannot write key: {e}"));
            }
            eprintln!(
                "wrote a new 256-bit audit key to {path}; keep it out of backups of the data directory"
            );
        }
        Err(e) => fail(&format!("cannot create {path}: {e}")),
    }
}

fn audit_keys() -> open365_server::config::AuditKeys {
    let p = keys::provider_from_env().unwrap_or_else(|e| fail(&e));
    keys::audit_keys(p.as_ref()).unwrap_or_else(|e| fail(&e))
}

fn secret_keys() -> Option<open365_server::secrets::SecretKeys> {
    let p = keys::provider_from_env().unwrap_or_else(|e| fail(&e));
    keys::secret_keys(p.as_ref()).unwrap_or_else(|e| fail(&e))
}

fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var("OPEN365_DATA_DIR").unwrap_or_else(|_| "./data".into()))
}

fn env_num<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok().map(|v| {
        v.parse()
            .unwrap_or_else(|_| fail(&format!("{name} must be a number")))
    })
}

fn oidc_from_env() -> Vec<OidcProvider> {
    let Ok(issuer) = std::env::var("OPEN365_OIDC_ISSUER") else {
        return vec![];
    };
    let need = |n: &str| {
        std::env::var(n).unwrap_or_else(|_| fail(&format!("set {n} with OPEN365_OIDC_ISSUER")))
    };
    vec![OidcProvider {
        tenant_slug: need("OPEN365_OIDC_TENANT"),
        issuer,
        client_id: need("OPEN365_OIDC_CLIENT_ID"),
        client_secret: load_secret(
            "OPEN365_OIDC_CLIENT_SECRET_FILE",
            "OPEN365_OIDC_CLIENT_SECRET",
        )
        .unwrap_or_else(|e| fail(&e)),
        redirect_uri: need("OPEN365_OIDC_REDIRECT_URI"),
        extra_hosts: std::env::var("OPEN365_OIDC_EXTRA_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
            .collect(),
    }]
}

fn operator_command(args: &[String]) -> bool {
    match args.get(1).map(String::as_str) {
        Some("gen-key") => gen_key(args.get(2).map_or_else(
            || fail("usage: open365-server gen-key <path>"),
            String::as_str,
        )),
        Some("backup") => {
            let dest = args
                .get(2)
                .unwrap_or_else(|| fail("usage: open365-server backup <dest-dir>"));
            match ops::backup(&data_dir(), std::path::Path::new(dest)) {
                Ok(r) => eprintln!(
                    "backup written to {dest}: {} versions, {} blobs, {} bytes (keys are not included)",
                    r.versions, r.blobs, r.bytes
                ),
                Err(e) => fail(&e),
            }
        }
        Some("restore") => {
            let src = args.get(2).unwrap_or_else(|| {
                fail("usage: open365-server restore <backup-dir> [--no-audit-check]")
            });
            let keys = if args.iter().any(|a| a == "--no-audit-check") {
                None
            } else {
                Some(audit_keys())
            };
            match ops::restore(std::path::Path::new(src), &data_dir(), keys.as_ref()) {
                Ok(r) => eprintln!(
                    "restored {} versions ({} blobs) into {}; integrity, blob hashes{} verified",
                    r.versions,
                    r.blobs,
                    data_dir().display(),
                    if keys.is_some() {
                        " and audit chains"
                    } else {
                        ""
                    }
                ),
                Err(e) => fail(&e),
            }
        }
        Some("audit-verify") => {
            let keys = audit_keys();
            let results = ops::audit_verify_all(&data_dir(), &keys).unwrap_or_else(|e| fail(&e));
            let mut bad = false;
            for (chain, v) in results {
                // Record `head_mac` somewhere the server cannot write: it is what detects deletion of the newest rows.
                println!(
                    "chain {chain}: {} entries, {}, head_mac {}",
                    v.entries,
                    if v.ok {
                        "ok".to_string()
                    } else {
                        format!("BROKEN at seq {:?}", v.first_bad_seq)
                    },
                    v.head_mac
                );
                bad |= !v.ok;
            }
            if bad {
                std::process::exit(1);
            }
        }
        Some("rotate-secrets") => {
            let keys = secret_keys().unwrap_or_else(|| {
                fail("no secret key: configure OPEN365_SECRET_KEY_FILE(S) or OPEN365_KEY_PROVIDER")
            });
            let r = ops::rotate_secrets(&data_dir(), &keys).unwrap_or_else(|e| fail(&e));
            eprintln!(
                "secrets: {} plaintext sealed, {} moved to key {}, {} unreadable with the listed keys",
                r.sealed_plaintext,
                r.rotated,
                keys.current_id(),
                r.unreadable
            );
            if r.unreadable > 0 {
                std::process::exit(1);
            }
        }
        Some("key-status") => {
            let u = ops::key_status(&data_dir()).unwrap_or_else(|e| fail(&e));
            for (id, n) in &u.audit {
                println!("audit key {id}: {n} rows (keep this key listed while rows exist)");
            }
            for (id, n) in &u.secrets {
                println!(
                    "secret key {id}: {n} TOTP secrets (run rotate-secrets, then retire when 0)"
                );
            }
        }
        Some("gc-blobs") => {
            let dry = args.iter().any(|a| a == "--dry-run");
            let age = args
                .iter()
                .position(|a| a == "--min-age-secs")
                .and_then(|i| args.get(i + 1))
                .map_or(3600, |v| {
                    v.parse()
                        .unwrap_or_else(|_| fail("--min-age-secs takes a number"))
                });
            let r = ops::gc_blobs(&data_dir(), age, dry).unwrap_or_else(|e| fail(&e));
            eprintln!(
                "gc-blobs{}: {} orphan files ({} bytes), {} removed, {} spared as too new",
                if dry { " (dry run)" } else { "" },
                r.orphans,
                r.bytes,
                r.removed,
                r.too_new
            );
        }
        _ => return false,
    }
    true
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if operator_command(&args) {
        return;
    }
    let bind: SocketAddr = std::env::var("OPEN365_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8443".into())
        .parse()
        .unwrap_or_else(|_| fail("OPEN365_BIND must look like 127.0.0.1:8443"));
    let tls = match (
        std::env::var("OPEN365_TLS_CERT"),
        std::env::var("OPEN365_TLS_KEY"),
    ) {
        (Ok(c), Ok(k)) => Some((c, k)),
        (Err(_), Err(_)) => None,
        _ => fail("set both OPEN365_TLS_CERT and OPEN365_TLS_KEY, or neither"),
    };
    if tls.is_none()
        && !bind.ip().is_loopback()
        && std::env::var("OPEN365_ALLOW_PLAIN_HTTP").is_err()
    {
        fail(
            "refusing to serve plain HTTP on a non-loopback address; configure TLS or put it behind a TLS proxy and set OPEN365_ALLOW_PLAIN_HTTP=1",
        );
    }
    let mut cfg = Config::new(data_dir());
    cfg.bootstrap_token = load_secret("OPEN365_BOOTSTRAP_TOKEN_FILE", "OPEN365_BOOTSTRAP_TOKEN")
        .unwrap_or_else(|e| fail(&e));
    if let Some(v) = env_num("OPEN365_MAX_UPLOAD_BYTES") {
        cfg.max_upload_bytes = v;
    }
    if let Some(v) = env_num("OPEN365_REQUEST_TIMEOUT_SECS") {
        cfg.request_timeout_secs = v;
    }
    if let Some(v) = env_num("OPEN365_UPLOAD_TIMEOUT_SECS") {
        cfg.upload_timeout_secs = v;
    }
    if let Some(v) = env_num("OPEN365_ARGON2_CONCURRENCY") {
        cfg.argon2_concurrency = v;
    }
    if let Ok(list) = std::env::var("OPEN365_TRUSTED_PROXIES") {
        cfg.trusted_proxies = Cidr::parse_list(&list).unwrap_or_else(|e| fail(&e));
    }
    // HSTS when this process serves TLS, or when the operator says a proxy terminates it.
    cfg.hsts = tls.is_some() || std::env::var("OPEN365_HSTS").is_ok();
    cfg.oidc = oidc_from_env();
    cfg.secret_keys = secret_keys();
    if cfg.secret_keys.is_none() {
        eprintln!(
            "open365-server: no OPEN365_SECRET_KEY_FILE; TOTP is unavailable until one is set (see `gen-key`)"
        );
    }
    if let Some(v) = env_num("OPEN365_HEADER_READ_TIMEOUT_SECS") {
        cfg.header_read_timeout_secs = v;
    }
    if let Some(v) = env_num("OPEN365_MAX_CONNECTIONS") {
        cfg.max_connections = v;
    }
    if let Some(v) = env_num("OPEN365_MAX_CONNECTIONS_PER_IP") {
        cfg.max_connections_per_ip = v;
    }
    let keys = audit_keys();
    let state = AppState::new(cfg, keys)
        .unwrap_or_else(|_| fail("cannot open the data directory or configuration"));
    let limits = ServeLimits::from_config(&state.cfg);
    let app = router(state);
    let listener =
        std::net::TcpListener::bind(bind).unwrap_or_else(|e| fail(&format!("cannot bind: {e}")));
    let conf = match tls {
        Some((cert, key)) => {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let conf = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
                .await
                .unwrap_or_else(|e| fail(&format!("cannot load TLS certificate or key: {e}")));
            eprintln!("open365-server listening on https://{bind}");
            Some(conf)
        }
        None => {
            eprintln!("open365-server listening on http://{bind} (no TLS)");
            None
        }
    };
    serve(listener, app, limits, conf, axum_server::Handle::new())
        .await
        .unwrap_or_else(|e| fail(&format!("server error: {e}")));
}
