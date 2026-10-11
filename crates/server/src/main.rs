//! `open365-server`: single binary, SQLite, local-disk blobs, TLS via rustls.
//!
//! Configuration is by environment variable (see the README). `open365-server gen-key <path>` writes a fresh audit key.

use std::net::SocketAddr;
use std::path::PathBuf;

use open365_server::config::{Config, audit_keys_from_env, load_secret};
use open365_server::util::{hex, random_bytes};
use open365_server::{AppState, router};

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

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("gen-key") {
        gen_key(args.get(2).map_or_else(
            || fail("usage: open365-server gen-key <path>"),
            String::as_str,
        ));
        return;
    }
    let bind: SocketAddr = std::env::var("OPEN365_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8443".into())
        .parse()
        .unwrap_or_else(|_| fail("OPEN365_BIND must look like 127.0.0.1:8443"));
    let data_dir =
        PathBuf::from(std::env::var("OPEN365_DATA_DIR").unwrap_or_else(|_| "./data".into()));
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
    let mut cfg = Config::new(data_dir);
    cfg.bootstrap_token = load_secret("OPEN365_BOOTSTRAP_TOKEN_FILE", "OPEN365_BOOTSTRAP_TOKEN")
        .unwrap_or_else(|e| fail(&e));
    if let Ok(v) = std::env::var("OPEN365_MAX_UPLOAD_BYTES") {
        cfg.max_upload_bytes = v
            .parse()
            .unwrap_or_else(|_| fail("OPEN365_MAX_UPLOAD_BYTES must be a number"));
    }
    let keys = audit_keys_from_env().unwrap_or_else(|e| fail(&e));
    let state = AppState::new(cfg, keys).unwrap_or_else(|_| fail("cannot open the data directory"));
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();

    match tls {
        Some((cert, key)) => {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let conf = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
                .await
                .unwrap_or_else(|e| fail(&format!("cannot load TLS certificate or key: {e}")));
            eprintln!("open365-server listening on https://{bind}");
            axum_server::bind_rustls(bind, conf)
                .serve(app)
                .await
                .unwrap_or_else(|e| fail(&format!("server error: {e}")));
        }
        None => {
            eprintln!("open365-server listening on http://{bind} (no TLS)");
            let listener = tokio::net::TcpListener::bind(bind)
                .await
                .unwrap_or_else(|e| fail(&format!("cannot bind: {e}")));
            axum::serve(listener, app)
                .await
                .unwrap_or_else(|e| fail(&format!("server error: {e}")));
        }
    }
}
