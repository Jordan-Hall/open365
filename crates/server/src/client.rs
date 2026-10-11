//! A small outbound HTTP/1.1 client with SSRF controls (T12), used for OpenID Connect discovery, key fetches and the
//! code exchange, and by tests as the API client.
//!
//! Rules: only `http` and `https` URLs without credentials; plain `http` only to a loopback host when the caller
//! opted in; every resolved address must be public (or loopback when opted in); the connection is pinned to the
//! address that was checked; redirects are never followed; the body and the whole exchange are bounded.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::{Method, Request, Uri};
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

#[derive(Debug)]
pub enum ClientError {
    BadUrl(&'static str),
    Blocked(&'static str),
    Network(String),
    Body,
    Timeout,
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

#[derive(Clone)]
pub struct Client {
    tls: TlsConnector,
    allow_loopback: bool,
    /// Private and link-local addresses are allowed (a server the user chose, such as one on the office network).
    allow_private: bool,
    max_body: usize,
    timeout: Duration,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("allow_loopback", &self.allow_loopback)
            .finish_non_exhaustive()
    }
}

fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 @ IpAddr::V4(_) => v4,
    }
}

/// True for an address that is routable on the public internet.
pub fn is_public(ip: IpAddr) -> bool {
    match unmap(ip) {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || v.is_documentation()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (o[1] & 0xfe) == 18))
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                || (s[0] == 0x0064 && s[1] == 0xff9b))
        }
    }
}

/// Host part of a URL, without brackets.
pub fn host_of(uri: &Uri) -> Option<String> {
    uri.host().map(|h| h.trim_matches(['[', ']']).to_string())
}

pub fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|ip| unmap(ip).is_loopback())
}

/// Percent-encodes for a query string or form body (RFC 3986 unreserved characters stay).
pub fn pct_encode(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

impl Client {
    /// `allow_loopback` lets the client reach loopback addresses (and plain `http` there): for a loopback test
    /// issuer and for tests. `extra_roots` are DER certificates trusted in addition to the system store.
    pub fn new(allow_loopback: bool, extra_roots: &[Vec<u8>]) -> Result<Self, String> {
        let mut roots = rustls::RootCertStore::empty();
        for c in rustls_native_certs::load_native_certs().certs {
            let _ = roots.add(c);
        }
        for d in extra_roots {
            roots
                .add(CertificateDer::from(d.clone()))
                .map_err(|e| format!("bad root certificate: {e}"))?;
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("tls: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self {
            tls: TlsConnector::from(Arc::new(cfg)),
            allow_loopback,
            allow_private: false,
            max_body: 1024 * 1024,
            timeout: Duration::from_secs(10),
        })
    }

    /// A client for a server the user typed in (the desktop Share dialog): `https` anywhere including private
    /// networks, plain `http` only to loopback. Never use it for a URL that came from a document or a third party.
    pub fn for_user_server(extra_roots: &[Vec<u8>]) -> Result<Self, String> {
        let mut c = Self::new(true, extra_roots)?;
        c.allow_private = true;
        Ok(c)
    }

    pub fn with_limits(mut self, max_body: usize, timeout: Duration) -> Self {
        self.max_body = max_body;
        self.timeout = timeout;
        self
    }

    pub async fn request(
        &self,
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> Result<Response, ClientError> {
        tokio::time::timeout(self.timeout, self.request_inner(method, url, headers, body))
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    pub async fn get(&self, url: &str) -> Result<Response, ClientError> {
        self.request(Method::GET, url, &[], vec![]).await
    }

    async fn request_inner(
        &self,
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> Result<Response, ClientError> {
        let uri: Uri = url.parse().map_err(|_| ClientError::BadUrl("unparsable"))?;
        let https = match uri.scheme_str() {
            Some("https") => true,
            Some("http") => false,
            _ => return Err(ClientError::BadUrl("scheme")),
        };
        let authority = uri.authority().ok_or(ClientError::BadUrl("no host"))?;
        if authority.as_str().contains('@') {
            return Err(ClientError::BadUrl("credentials in url"));
        }
        let host = host_of(&uri).ok_or(ClientError::BadUrl("no host"))?;
        if !(https || self.allow_loopback && is_loopback_host(&host)) {
            return Err(ClientError::Blocked(
                "plain http is only allowed to loopback",
            ));
        }
        let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?
            .collect();
        let first = *addrs
            .first()
            .ok_or_else(|| ClientError::Network("no address".into()))?;
        for a in &addrs {
            let ip = unmap(a.ip());
            if !(is_public(ip) || self.allow_private || (self.allow_loopback && ip.is_loopback())) {
                return Err(ClientError::Blocked("address is not public"));
            }
        }
        let path = uri.path_and_query().map_or("/", |p| p.as_str());
        let mut b = Request::builder()
            .method(method)
            .uri(path)
            .header("host", authority.as_str())
            .header("connection", "close");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let req = b
            .body(Full::new(Bytes::from(body)))
            .map_err(|_| ClientError::BadUrl("request"))?;
        // Connect to the address that was just checked, never to a second lookup.
        let tcp = TcpStream::connect(first)
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        if https {
            let name =
                ServerName::try_from(host).map_err(|_| ClientError::BadUrl("server name"))?;
            let tls = self
                .tls
                .connect(name, tcp)
                .await
                .map_err(|e| ClientError::Network(format!("tls: {e}")))?;
            send(tls, req, self.max_body).await
        } else {
            send(tcp, req, self.max_body).await
        }
    }
}

async fn send<T>(io: T, req: Request<Full<Bytes>>, max: usize) -> Result<Response, ClientError>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let res = sender
        .send_request(req)
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;
    let status = res.status().as_u16();
    let headers = res
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
        .collect();
    let body = Limited::new(res.into_body(), max)
        .collect()
        .await
        .map_err(|_| ClientError::Body)?
        .to_bytes()
        .to_vec();
    Ok(Response {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_classes() {
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "224.0.0.1",
        ] {
            assert!(!is_public(bad.parse().expect("ip")), "{bad}");
        }
        for good in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public(good.parse().expect("ip")), "{good}");
        }
    }

    #[tokio::test]
    async fn refuses_private_and_plain_http_targets() {
        let c = Client::new(false, &[]).expect("client");
        for url in [
            "http://example.com/",
            "https://127.0.0.1/",
            "https://169.254.169.254/latest/meta-data",
            "https://10.0.0.5/",
            "https://user:pw@example.com/",
            "ftp://example.com/",
        ] {
            assert!(
                matches!(
                    c.get(url).await,
                    Err(ClientError::Blocked(_) | ClientError::BadUrl(_))
                ),
                "{url}"
            );
        }
        // Even with loopback allowed, plain http to a non-loopback host stays refused.
        let c = Client::new(true, &[]).expect("client");
        assert!(matches!(
            c.get("http://10.0.0.5/").await,
            Err(ClientError::Blocked(_))
        ));
    }

    #[test]
    fn pct_encoding() {
        assert_eq!(pct_encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }
}
