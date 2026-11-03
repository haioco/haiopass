use crate::domains::fallback::FALLBACK_DOMAINS;

/// Build the PAC script served at `/pac.js`.
///
/// Order matters: the bypass list is checked first, so a host that appears in
/// both lists always goes DIRECT. Matching is the same "host or subdomain"
/// rule the in-process router uses.
pub fn build_pac_script(domains: &[String], bypass_domains: &[String], port: u16) -> String {
    // A JSON object turns lookup into a hash probe. The previous array +
    // linear scan did up to 20k allocations for every URL the OS resolved.
    let list = domain_object(domains);
    let bypass = domain_object(bypass_domains);
    format!(
        r#"function haioMatch(host, domains) {{
  var h = host;
  while (h.length > 0) {{
    if (domains[h] === 1) {{
      return true;
    }}
    var dot = h.indexOf('.');
    if (dot === -1) {{
      return false;
    }}
    h = h.slice(dot + 1);
  }}
  return false;
}}

function FindProxyForURL(url, host) {{
  var bypass = {bypass};
  var domains = {list};
  var h = host.toLowerCase();
  if (h.charAt(h.length - 1) === '.') {{
    h = h.slice(0, -1);
  }}
  if (haioMatch(h, bypass)) {{
    return "DIRECT";
  }}
  if (haioMatch(h, domains)) {{
    return "PROXY 127.0.0.1:{port}";
  }}
  return "DIRECT";
}}"#,
        list = list,
        bypass = bypass,
        port = port,
    )
}

fn domain_object(domains: &[String]) -> String {
    let map: std::collections::BTreeMap<&str, u8> =
        domains.iter().map(|d| (d.as_str(), 1)).collect();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
}

pub fn build_pac_from_fallback(port: u16) -> String {
    let domains: Vec<String> = FALLBACK_DOMAINS.iter().map(|s| s.to_string()).collect();
    build_pac_script(&domains, &[], port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pac_checks_bypass_before_proxy_list() {
        let script = build_pac_script(
            &["google.com".to_string()],
            &["ads.google.com".to_string()],
            11032,
        );
        let allow_pos = script.find("haioMatch(h, bypass)").unwrap();
        let proxy_pos = script.find("haioMatch(h, domains)").unwrap();
        assert!(allow_pos < proxy_pos);
        assert!(script.contains("127.0.0.1:11032"));
        assert!(script.contains("\"ads.google.com\""));
    }

    #[test]
    fn pac_uses_hash_lookup_without_scanning_every_domain() {
        let script = build_pac_script(&["example.com".to_string()], &[], 1);
        assert!(script.contains("domains[h] === 1"));
        assert!(script.contains("h = h.slice(dot + 1)"));
        assert!(!script.contains("for (var i"));
    }
}
