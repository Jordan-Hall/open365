//! Tests for the hardening and feature work: timeouts and limits, trusted proxies, sign-in lockout under
//! concurrency, audit of failed sign-ins, delete/purge/transfer, `specific` links, TOTP, backup/restore.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{Method, StatusCode};
use common::*;
use open365_server::config::{Argon2Cost, Config, Rate};
use open365_server::proxy::Cidr;
use open365_server::{AppState, ops, router, totp};
use serde_json::json;

fn now() -> i64 {
    open365_server::util::now()
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
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

// ---- M1, M2: sign-in limits under concurrency ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_wrong_passwords_cannot_exceed_the_lockout() {
    let e = Arc::new(env_with(|c| {
        c.login_fail_rate = Rate {
            max: 5,
            window_secs: 900,
        }
    }));
    make_tenant(&e, "acme", "admin@acme.test").await;
    let mut tasks = vec![];
    for i in 0..40 {
        let e = e.clone();
        tasks.push(tokio::spawn(async move {
            send(
                &e,
                req(Method::POST, "/api/v1/login").json(
                    json!({"tenant": "acme", "email": "admin@acme.test", "password": format!("wrong-guess-{i}")}),
                ),
            )
            .await
            .status
        }));
    }
    let mut checked = 0;
    let mut limited = 0;
    for t in tasks {
        match t.await.expect("task") {
            StatusCode::UNAUTHORIZED => checked += 1,
            StatusCode::TOO_MANY_REQUESTS => limited += 1,
            other => panic!("unexpected {other}"),
        }
    }
    assert!(checked <= 5, "{checked} guesses were verified, limit is 5");
    assert_eq!(checked + limited, 40);
    // Still locked, even for the right password.
    let r = send(
        &e,
        req(Method::POST, "/api/v1/login")
            .json(json!({"tenant": "acme", "email": "admin@acme.test", "password": PW})),
    )
    .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_link_password_guesses_cannot_exceed_the_limit() {
    let e = Arc::new(env_with(|c| {
        c.link_password_fail_rate = Rate {
            max: 3,
            window_secs: 900,
        }
    }));
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let doc = upload(&e, &admin, "a.bin", b"secret bytes").await;
    let (_, tok) = new_link(
        &e,
        &admin,
        &doc,
        json!({"scope": "view", "audience": "anyone", "password": "link-secret-1"}),
    )
    .await;
    let mut tasks = vec![];
    for i in 0..30 {
        let (e, tok) = (e.clone(), tok.clone());
        tasks.push(tokio::spawn(async move {
            send(
                &e,
                req(Method::GET, format!("/s/{tok}"))
                    .hdr("x-share-password", &format!("guess-{i}"))
                    .via_ip("203.0.113.9"),
            )
            .await
            .status
        }));
    }
    let mut verified = 0;
    for t in tasks {
        if t.await.expect("task") == StatusCode::UNAUTHORIZED {
            verified += 1;
        }
    }
    assert!(
        verified <= 3,
        "{verified} guesses were verified, limit is 3"
    );
}

#[tokio::test]
async fn every_sign_in_attempt_counts_against_the_address() {
    let e = env_with(|c| {
        c.login_ip_rate = Rate {
            max: 4,
            window_secs: 60,
        }
    });
    make_tenant(&e, "acme", "admin@acme.test").await;
    let mut codes = vec![];
    for _ in 0..7 {
        codes.push(
            send(
                &e,
                req(Method::POST, "/api/v1/login")
                    .json(json!({"tenant": "acme", "email": "admin@acme.test", "password": PW}))
                    .via_ip("198.51.100.20"),
            )
            .await
            .status,
        );
    }
    assert_eq!(codes.iter().filter(|c| **c == StatusCode::OK).count(), 4);
    assert_eq!(
        codes
            .iter()
            .filter(|c| **c == StatusCode::TOO_MANY_REQUESTS)
            .count(),
        3
    );
    let other = send(
        &e,
        req(Method::POST, "/api/v1/login")
            .json(json!({"tenant": "acme", "email": "admin@acme.test", "password": PW}))
            .via_ip("198.51.100.21"),
    )
    .await;
    assert_eq!(
        other.status,
        StatusCode::OK,
        "another address has its own budget"
    );
    assert!(e.state.argon_sem.available_permits() >= 1);
}

// ---- M3 and item 1: limits and timeouts ----

#[tokio::test]
async fn json_routes_have_a_small_body_limit_but_uploads_do_not() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let huge = "x".repeat(100_000);
    let r = send(
        &e,
        req(Method::POST, "/api/v1/login")
            .json(json!({"tenant": "acme", "email": huge, "password": "p"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.json()["error"], "too_large");
    let big = vec![9u8; 300_000];
    let doc = upload(&e, &admin, "big.bin", &big).await;
    let d = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{doc}/content")).auth(&admin),
    )
    .await;
    assert_eq!(d.body, big);
}

#[tokio::test]
async fn overlong_identifiers_are_rejected_before_use() {
    let e = env();
    make_tenant(&e, "acme", "admin@acme.test").await;
    for body in [
        json!({"tenant": "acme", "email": format!("{}@x.test", "a".repeat(300)), "password": "p"}),
        json!({"tenant": "t".repeat(100), "email": "a@x.test", "password": "p"}),
    ] {
        let r = send(&e, req(Method::POST, "/api/v1/login").json(body)).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
    }
}

struct Never;
impl hyper::body::Body for Never {
    type Data = Bytes;
    type Error = std::convert::Infallible;
    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        std::task::Poll::Pending
    }
}

#[tokio::test]
async fn stalled_requests_time_out() {
    let e = env_with(|c| {
        c.request_timeout_secs = 1;
        c.upload_timeout_secs = 2;
    });
    let started = std::time::Instant::now();
    let r = send_body(
        &e,
        req(Method::POST, "/api/v1/login").hdr("content-type", "application/json"),
        Some(Body::new(Never)),
    )
    .await;
    assert_eq!(r.status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(r.json()["error"], "timeout");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "short limit applies to JSON routes"
    );
    // Upload routes get the long limit.
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let started = std::time::Instant::now();
    let r = send_body(
        &e,
        req(Method::POST, "/api/v1/documents?name=a.bin").auth(&admin),
        Some(Body::new(Never)),
    )
    .await;
    assert_eq!(r.status, StatusCode::REQUEST_TIMEOUT);
    assert!(started.elapsed() >= Duration::from_millis(1900));
}

// ---- item 2 and M5: trusted proxies ----

fn bad_token() -> String {
    "a".repeat(43)
}

#[tokio::test]
async fn forwarded_for_is_ignored_unless_the_peer_is_a_trusted_proxy() {
    let e = env_with(|c| {
        c.link_fail_rate = Rate {
            max: 3,
            window_secs: 300,
        }
    });
    let t = bad_token();
    let mut seen = vec![];
    for i in 0..5 {
        let r = send(
            &e,
            req(Method::GET, format!("/s/{t}"))
                .via_ip("10.0.0.1")
                .hdr("x-forwarded-for", &format!("203.0.113.{i}")),
        )
        .await;
        seen.push(r.status);
    }
    assert_eq!(
        seen,
        [404, 404, 404, 429, 429].map(|c| StatusCode::from_u16(c).expect("status")),
        "a spoofed header does not buy a fresh bucket"
    );
}

#[tokio::test]
async fn trusted_proxy_gives_each_real_client_its_own_bucket() {
    let e = env_with(|c| {
        c.trusted_proxies = Cidr::parse_list("10.0.0.0/8").expect("cidr");
        c.link_fail_rate = Rate {
            max: 3,
            window_secs: 300,
        }
    });
    let t = bad_token();
    let hit = |client: &'static str| {
        send(
            &e,
            req(Method::GET, format!("/s/{t}"))
                .via_ip("10.0.0.1")
                .hdr("x-forwarded-for", &format!("6.6.6.6, {client}")),
        )
    };
    for _ in 0..3 {
        assert_eq!(hit("203.0.113.50").await.status, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        hit("203.0.113.50").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        hit("203.0.113.51").await.status,
        StatusCode::NOT_FOUND,
        "another real client is unaffected"
    );
}

// ---- item 3: failed sign-ins in the tenant chain ----

#[tokio::test]
async fn failed_sign_ins_are_audited_on_the_tenant_chain_when_known() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    for (tenant, email) in [
        ("acme", "admin@acme.test"),
        ("acme", "ghost@acme.test"),
        ("nope", "x@y.test"),
    ] {
        let r = send(
            &e,
            req(Method::POST, "/api/v1/login")
                .json(json!({"tenant": tenant, "email": email, "password": "wrong-password-1"})),
        )
        .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    let failed = audit_actions(&e, &admin)
        .await
        .iter()
        .filter(|a| *a == "login.failed")
        .count();
    assert_eq!(failed, 2, "known tenant: wrong password and unknown user");
    let system: i64 = e
        .state
        .db
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM audit WHERE tenant_id = '-' AND action = 'login.failed'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(system, 1, "an unknown tenant lands on the system chain");
    let v = send(&e, req(Method::GET, "/api/v1/audit/verify").auth(&admin)).await;
    assert_eq!(v.json()["ok"], true);
}

// ---- L7, L8, L9 ----

#[tokio::test]
async fn token_is_in_the_body_only_in_bearer_mode_and_the_cookie_works() {
    let e = env();
    make_tenant(&e, "acme", "admin@acme.test").await;
    let r = send(
        &e,
        req(Method::POST, "/api/v1/login")
            .json(json!({"tenant": "acme", "email": "admin@acme.test", "password": PW})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.json().get("token").is_none(), "{:?}", r.json());
    let cookie = r.headers["set-cookie"].to_str().expect("cookie");
    let pair = cookie.split(';').next().expect("pair").to_string();
    let me = send(&e, req(Method::GET, "/api/v1/me").hdr("cookie", &pair)).await;
    assert_eq!(me.status, StatusCode::OK);
}

#[tokio::test]
async fn hsts_is_sent_only_when_configured() {
    let plain = env();
    let r = send(&plain, req(Method::GET, "/healthz")).await;
    assert!(r.headers.get("strict-transport-security").is_none());
    let tls = env_with(|c| c.hsts = true);
    let r = send(&tls, req(Method::GET, "/healthz")).await;
    assert!(
        r.headers["strict-transport-security"]
            .to_str()
            .expect("hsts")
            .contains("max-age=")
    );
}

#[cfg(unix)]
#[test]
fn data_files_are_private_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let e = env();
    let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode(e.dir.path().join("open365.db").as_path()), 0o600);
    assert_eq!(mode(e.dir.path().join("blobs").as_path()), 0o700);
}

