use crate::config::TrojanConfig;

pub fn parse_trojan_url(uri: &str) -> Option<TrojanConfig> {
    let uri = uri.trim();
    let re = regex::Regex::new(
        r"(?i)^trojan://([^@]+)@([^:/?#]+):(\d+)(?:\?([^#]*))?"
    ).ok()?;

    let caps = re.captures(uri)?;
    let password = urlencoding::decode(&caps[1]).ok()?.to_string();
    let server = caps[2].to_string();
    let port: u16 = caps[3].parse().ok()?;

    let sni = caps.get(4)
        .and_then(|m| {
            let params: std::collections::HashMap<String, String> =
                m.as_str().split('&')
                    .filter_map(|p| {
                        let mut parts = p.splitn(2, '=');
                        Some((parts.next()?.to_string(), parts.next()?.to_string()))
                    })
                    .collect();
            params.get("sni").cloned()
        })
        .unwrap_or_else(|| server.clone());

    if password.is_empty() || server.is_empty() || port == 0 {
        return None;
    }

    Some(TrojanConfig {
        password,
        server,
        port,
        sni,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic() {
        let cfg = parse_trojan_url("trojan://mypassword@server.example.com:443?sni=server.example.com#tag")
            .unwrap();
        assert_eq!(cfg.password, "mypassword");
        assert_eq!(cfg.server, "server.example.com");
        assert_eq!(cfg.port, 443);
        assert_eq!(cfg.sni, "server.example.com");
    }

    #[test]
    fn test_parse_no_sni() {
        let cfg = parse_trojan_url("trojan://pass@1.2.3.4:443").unwrap();
        assert_eq!(cfg.sni, "1.2.3.4");
    }

    #[test]
    fn test_invalid() {
        assert!(parse_trojan_url("not-a-url").is_none());
    }
}
