//! The listener: header-read timeout and connection limits (slowloris defence), plain HTTP or rustls.
//!
//! The header-read timeout closes a connection that has not delivered a complete request head in time (HTTP/1), and
//! HTTP/2 keep-alive pings close a peer that stops answering. The connection limits count a connection from
//! `accept` until the socket is dropped, so a client that opens many sockets and says nothing cannot starve others.
//! Peers inside `trusted_proxies` are exempt from the per-address limit (they front many clients) but still count
//! against the global limit.

use std::collections::HashMap;
use std::future::{Ready, ready};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum_server::Handle;
use axum_server::accept::Accept;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use hyper_util::rt::TokioTimer;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use crate::config::Config;
use crate::proxy::{Cidr, is_trusted};

/// Limits applied to every accepted connection.
#[derive(Clone, Debug)]
pub struct ServeLimits {
    /// Time a client has to send a complete request head after it connects (or after the previous response).
    pub header_read_timeout: Duration,
    /// Open connections, all addresses together (0 = unlimited).
    pub max_connections: usize,
    /// Open connections per peer address (0 = unlimited).
    pub max_connections_per_ip: usize,
    pub trusted_proxies: Vec<Cidr>,
}

impl ServeLimits {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            header_read_timeout: Duration::from_secs(cfg.header_read_timeout_secs.max(1)),
            max_connections: cfg.max_connections,
            max_connections_per_ip: cfg.max_connections_per_ip,
            trusted_proxies: cfg.trusted_proxies.clone(),
        }
    }
}

#[derive(Debug, Default)]
struct Counts {
    total: usize,
    per_ip: HashMap<IpAddr, usize>,
}

/// Admits connections while they fit the limits.
#[derive(Clone, Debug)]
pub struct LimitAcceptor {
    limits: Arc<ServeLimits>,
    counts: Arc<Mutex<Counts>>,
}

impl LimitAcceptor {
    pub fn new(limits: ServeLimits) -> Self {
        Self {
            limits: Arc::new(limits),
            counts: Arc::new(Mutex::new(Counts::default())),
        }
    }

    fn admit(&self, peer: Option<IpAddr>) -> Option<Slot> {
        let mut c = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if self.limits.max_connections > 0 && c.total >= self.limits.max_connections {
            return None;
        }
        let counted_ip = peer.filter(|ip| {
            self.limits.max_connections_per_ip > 0 && !is_trusted(&self.limits.trusted_proxies, *ip)
        });
        if let Some(ip) = counted_ip {
            let n = c.per_ip.entry(ip).or_insert(0);
            if *n >= self.limits.max_connections_per_ip {
                return None;
            }
            *n += 1;
        }
        c.total += 1;
        Some(Slot {
            counts: self.counts.clone(),
            ip: counted_ip,
        })
    }
}

/// Releases a connection's place when dropped.
#[derive(Debug)]
pub struct Slot {
    counts: Arc<Mutex<Counts>>,
    ip: Option<IpAddr>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut c = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        c.total = c.total.saturating_sub(1);
        if let Some(ip) = self.ip
            && let Some(n) = c.per_ip.get_mut(&ip)
        {
            *n = n.saturating_sub(1);
            if *n == 0 {
                c.per_ip.remove(&ip);
            }
        }
    }
}

/// A socket that keeps its [`Slot`] until it is dropped.
#[derive(Debug)]
pub struct CountedStream {
    inner: TcpStream,
    _slot: Slot,
}

impl AsyncRead for CountedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for CountedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl<S> Accept<TcpStream, S> for LimitAcceptor {
    type Stream = CountedStream;
    type Service = S;
    type Future = Ready<io::Result<(CountedStream, S)>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let peer = stream.peer_addr().ok().map(|a| a.ip());
        ready(match self.admit(peer) {
            Some(slot) => {
                let _ = stream.set_nodelay(true);
                Ok((
                    CountedStream {
                        inner: stream,
                        _slot: slot,
                    },
                    service,
                ))
            }
            None => Err(io::Error::other("connection limit reached")),
        })
    }
}

/// Serves `app` on `listener` until `handle` shuts it down. `tls` selects HTTPS (the caller installs the rustls
/// crypto provider first).
pub async fn serve(
    listener: std::net::TcpListener,
    app: Router,
    limits: ServeLimits,
    tls: Option<RustlsConfig>,
    handle: Handle<SocketAddr>,
) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let header_timeout = limits.header_read_timeout;
    let acceptor = LimitAcceptor::new(limits);
    let make = app.into_make_service_with_connect_info::<SocketAddr>();
    match tls {
        Some(conf) => {
            let mut server = axum_server::from_tcp(listener)?
                .acceptor(RustlsAcceptor::new(conf).acceptor(acceptor))
                .handle(handle);
            tune(server.http_builder(), header_timeout);
            server.serve(make).await
        }
        None => {
            let mut server = axum_server::from_tcp(listener)?
                .acceptor(acceptor)
                .handle(handle);
            tune(server.http_builder(), header_timeout);
            server.serve(make).await
        }
    }
}

fn tune(
    b: &mut hyper_util::server::conn::auto::Builder<hyper_util::rt::TokioExecutor>,
    header_timeout: Duration,
) {
    b.http1()
        .timer(TokioTimer::new())
        .header_read_timeout(header_timeout);
    b.http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(Some(Duration::from_secs(20)))
        .keep_alive_timeout(Duration::from_secs(20));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(total: usize, per_ip: usize) -> LimitAcceptor {
        LimitAcceptor::new(ServeLimits {
            header_read_timeout: Duration::from_secs(1),
            max_connections: total,
            max_connections_per_ip: per_ip,
            trusted_proxies: vec![Cidr::parse("10.0.0.0/8").expect("cidr")],
        })
    }

    #[test]
    fn limits_count_and_release() {
        let a = acc(3, 2);
        let ip = Some("192.0.2.1".parse().expect("ip"));
        let s1 = a.admit(ip).expect("first");
        let _s2 = a.admit(ip).expect("second");
        assert!(a.admit(ip).is_none(), "per-address limit");
        let _s3 = a
            .admit(Some("192.0.2.2".parse().expect("ip")))
            .expect("other address");
        assert!(
            a.admit(Some("192.0.2.3".parse().expect("ip"))).is_none(),
            "global limit"
        );
        drop(s1);
        assert!(
            a.admit(Some("192.0.2.3".parse().expect("ip"))).is_some(),
            "slot released"
        );
    }

    #[test]
    fn trusted_proxies_skip_the_per_address_limit_only() {
        let a = acc(3, 1);
        let proxy = Some("10.1.2.3".parse().expect("ip"));
        let _a = a.admit(proxy).expect("1");
        let _b = a.admit(proxy).expect("2");
        let _c = a.admit(proxy).expect("3");
        assert!(a.admit(proxy).is_none(), "global limit still applies");
    }
}
