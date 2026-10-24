//! T091: the client's address, for rate limits and the audit log.
//!
//! Forwarding headers are believed only from a peer inside `DEPTHS_TRUSTED_PROXIES`. From such a
//! peer, `CF-Connecting-IP` (set by Cloudflare at the edge) wins; otherwise the right-most
//! `X-Forwarded-For` entry that is not itself a trusted proxy. From any other peer, both headers
//! are ignored and the TCP peer is the client: a header anyone can send is not an address.

use std::net::{IpAddr, SocketAddr};

use http::HeaderMap;

/// An address block, `10.0.0.0/8` or `fd00::/8`. A bare address is a single host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    net: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> Option<Self> {
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (s, None),
        };
        let net: IpAddr = addr.trim().parse().ok()?;
        let max = if net.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p.trim().parse::<u8>().ok().filter(|p| *p <= max)?,
            None => max,
        };
        Some(Self { net, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = canonical(ip);
        match (self.net, ip) {
            (IpAddr::V4(n), IpAddr::V4(a)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(n) & mask == u32::from(a) & mask
            }
            (IpAddr::V6(n), IpAddr::V6(a)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(n) & mask == u128::from(a) & mask
            }
            _ => false,
        }
    }
}

/// `::ffff:1.2.3.4` is `1.2.3.4`: a dual-stack listener reports v4 peers that way.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 @ IpAddr::V4(_) => v4,
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClientIp {
    trusted: Vec<Cidr>,
}

impl ClientIp {
    pub fn new(trusted: Vec<Cidr>) -> Self {
        Self { trusted }
    }

    fn is_trusted(&self, ip: IpAddr) -> bool {
        self.trusted.iter().any(|c| c.contains(ip))
    }

    /// The client address for a request that arrived from `peer`.
    pub fn resolve(&self, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
        let peer = canonical(peer.ip());
        if !self.is_trusted(peer) {
            return peer;
        }
        if let Some(ip) = single(headers, "cf-connecting-ip") {
            return ip;
        }
        // Every X-Forwarded-For header, in order, as one list. A malformed entry ends the walk:
        // nothing to its left can be attributed.
        let mut entries: Vec<&str> = Vec::new();
        for v in headers.get_all("x-forwarded-for") {
            let Ok(s) = v.to_str() else { return peer };
            entries.extend(s.split(',').map(str::trim));
        }
        let mut last_trusted = peer;
        for raw in entries.iter().rev() {
            let Ok(ip) = raw.parse::<IpAddr>().map(canonical) else {
                return last_trusted;
            };
            if !self.is_trusted(ip) {
                return ip;
            }
            last_trusted = ip;
        }
        last_trusted
    }
}

fn single(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    let mut all = headers.get_all(name).iter();
    let v = all.next()?;
    if all.next().is_some() {
        return None;
    }
    v.to_str().ok()?.trim().parse().ok().map(canonical)
}

/// Requests looked at before the proxy heuristic settles.
pub const PROXY_SAMPLE: u32 = 20;

/// FR-021 (spec 052): with the network gate on and no `DEPTHS_TRUSTED_PROXIES`, a server behind
/// a reverse proxy sees the proxy's private address for everyone, so every person looks like
/// they are on one network. When the first [`PROXY_SAMPLE`] authenticated requests all come from
/// private addresses, this warns once in the log; admins see a notice from the first one.
pub struct ProxyWatch {
    armed: bool,
    seen: std::sync::atomic::AtomicU32,
    public: std::sync::atomic::AtomicBool,
    logged: std::sync::atomic::AtomicBool,
}

