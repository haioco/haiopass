//! Domain input normalisation.
//!
//! Users paste all kinds of things into a domain field: full URLs, wildcard
//! prefixes, trailing dots, uppercase, ports. The routing rules only ever see
//! a canonical lowercase hostname (or IP literal), so every user-supplied
//! entry passes through here first.

use std::net::IpAddr;

const MAX_DOMAIN_LEN: usize = 253;

/// Normalise a user-supplied domain or URL to a canonical routing entry.
///
/// `https://API.Example.com:443/path?q=1` → `api.example.com`
/// `*.example.com` → `example.com`
/// `192.168.1.10` → `192.168.1.10`
///
/// Returns `None` for anything that is not a valid hostname or IP literal.
pub fn normalize_domain(input: &str) -> Option<String> {
    let mut value = input.trim();
    if value.is_empty() {
        return None;
    }

    // Strip a scheme (`https://`, `trojan://`, …).
    if let Some((_, rest)) = value.split_once("://") {
        value = rest;
    }

    // Strip userinfo (`user:pass@host`).
    if let Some((_, rest)) = value.rsplit_once('@') {
        value = rest;
    }

    // Strip path / query / fragment.
    let end = value
        .find(|c| c == '/' || c == '?' || c == '#')
        .unwrap_or(value.len());
    value = &value[..end];

    // IPv6 literals may be bracketed and may carry a port (`[::1]:8080`).
    if value.starts_with('[') {
        let close = value.find(']')?;
        let ip = &value[1..close];
        return ip.parse::<IpAddr>().ok().map(|ip| ip.to_string());
    }

    // Bare IP literals (IPv4 or IPv6) are matched exactly.
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip.to_string());
    }

    // Strip a port suffix. Anything left after that which still contains a
    // colon is not a hostname.
    if let Some((host, port)) = value.rsplit_once(':') {
        if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        value = host;
    }
    if value.contains(':') {
        return None;
    }

    // Wildcards mean "this domain and every subdomain" in our matcher, so the
    // canonical entry is simply the bare domain.
    while let Some(rest) = value.strip_prefix("*.") {
        value = rest;
    }
    value = value.trim_start_matches('.');
    value = value.trim_end_matches('.');

    let lower = value.to_ascii_lowercase();
    if !valid_domain_shape(&lower) {
        return None;
    }
    Some(lower)
}

/// A strictly valid lowercase domain: dot-separated labels of a-z, 0-9,
/// hyphen; labels do not start or end with a hyphen and are ≤ 63 chars.
///
/// Shared with the remote-list parser so user input and downloaded entries
/// follow exactly the same shape rules.
pub(crate) fn valid_domain_shape(s: &str) -> bool {
    if s.is_empty() || s.len() > MAX_DOMAIN_LEN {
        return false;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
    {
        return false;
    }
    for label in s.split('.') {
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return false;
        }
    }
    // The last label (TLD) must be alphabetic — "1.2.3.4" and friends are
    // not routable domain names.
    match s.rsplit_once('.') {
        Some((_, tld)) => tld.chars().all(|c| c.is_ascii_lowercase()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_urls_and_wildcards() {
        assert_eq!(
            normalize_domain("https://API.Example.com:443/path?q=1").as_deref(),
            Some("api.example.com")
        );
        assert_eq!(
            normalize_domain("*.example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            normalize_domain(".Example.COM.").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            normalize_domain("  example.com  ").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn keeps_ip_literals() {
        assert_eq!(
            normalize_domain("192.168.1.10").as_deref(),
            Some("192.168.1.10")
        );
        assert_eq!(normalize_domain("[::1]:8080").as_deref(), Some("::1"));
    }

    #[test]
    fn rejects_garbage() {
        for input in [
            "",
            "   ",
            "not a domain",
            "example",
            "http://",
            "exämple.com",
            "bad_.com",
            "-bad.com",
            "bad-.com",
            "1.2.3.4:8080", // port on an IP is not a rule entry
            "[nope]",
        ] {
            assert!(normalize_domain(input).is_none(), "accepted {input:?}");
        }
    }

    #[test]
    fn accepts_punycode_and_hyphenated_labels() {
        assert_eq!(
            normalize_domain("xn--80ak6aa92e.com").as_deref(),
            Some("xn--80ak6aa92e.com")
        );
        assert_eq!(
            normalize_domain("a-b.example.co.uk").as_deref(),
            Some("a-b.example.co.uk")
        );
    }
}
