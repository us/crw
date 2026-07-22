use std::net::{IpAddr, ToSocketAddrs};

/// Build a reqwest redirect policy that validates each redirect target
/// against the SSRF safety checks. This prevents attackers from using
/// `https://evil.com` → 302 → `http://169.254.169.254/metadata` bypasses.
pub fn safe_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else if let Err(e) = validate_safe_url_blocking_resolved(attempt.url()) {
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
///
/// **Test-only escape hatch**: when the env var
/// `CRW_ALLOW_LOOPBACK_FOR_TESTS=1` is set, the loopback/private-range
/// checks are skipped so wiremock-backed integration tests can target
/// `127.0.0.1:<random>`. The opt-in is read at every call (cheap) so it
/// can be flipped per-test. Never set this in production.
pub fn validate_safe_url(url: &url::Url) -> Result<(), String> {
    let test_allow_loopback = std::env::var("CRW_ALLOW_LOOPBACK_FOR_TESTS").as_deref() == Ok("1");
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

    if !test_allow_loopback && is_blocked_host_name(host) {
        return Err("host is not allowed".into());
    }

    // Parse as IP if possible and check ranges
    if !test_allow_loopback
        && let Ok(ip) = host.parse::<IpAddr>()
        && is_blocked_ip(&ip)
    {
        return Err(format!("Access to {ip} is not allowed"));
    }

    // Also check bracket-stripped IPv6 (e.g. [::1])
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if !test_allow_loopback
        && let Ok(ip) = stripped.parse::<IpAddr>()
        && is_blocked_ip(&ip)
    {
        return Err(format!("Access to {ip} is not allowed"));
    }

    Ok(())
}

/// Validate the URL and resolve DNS for hostname targets, rejecting any private,
/// loopback, link-local, or otherwise non-public address. This is intentionally
/// fail-closed: if DNS cannot be resolved, callers cannot prove the destination
/// is public and must not fetch it.
pub async fn validate_safe_url_resolved(url: &url::Url) -> Result<(), String> {
    validate_safe_url(url)?;

    let test_allow_loopback = std::env::var("CRW_ALLOW_LOOPBACK_FOR_TESTS").as_deref() == Ok("1");
    if test_allow_loopback {
        return Ok(());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if stripped.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "URL has no resolvable port".to_string())?;
    // Bound the lookup. `lookup_host` inherits the OS resolver's own retry
    // behaviour, which against a stalled or blackholed nameserver can hang for
    // tens of seconds — long past any route backstop. This is the single choke
    // point every SSRF caller routes through, so bounding it here covers the ~14
    // callers that have no request-scoped timeout of their own; the two that do
    // (crw-crawl seed + BFS) keep winning since the tighter timeout applies.
    // Fails CLOSED: an elapsed lookup maps to the same rejection a resolver error
    // already produces.
    let addrs = tokio::time::timeout(
        DNS_RESOLVE_TIMEOUT,
        tokio::net::lookup_host((stripped, port)),
    )
    .await
    .map_err(|_| "DNS resolution timed out".to_string())?
    .map_err(|_| "DNS resolution failed".to_string())?;
    validate_resolved_ips(addrs.map(|addr| addr.ip()))
}

/// Upper bound on a single SSRF DNS resolution. Purpose is to bound a stalled /
/// blackholed resolver, not to race a slow-but-working one: recall is the hard
/// invariant, so this sits high enough (8s) to still resolve a legitimate host
/// behind a CNAME chain or a slow authoritative server that needs a UDP retry,
/// while staying under the search enrichment per-result budget. Two orders of
/// magnitude above a healthy lookup (tens of ms).
const DNS_RESOLVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// Synchronous resolved validation for places that cannot await, notably
/// reqwest's redirect-policy callback. Keep use narrow; it performs DNS on the
/// current thread and intentionally fails closed.
pub fn validate_safe_url_blocking_resolved(url: &url::Url) -> Result<(), String> {
    validate_safe_url(url)?;

    let test_allow_loopback = std::env::var("CRW_ALLOW_LOOPBACK_FOR_TESTS").as_deref() == Ok("1");
    if test_allow_loopback {
        return Ok(());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if stripped.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "URL has no resolvable port".to_string())?;
    (stripped, port)
        .to_socket_addrs()
        .map_err(|_| "DNS resolution failed".to_string())
        .and_then(|addrs| validate_resolved_ips(addrs.map(|addr| addr.ip())))
}

pub fn validate_resolved_ips<I>(ips: I) -> Result<(), String>
where
    I: IntoIterator<Item = IpAddr>,
{
    let mut saw_ip = false;
    for ip in ips {
        saw_ip = true;
        if is_blocked_ip(&ip) {
            return Err(format!("Access to {ip} is not allowed"));
        }
    }
    if !saw_ip {
        return Err("DNS resolution returned no addresses".into());
    }
    Ok(())
}

fn is_blocked_host_name(host: &str) -> bool {
    let host_lower = host.to_lowercase();
    let host_lower = host_lower.trim_end_matches('.');
    host_lower == "localhost"
        || host_lower == "metadata.google.internal"
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".localtest.me")
        || host_lower.ends_with(".lvh.me")
        || host_lower.ends_with(".nip.io")
        || host_lower.ends_with(".xip.io")
        || host_lower.ends_with(".sslip.io")
}