// ---- M4: anonymous writers ----

#[tokio::test]
async fn edit_links_have_an_upload_rate_and_a_byte_budget() {
    let e = env_with(|c| {
        c.link_upload_rate = Rate {
            max: 2,
            window_secs: 3600,
        }
    });
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let doc = upload(&e, &admin, "a.bin", b"v1").await;
    let (_, tok) = new_link(
        &e,
        &admin,
        &doc,
        json!({"scope": "edit", "audience": "anyone"}),
    )
    .await;
    let put = |n: u8| {
        send(
            &e,
            req(Method::PUT, format!("/s/{tok}/content")).bytes(&[n; 10]),
        )
    };
    assert_eq!(put(1).await.status, StatusCode::OK);
    assert_eq!(put(2).await.status, StatusCode::OK);
    assert_eq!(put(3).await.status, StatusCode::TOO_MANY_REQUESTS);

    let e = env_with(|c| c.link_upload_bytes_per_hour = 100);
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let doc = upload(&e, &admin, "a.bin", b"v1").await;
    let (_, tok) = new_link(
        &e,
        &admin,
        &doc,
        json!({"scope": "edit", "audience": "anyone"}),
    )
    .await;
    let big = |n: u8| {
        send(
            &e,
            req(Method::PUT, format!("/s/{tok}/content")).bytes(&[n; 60]),
        )
    };
    assert_eq!(big(1).await.status, StatusCode::OK);
    assert_eq!(
        big(2).await.status,
        StatusCode::TOO_MANY_REQUESTS,
        "budget of 100 bytes per hour"
    );
    // The owner is not subject to the link's budget.
    let v = send(
        &e,
        req(Method::PUT, format!("/api/v1/documents/{doc}"))
            .auth(&admin)
            .bytes(&[7; 500]),
    )
    .await;
    assert_eq!(v.status, StatusCode::OK);
}

