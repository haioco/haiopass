pub async fn check_proxy_health(
    http_proxy_port: u16,
    cached_domains: &[String],
) -> (bool, u16) {
    let targets: Vec<String> = if !cached_domains.is_empty() {
        cached_domains.iter().take(3).map(|d| format!("https://{}/", d)).collect()
    } else {
        vec!["https://www.youtube.com/".to_string()]
    };

    let proxy_addr = format!("http://127.0.0.1:{}", http_proxy_port);
    let proxy = match reqwest::Proxy::http(&proxy_addr) {
        Ok(p) => p,
        Err(_) => return (false, http_proxy_port),
    };
    let client = match reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(6))
        .build()
    {
        Ok(c) => c,
        Err(_) => return (false, http_proxy_port),
    };

    for url in &targets {
        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() || resp.status().is_server_error() {
                return (true, http_proxy_port);
            }
        }
    }

    (false, http_proxy_port)
}
