//! Optional, offline geo-IP for the sessions page (spec 052 R6). Display only: a location never
//! affects trust and is never written to the audit log.
//!
//! `DEPTHS_GEOIP_DB` names a City database (DB-IP City Lite or GeoLite2 City, `.mmdb`) on disk.
//! No lookup ever leaves the machine. Private, loopback, link-local, CGNAT (`100.64.0.0/10`) and
//! unique-local addresses show `private network`; anything not found, or no database, shows
//! `unknown`. A database that cannot be opened is logged once at start and treated as absent.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use maxminddb::{Reader, geoip2};

pub const PRIVATE: &str = "private network";
pub const UNKNOWN: &str = "unknown";

/// The lookup. Cheap to share behind an `Arc`.
pub struct GeoIp {
    reader: Option<Reader<Vec<u8>>>,
}

impl GeoIp {
    /// No database: every public address is `unknown`.
    pub fn none() -> Self {
        Self { reader: None }
    }

    /// Opens `path` if set. A failure is logged here, once, and leaves the lookup empty.
    pub fn open(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::none();
        };
        match Reader::open_readfile(path) {
            Ok(reader) => {
                tracing::info!(path = %path.display(), "geo-IP database loaded (display only)");
                Self {
                    reader: Some(reader),
                }
            }
            Err(e) => {
                tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "DEPTHS_GEOIP_DB cannot be read; locations show as unknown"
                );
                Self::none()
            }
        }
    }

    pub fn loaded(&self) -> bool {
        self.reader.is_some()
    }

    /// `City, Region, Country`, as much of it as the database has; [`PRIVATE`] or [`UNKNOWN`].
    pub fn locate(&self, ip: IpAddr) -> String {
        let ip = depths_core::network::canonical(ip);
        if is_private(ip) {
            return PRIVATE.to_owned();
        }
        let Some(reader) = &self.reader else {
            return UNKNOWN.to_owned();
        };
        let Ok(found) = reader.lookup(ip) else {
            return UNKNOWN.to_owned();
        };
        let Ok(Some(city)) = found.decode::<geoip2::City<'_>>() else {
            return UNKNOWN.to_owned();
        };
        let parts: Vec<&str> = [
            city.city.names.english,
            city.subdivisions.first().and_then(|s| s.names.english),
            city.country.names.english.or(city.country.iso_code),
        ]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect();
        if parts.is_empty() {
            return UNKNOWN.to_owned();
        }
        let s: String = parts
            .join(", ")
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        s
    }
}

/// Addresses that say nothing about where someone is.
pub fn is_private(ip: IpAddr) -> bool {
    match depths_core::network::canonical(ip) {
        IpAddr::V4(a) => is_private_v4(a),
        IpAddr::V6(a) => is_private_v6(a),
    }
}

fn is_private_v4(a: Ipv4Addr) -> bool {
    let [x, y, ..] = a.octets();
    a.is_private()
        || a.is_loopback()
        || a.is_link_local()
        || a.is_unspecified()
        || a.is_broadcast()
        // CGNAT, and tailnets that use it.
        || (x == 100 && (64..128).contains(&y))
}

fn is_private_v6(a: Ipv6Addr) -> bool {
    let first = a.segments()[0];
    a.is_loopback()
        || a.is_unspecified()
        // Unique local fc00::/7 and link-local fe80::/10.
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn private_addresses_say_so_without_a_database() {
        let g = GeoIp::none();
        for a in [
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.10",
            "127.0.0.1",
            "100.64.0.1",
            "100.127.255.254",
            "169.254.1.1",
            "::1",
            "fd12:3456::1",
            "fe80::1",
            "::ffff:192.168.1.10",
        ] {
            assert_eq!(g.locate(ip(a)), PRIVATE, "{a}");
        }
        assert_eq!(g.locate(ip("100.128.0.1")), UNKNOWN);
        assert_eq!(g.locate(ip("203.0.113.7")), UNKNOWN);
        assert_eq!(g.locate(ip("2001:db8::1")), UNKNOWN);
        assert!(!g.loaded());
    }

    #[test]
    fn a_missing_or_broken_file_is_unknown_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let g = GeoIp::open(Some(&dir.path().join("absent.mmdb")));
        assert!(!g.loaded());
        let bad = dir.path().join("bad.mmdb");
        std::fs::write(&bad, b"not a database").unwrap();
        let g = GeoIp::open(Some(&bad));
        assert!(!g.loaded());
        assert_eq!(g.locate(ip("203.0.113.7")), UNKNOWN);
        assert!(!GeoIp::open(None).loaded());
    }
}