#[tokio::test]
async fn blob_is_written_without_the_database_lock() {
    let e = env();
    let st = e.state.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _g = st.db.lock();
        tx.send(()).expect("signal");
        std::thread::sleep(Duration::from_millis(1500));
    });
    rx.recv().expect("lock taken");
    let r = tokio::time::timeout(
        Duration::from_millis(700),
        e.state.store_blob(
            "00000000000000000000000000000001",
            Bytes::from_static(b"abc"),
        ),
    )
    .await;
    assert!(r.is_ok(), "store_blob must not wait for the database lock");
    holder.join().expect("join");
}

// ---- item 4: delete, purge, transfer ----

#[tokio::test]
async fn soft_delete_restore_and_purge() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let bob = add_user(&e, &admin, "acme", "bob@acme.test").await;
    let owner = add_user(&e, &admin, "acme", "own@acme.test").await;
    let doc = upload(&e, &owner, "Plan.docx", b"shared content").await;
    let twin = upload(&e, &owner, "Copy.docx", b"shared content").await;
    let (_, tok) = new_link(
        &e,
        &owner,
        &doc,
        json!({"scope": "view", "audience": "anyone"}),
    )
    .await;
    let del = |tok: &str, id: &str| {
        send(
            &e,
            req(Method::DELETE, format!("/api/v1/documents/{id}")).auth(tok),
        )
    };

    assert_eq!(
        del(&bob, &doc).await.status,
        StatusCode::NOT_FOUND,
        "no access: invisible"
    );
    assert_eq!(del(&owner, &doc).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        send(
            &e,
            req(Method::GET, format!("/api/v1/documents/{doc}")).auth(&owner)
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&e, req(Method::GET, format!("/s/{tok}"))).await.status,
        StatusCode::NOT_FOUND
    );
    let list = send(&e, req(Method::GET, "/api/v1/documents").auth(&owner))
        .await
        .json();
    assert_eq!(list.as_array().expect("list").len(), 1);
    let trash = send(&e, req(Method::GET, "/api/v1/documents/trash").auth(&owner))
        .await
        .json();
    assert_eq!(trash[0]["id"], doc.as_str());

    let restore = |tok: &str| {
        send(
            &e,
            req(Method::POST, format!("/api/v1/documents/{doc}/restore"))
                .auth(tok)
                .json(json!({})),
        )
    };
    assert_eq!(
        restore(&bob).await.status,
        StatusCode::NOT_FOUND,
        "only the owner or an admin"
    );
    assert_eq!(restore(&owner).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        send(&e, req(Method::GET, format!("/s/{tok}"))).await.status,
        StatusCode::OK
    );

    let purge = |tok: &str, id: &str| {
        send(
            &e,
            req(Method::POST, format!("/api/v1/documents/{id}/purge"))
                .auth(tok)
                .json(json!({})),
        )
    };
    assert_eq!(
        purge(&owner, &doc).await.status,
        StatusCode::NOT_FOUND,
        "must be deleted first"
    );
    assert_eq!(del(&owner, &doc).await.status, StatusCode::NO_CONTENT);
    assert_eq!(purge(&bob, &doc).await.status, StatusCode::NOT_FOUND);
    assert_eq!(purge(&owner, &doc).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        send(&e, req(Method::GET, format!("/s/{tok}"))).await.status,
        StatusCode::NOT_FOUND
    );
    let trash = send(&e, req(Method::GET, "/api/v1/documents/trash").auth(&owner))
        .await
        .json();
    assert!(trash.as_array().expect("trash").is_empty());
    // The twin shares the content-addressed blob, which must survive.
    let d = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{twin}/content")).auth(&owner),
    )
    .await;
    assert_eq!(d.body, b"shared content");
    assert_eq!(walk(&e.dir.path().join("blobs")).len(), 1);
    // Purging the last user of the blob removes the file.
    assert_eq!(del(&owner, &twin).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        purge(&admin, &twin).await.status,
        StatusCode::NO_CONTENT,
        "admins may purge"
    );
    assert!(walk(&e.dir.path().join("blobs")).is_empty());

    let actions = audit_actions(&e, &admin).await;
    for a in ["document.delete", "document.restore", "document.purge"] {
        assert!(
            actions.iter().any(|x| x == a),
            "{a} missing from {actions:?}"
        );
    }
    let v = send(&e, req(Method::GET, "/api/v1/audit/verify").auth(&admin)).await;
    assert_eq!(v.json()["ok"], true);
}

