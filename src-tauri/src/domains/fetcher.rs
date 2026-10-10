use std::collections::HashSet;

use crate::domains::fallback::DOMAINS_URLS;

/// Shape pins: a domain list that deviates from these is treated as a
/// compromised endpoint, not as data. A remote list may only ever influence
/// *domain routing* — the parsed result feeds `cached_domains` and the
/// router, nothing else. It can never change the server, credentials, or
/// enable features, and no other state is written from fetch results.
const MAX_LIST_BYTES: usize = 512 * 1024;
const MAX_DOMAINS: usize = 20_000;
const MAX_DOMAIN_LEN: usize = 253;

/// A strictly valid lowercase domain: dot-separated labels of a-z, 0-9,
/// hyphen; labels do not start or end with a hyphen and are ≤ 63 chars.
fn valid_domain_shape(s: &str) -> bool {
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

pub async fn fetch_domains() -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    for url in DOMAINS_URLS {
        match client.get(*url).header("Cache-Control", "no-cache").send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    if let Ok(text) = resp.text().await {
                        // Shape pin: reject oversized or absurd payloads
                        // before parsing anything.
                        if text.len() > MAX_LIST_BYTES {
                            tracing::error!(
                                "Domain list from {} is {} bytes (cap {}); rejecting as anomalous",
                                url,
                                text.len(),
                                MAX_LIST_BYTES
                            );
                            return Err(format!(
                                "domain list too large ({} bytes, cap {})",
                                text.len(),
                                MAX_LIST_BYTES
                            ));
                        }
                        let domains = parse_domain_text(&text);
                        if domains.len() > MAX_DOMAINS {
                            tracing::error!(
                                "Domain list from {} has {} entries (cap {}); rejecting as anomalous",
                                url,
                                domains.len(),
                                MAX_DOMAINS
                            );
                            return Err(format!(
                                "domain list has too many entries ({} , cap {})",
                                domains.len(),
                                MAX_DOMAINS
                            ));
                        }
                        if !domains.is_empty() {
                            return Ok(domains);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Domain fetch failed ({}): {}", url, e);
            }
        }
    }

    Err("All domain sources failed".into())
}

fn parse_domain_text(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.lines()
        .map(|l| l.trim().to_lowercase())
        .filter(|l| {
            if l.is_empty() || l.starts_with('#') {
                return false;
            }
            // Charset pin: anything outside [a-z0-9.-] — including paths,
            // ports, whitespace, or non-ASCII — is rejected outright.
            if !valid_domain_shape(l) {
                return false;
            }
            // Reject obvious DNS-infrastructure entries
            if l.starts_with("ns") && l.contains('.')
                || l.ends_with("-hostmaster.com")
                || l.ends_with("-hostmaster.net")
                || l.ends_with("-hostmaster.org")
                || l.starts_with("dns-admin.")
                || l.starts_with("hostmaster.")
                || l.starts_with("dns1.") && l.contains("nsone.net")
            {
                return false;
            }
            // Deduplicate
            if seen.contains(l) {
                return false;
            }
            seen.insert(l.clone());
            true
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_domain_text() {
        let input = "# comment\nyoutube.com\ngoogle.com\nns1.google.com\nhostmaster.nsone.net\ndl.google.com/android/repository\n192.168.1.1:8080\n\n";
        let result = parse_domain_text(input);
        let youtube = "youtube.com".to_string();
        let google = "google.com".to_string();
        let ns1 = "ns1.google.com".to_string();
        let hostmaster = "hostmaster.nsone.net".to_string();
        let dl_path = "dl.google.com/android/repository".to_string();
        let ip_port = "192.168.1.1:8080".to_string();
        let comment = "# comment".to_string();
        assert!(result.contains(&youtube));
        assert!(result.contains(&google));
        assert!(!result.contains(&ns1));
        assert!(!result.contains(&hostmaster));
        assert!(!result.contains(&dl_path));
        assert!(!result.contains(&ip_port));
        assert!(!result.contains(&comment));
    }

    #[test]
    fn rejects_ip_addresses_and_nonascii() {
        let result = parse_domain_text("1.2.3.4\nexämple.com\nexample.com\n");
        assert_eq!(result, vec!["example.com".to_string()]);
    }

    #[test]
    fn rejects_hyphen_edges_and_bare_hosts() {
        let result = parse_domain_text("-bad.com\nbad-.com\nlocalhost\n12345\n");
        assert!(result.is_empty());
    }

    #[test]
    fn accepts_normal_domains() {
        let result = parse_domain_text("a-b.example.com\nxn--80ak6aa92e.com\n");
        assert_eq!(
            result,
            vec!["a-b.example.com".to_string(), "xn--80ak6aa92e.com".to_string()]
        );
    }
}
