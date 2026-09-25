//! Rate-limit key and quota for the public API.

use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::GovernorError;

/// The address a request is rate-limited by.
///
/// A peer on loopback or a private/link-local network is taken to be our
/// reverse proxy: the first `X-Forwarded-For` hop (the original client) is used
/// when present and valid. Any other peer is keyed by its own address, so a
/// client cannot pick its bucket by sending the header. A header that does not
/// parse falls back to the peer. Without connection info (a server started
/// without `into_make_service_with_connect_info`) the header is used, then
/// localhost.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> IpAddr {
    match peer {
        Some(addr) if is_proxy(addr.ip()) => forwarded_for(headers).unwrap_or(addr.ip()),
        Some(addr) => addr.ip(),
        None => forwarded_for(headers).unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    }
}

/// First hop of `X-Forwarded-For`, if it parses as an IP address.
fn forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")?
        .to_str()
        .ok()?
        .split(',')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// Loopback, private or link-local addresses: where a reverse proxy in front
/// of us lives. IPv4-mapped IPv6 addresses are judged as the IPv4 address.
fn is_proxy(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

/// `tower_governor` key extractor using [`client_ip`] with axum's `ConnectInfo`.
#[derive(Clone, Copy, Debug)]
pub struct ClientIpKeyExtractor;

impl KeyExtractor for ClientIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| *addr);
        Ok(client_ip(req.headers(), peer))
    }
}

/// Replenish interval for `requests_per_second` sustained requests per key.
/// `GovernorConfigBuilder::per_second(n)` means one request every `n` seconds,
/// the inverse of what `API_RATE_LIMIT` promises, so the period is set directly.
/// Never zero (governor rejects a zero period).
pub fn governor_period(requests_per_second: u64) -> Duration {
    Duration::from_nanos((1_000_000_000 / requests_per_second.max(1)).max(1))
}

/// Burst size per key: twice the per-second rate, at least 1.
pub fn governor_burst(requests_per_second: u64) -> u32 {
    u32::try_from(requests_per_second.saturating_mul(2))
        .unwrap_or(u32::MAX)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(forwarded_for: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(value) = forwarded_for {
            headers.insert("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    fn peer(addr: &str) -> Option<SocketAddr> {
        Some(addr.parse().unwrap())
    }

    fn ip(addr: &str) -> IpAddr {
        addr.parse().unwrap()
    }

    #[test]
    fn direct_clients_are_keyed_by_their_address() {
        assert_eq!(
            client_ip(&headers(None), peer("203.0.113.7:5000")),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn a_public_peer_cannot_choose_its_key_with_a_header() {
        assert_eq!(
            client_ip(&headers(Some("198.51.100.1")), peer("203.0.113.7:5000")),
            ip("203.0.113.7")
        );
        assert_eq!(
            client_ip(&headers(Some("198.51.100.1")), peer("[2001:db8::7]:5000")),
            ip("2001:db8::7")
        );
    }

    #[test]
    fn behind_a_local_proxy_the_first_forwarded_hop_is_the_key() {
        assert_eq!(
            client_ip(
                &headers(Some("198.51.100.1, 10.0.0.2")),
                peer("127.0.0.1:40000")
            ),
            ip("198.51.100.1")
        );
        assert_eq!(
            client_ip(&headers(Some(" 198.51.100.9 ")), peer("172.18.0.1:40000")),
            ip("198.51.100.9")
        );
        assert_eq!(
            client_ip(&headers(Some("2001:db8::1")), peer("[::1]:40000")),
            ip("2001:db8::1")
        );
    }

    #[test]
    fn private_and_link_local_ipv6_peers_and_mapped_ipv4_are_proxies() {
        for proxy in [
            "[fd12:3456::1]:40000",        // fc00::/7 unique local
            "[fe80::1]:40000",             // fe80::/10 link local
            "[::ffff:127.0.0.1]:40000",    // IPv4-mapped loopback
            "[::ffff:192.168.1.10]:40000", // IPv4-mapped private
        ] {
            assert_eq!(
                client_ip(&headers(Some("198.51.100.1")), peer(proxy)),
                ip("198.51.100.1"),
                "{proxy}"
            );
        }
        // An IPv4-mapped public peer is not a proxy.
        assert_eq!(
            client_ip(
                &headers(Some("198.51.100.1")),
                peer("[::ffff:203.0.113.7]:5000")
            ),
            ip("::ffff:203.0.113.7")
        );
    }

    #[test]
    fn a_proxy_without_a_usable_header_is_keyed_by_itself() {
        assert_eq!(
            client_ip(&headers(None), peer("127.0.0.1:40000")),
            ip("127.0.0.1")
        );
        for malformed in ["not-an-ip", "", " , 198.51.100.1", "198.51.100.1:443"] {
            assert_eq!(
                client_ip(&headers(Some(malformed)), peer("10.0.0.5:40000")),
                ip("10.0.0.5"),
                "{malformed:?}"
            );
        }
    }

    #[test]
    fn without_connect_info_the_header_then_localhost_is_used() {
        assert_eq!(
            client_ip(&headers(Some("198.51.100.1")), None),
            ip("198.51.100.1")
        );
        assert_eq!(client_ip(&headers(None), None), ip("127.0.0.1"));
    }

    #[test]
    fn extractor_reads_axum_connect_info() {
        let mut request = Request::builder().uri("/api/search").body(()).unwrap();
        request.extensions_mut().insert(ConnectInfo::<SocketAddr>(
            "203.0.113.7:5000".parse().unwrap(),
        ));
        assert_eq!(
            ClientIpKeyExtractor.extract(&request).unwrap(),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn governor_period_spreads_the_rate_over_one_second() {
        assert_eq!(governor_period(100), Duration::from_millis(10));
        assert_eq!(governor_period(1), Duration::from_secs(1));
        assert_eq!(governor_period(0), Duration::from_secs(1));
        assert_eq!(governor_period(u64::MAX), Duration::from_nanos(1));
    }

    #[test]
    fn burst_is_twice_the_rate() {
        assert_eq!(governor_burst(100), 200);
        assert_eq!(governor_burst(0), 1);
        assert_eq!(governor_burst(u64::MAX), u32::MAX);
    }
}