#[tokio::test]
async fn ownership_transfer_checks_acl_and_audits() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let alice = add_user(&e, &admin, "acme", "alice@acme.test").await;
    let bob = add_user(&e, &admin, "acme", "bob@acme.test").await;
    let other = make_tenant(&e, "globex", "root@globex.test").await;
    let doc = upload(&e, &alice, "Plan.docx", b"x").await;
    let xfer = |tok: &str, email: &str| {
        send(
            &e,
            req(Method::POST, format!("/api/v1/documents/{doc}/transfer"))
                .auth(tok)
                .json(json!({"email": email})),
        )
    };
    assert_eq!(
        xfer(&bob, "bob@acme.test").await.status,
        StatusCode::NOT_FOUND,
        "no access"
    );
    let acl = send(
        &e,
        req(Method::PUT, format!("/api/v1/documents/{doc}/acl"))
            .auth(&alice)
            .json(json!({"email": "bob@acme.test", "role": "editor"})),
    )
    .await;
    assert_eq!(acl.status, StatusCode::NO_CONTENT);
    assert_eq!(
        xfer(&bob, "bob@acme.test").await.status,
        StatusCode::FORBIDDEN,
        "editors cannot"
    );
    assert_eq!(other_status(&e, &other, &doc).await, StatusCode::NOT_FOUND);
    assert_eq!(
        xfer(&alice, "ghost@acme.test").await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        xfer(&alice, "root@globex.test").await.status,
        StatusCode::BAD_REQUEST,
        "other tenant"
    );
    assert_eq!(
        xfer(&alice, "alice@acme.test").await.status,
        StatusCode::BAD_REQUEST,
        "already owner"
    );
    assert_eq!(
        xfer(&alice, "bob@acme.test").await.status,
        StatusCode::NO_CONTENT
    );

    let meta = |tok: &str| {
        send(
            &e,
            req(Method::GET, format!("/api/v1/documents/{doc}")).auth(tok),
        )
    };
    assert_eq!(meta(&bob).await.json()["role"], "owner");
    assert_eq!(
        meta(&alice).await.json()["role"],
        "editor",
        "previous owner keeps editing"
    );
    assert_eq!(
        xfer(&alice, "alice@acme.test").await.status,
        StatusCode::FORBIDDEN
    );
    let acl = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{doc}/acl")).auth(&bob),
    )
    .await
    .json();
    assert_eq!(acl[0]["role"], "owner");
    assert_eq!(acl[0]["email"], "bob@acme.test");
    assert!(
        audit_actions(&e, &admin)
            .await
            .iter()
            .any(|a| a == "document.transfer")
    );
}