fn is_blocked_ipv4(v4: &std::net::Ipv4Addr) -> bool {
    let [a, b, _, _] = v4.octets();
    v4.is_loopback()                       // 127.0.0.0/8
        || v4.is_private()                 // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
        || v4.is_link_local()              // 169.254.0.0/16 (metadata)
        || v4.is_unspecified()             // 0.0.0.0
        || v4.is_broadcast()               // 255.255.255.255
        || (a == 100 && (64..=127).contains(&b)) // carrier-grade NAT
        || a == 0
        || a >= 224                        // multicast/reserved
        || (a == 192 && b == 0)
        || (a == 198 && (b == 18 || b == 19 || b == 51))
        || (a == 203 && b == 0)
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
                // IPv6 multicast and special/reserved ranges.
                || (v6.segments()[0] & 0xff00) == 0xff00
                || (v6.segments()[0] & 0xff00) == 0x0200
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
        assert!(validate_safe_url(&url("http://metadata.google.internal")).is_err());
        assert!(validate_safe_url(&url("http://127.0.0.1.nip.io")).is_err());
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
        assert!(validate_safe_url(&url("http://100.64.0.1")).is_err());
        assert!(validate_safe_url(&url("http://224.0.0.1")).is_err());
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

    #[test]
    fn resolved_ips_fail_closed_for_private_or_empty_answers() {
        assert!(validate_resolved_ips([IpAddr::from([93, 184, 216, 34])]).is_ok());
        assert!(
            validate_resolved_ips([
                IpAddr::from([93, 184, 216, 34]),
                IpAddr::from([10, 0, 0, 1])
            ])
            .is_err()
        );
        assert!(validate_resolved_ips([]).is_err());
    }

    #[test]
    fn blocking_resolved_validation_rejects_denied_literal_ips() {
        assert!(validate_safe_url_blocking_resolved(&url("http://169.254.169.254")).is_err());
        assert!(validate_safe_url_blocking_resolved(&url("http://127.0.0.1")).is_err());
        assert!(validate_safe_url_blocking_resolved(&url("http://[::1]")).is_err());
    }

    #[tokio::test]
    async fn resolution_fails_closed_and_bounded() {
        // `.invalid` is reserved (RFC 6761) and never resolves. The result must be
        // a fail-closed `Err`, and it must return in well under the DNS timeout
        // (proving the wrapper doesn't hang the caller). NXDOMAIN returns fast; the
        // bound here only asserts we never sit past the timeout.
        let start = std::time::Instant::now();
        let res = validate_safe_url_resolved(&url("https://nonexistent.invalid")).await;
        assert!(res.is_err());
        assert!(start.elapsed() < DNS_RESOLVE_TIMEOUT + std::time::Duration::from_secs(1));
    }
}
