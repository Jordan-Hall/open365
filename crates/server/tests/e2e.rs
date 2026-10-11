//! End to end: the compiled binary serves HTTPS on loopback with a self-signed certificate generated here, and the
//! crate's own client drives tenant creation, sign-in, upload, link creation and an anonymous download. Afterwards
//! the operator commands (backup, restore, audit-verify) run against the data the server wrote.
//! Every secret is generated at test time; nothing is a real credential.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use hyper::Method;
use open365_server::client::Client;
use open365_server::util::{hex, random_bytes};
use serde_json::{Value, json};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_open365-server")
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

fn operator(args: &[&str], data: &Path, key: &Path) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .env("OPEN365_DATA_DIR", data)
        .env("OPEN365_AUDIT_KEY_FILE", key)
        .output()
        .expect("run")
}

async fn call(
    c: &Client,
    m: Method,
    url: &str,
    token: Option<&str>,
    body: Value,
) -> (u16, Vec<u8>, Vec<(String, String)>) {
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers = vec![("content-type", "application/json")];
    if let Some(a) = &auth {
        headers.push(("authorization", a.as_str()));
    }
    let bytes = if body.is_null() {
        vec![]
    } else {
        body.to_string().into_bytes()
    };
    let r = c.request(m, url, &headers, bytes).await.expect("request");
    (r.status, r.body, r.headers)
}

#[tokio::test]
async fn binary_over_tls_upload_link_download_then_backup_restore() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = |n: &str| dir.path().join(n);

    // A self-signed certificate for loopback, generated now.
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("cert");
    std::fs::write(p("cert.pem"), cert.cert.pem()).expect("cert");
    std::fs::write(p("key.pem"), cert.key_pair.serialize_pem()).expect("key");
    let bootstrap = hex(&random_bytes::<16>());
    std::fs::write(p("bootstrap.token"), &bootstrap).expect("token");
    let keygen = Command::new(bin())
        .args(["gen-key", p("audit.key").to_str().expect("path")])
        .output()
        .expect("gen-key");
    assert!(keygen.status.success());

    let port = free_port();
    let data = p("data");
    let server = Server(
        Command::new(bin())
            .env("OPEN365_BIND", format!("127.0.0.1:{port}"))
            .env("OPEN365_DATA_DIR", &data)
            .env("OPEN365_AUDIT_KEY_FILE", p("audit.key"))
            .env("OPEN365_BOOTSTRAP_TOKEN_FILE", p("bootstrap.token"))
            .env("OPEN365_TLS_CERT", p("cert.pem"))
            .env("OPEN365_TLS_KEY", p("key.pem"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn server"),
    );

    let client = Client::new(true, &[cert.cert.der().as_ref().to_vec()]).expect("client");
    let base = format!("https://127.0.0.1:{port}");
    let started = Instant::now();
    loop {
        if client
            .get(&format!("{base}/healthz"))
            .await
            .is_ok_and(|r| r.status == 200)
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "server did not start"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let (s, ..) = call(
        &client,
        Method::POST,
        &format!("{base}/api/v1/admin/tenants"),
        None,
        json!({"slug": "acme", "name": "Acme", "admin_email": "admin@acme.test", "admin_password": "a long passphrase for tests"}),
    )
    .await;
    // The bootstrap header is required: the first attempt without it is refused.
    assert_eq!(s, 401);
    let r = client
        .request(
            Method::POST,
            &format!("{base}/api/v1/admin/tenants"),
            &[("content-type", "application/json"), ("x-bootstrap-token", &bootstrap)],
            json!({"slug": "acme", "name": "Acme", "admin_email": "admin@acme.test", "admin_password": "a long passphrase for tests"})
                .to_string()
                .into_bytes(),
        )
        .await
        .expect("tenant");
    assert_eq!(r.status, 201);

    let (s, body, _) = call(
        &client,
        Method::POST,
        &format!("{base}/api/v1/login"),
        None,
        json!({"tenant": "acme", "email": "admin@acme.test", "password": "a long passphrase for tests", "bearer": true}),
    )
    .await;
    assert_eq!(s, 200);
    let token = serde_json::from_slice::<Value>(&body).expect("json")["token"]
        .as_str()
        .expect("token")
        .to_string();

    // A document with every byte value, so any text mangling would show.
    let doc_bytes: Vec<u8> = (0..300_000u32)
        .map(|i| u8::try_from(i % 251).expect("byte"))
        .collect();
    let auth = format!("Bearer {token}");
    let up = client
        .request(
            Method::POST,
            &format!("{base}/api/v1/documents?name=Budget.xlsx"),
            &[("authorization", &auth)],
            doc_bytes.clone(),
        )
        .await
        .expect("upload");
    assert_eq!(up.status, 201);
    let doc_id = up.json()["id"].as_str().expect("id").to_string();

    let (s, body, _) = call(
        &client,
        Method::POST,
        &format!("{base}/api/v1/documents/{doc_id}/links"),
        Some(&token),
        json!({"scope": "view", "audience": "anyone", "expires_in_secs": 3600}),
    )
    .await;
    assert_eq!(s, 201);
    let link = serde_json::from_slice::<Value>(&body).expect("json")["url_path"]
        .as_str()
        .expect("url_path")
        .to_string();

    // Anyone with the link downloads the identical bytes, as an attachment, over HTTPS with HSTS.
    let dl = client
        .get(&format!("{base}{link}/content"))
        .await
        .expect("download");
    assert_eq!(dl.status, 200);
    assert_eq!(dl.body, doc_bytes);
    assert!(
        dl.header("content-disposition")
            .is_some_and(|d| d.starts_with("attachment;"))
    );
    assert!(dl.header("strict-transport-security").is_some());
    let unknown = client
        .get(&format!("{base}/s/{}/content", "A".repeat(43)))
        .await
        .expect("unknown");
    assert_eq!(unknown.status, 404);
    // Without the client trusting the test certificate the connection fails: TLS is really in force.
    let strict = Client::new(true, &[]).expect("client");
    assert!(strict.get(&format!("{base}/healthz")).await.is_err());
    // Plain HTTP to the TLS port gets nothing useful.
    let plain = Client::new(true, &[]).expect("client");
    assert!(
        plain
            .get(&format!("http://127.0.0.1:{port}/healthz"))
            .await
            .map_or(true, |r| r.status != 200)
    );

    drop(server);

    // Operator commands on what the server wrote.
    let key = p("audit.key");
    let verify = operator(&["audit-verify"], &data, &key);
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let out = String::from_utf8_lossy(&verify.stdout).to_string();
    assert!(out.contains("chain -") || out.contains("ok"), "{out}");
    assert!(!out.contains("BROKEN"), "{out}");

    let backup = p("backup");
    let b = operator(&["backup", backup.to_str().expect("path")], &data, &key);
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let restored = p("restored");
    let r = operator(
        &["restore", backup.to_str().expect("path")],
        &restored,
        &key,
    );
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let again = operator(&["audit-verify"], &restored, &key);
    assert!(again.status.success());
}