async fn other_status(e: &Env, tok: &str, doc: &str) -> StatusCode {
    send(
        e,
        req(Method::POST, format!("/api/v1/documents/{doc}/transfer"))
            .auth(tok)
            .json(json!({"email": "root@globex.test"})),
    )
    .await
    .status
}

// ---- item 5: specific audience ----

#[tokio::test]
async fn specific_links_open_only_for_the_named_people() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let bob = add_user(&e, &admin, "acme", "bob@acme.test").await;
    let carol = add_user(&e, &admin, "acme", "carol@acme.test").await;
    let dave = add_user(&e, &admin, "acme", "dave@acme.test").await;
    let outsider = make_tenant(&e, "globex", "bob@acme.test").await;
    let doc = upload(&e, &admin, "Plan.docx", b"for two people").await;
    let mk = |body: serde_json::Value| {
        send(
            &e,
            req(Method::POST, format!("/api/v1/documents/{doc}/links"))
                .auth(&admin)
                .json(body),
        )
    };
    for bad in [
        json!({"scope": "view", "audience": "specific"}),
        json!({"scope": "view", "audience": "specific", "emails": []}),
        json!({"scope": "view", "audience": "org", "emails": ["bob@acme.test"]}),
        json!({"scope": "view", "audience": "specific", "emails": ["nobody@acme.test"]}),
    ] {
        assert_eq!(mk(bad).await.status, StatusCode::BAD_REQUEST);
    }
    let r = mk(json!({"scope": "view", "audience": "specific", "emails": ["bob@acme.test", "Carol@acme.test"]})).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json());
    let tok = r.json()["token"].as_str().expect("token").to_string();
    let open = |t: Option<&str>| {
        let mut rq = req(Method::GET, format!("/s/{tok}/content"));
        if let Some(t) = t {
            rq = rq.auth(t);
        }
        send(&e, rq)
    };
    let anon = open(None).await;
    assert_eq!(anon.status, StatusCode::NOT_FOUND);
    for named in [&bob, &carol] {
        let r = open(Some(named)).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.body, b"for two people");
    }
    let dave_r = open(Some(&dave)).await;
    assert_eq!(dave_r.status, StatusCode::NOT_FOUND);
    assert_eq!(dave_r.body, anon.body, "same answer as an unknown link");
    assert_eq!(
        open(Some(&outsider)).await.status,
        StatusCode::NOT_FOUND,
        "same email, other tenant"
    );
    let list = send(
        &e,
        req(Method::GET, format!("/api/v1/documents/{doc}/links")).auth(&admin),
    )
    .await
    .json();
    assert_eq!(list[0]["audience"], "specific");
    assert_eq!(
        list[0]["emails"],
        json!(["bob@acme.test", "carol@acme.test"])
    );
}

