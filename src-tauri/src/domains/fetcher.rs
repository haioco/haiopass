use std::collections::HashSet;

use crate::domains::fallback::DOMAINS_URLS;

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
                        let domains = parse_domain_text(&text);
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
            // Reject lines that contain paths, ports, or whitespace
            if l.contains('/') || l.contains(':') || l.contains(char::is_whitespace) {
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
}