impl ProxyWatch {
    /// `armed`: the gate is on and no trusted proxies are configured.
    pub fn new(armed: bool) -> Self {
        Self {
            armed,
            seen: std::sync::atomic::AtomicU32::new(0),
            public: std::sync::atomic::AtomicBool::new(false),
            logged: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// One authenticated request (or sign-in) from `ip`.
    pub fn saw(&self, ip: IpAddr) {
        use std::sync::atomic::Ordering::Relaxed;
        if !self.armed || self.public.load(Relaxed) {
            return;
        }
        let n = self.seen.fetch_add(1, Relaxed);
        if n >= PROXY_SAMPLE {
            self.seen.store(PROXY_SAMPLE, Relaxed);
            return;
        }
        if !crate::geoip::is_private(ip) {
            self.public.store(true, Relaxed);
            return;
        }
        if n + 1 == PROXY_SAMPLE && !self.logged.swap(true, Relaxed) {
            tracing::warn!(
                "every request so far came from a private address and DEPTHS_TRUSTED_PROXIES is \
                 not set: behind a reverse proxy, set DEPTHS_TRUSTED_PROXIES so the network gate \
                 sees real client addresses"
            );
        }
    }

    /// Whether admins should see the notice.
    pub fn warning(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        self.armed && !self.public.load(Relaxed) && self.seen.load(Relaxed) > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn peer(s: &str) -> SocketAddr {
        SocketAddr::new(s.parse().unwrap(), 4242)
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn resolver() -> ClientIp {
        ClientIp::new(vec![
            Cidr::parse("172.18.0.0/16").unwrap(),
            Cidr::parse("100.64.0.0/10").unwrap(),
        ])
    }

    #[test]
    fn cidrs_parse_and_match() {
        let c = Cidr::parse("10.1.0.0/16").unwrap();
        assert!(c.contains("10.1.200.3".parse().unwrap()));
        assert!(!c.contains("10.2.0.1".parse().unwrap()));
        assert!(c.contains("::ffff:10.1.0.9".parse().unwrap()));
        assert!(
            Cidr::parse("0.0.0.0/0")
                .unwrap()
                .contains("8.8.8.8".parse().unwrap())
        );
        assert!(
            Cidr::parse("fd00::/8")
                .unwrap()
                .contains("fd12::1".parse().unwrap())
        );
        assert!(
            Cidr::parse("1.2.3.4")
                .unwrap()
                .contains("1.2.3.4".parse().unwrap())
        );
        for bad in ["10.0.0.0/33", "::/129", "nope", "10.0.0.0/x", ""] {
            assert!(Cidr::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn headers_from_an_untrusted_peer_are_ignored() {
        let h = headers(&[
            ("x-forwarded-for", "9.9.9.9"),
            ("cf-connecting-ip", "8.8.8.8"),
        ]);
        assert_eq!(
            resolver().resolve(peer("203.0.113.7"), &h),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
        // With no trusted proxies at all, the same.
        assert_eq!(
            ClientIp::default().resolve(peer("172.18.0.2"), &h),
            "172.18.0.2".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn cf_connecting_ip_wins_from_a_trusted_peer() {
        let h = headers(&[
            ("x-forwarded-for", "1.1.1.1, 104.16.0.1"),
            ("cf-connecting-ip", "198.51.100.4"),
        ]);
        assert_eq!(
            resolver().resolve(peer("172.18.0.5"), &h),
            "198.51.100.4".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn the_right_most_untrusted_xff_entry_is_the_client() {
        // The left entry is client-supplied and must not be believed.
        let h = headers(&[("x-forwarded-for", "6.6.6.6, 198.51.100.9, 100.64.0.3")]);
        assert_eq!(
            resolver().resolve(peer("172.18.0.5"), &h),
            "198.51.100.9".parse::<IpAddr>().unwrap()
        );
        // Split across two headers reads the same.
        let h = headers(&[
            ("x-forwarded-for", "6.6.6.6"),
            ("x-forwarded-for", "198.51.100.9, 100.64.0.3"),
        ]);
        assert_eq!(
            resolver().resolve(peer("172.18.0.5"), &h),
            "198.51.100.9".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn garbage_in_the_chain_stops_at_the_last_trusted_hop() {
        let h = headers(&[("x-forwarded-for", "1.2.3.4, not-an-ip, 100.64.0.3")]);
        assert_eq!(
            resolver().resolve(peer("172.18.0.5"), &h),
            "100.64.0.3".parse::<IpAddr>().unwrap()
        );
        // Two CF headers are ambiguous; fall back to XFF.
        let h = headers(&[
            ("cf-connecting-ip", "1.1.1.1"),
            ("cf-connecting-ip", "2.2.2.2"),
            ("x-forwarded-for", "198.51.100.1"),
        ]);
        assert_eq!(
            resolver().resolve(peer("172.18.0.5"), &h),
            "198.51.100.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn a_trusted_peer_without_headers_is_itself() {
        assert_eq!(
            resolver().resolve(peer("172.18.0.5"), &HeaderMap::new()),
            "172.18.0.5".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn proxy_warning_needs_private_addresses_only_and_an_armed_gate() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let w = ProxyWatch::new(true);
        assert!(!w.warning(), "nothing seen yet");
        for _ in 0..PROXY_SAMPLE + 5 {
            w.saw(ip("172.18.0.1"));
        }
        assert!(w.warning());
        let w = ProxyWatch::new(true);
        w.saw(ip("10.0.0.2"));
        w.saw(ip("203.0.113.7"));
        w.saw(ip("10.0.0.2"));
        assert!(
            !w.warning(),
            "a public address means the addresses are real"
        );
        let w = ProxyWatch::new(false);
        w.saw(ip("10.0.0.2"));
        assert!(!w.warning(), "proxies configured or gate off");
        let w = ProxyWatch::new(true);
        for _ in 0..PROXY_SAMPLE {
            w.saw(ip("10.0.0.2"));
        }
        w.saw(ip("203.0.113.7"));
        assert!(w.warning(), "settled after the sample");
    }
}
