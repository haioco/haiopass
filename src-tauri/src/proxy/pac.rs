use crate::domains::fallback::FALLBACK_DOMAINS;

/// Build the PAC script served at `/pac.js`.
///
/// Order matters: the bypass list is checked first, so a host that appears in
/// both lists always goes DIRECT. Matching is the same "host or subdomain"
/// rule the in-process router uses.
pub fn build_pac_script(domains: &[String], bypass_domains: &[String], port: u16) -> String {
    let list = serde_json::to_string(domains).unwrap_or_else(|_| "[]".into());
    let bypass = serde_json::to_string(bypass_domains).unwrap_or_else(|_| "[]".into());
    format!(
        r#"function haioMatch(host, list) {{
  for (var i = 0; i < list.length; i++) {{
    var d = list[i];
    if (host === d || host.slice(-(d.length + 1)) === '.' + d) {{
      return true;
    }}
  }}
  return false;
}}

function FindProxyForURL(url, host) {{
  var bypass = {bypass};
  var domains = {list};
  var h = host.toLowerCase();
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
    fn pac_handles_suffix_without_allocating_in_js() {
        let script = build_pac_script(&["example.com".to_string()], &[], 1);
        assert!(script.contains("host.slice(-(d.length + 1)) === '.' + d"));
    }
}
