use std::net::IpAddr;

/// Build a reqwest redirect policy that validates each redirect target
/// against the SSRF safety checks. This prevents attackers from using
/// `https://evil.com` → 302 → `http://169.254.169.254/metadata` bypasses.
pub fn safe_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else if let Err(e) = validate_safe_url(attempt.url()) {
            attempt.error(format!("redirect blocked: {e}"))
        } else {
            attempt.follow()
        }
    })
}

/// Validate that a URL is safe to fetch (not targeting internal/private networks).
///
/// Blocks:
/// - Non-http(s) schemes
/// - Loopback addresses (127.x, ::1, localhost)
/// - Private IP ranges (10.x, 172.16-31.x, 192.168.x)
/// - Link-local addresses (169.254.x — e.g. AWS metadata endpoint)
/// - 0.0.0.0
pub fn validate_safe_url(url: &url::Url) -> Result<(), String> {
    // URL length limit
    const MAX_URL_LENGTH: usize = 2048;
    if url.as_str().len() > MAX_URL_LENGTH {
        return Err(format!(
            "URL exceeds maximum length of {MAX_URL_LENGTH} characters"
        ));
    }

    // Null byte check
    if url.as_str().contains("%00") || url.as_str().contains('\0') {
        return Err("URL contains null bytes".into());
    }

    // Scheme check
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http/https URLs are allowed".into());
    }

    // Host check
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    // Block localhost by name
    let host_lower = host.to_lowercase();
    if host_lower == "localhost" || host_lower.ends_with(".localhost") {
        return Err("Localhost URLs are not allowed".into());
    }

    // Parse as IP if possible and check ranges
    if let Ok(ip) = host.parse::<IpAddr>()
        && is_blocked_ip(&ip)
    {
        return Err(format!("Access to {ip} is not allowed"));
    }

    // Also check bracket-stripped IPv6 (e.g. [::1])
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = stripped.parse::<IpAddr>()
        && is_blocked_ip(&ip)
    {
        return Err(format!("Access to {ip} is not allowed"));
    }

    Ok(())
}

fn is_blocked_ipv4(v4: &std::net::Ipv4Addr) -> bool {
    v4.is_loopback()                       // 127.0.0.0/8
        || v4.is_private()                  // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
        || v4.is_link_local()               // 169.254.0.0/16 (AWS metadata)
        || v4.is_unspecified()              // 0.0.0.0
        || v4.is_broadcast() // 255.255.255.255
}

fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => {
            v6.is_loopback()                       // ::1
                || v6.is_unspecified()              // ::
                // IPv4-mapped IPv6 (::ffff:127.0.0.1, ::ffff:10.x.x.x, etc.)
                || v6.to_ipv4_mapped().is_some_and(|v4| is_blocked_ipv4(&v4))
                // IPv6 link-local (fe80::/10)
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv6 unique-local / ULA (fc00::/7) — private network equivalent
                || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    #[test]
    fn allows_normal_urls() {
        assert!(validate_safe_url(&url("https://example.com")).is_ok());
        assert!(validate_safe_url(&url("http://example.com/path")).is_ok());
    }

    #[test]
    fn blocks_non_http_schemes() {
        assert!(validate_safe_url(&url("ftp://example.com")).is_err());
        assert!(validate_safe_url(&url("file:///etc/passwd")).is_err());
    }

    #[test]
    fn blocks_localhost() {
        assert!(validate_safe_url(&url("http://localhost")).is_err());
        assert!(validate_safe_url(&url("http://localhost:8080")).is_err());
        assert!(validate_safe_url(&url("http://127.0.0.1")).is_err());
        assert!(validate_safe_url(&url("http://127.0.0.1:9999")).is_err());
    }

    #[test]
    fn blocks_private_ips() {
        assert!(validate_safe_url(&url("http://10.0.0.1")).is_err());
        assert!(validate_safe_url(&url("http://172.16.0.1")).is_err());
        assert!(validate_safe_url(&url("http://192.168.1.1")).is_err());
    }

    #[test]
    fn blocks_link_local() {
        assert!(validate_safe_url(&url("http://169.254.169.254/latest/meta-data/")).is_err());
    }

    #[test]
    fn blocks_zero_ip() {
        assert!(validate_safe_url(&url("http://0.0.0.0")).is_err());
    }

    #[test]
    fn blocks_ipv6_loopback() {
        assert!(validate_safe_url(&url("http://[::1]")).is_err());
    }

    #[test]
    fn blocks_ipv4_mapped_ipv6() {
        // ::ffff:127.0.0.1 — IPv4-mapped loopback
        assert!(validate_safe_url(&url("http://[::ffff:127.0.0.1]")).is_err());
        // ::ffff:169.254.169.254 — IPv4-mapped AWS metadata
        assert!(validate_safe_url(&url("http://[::ffff:169.254.169.254]")).is_err());
        // ::ffff:10.0.0.1 — IPv4-mapped private
        assert!(validate_safe_url(&url("http://[::ffff:10.0.0.1]")).is_err());
    }

    #[test]
    fn blocks_ipv6_link_local() {
        assert!(validate_safe_url(&url("http://[fe80::1]")).is_err());
    }

    #[test]
    fn blocks_ipv6_ula() {
        assert!(validate_safe_url(&url("http://[fc00::1]")).is_err());
        assert!(validate_safe_url(&url("http://[fd00::1]")).is_err());
    }

    #[test]
    fn blocks_extremely_long_urls() {
        let long = format!("https://example.com/{}", "a".repeat(3000));
        assert!(validate_safe_url(&url(&long)).is_err());
    }

    #[test]
    fn allows_url_within_length_limit() {
        let ok = format!("https://example.com/{}", "a".repeat(1000));
        assert!(validate_safe_url(&url(&ok)).is_ok());
    }

    #[test]
    fn safe_redirect_policy_exists() {
        // Verify the policy is constructible (runtime redirect validation
        // is tested via integration tests with actual HTTP redirects).
        let _policy = super::safe_redirect_policy();
    }
}
