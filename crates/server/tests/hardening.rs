//! Secrets at rest, recovery codes, key rotation, orphan-blob GC, and the listener limits (slowloris).

mod common;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use common::*;
use open365_server::secrets::SecretKeys;
use open365_server::{ops, totp};
use serde_json::json;

fn now() -> i64 {
    open365_server::util::now()
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

async fn enroll(e: &Env, tok: &str) -> (String, Vec<String>) {
    let r = send(
        e,
        req(Method::POST, "/api/v1/me/totp/enroll")
            .auth(tok)
            .json(json!({})),
    )
    .await;
    let secret = r.json()["secret"].as_str().expect("secret").to_string();
    let code = totp::code_at(&secret, now()).expect("code");
    let c = send(
        e,
        req(Method::POST, "/api/v1/me/totp/confirm")
            .auth(tok)
            .json(json!({ "code": code })),
    )
    .await;
    assert_eq!(c.status, StatusCode::OK, "{:?}", c.json());
    let codes = c.json()["recovery_codes"]
        .as_array()
        .expect("codes")
        .iter()
        .map(|c| c.as_str().expect("code").to_string())
        .collect();
    (secret, codes)
}

#[tokio::test]
async fn totp_secret_is_encrypted_at_rest_and_absent_from_backups() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let bob = add_user(&e, &admin, "acme", "bob@acme.test").await;
    let (secret, _) = enroll(&e, &bob).await;
    let stored: String = e
        .state
        .db
        .lock()
        .query_row(
            "SELECT totp_secret FROM users WHERE email = 'bob@acme.test'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert!(
        stored.starts_with("v1:test-s1:") && !stored.contains(&secret),
        "{stored}"
    );
    // Not in the live files or in a backup either (WAL included).
    let dest = tempfile::tempdir().expect("dest");
    ops::backup(e.dir.path(), &dest.path().join("b")).expect("backup");
    for f in walk(dest.path()) {
        assert!(
            !contains(&std::fs::read(&f).expect("read"), secret.as_bytes()),
            "{}",
            f.display()
        );
    }
    for f in walk(e.dir.path()) {
        assert!(
            !contains(&std::fs::read(&f).expect("read"), secret.as_bytes()),
            "{}",
            f.display()
        );
    }
}

#[tokio::test]
async fn totp_is_refused_without_a_server_key() {
    let e = env_with(|c| c.secret_keys = None);
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let r = send(
        &e,
        req(Method::POST, "/api/v1/me/totp/enroll")
            .auth(&admin)
            .json(json!({})),
    )
    .await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.json()["error"], "totp_unavailable");
}

