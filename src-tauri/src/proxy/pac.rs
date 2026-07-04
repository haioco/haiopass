use crate::domains::fallback::FALLBACK_DOMAINS;

pub fn build_pac_script(domains: &[String], port: u16) -> String {
    let list = serde_json::to_string(domains).unwrap_or_default();
    format!(
        r#"function FindProxyForURL(url, host) {{
  var domains = {list};
  var h = host.toLowerCase();
  for (var i = 0; i < domains.length; i++) {{
    var d = domains[i];
    if (h === d || h.endsWith('.' + d)) {{
      return "PROXY 127.0.0.1:{port}";
    }}
  }}
  return "DIRECT";
}}"#,
        list = list,
        port = port,
    )
}

pub fn build_pac_from_fallback(port: u16) -> String {
    let domains: Vec<String> = FALLBACK_DOMAINS.iter().map(|s| s.to_string()).collect();
    build_pac_script(&domains, port)
}
