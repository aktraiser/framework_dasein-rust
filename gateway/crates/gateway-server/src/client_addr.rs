//! Who is really on the other end of a connection.
//!
//! Behind a reverse proxy every connection comes from the proxy. Its
//! `X-Forwarded-For` and `X-Forwarded-Proto` headers say who the client is
//! and whether they used HTTPS, but anyone can send those headers: they are
//! believed only when the connection comes from a configured proxy.

use axum::http::HeaderMap;
use std::net::IpAddr;

/// An address range in CIDR notation; a bare address is a range of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    address: IpAddr,
    prefix: u8,
}

impl std::str::FromStr for Network {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (address, prefix) = text
            .split_once('/')
            .map_or((text, None), |(a, p)| (a, Some(p)));
        let address: IpAddr = address
            .trim()
            .parse()
            .map_err(|_| format!("`{text}` is not an IP address or CIDR range"))?;
        let address = address.to_canonical();
        let width = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => width,
            Some(prefix) => prefix
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|prefix| *prefix <= width)
                .ok_or_else(|| format!("`{text}` has an invalid prefix length"))?,
        };
        Ok(Self { address, prefix })
    }
}

impl Network {
    /// Whether `ip` belongs to this range.
    #[must_use]
    pub fn contains(&self, ip: IpAddr) -> bool {
        let bits = |ip: IpAddr| match ip {
            IpAddr::V4(v4) => (u128::from(u32::from(v4)), 32u32),
            IpAddr::V6(v6) => (u128::from(v6), 128u32),
        };
        let (network, width) = bits(self.address);
        let (candidate, candidate_width) = bits(ip.to_canonical());
        if width != candidate_width {
            return false;
        }
        let shift = width - u32::from(self.prefix);
        // Shifting by the full width means "match everything".
        network.checked_shr(shift).unwrap_or(0) == candidate.checked_shr(shift).unwrap_or(0)
    }
}

/// Parse configured proxy ranges, skipping (and reporting) invalid entries:
/// an entry that is ignored means less trust, never more.
#[must_use]
pub fn parse_networks(entries: &[String]) -> Vec<Network> {
    entries
        .iter()
        .filter_map(|entry| match entry.parse() {
            Ok(network) => Some(network),
            Err(error) => {
                tracing::warn!("Ignoring trusted proxy entry: {error}");
                None
            }
        })
        .collect()
}

fn is_trusted(ip: IpAddr, trusted: &[Network]) -> bool {
    trusted.iter().any(|network| network.contains(ip))
}

/// The client's address: the peer itself, or, when the peer is a trusted
/// proxy, the last address in `X-Forwarded-For` that is not a trusted proxy.
#[must_use]
pub fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[Network]) -> IpAddr {
    if !is_trusted(peer, trusted) {
        return peer;
    }
    let forwarded = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','));
    // Each proxy appends the address it saw: read from the nearest hop back,
    // and stop at the first one we have no reason to believe.
    let hops: Vec<&str> = forwarded.collect();
    for hop in hops.into_iter().rev() {
        match parse_hop(hop) {
            Some(ip) if is_trusted(ip, trusted) => {}
            Some(ip) => return ip,
            None => break,
        }
    }
    peer
}

/// Parse one `X-Forwarded-For` entry. Some proxies add the source port
/// (`203.0.113.9:51234`, `[2001:db8::1]:443`) or bracket IPv6 addresses.
fn parse_hop(hop: &str) -> Option<IpAddr> {
    let hop = hop.trim();
    hop.parse::<IpAddr>()
        .ok()
        .or_else(|| {
            hop.parse::<std::net::SocketAddr>()
                .ok()
                .map(|addr| addr.ip())
        })
        .or_else(|| {
            hop.strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(|inner| inner.parse().ok())
        })
}

/// The first host in `X-Forwarded-Host`, when the peer is a trusted proxy.
#[must_use]
pub fn forwarded_host<'a>(
    peer: IpAddr,
    headers: &'a HeaderMap,
    trusted: &[Network],
) -> Option<&'a str> {
    if !is_trusted(peer, trusted) {
        return None;
    }
    headers
        .get("x-forwarded-host")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|host| !host.is_empty())
}

/// Whether the client reached us over HTTPS according to a trusted proxy.
#[must_use]
pub fn forwarded_https(peer: IpAddr, headers: &HeaderMap, trusted: &[Network]) -> bool {
    is_trusted(peer, trusted)
        && headers
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|proto| proto.eq_ignore_ascii_case("https"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn networks(entries: &[&str]) -> Vec<Network> {
        parse_networks(&entries.iter().map(ToString::to_string).collect::<Vec<_>>())
    }

    fn headers(forwarded_for: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", forwarded_for.parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        headers
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn ranges_match_addresses() {
        let trusted = networks(&[
            "10.0.0.0/8",
            "192.168.1.7",
            "fd00::/8",
            "not-an-ip",
            "1.2.3.4/40",
        ]);
        assert_eq!(trusted.len(), 3, "invalid entries are dropped");
        assert!(is_trusted(ip("10.200.3.4"), &trusted));
        assert!(is_trusted(ip("192.168.1.7"), &trusted));
        assert!(is_trusted(ip("fd12::1"), &trusted));
        assert!(is_trusted(ip("::ffff:10.0.0.1"), &trusted), "IPv4-mapped");
        assert!(!is_trusted(ip("192.168.1.8"), &trusted));
        assert!(!is_trusted(ip("11.0.0.1"), &trusted));
        assert!(is_trusted(ip("8.8.8.8"), &networks(&["0.0.0.0/0"])));
    }

    #[test]
    fn forwarded_headers_from_strangers_are_ignored() {
        let trusted = networks(&["10.0.0.1"]);
        let spoofed = headers("1.1.1.1");
        // Not from the proxy: the header is whatever the client typed.
        assert_eq!(
            client_ip(ip("203.0.113.9"), &spoofed, &trusted),
            ip("203.0.113.9")
        );
        assert!(!forwarded_https(ip("203.0.113.9"), &spoofed, &trusted));
        // No proxies configured: never believed.
        assert_eq!(client_ip(ip("10.0.0.1"), &spoofed, &[]), ip("10.0.0.1"));
    }

    #[test]
    fn client_is_the_last_untrusted_hop() {
        let trusted = networks(&["10.0.0.0/24"]);
        // The client put a fake address first; the proxies appended real ones.
        let chain = headers("6.6.6.6, 198.51.100.4, 10.0.0.2");
        assert_eq!(
            client_ip(ip("10.0.0.1"), &chain, &trusted),
            ip("198.51.100.4")
        );
        assert!(forwarded_https(ip("10.0.0.1"), &chain, &trusted));
        // Proxies that add ports or brackets are understood.
        for (entry, expected) in [
            ("198.51.100.4:51234", "198.51.100.4"),
            ("[2001:db8::7]:443", "2001:db8::7"),
            ("[2001:db8::7]", "2001:db8::7"),
        ] {
            assert_eq!(
                client_ip(ip("10.0.0.1"), &headers(entry), &trusted),
                ip(expected)
            );
        }
        // Nothing usable in the header: fall back to the proxy itself.
        assert_eq!(
            client_ip(ip("10.0.0.1"), &headers("garbage"), &trusted),
            ip("10.0.0.1")
        );
        assert_eq!(
            client_ip(ip("10.0.0.1"), &HeaderMap::new(), &trusted),
            ip("10.0.0.1")
        );
    }
}
