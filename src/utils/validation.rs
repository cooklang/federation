// Validation utilities
use crate::error::{Error, Result};
use std::net::IpAddr;
use tracing::warn;
use url::Url;

// List of commonly blocked ports
const BLOCKED_PORTS: &[u16] = &[
    22,    // SSH
    23,    // Telnet
    25,    // SMTP
    3306,  // MySQL
    5432,  // PostgreSQL
    6379,  // Redis
    27017, // MongoDB
];

/// Check if an IP address is in a private range
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // 10.0.0.0/8
            octets[0] == 10
                // 172.16.0.0/12
                || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
                // 192.168.0.0/16
                || (octets[0] == 192 && octets[1] == 168)
                // 169.254.0.0/16 (link-local)
                || (octets[0] == 169 && octets[1] == 254)
                // 127.0.0.0/8 (loopback)
                || octets[0] == 127
        }
        IpAddr::V6(ipv6) => {
            // Check for IPv6 loopback (::1)
            ipv6.is_loopback()
                // Check for IPv6 link-local (fe80::/10)
                || (ipv6.segments()[0] & 0xffc0) == 0xfe80
                // Check for IPv6 unique local (fc00::/7)
                || (ipv6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

/// Validate a URL is valid, uses http/https scheme, and doesn't point to private resources
pub fn validate_url(url_str: &str) -> Result<Url> {
    let url = Url::parse(url_str)?;

    // Check scheme
    match url.scheme() {
        "http" | "https" => {}
        _ => {
            warn!(
                "Security: Blocked non-HTTP(S) URL scheme: {} in URL: {}",
                url.scheme(),
                url_str
            );
            return Err(Error::Validation(format!(
                "URL must use http or https scheme: {url_str}"
            )));
        }
    }

    // Check for host
    let host = url
        .host_str()
        .ok_or_else(|| Error::Validation("URL must have a valid host".to_string()))?;

    // Block localhost explicitly
    if host == "localhost" || host == "127.0.0.1" || host == "0.0.0.0" {
        warn!("Security: Blocked localhost URL: {}", url_str);
        return Err(Error::Validation(
            "Localhost URLs are not allowed".to_string(),
        ));
    }

    // Try to parse as IP address and check if private or loopback
    // Strip brackets from IPv6 addresses like "[::1]"
    let host_for_ip = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host_for_ip.parse::<IpAddr>() {
        if ip.is_loopback() {
            warn!("Security: Blocked loopback IP: {} in URL: {}", ip, url_str);
            return Err(Error::Validation(
                "Loopback addresses are not allowed".to_string(),
            ));
        }
        if ip.is_unspecified() {
            warn!(
                "Security: Blocked unspecified IP: {} in URL: {}",
                ip, url_str
            );
            return Err(Error::Validation(
                "Unspecified addresses are not allowed".to_string(),
            ));
        }
        if is_private_ip(&ip) {
            warn!("Security: Blocked private IP: {} in URL: {}", ip, url_str);
            return Err(Error::Validation(
                "Private IP addresses are not allowed".to_string(),
            ));
        }
    }

    // Check port restrictions
    if let Some(port) = url.port() {
        if BLOCKED_PORTS.contains(&port) {
            warn!(
                "Security: Blocked restricted port {} in URL: {}",
                port, url_str
            );
            return Err(Error::Validation(format!(
                "Port {port} is not allowed for security reasons"
            )));
        }
    }

    Ok(url)
}

/// Validate difficulty level
pub fn validate_difficulty(difficulty: &str) -> Result<()> {
    match difficulty {
        "easy" | "medium" | "hard" => Ok(()),
        _ => Err(Error::Validation(format!(
            "Invalid difficulty level: {difficulty}. Must be easy, medium, or hard"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url() {
        // Valid URLs
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com").is_ok());
        assert!(validate_url("https://example.com:8080").is_ok());

        // Invalid schemes
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("not-a-url").is_err());

        // Localhost and loopback addresses
        assert!(validate_url("http://localhost").is_err());
        assert!(validate_url("http://127.0.0.1").is_err());
        assert!(validate_url("http://[::1]").is_err());
        assert!(validate_url("http://0.0.0.0").is_err());

        // Private IP ranges
        assert!(validate_url("http://10.0.0.1").is_err());
        assert!(validate_url("http://172.16.0.1").is_err());
        assert!(validate_url("http://192.168.1.1").is_err());
        assert!(validate_url("http://169.254.169.254").is_err()); // AWS metadata

        // Blocked ports
        assert!(validate_url("http://example.com:22").is_err()); // SSH
        assert!(validate_url("http://example.com:3306").is_err()); // MySQL
        assert!(validate_url("http://example.com:5432").is_err()); // PostgreSQL
    }

    #[test]
    fn test_validate_difficulty() {
        assert!(validate_difficulty("easy").is_ok());
        assert!(validate_difficulty("medium").is_ok());
        assert!(validate_difficulty("hard").is_ok());
        assert!(validate_difficulty("impossible").is_err());
    }
}
