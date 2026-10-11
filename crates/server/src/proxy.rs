//! Trusted-proxy handling. `X-Forwarded-For` is honoured only when the TCP peer is inside a configured CIDR; the
//! default is to trust nothing, so a client can never choose its own rate-limit bucket.

use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 @ IpAddr::V4(_) => v4,
    }
}

impl Cidr {
    /// `10.0.0.0/8`, `fd00::/8` or a bare address (a host route).
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let (a, p) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (s, None),
        };
        let addr: IpAddr = a.parse().map_err(|_| format!("bad proxy address {s:?}"))?;
        let addr = unmap(addr);
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match p {
            Some(p) => p
                .parse::<u8>()
                .map_err(|_| format!("bad prefix in {s:?}"))?,
            None => max,
        };
        if prefix > max {
            return Err(format!("prefix too long in {s:?}"));
        }
        Ok(Self { addr, prefix })
    }

    pub fn parse_list(s: &str) -> Result<Vec<Self>, String> {
        s.split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(Self::parse)
            .collect()
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, unmap(ip)) {
            (IpAddr::V4(a), IpAddr::V4(b)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - u32::from(self.prefix))
                };
                u32::from(a) & mask == u32::from(b) & mask
            }
            (IpAddr::V6(a), IpAddr::V6(b)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - u32::from(self.prefix))
                };
                u128::from(a) & mask == u128::from(b) & mask
            }
            _ => false,
        }
    }
}

pub fn is_trusted(trusted: &[Cidr], ip: IpAddr) -> bool {
    trusted.iter().any(|c| c.contains(ip))
}

/// An `X-Forwarded-For` entry: a bare address, `ip:port` (IPv4), `[v6]` or `[v6]:port`. Proxies such as some cloud
/// load balancers append the port. Anything else is malformed.
pub fn parse_forwarded_entry(entry: &str) -> Option<IpAddr> {
    let e = entry.trim();
    if let Ok(ip) = e.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Ok(sa) = e.parse::<std::net::SocketAddr>() {
        return Some(sa.ip());
    }
    let inner = e.strip_prefix('[')?.strip_suffix(']')?;
    inner.parse::<std::net::Ipv6Addr>().ok().map(IpAddr::V6)
}

/// Logs (once per process, with fixed text: the header is attacker-influenced) that a trusted proxy sent an entry
/// that is not an address, so a misconfigured proxy is noticed instead of silently rate-limiting the proxy itself.
fn warn_malformed_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        eprintln!(
            "open365-server: a trusted proxy sent an X-Forwarded-For entry that is not an IP address; using the previous hop (logged once)"
        );
    });
}

/// The client address used for rate limits and audit. With no trusted proxies (or an untrusted peer) it is the TCP
/// peer. Otherwise the `X-Forwarded-For` chain is walked from the right and the first address that is not itself a
/// trusted proxy is the client; entries further left are attacker-controlled and ignored.
pub fn client_ip(peer: Option<IpAddr>, forwarded_for: &[&str], trusted: &[Cidr]) -> String {
    let Some(peer) = peer else {
        return "unknown".into();
    };
    if !is_trusted(trusted, peer) {
        return unmap(peer).to_string();
    }
    let mut cur = peer;
    'outer: for header in forwarded_for.iter().rev() {
        for part in header.rsplit(',') {
            let Some(ip) = parse_forwarded_entry(part) else {
                warn_malformed_once();
                break 'outer;
            };
            cur = ip;
            if !is_trusted(trusted, ip) {
                break 'outer;
            }
        }
    }
    unmap(cur).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn t(s: &str) -> Vec<Cidr> {
        Cidr::parse_list(s).expect("cidrs")
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("ip")
    }

    #[test]
    fn forwarded_entries_with_ports_are_accepted_and_junk_is_not() {
        assert_eq!(
            parse_forwarded_entry("203.0.113.5"),
            Some(ip("203.0.113.5"))
        );
        assert_eq!(
            parse_forwarded_entry(" 203.0.113.5:51234 "),
            Some(ip("203.0.113.5"))
        );
        assert_eq!(
            parse_forwarded_entry("[2001:db8::1]:443"),
            Some(ip("2001:db8::1"))
        );
        assert_eq!(
            parse_forwarded_entry("[2001:db8::1]"),
            Some(ip("2001:db8::1"))
        );
        assert_eq!(
            parse_forwarded_entry("2001:db8::1"),
            Some(ip("2001:db8::1"))
        );
        for bad in [
            "",
            "unknown",
            "_hidden",
            "203.0.113.5:notaport",
            "[2001:db8::1",
            "1.2.3.4.5",
            "[1.2.3.4]",
        ] {
            assert_eq!(parse_forwarded_entry(bad), None, "{bad}");
        }
        // A trusted proxy that appends ports no longer collapses every client into the proxy's own bucket.
        let trusted = t("10.0.0.0/8");
        assert_eq!(
            client_ip(Some(ip("10.0.0.1")), &["203.0.113.7:50000"], &trusted),
            "203.0.113.7"
        );
        assert_eq!(
            client_ip(
                Some(ip("10.0.0.1")),
                &["198.51.100.1, [2001:db8::9]:443, 10.0.0.2:80"],
                &trusted
            ),
            "2001:db8::9"
        );
    }

    #[test]
    fn cidr_matching() {
        let c = Cidr::parse("10.1.0.0/16").expect("c");
        assert!(c.contains(ip("10.1.200.3")));
        assert!(!c.contains(ip("10.2.0.1")));
        assert!(Cidr::parse("0.0.0.0/0").expect("c").contains(ip("8.8.8.8")));
        assert!(Cidr::parse("::1").expect("c").contains(ip("::1")));
        assert!(Cidr::parse("10.0.0.0/33").is_err());
        assert!(Cidr::parse("nope").is_err());
    }

    #[test]
    fn default_trusts_nobody() {
        assert_eq!(
            client_ip(Some(ip("203.0.113.5")), &["1.2.3.4"], &[]),
            "203.0.113.5"
        );
    }

    #[test]
    fn walks_from_the_right_and_ignores_spoofed_left() {
        let trusted = t("10.0.0.0/8");
        let got = client_ip(
            Some(ip("10.0.0.1")),
            &["6.6.6.6, 198.51.100.7, 10.0.0.9"],
            &trusted,
        );
        assert_eq!(got, "198.51.100.7");
    }

    proptest! {
        #[test]
        fn untrusted_peer_never_uses_header(peer in any::<u32>(), xff in "[0-9., ]{0,40}") {
            let peer = std::net::Ipv4Addr::from(peer);
            let trusted = t("10.0.0.0/8");
            prop_assume!(!trusted[0].contains(IpAddr::V4(peer)));
            prop_assert_eq!(client_ip(Some(IpAddr::V4(peer)), &[&xff], &trusted), peer.to_string());
        }

        #[test]
        fn rightmost_untrusted_entry_wins(spoof in any::<u32>(), real in any::<u32>()) {
            let trusted = t("10.0.0.0/8");
            let real_ip = std::net::Ipv4Addr::from(real);
            prop_assume!(!trusted[0].contains(IpAddr::V4(real_ip)));
            let header = format!("{}, {}, 10.9.9.9", std::net::Ipv4Addr::from(spoof), real_ip);
            let got = client_ip(Some(ip("10.0.0.2")), &[&header], &trusted);
            prop_assert_eq!(got, real_ip.to_string());
        }
    }
}