#[tokio::test]
async fn recovery_codes_are_single_use_hashed_and_replaceable() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let bob = add_user(&e, &admin, "acme", "bob@acme.test").await;
    let (secret, codes) = enroll(&e, &bob).await;
    assert_eq!(codes.len(), 10);
    let stored: i64 = e
        .state
        .db
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM totp_recovery_codes WHERE code_hash = ?1",
            [&codes[0]],
            |r| r.get(0),
        )
        .expect("n");
    assert_eq!(stored, 0, "only hashes are stored");
    let login = |code: String| {
        send(
            &e,
            req(Method::POST, "/api/v1/login")
                .json(json!({"tenant": "acme", "email": "bob@acme.test", "password": PW, "bearer": true, "recovery_code": code})),
        )
    };
    assert_eq!(login(codes[0].clone()).await.status, StatusCode::OK);
    assert_eq!(
        login(codes[0].clone()).await.status,
        StatusCode::UNAUTHORIZED,
        "single use"
    );
    assert_eq!(
        login(codes[1].to_lowercase().replace('-', " "))
            .await
            .status,
        StatusCode::OK,
        "format is forgiving"
    );
    assert_eq!(
        login("AAAA-AAAA-AAAA-AAAA".into()).await.json()["error"],
        "invalid_code"
    );
    // Regenerating needs a current authenticator code and retires the old set.
    let bad = send(
        &e,
        req(Method::POST, "/api/v1/me/totp/recovery-codes")
            .auth(&bob)
            .json(json!({"code": "000000"})),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let fresh = totp::code_at(&secret, now() + 30).expect("code");
    let r = send(
        &e,
        req(Method::POST, "/api/v1/me/totp/recovery-codes")
            .auth(&bob)
            .json(json!({"code": fresh})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
    assert_eq!(
        r.json()["recovery_codes"].as_array().expect("codes").len(),
        10
    );
    assert_eq!(
        login(codes[2].clone()).await.status,
        StatusCode::UNAUTHORIZED,
        "old set retired"
    );
}

#[tokio::test]
async fn key_rotation_reseals_values_and_legacy_plaintext() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let bob = add_user(&e, &admin, "acme", "bob@acme.test").await;
    let (secret, _) = enroll(&e, &bob).await;
    // A row from before encryption existed, and a rotation to a new key.
    let mut conn = e.state.db.lock();
    let uid: String = conn
        .query_row(
            "SELECT id FROM users WHERE email = 'admin@acme.test'",
            [],
            |r| r.get(0),
        )
        .expect("id");
    conn.execute(
        "UPDATE users SET totp_secret = 'JBSWY3DPEHPK3PXP' WHERE id = ?1",
        [&uid],
    )
    .expect("legacy");
    let keys = SecretKeys::list(vec![
        ("test-s1".into(), vec![0; 32]),
        ("s2".into(), vec![9; 32]),
    ])
    .expect("keys");
    let r = open365_server::secrets::reseal_all(&mut conn, &keys).expect("reseal");
    // The legacy row is sealed; bob's row was sealed under a key this list does not hold (different bytes), so it is
    // reported rather than destroyed.
    assert_eq!(r.sealed_plaintext, 1);
    assert_eq!(r.unreadable, 1, "{r:?}");
    let admin_row: String = conn
        .query_row("SELECT totp_secret FROM users WHERE id = ?1", [&uid], |r| {
            r.get(0)
        })
        .expect("row");
    assert!(admin_row.starts_with("v1:s2:"));
    assert_eq!(
        keys.open(&format!("totp:{uid}"), &admin_row).as_deref(),
        Some("JBSWY3DPEHPK3PXP")
    );
    let bob_row: String = conn
        .query_row(
            "SELECT totp_secret FROM users WHERE email = 'bob@acme.test'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert!(
        bob_row.starts_with("v1:test-s1:") && !bob_row.contains(&secret),
        "untouched"
    );
}

#[tokio::test]
async fn rotation_with_the_old_key_listed_moves_values_to_the_new_key() {
    let dir = tempfile::tempdir().expect("dir");
    let old = SecretKeys::single("s1", vec![1; 32]).expect("k");
    let mut cfg = open365_server::config::Config::new(dir.path());
    cfg.argon2 = open365_server::config::Argon2Cost {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    cfg.bootstrap_token = Some("b".repeat(32));
    cfg.secret_keys = Some(old);
    let st = open365_server::AppState::new(
        cfg,
        open365_server::config::AuditKeys::single("a", vec![5; 32]).expect("audit"),
    )
    .expect("state");
    let app = open365_server::router(st.clone());
    let e = Env {
        app,
        state: st,
        dir,
        bootstrap: "b".repeat(32),
    };
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let (secret, _) = enroll(&e, &admin).await;
    let both =
        SecretKeys::list(vec![("s1".into(), vec![1; 32]), ("s2".into(), vec![2; 32])]).expect("k");
    let r = ops::rotate_secrets(e.dir.path(), &both).expect("rotate");
    assert_eq!((r.rotated, r.unreadable), (1, 0));
    let row: String = e
        .state
        .db
        .lock()
        .query_row(
            "SELECT totp_secret FROM users WHERE email = 'admin@acme.test'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert!(row.starts_with("v1:s2:"));
    // The old key can now be retired: only the new key is needed to read it.
    let only_new = SecretKeys::single("s2", vec![2; 32]).expect("k");
    let uid: String = e
        .state
        .db
        .lock()
        .query_row(
            "SELECT id FROM users WHERE email = 'admin@acme.test'",
            [],
            |r| r.get(0),
        )
        .expect("id");
    assert_eq!(
        only_new.open(&format!("totp:{uid}"), &row).as_deref(),
        Some(secret.as_str())
    );
    assert_eq!(
        ops::rotate_secrets(e.dir.path(), &both)
            .expect("again")
            .rotated,
        0,
        "idempotent"
    );
}

#[tokio::test]
async fn gc_removes_only_old_unreferenced_blobs() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let doc = upload(&e, &admin, "a.bin", b"referenced").await;
    let tenant: String = e
        .state
        .db
        .lock()
        .query_row("SELECT id FROM tenants", [], |r| r.get(0))
        .expect("t");
    let orphan = e.state.blobs.put(&tenant, b"orphan bytes").expect("put");
    let young = e.state.blobs.put(&tenant, b"young orphan").expect("put");
    let path = |id: &str| {
        e.dir
            .path()
            .join("blobs")
            .join(&tenant)
            .join(&id[..2])
            .join(id)
    };
    let old = std::time::SystemTime::now() - Duration::from_secs(7200);
    let stale_tmp = path(&orphan).with_file_name(".tmp-leftover");
    std::fs::write(&stale_tmp, b"partial").expect("tmp");
    for p in [path(&orphan), stale_tmp.clone()] {
        std::fs::OpenOptions::new()
            .write(true)
            .open(p)
            .expect("open")
            .set_modified(old)
            .expect("age");
    }
    let dry = ops::gc_blobs(e.dir.path(), 3600, true).expect("dry");
    assert_eq!(
        (dry.orphans, dry.removed, dry.too_new),
        (2, 0, 1),
        "{dry:?}"
    );
    assert!(path(&orphan).exists());
    let real = ops::gc_blobs(e.dir.path(), 3600, false).expect("gc");
    assert_eq!((real.orphans, real.removed), (2, 2));
    assert!(!path(&orphan).exists() && !stale_tmp.exists());
    assert!(path(&young).exists(), "recent files are spared");
    let kept = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{doc}/content")).auth(&admin),
    )
    .await;
    assert_eq!(kept.body, b"referenced");
    // Re-uploading identical bytes refreshes the file's age, so a concurrent GC cannot take it.
    let again = e.state.blobs.put(&tenant, b"young orphan").expect("put");
    assert_eq!(again, young);
}

// ---- listener limits ----

fn host(base: &str) -> String {
    base.trim_start_matches("http://").to_string()
}

/// True if the server closed the connection within `within`.
fn closed_within(s: &mut TcpStream, within: Duration) -> bool {
    s.set_read_timeout(Some(within)).expect("timeout");
    let mut buf = [0u8; 1024];
    let start = Instant::now();
    loop {
        match s.read(&mut buf) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) =>
            {
                return true;
            }
            Err(_) => return false,
        }
        if start.elapsed() >= within {
            return false;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_never_finishes_its_headers_is_dropped() {
    let e = env_with(|c| c.header_read_timeout_secs = 1);
    let (base, handle) = serve_env(&e).await;
    let h = host(&base);
    let res = tokio::task::spawn_blocking(move || {
        let mut slow = TcpStream::connect(&h).expect("connect");
        slow.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\nX-Slow: ")
            .expect("write");
        let dropped = closed_within(&mut slow, Duration::from_secs(6));
        // A well-behaved client on the same server is served.
        let mut ok = TcpStream::connect(&h).expect("connect");
        ok.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .expect("write");
        let mut out = String::new();
        ok.read_to_string(&mut out).expect("read");
        (dropped, out)
    })
    .await
    .expect("join");
    assert!(res.0, "slow-header connection must be closed");
    assert!(res.1.starts_with("HTTP/1.1 200"), "{}", res.1);
    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_beyond_the_per_address_limit_are_refused() {
    let e = env_with(|c| {
        c.max_connections_per_ip = 2;
        c.header_read_timeout_secs = 30;
    });
    let (base, handle) = serve_env(&e).await;
    let h = host(&base);
    let (third_closed, after_release) = tokio::task::spawn_blocking(move || {
        let a = TcpStream::connect(&h).expect("a");
        let b = TcpStream::connect(&h).expect("b");
        std::thread::sleep(Duration::from_millis(300));
        let mut c = TcpStream::connect(&h).expect("c");
        let third_closed = closed_within(&mut c, Duration::from_secs(3));
        drop((a, b));
        std::thread::sleep(Duration::from_millis(500));
        let mut d = TcpStream::connect(&h).expect("d");
        d.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .expect("write");
        let mut out = String::new();
        d.read_to_string(&mut out).expect("read");
        (third_closed, out)
    })
    .await
    .expect("join");
    assert!(
        third_closed,
        "the third connection from one address is closed"
    );
    assert!(after_release.starts_with("HTTP/1.1 200"), "{after_release}");
    handle.shutdown();
}