// ---- item 7: TOTP ----

#[tokio::test]
async fn totp_second_factor_for_local_accounts() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let bob = add_user(&e, &admin, "acme", "bob@acme.test").await;
    let enroll = send(
        &e,
        req(Method::POST, "/api/v1/me/totp/enroll")
            .auth(&bob)
            .json(json!({})),
    )
    .await;
    assert_eq!(enroll.status, StatusCode::OK);
    let secret = enroll.json()["secret"]
        .as_str()
        .expect("secret")
        .to_string();
    assert!(
        enroll.json()["otpauth_uri"]
            .as_str()
            .expect("uri")
            .starts_with("otpauth://totp/")
    );
    // Not active until confirmed: sign-in still needs no code.
    let login_with = |code: Option<String>| {
        let mut b =
            json!({"tenant": "acme", "email": "bob@acme.test", "password": PW, "bearer": true});
        if let Some(c) = code {
            b["totp"] = json!(c);
        }
        send(&e, req(Method::POST, "/api/v1/login").json(b))
    };
    assert_eq!(login_with(None).await.status, StatusCode::OK);
    let bad = send(
        &e,
        req(Method::POST, "/api/v1/me/totp/confirm")
            .auth(&bob)
            .json(json!({"code": "12ab56"})),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let t = now();
    let ok = send(
        &e,
        req(Method::POST, "/api/v1/me/totp/confirm")
            .auth(&bob)
            .json(json!({"code": totp::code_at(&secret, t).expect("code")})),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK);

    let missing = login_with(None).await;
    assert_eq!(missing.status, StatusCode::UNAUTHORIZED);
    assert_eq!(missing.json()["error"], "totp_required");
    let far = totp::code_at(&secret, t + 3600).expect("code");
    assert_eq!(login_with(Some(far)).await.json()["error"], "invalid_code");
    // The code used to confirm cannot be replayed, the next step's code works once.
    let replay = totp::code_at(&secret, t).expect("code");
    assert_eq!(
        login_with(Some(replay)).await.status,
        StatusCode::UNAUTHORIZED
    );
    let next = totp::code_at(&secret, t + 30).expect("code");
    assert_eq!(login_with(Some(next.clone())).await.status, StatusCode::OK);
    assert_eq!(
        login_with(Some(next)).await.status,
        StatusCode::UNAUTHORIZED,
        "single use"
    );
    assert_eq!(
        send(
            &e,
            req(Method::POST, "/api/v1/me/totp/enroll")
                .auth(&bob)
                .json(json!({}))
        )
        .await
        .status,
        StatusCode::CONFLICT
    );

    // Recovery: only an admin of the same tenant can clear it.
    let me = send(&e, req(Method::GET, "/api/v1/me").auth(&bob))
        .await
        .json();
    let uid = me["user_id"].as_str().expect("id").to_string();
    let reset = |tok: &str| {
        send(
            &e,
            req(Method::DELETE, format!("/api/v1/users/{uid}/totp")).auth(tok),
        )
    };
    assert_eq!(reset(&bob).await.status, StatusCode::FORBIDDEN);
    let other = make_tenant(&e, "globex", "root@globex.test").await;
    assert_eq!(reset(&other).await.status, StatusCode::NOT_FOUND);
    assert_eq!(reset(&admin).await.status, StatusCode::NO_CONTENT);
    assert_eq!(login_with(None).await.status, StatusCode::OK);
    assert!(
        audit_actions(&e, &admin)
            .await
            .iter()
            .any(|a| a == "totp.reset")
    );
}

