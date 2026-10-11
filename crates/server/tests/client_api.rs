//! The typed client (`open365_server::api`) against an in-process server on a loopback socket: what the desktop
//! Share dialog does, end to end.

mod common;

use common::*;
use open365_server::api::{
    ApiClientError, Audience, LinkScope, NewLink, Role, SecondFactor, SharingClient,
};
use open365_server::totp;

fn now() -> i64 {
    open365_server::util::now()
}

#[tokio::test(flavor = "multi_thread")]
async fn share_dialog_flow_through_the_typed_client() {
    let e = env();
    let admin_tok = make_tenant(&e, "acme", "admin@acme.test").await;
    add_user(&e, &admin_tok, "acme", "bob@acme.test").await;
    let (base, handle) = serve_env(&e).await;

    let mut c = SharingClient::new(&format!("{base}/"), &[]).expect("client");
    // Not signed in, then a wrong password.
    assert!(matches!(c.me().await, Err(ApiClientError::NotSignedIn)));
    let bad = c
        .sign_in_password("acme", "admin@acme.test", "wrong", SecondFactor::None)
        .await;
    assert_eq!(
        bad.expect_err("refused").code(),
        Some("invalid_credentials")
    );
    assert!(!c.is_signed_in());
    let session = c
        .sign_in_password("acme", "admin@acme.test", PW, SecondFactor::None)
        .await
        .expect("sign in");
    assert!(session.expires_in > 0);
    assert_eq!(c.me().await.expect("me").email, "admin@acme.test");
    assert!(
        !format!("{c:?}").contains(c.token().expect("token")),
        "Debug hides the token"
    );

    // Upload, update with optimistic concurrency, list, download.
    let saved = c
        .upload("Budget.xlsx", b"v1 bytes".to_vec())
        .await
        .expect("upload");
    assert_eq!(saved.version, 1);
    let v2 = c
        .update(&saved.id, b"v2 bytes".to_vec(), Some(1))
        .await
        .expect("update");
    assert_eq!(v2.version, 2);
    let stale = c
        .update(&saved.id, b"lost".to_vec(), Some(1))
        .await
        .expect_err("stale");
    assert!(stale.is_conflict(), "{stale}");
    let docs = c.list_documents().await.expect("list");
    assert_eq!(docs.len(), 1);
    assert_eq!(
        (
            docs[0].name.as_str(),
            docs[0].version,
            docs[0].role.as_str()
        ),
        ("Budget.xlsx", 2, "owner")
    );
    assert_eq!(
        c.download(&saved.id, None).await.expect("current"),
        b"v2 bytes"
    );
    assert_eq!(
        c.download(&saved.id, Some(1)).await.expect("v1"),
        b"v1 bytes"
    );
    assert_eq!(c.versions(&saved.id).await.expect("versions").len(), 2);
    assert_eq!(c.document(&saved.id).await.expect("doc").id, saved.id);

    // Links: create with a password and an expiry, list, open, revoke.
    let mut spec = NewLink::new(LinkScope::View, Audience::Anyone);
    spec.expires_in_secs = Some(3600);
    spec.password = Some("link passphrase".into());
    let link = c.create_link(&saved.id, &spec).await.expect("link");
    assert!(c.share_url(&link).starts_with(&base) && c.share_url(&link).contains("/s/"));
    assert!(
        !format!("{link:?}").contains(&link.token),
        "Debug hides the link token"
    );
    let listed = c.list_links(&saved.id).await.expect("links");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].has_password && listed[0].revoked_at.is_none());
    let open = send(
        &e,
        req(axum::http::Method::GET, link.url_path.clone())
            .hdr("x-share-password", "link passphrase"),
    )
    .await;
    assert_eq!(open.status, axum::http::StatusCode::OK);
    c.revoke_link(&link.id).await.expect("revoke");
    assert!(
        c.list_links(&saved.id).await.expect("links")[0]
            .revoked_at
            .is_some()
    );
    let gone = send(
        &e,
        req(axum::http::Method::GET, link.url_path.clone())
            .hdr("x-share-password", "link passphrase"),
    )
    .await;
    assert_ne!(gone.status, axum::http::StatusCode::OK);
    // A specific link needs people; the server's refusal reaches the caller as a code.
    let no_people = c
        .create_link(
            &saved.id,
            &NewLink::new(LinkScope::Edit, Audience::Specific),
        )
        .await
        .expect_err("emails");
    assert_eq!(no_people.code(), Some("invalid_emails"));
    let mut named = NewLink::new(LinkScope::Edit, Audience::Specific);
    named.emails = Some(vec!["bob@acme.test".into()]);
    let named = c.create_link(&saved.id, &named).await.expect("named link");
    assert_eq!(named.audience, Audience::Specific);

    // Access: grant, list, a second client sees the document, then remove.
    c.set_access(&saved.id, "bob@acme.test", Role::Viewer)
        .await
        .expect("grant");
    let access = c.list_access(&saved.id).await.expect("access");
    assert_eq!(access[0].role, "owner");
    let bob_entry = access
        .iter()
        .find(|a| a.email == "bob@acme.test")
        .expect("bob");
    assert_eq!(bob_entry.role, "viewer");
    let mut bob = SharingClient::new(&base, &[]).expect("bob client");
    bob.sign_in_password("acme", "bob@acme.test", PW, SecondFactor::None)
        .await
        .expect("bob in");
    assert_eq!(bob.list_documents().await.expect("bob docs").len(), 1);
    let denied = bob
        .update(&saved.id, b"x".to_vec(), None)
        .await
        .expect_err("viewer cannot write");
    assert!(
        matches!(denied, ApiClientError::Status { status: 403, .. }),
        "{denied}"
    );
    c.set_access(&saved.id, "bob@acme.test", Role::Editor)
        .await
        .expect("upgrade");
    bob.update(&saved.id, b"bob wrote".to_vec(), Some(2))
        .await
        .expect("editor writes");
    c.remove_access(&saved.id, &bob_entry.user_id)
        .await
        .expect("remove");
    assert!(bob.list_documents().await.expect("bob docs").is_empty());
    let unknown = c
        .set_access(&saved.id, "nobody@acme.test", Role::Viewer)
        .await
        .expect_err("unknown");
    assert_eq!(unknown.code(), Some("unknown_user"));

    // Two-factor: enrol, confirm, recovery-code sign-in, then code sign-in.
    let uri = bob.totp_enroll().await.expect("enroll");
    let secret = uri
        .split("secret=")
        .nth(1)
        .expect("secret")
        .split('&')
        .next()
        .expect("s")
        .to_string();
    let t = now();
    let codes = bob
        .totp_confirm(&totp::code_at(&secret, t).expect("code"))
        .await
        .expect("confirm");
    assert_eq!(codes.len(), 10);
    let mut again = SharingClient::new(&base, &[]).expect("client");
    let needs = again
        .sign_in_password("acme", "bob@acme.test", PW, SecondFactor::None)
        .await
        .expect_err("needs 2fa");
    assert!(needs.needs_totp());
    again
        .sign_in_password(
            "acme",
            "bob@acme.test",
            PW,
            SecondFactor::Recovery(codes[0].clone()),
        )
        .await
        .expect("recovery");
    let reuse = again
        .sign_in_password(
            "acme",
            "bob@acme.test",
            PW,
            SecondFactor::Recovery(codes[0].clone()),
        )
        .await;
    assert_eq!(reuse.expect_err("spent").code(), Some("invalid_code"));
    again
        .sign_in_password(
            "acme",
            "bob@acme.test",
            PW,
            SecondFactor::Totp(totp::code_at(&secret, t + 30).expect("code")),
        )
        .await
        .expect("totp");

    // Sign out revokes the session on the server.
    let token = c.token().expect("token").to_string();
    c.sign_out().await.expect("sign out");
    assert!(!c.is_signed_in());
    let mut stale = SharingClient::new(&base, &[]).expect("client");
    stale.set_token(token);
    assert!(matches!(
        stale.me().await,
        Err(ApiClientError::Status { status: 401, .. })
    ));
    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_http_to_a_non_loopback_host_is_refused_by_the_client() {
    let mut c = SharingClient::new("http://192.0.2.10:8443", &[]).expect("client");
    let r = c
        .sign_in_password("acme", "a@b.test", PW, SecondFactor::None)
        .await;
    assert!(matches!(r, Err(ApiClientError::Transport(_))), "{r:?}");
}
