use tokio::net::TcpStream;

pub async fn check_proxy_health(
    http_proxy_port: u16,
    _cached_domains: &[String],
) -> (bool, u16) {
    let addr = format!("127.0.0.1:{}", http_proxy_port);

    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        TcpStream::connect(&addr),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .map(|_| (true, http_proxy_port))
    .unwrap_or((false, http_proxy_port))
}
