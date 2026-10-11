//! Persistent per-address / per-account / per-link rate limits, lockout and backoff, Retry-After, and key use.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use common::*;
use open365_server::config::{AuditKeys, Rate};
use open365_server::proxy::Cidr;
use open365_server::{AppState, ops, router};
use serde_json::json;
use tower::ServiceExt;

const WRONG: &str = "definitely not the password";

fn rate(max: u32, window_secs: i64) -> Rate {
    Rate { max, window_secs }
}

async fn login_from(e: &Env, ip: &str, password: &str) -> Resp {
    send(
        e,
        req(Method::POST, "/api/v1/login")
            .json(json!({"tenant": "acme", "email": "admin@acme.test", "password": password}))
            .via_ip(ip),
    )
    .await
}

fn retry_after(r: &Resp) -> u64 {
    r.headers
        .get("retry-after")
        .expect("Retry-After header")
        .to_str()
        .expect("ascii")
        .parse()
        .expect("seconds")
}

#[tokio::test]
async fn lockout_is_per_address_and_account_and_never_locks_the_owner_out() {
    let e = env_with(|c| c.login_fail_rate = rate(3, 900));
    make_tenant(&e, "acme", "admin@acme.test").await;
    for _ in 0..3 {
        assert_eq!(
            login_from(&e, "203.0.113.7", WRONG).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let locked = login_from(&e, "203.0.113.7", WRONG).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    let ra = retry_after(&locked);
    assert!((1..=900).contains(&ra), "Retry-After {ra}");
    assert_eq!(
        login_from(&e, "203.0.113.7", PW).await.status,
        StatusCode::TOO_MANY_REQUESTS,
        "the attacker's pair stays locked, even with the right password"
    );
    // The owner, from another address, is unaffected: the attacker cannot lock the account itself.
    assert_eq!(
        login_from(&e, "198.51.100.4", PW).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn limits_and_lockouts_survive_a_restart() {
    let e = env_with(|c| c.login_fail_rate = rate(2, 900));
    make_tenant(&e, "acme", "admin@acme.test").await;
    for _ in 0..2 {
        let _ = login_from(&e, "203.0.113.7", WRONG).await;
    }
    assert_eq!(
        login_from(&e, "203.0.113.7", WRONG).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    // A second server process over the same data directory (as after a restart).
    let st2 = AppState::new(e.state.cfg.clone(), e.state.audit.clone()).expect("restart");
    let app2 = router(st2);
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/login")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"tenant": "acme", "email": "admin@acme.test", "password": PW}).to_string(),
        ))
        .expect("request");
    request.extensions_mut().insert(ConnectInfo(
        "203.0.113.7:4000"
            .parse::<std::net::SocketAddr>()
            .expect("addr"),
    ));
    let res = app2.oneshot(request).await.expect("response");
    assert_eq!(
        res.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "still locked after restart"
    );
    assert!(res.headers().contains_key("retry-after"));
}

#[tokio::test]
async fn distributed_guessing_is_capped_per_account_but_known_addresses_are_exempt() {
    let e = env_with(|c| {
        c.login_fail_rate = rate(100, 900);
        c.login_ip_rate = rate(1000, 60);
        c.login_account_rate = rate(3, 900);
    });
    make_tenant(&e, "acme", "admin@acme.test").await;
    // The owner has signed in from 198.51.100.4 before.
    assert_eq!(
        login_from(&e, "198.51.100.4", PW).await.status,
        StatusCode::OK
    );
    // A botnet: one wrong guess per address, so no per-address limit trips.
    for i in 0..3 {
        assert_eq!(
            login_from(&e, &format!("192.0.2.{}", 10 + i), WRONG)
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
    }
    let blocked = login_from(&e, "192.0.2.50", PW).await;
    assert_eq!(
        blocked.status,
        StatusCode::TOO_MANY_REQUESTS,
        "a new address is held back"
    );
    assert!(retry_after(&blocked) >= 1);
    assert_eq!(
        login_from(&e, "198.51.100.4", PW).await.status,
        StatusCode::OK,
        "the owner's known address is not locked out by the botnet"
    );
}

#[tokio::test]
async fn ipv6_addresses_share_one_budget_per_64() {
    let e = env_with(|c| c.login_ip_rate = rate(2, 60));
    make_tenant(&e, "acme", "admin@acme.test").await;
    assert_eq!(
        login_from(&e, "2001:db8:aa:bb::1", PW).await.status,
        StatusCode::OK
    );
    assert_eq!(
        login_from(&e, "2001:db8:aa:bb:1:2:3:4", PW).await.status,
        StatusCode::OK
    );
    assert_eq!(
        login_from(&e, "2001:db8:aa:bb:ffff::9", PW).await.status,
        StatusCode::TOO_MANY_REQUESTS,
        "rotating inside a /64 buys nothing"
    );
    assert_eq!(
        login_from(&e, "2001:db8:aa:cc::1", PW).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn trusted_proxy_forwarded_address_keys_the_limits() {
    let e = env_with(|c| {
        c.trusted_proxies = Cidr::parse_list("10.0.0.0/8").expect("cidr");
        c.login_ip_rate = rate(1, 60);
    });
    make_tenant(&e, "acme", "admin@acme.test").await;
    let via = |client: &'static str| {
        send(
            &e,
            req(Method::POST, "/api/v1/login")
                .json(json!({"tenant": "acme", "email": "admin@acme.test", "password": PW}))
                .via_ip("10.0.0.1")
                .hdr("x-forwarded-for", client),
        )
    };
    assert_eq!(via("203.0.113.1").await.status, StatusCode::OK);
    assert_eq!(
        via("203.0.113.1").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(via("203.0.113.2").await.status, StatusCode::OK);
    // An untrusted peer cannot choose its bucket with the header.
    let spoof = |xff: &'static str| {
        send(
            &e,
            req(Method::POST, "/api/v1/login")
                .json(json!({"tenant": "acme", "email": "admin@acme.test", "password": PW}))
                .via_ip("198.51.100.99")
                .hdr("x-forwarded-for", xff),
        )
    };
    assert_eq!(spoof("1.1.1.1").await.status, StatusCode::OK);
    assert_eq!(spoof("2.2.2.2").await.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn link_password_lockout_is_per_link_and_address_with_retry_after() {
    let e = env_with(|c| {
        c.link_password_fail_rate = rate(2, 900);
        c.link_fail_rate = rate(100, 300);
    });
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let doc = upload(&e, &admin, "a.bin", b"secret bytes").await;
    let (_, tok) = new_link(
        &e,
        &admin,
        &doc,
        json!({"scope": "view", "audience": "anyone", "password": "link-secret-1"}),
    )
    .await;
    let guess = |ip: &'static str, pw: &'static str| {
        send(
            &e,
            req(Method::GET, format!("/s/{tok}"))
                .hdr("x-share-password", pw)
                .via_ip(ip),
        )
    };
    assert_eq!(
        guess("203.0.113.9", "nope-1").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        guess("203.0.113.9", "nope-2").await.status,
        StatusCode::UNAUTHORIZED
    );
    let r = guess("203.0.113.9", "link-secret-1").await;
    assert_eq!(
        r.status,
        StatusCode::TOO_MANY_REQUESTS,
        "locked even for the right password"
    );
    assert!(retry_after(&r) >= 1);
    assert_eq!(
        guess("203.0.113.10", "link-secret-1").await.status,
        StatusCode::OK,
        "another address can still open the link"
    );
}

#[tokio::test]
async fn link_creation_is_rate_limited_per_user() {
    let e = env_with(|c| c.link_create_rate = rate(2, 3600));
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let other = add_user(&e, &admin, "acme", "bob@acme.test").await;
    let doc = upload(&e, &admin, "a.bin", b"x").await;
    let body = json!({"scope": "view", "audience": "org"});
    for _ in 0..2 {
        let _ = new_link(&e, &admin, &doc, body.clone()).await;
    }
    let r = send(
        &e,
        req(Method::POST, format!("/api/v1/documents/{doc}/links"))
            .auth(&admin)
            .json(body.clone()),
    )
    .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(retry_after(&r) >= 1);
    // The budget is the user's own.
    let doc2 = upload(&e, &other, "b.bin", b"y").await;
    let _ = new_link(&e, &other, &doc2, body).await;
}

#[tokio::test]
async fn limiter_state_is_in_its_own_file_with_keyed_pseudonyms_only() {
    let e = env_with(|_| {});
    make_tenant(&e, "acme", "admin@acme.test").await;
    let _ = login_from(&e, "203.0.113.77", WRONG).await;
    let _ = login_from(&e, "203.0.113.77", PW).await;
    let main = rusqlite::Connection::open(e.dir.path().join("open365.db")).expect("open main");
    let n: i64 = main
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'rl_%'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 0, "the main database holds no limiter tables");
    let c = rusqlite::Connection::open(e.dir.path().join("ratelimit.db")).expect("open");
    let dump: String = c
        .query_row(
            "SELECT IFNULL((SELECT group_concat(key) FROM rl_events), '') || IFNULL((SELECT group_concat(account || ip) FROM rl_known), '')",
            [],
            |r| r.get(0),
        )
        .expect("dump");
    assert!(!dump.is_empty());
    assert!(!dump.contains("203.0.113") && !dump.contains("admin@acme"));
    let plain = open365_server::util::sha256_hex(b"loginip:203.0.113.77");
    assert!(
        !dump.contains(&plain),
        "keys are keyed, not a bare SHA-256 of a guessable string"
    );
}

fn set_cookies(r: &Resp) -> Vec<String> {
    r.headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn a_signed_device_cookie_keeps_the_owner_out_of_the_account_cap_on_a_new_address() {
    let e = env_with(|c| {
        c.login_fail_rate = rate(100, 900);
        c.login_ip_rate = rate(1000, 60);
        c.login_account_rate = rate(3, 900);
    });
    make_tenant(&e, "acme", "admin@acme.test").await;
    let r = login_from(&e, "198.51.100.4", PW).await;
    assert_eq!(r.status, StatusCode::OK);
    let cookie = set_cookies(&r)
        .into_iter()
        .find(|c| c.starts_with("__Host-"))
        .expect("known-device cookie");
    for attr in ["HttpOnly", "Secure", "Path=/", "SameSite=Strict"] {
        assert!(cookie.contains(attr), "{attr} in {cookie}");
    }
    assert!(!cookie.contains("Domain"), "__Host- cookies are host-only");
    let pair = cookie.split(';').next().expect("pair").to_string();
    // A botnet exhausts the account-wide cap from unknown addresses.
    for i in 0..3 {
        let _ = login_from(&e, &format!("192.0.2.{}", 10 + i), WRONG).await;
    }
    let with = |ip: &'static str, cookie: String| {
        send(
            &e,
            req(Method::POST, "/api/v1/login")
                .json(json!({"tenant": "acme", "email": "admin@acme.test", "password": PW}))
                .hdr("cookie", &cookie)
                .via_ip(ip),
        )
    };
    assert_eq!(
        login_from(&e, "203.0.113.200", PW).await.status,
        StatusCode::TOO_MANY_REQUESTS,
        "a new address without the cookie is held back"
    );
    assert_eq!(
        with("203.0.113.200", pair.clone()).await.status,
        StatusCode::OK,
        "the owner's device is recognised on a new address"
    );
    // A forged, truncated or other-account cookie does not exempt.
    let (name, value) = pair.split_once('=').expect("pair");
    let (exp, mac) = value.split_once('.').expect("shape");
    let forged = format!("{name}={}.{mac}", exp.parse::<i64>().expect("exp") + 1000);
    assert_eq!(
        with("203.0.113.201", forged).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        with(
            "203.0.113.202",
            format!("{name}={exp}.{}", "0".repeat(mac.len()))
        )
        .await
        .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        with("203.0.113.203", format!("{name}=garbage"))
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_guesses_from_unknown_addresses_cannot_exceed_the_account_cap() {
    let e = std::sync::Arc::new(env_with(|c| {
        c.login_fail_rate = rate(100, 900);
        c.login_ip_rate = rate(1000, 60);
        c.login_account_rate = rate(5, 900);
    }));
    make_tenant(&e, "acme", "admin@acme.test").await;
    let mut tasks = vec![];
    for i in 0..40 {
        let e = e.clone();
        tasks.push(tokio::spawn(async move {
            login_from(&e, &format!("192.0.2.{}", 10 + i), WRONG)
                .await
                .status
        }));
    }
    let (mut checked, mut limited) = (0, 0);
    for t in tasks {
        match t.await.expect("task") {
            StatusCode::UNAUTHORIZED => checked += 1,
            StatusCode::TOO_MANY_REQUESTS => limited += 1,
            other => panic!("unexpected {other}"),
        }
    }
    assert!(checked <= 5, "{checked} guesses were verified, cap is 5");
    assert_eq!(checked + limited, 40);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_probes_of_unknown_links_cannot_exceed_the_failure_limit() {
    let e = std::sync::Arc::new(env_with(|c| c.link_fail_rate = rate(3, 300)));
    let tok = "a".repeat(43);
    let mut tasks = vec![];
    for _ in 0..30 {
        let (e, tok) = (e.clone(), tok.clone());
        tasks.push(tokio::spawn(async move {
            send(
                &e,
                req(Method::GET, format!("/s/{tok}")).via_ip("203.0.113.9"),
            )
            .await
            .status
        }));
    }
    let mut resolved = 0;
    for t in tasks {
        if t.await.expect("task") == StatusCode::NOT_FOUND {
            resolved += 1;
        }
    }
    assert!(
        resolved <= 3,
        "{resolved} lookups were resolved, limit is 3"
    );
}

#[tokio::test]
async fn a_busy_main_database_does_not_turn_into_refusals() {
    let e = env_with(|_| {});
    make_tenant(&e, "acme", "admin@acme.test").await;
    // Another process holds the write lock on the main database.
    let hold = rusqlite::Connection::open(e.dir.path().join("open365.db")).expect("open");
    hold.execute_batch("BEGIN IMMEDIATE").expect("lock");
    let t = std::time::Instant::now();
    let r = send(&e, req(Method::GET, "/s/short").via_ip("203.0.113.9")).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "not 429 and not an error");
    assert!(t.elapsed() < std::time::Duration::from_secs(1));
    hold.execute_batch("ROLLBACK").expect("unlock");
}

#[tokio::test]
async fn a_slow_limiter_store_does_not_stall_the_async_executor() {
    let e = env_with(|_| {});
    make_tenant(&e, "acme", "admin@acme.test").await;
    // Hold the limiter's write lock for a while from another thread.
    let path = e.dir.path().join("ratelimit.db");
    let (tx, rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let c = rusqlite::Connection::open(path).expect("open");
        c.execute_batch("BEGIN IMMEDIATE").expect("lock");
        tx.send(()).expect("signal");
        std::thread::sleep(std::time::Duration::from_millis(800));
        c.execute_batch("COMMIT").expect("commit");
    });
    rx.recv().expect("locked");
    // Single-threaded runtime: if the limiter blocked the worker, this ticker could not run.
    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let t2 = ticks.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            t2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let r = login_from(&e, "203.0.113.5", PW).await;
    ticker.abort();
    holder.join().expect("holder");
    assert_eq!(
        r.status,
        StatusCode::OK,
        "waited for the store, then proceeded"
    );
    let n = ticks.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        n >= 20,
        "the executor kept running while the store was busy ({n} ticks)"
    );
}

#[tokio::test]
async fn forwarded_entries_with_ports_from_a_trusted_proxy_key_the_real_client() {
    let e = env_with(|c| {
        c.trusted_proxies = Cidr::parse_list("10.0.0.0/8").expect("cidr");
        c.login_ip_rate = rate(1, 60);
    });
    make_tenant(&e, "acme", "admin@acme.test").await;
    let via = |xff: &'static str| {
        send(
            &e,
            req(Method::POST, "/api/v1/login")
                .json(json!({"tenant": "acme", "email": "admin@acme.test", "password": PW}))
                .via_ip("10.0.0.1")
                .hdr("x-forwarded-for", xff),
        )
    };
    assert_eq!(via("203.0.113.1:50001").await.status, StatusCode::OK);
    assert_eq!(
        via("203.0.113.1:50002").await.status,
        StatusCode::TOO_MANY_REQUESTS,
        "same client, new port"
    );
    assert_eq!(
        via("203.0.113.2:50001").await.status,
        StatusCode::OK,
        "another client has its own bucket"
    );
    assert_eq!(via("[2001:db8:5:6::1]:443").await.status, StatusCode::OK);
}

#[tokio::test]
async fn key_status_counts_records_per_key_id_until_rotation_and_audit_stays_verifiable() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    // Enrol a TOTP secret (sealed under test-s1).
    let r = send(
        &e,
        req(Method::POST, "/api/v1/me/totp/enroll")
            .auth(&admin)
            .json(json!({})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let u = ops::key_status(e.dir.path()).expect("status");
    assert_eq!(u.secrets, vec![("test-s1".to_string(), 1)]);
    assert!(u.audit.iter().all(|(id, _)| id == "test-key") && !u.audit.is_empty());
    // Rotate the audit key (new key last) and restart: old rows verify, new rows use the new key.
    let rotated = AuditKeys::list(vec![
        e.state.audit.keys[0].clone(),
        ("test-key-2".into(), vec![9; 32]),
    ])
    .expect("keys");
    let st2 = AppState::new(e.state.cfg.clone(), rotated.clone()).expect("restart");
    let app2 = router(st2);
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/login")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"tenant": "acme", "email": "admin@acme.test", "password": PW}).to_string(),
        ))
        .expect("request");
    request.extensions_mut().insert(ConnectInfo(
        "203.0.113.1:4000"
            .parse::<std::net::SocketAddr>()
            .expect("addr"),
    ));
    assert_eq!(
        app2.oneshot(request).await.expect("res").status(),
        StatusCode::OK
    );
    let u = ops::key_status(e.dir.path()).expect("status");
    assert!(u.audit.iter().any(|(id, _)| id == "test-key"));
    assert!(
        u.audit.iter().any(|(id, _)| id == "test-key-2"),
        "{:?}",
        u.audit
    );
    for (chain, v) in ops::audit_verify_all(e.dir.path(), &rotated).expect("verify") {
        assert!(v.ok, "chain {chain} must verify across the rotation");
    }
    // Without the old key the old rows no longer verify: the old id must stay listed.
    let only_new = AuditKeys::single("test-key-2", vec![9; 32]).expect("keys");
    assert!(
        ops::audit_verify_all(e.dir.path(), &only_new)
            .expect("verify")
            .iter()
            .any(|(_, v)| !v.ok)
    );
}