// ---- item 8: backup and restore ----

fn cfg_for(dir: &Path) -> Config {
    let mut c = Config::new(dir);
    c.argon2 = Argon2Cost {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    c
}

#[tokio::test]
async fn backup_then_restore_reproduces_everything_and_rejects_damage() {
    let e = env();
    let admin = make_tenant(&e, "acme", "admin@acme.test").await;
    let doc = upload(&e, &admin, "Budget.xlsx", b"first").await;
    let _ = send(
        &e,
        req(Method::PUT, format!("/api/v1/documents/{doc}"))
            .auth(&admin)
            .bytes(b"second"),
    )
    .await;
    let (_, tok) = new_link(
        &e,
        &admin,
        &doc,
        json!({"scope": "view", "audience": "anyone"}),
    )
    .await;

    let scratch = tempfile::tempdir().expect("tmp");
    let backup = scratch.path().join("backup");
    let report = ops::backup(e.dir.path(), &backup).expect("backup");
    assert_eq!(report.versions, 2);
    assert!(backup.join("open365.db").is_file());
    assert!(
        ops::backup(e.dir.path(), &backup).is_err(),
        "never into a non-empty directory"
    );

    // Changes after the backup are not in it.
    let _ = upload(&e, &admin, "Later.xlsx", b"after the backup").await;

    let restored = scratch.path().join("restored");
    let rep = ops::restore(&backup, &restored, Some(&e.state.audit)).expect("restore");
    assert_eq!(rep.versions, 2);
    assert!(
        ops::restore(&backup, &restored, None).is_err(),
        "never over an existing database"
    );
    for (chain, v) in ops::audit_verify_all(&restored, &e.state.audit).expect("verify") {
        assert!(v.ok, "{chain}");
    }

    let state = AppState::new(cfg_for(&restored), e.state.audit.clone()).expect("state");
    let e2 = Env {
        app: router(state.clone()),
        state,
        dir: tempfile::tempdir().expect("tmp"),
        bootstrap: String::new(),
    };
    let tok2 = login(&e2, "acme", "admin@acme.test").await;
    let list = send(&e2, req(Method::GET, "/api/v1/documents").auth(&tok2))
        .await
        .json();
    assert_eq!(
        list.as_array().expect("list").len(),
        1,
        "the post-backup upload is absent"
    );
    let d = send(
        &e2,
        req(
            Method::GET,
            format!("/api/v1/documents/{doc}/content?version=1"),
        )
        .auth(&tok2),
    )
    .await;
    assert_eq!(d.body, b"first");
    let via_link = send(&e2, req(Method::GET, format!("/s/{tok}/content"))).await;
    assert_eq!(via_link.body, b"second", "links survive a restore");

    // A damaged backup is refused and leaves nothing startable behind.
    let blob = walk(&backup.join("blobs")).pop().expect("a blob");
    std::fs::write(&blob, b"tampered").expect("tamper");
    let bad_target = scratch.path().join("bad");
    assert!(ops::restore(&backup, &bad_target, None).is_err());
    assert!(!bad_target.join("open365.db").exists());

    // A tampered audit chain in a backup is caught when keys are supplied.
    let backup2 = scratch.path().join("backup2");
    ops::backup(e.dir.path(), &backup2).expect("second backup");
    let conn = rusqlite::Connection::open(backup2.join("open365.db")).expect("open");
    conn.execute("UPDATE audit SET detail = 'forged' WHERE rowid = 2", [])
        .expect("tamper");
    drop(conn);
    let bad2 = scratch.path().join("bad2");
    let err = ops::restore(&backup2, &bad2, Some(&e.state.audit)).expect_err("tampered chain");
    assert!(err.contains("audit chain"), "{err}");
    assert!(!bad2.join("open365.db").exists());
}
