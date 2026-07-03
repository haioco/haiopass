use tokio::net::TcpStream;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn check_proxy_health(
    http_proxy_port: u16,
    _cached_domains: &[String],
) -> (bool, u16) {
    let addr = format!("127.0.0.1:{}", http_proxy_port);

    let ok = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect(&addr),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .is_some();

    (ok, http_proxy_port)
}

pub async fn check_socks5_health(socks_port: u16) -> bool {
    let addr = format!("127.0.0.1:{}", socks_port);
    tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect(&addr),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .is_some()
}

pub async fn check_trojan_health(
    http_proxy_port: u16,
    socks_port: u16,
) -> TrojonHealthResult {
    let http_ok = check_proxy_health(http_proxy_port, &[]).await.0;
    let socks_ok = check_socks5_health(socks_port).await;

    TrojonHealthResult {
        http_proxy_ok: http_ok,
        socks5_ok: socks_ok,
        all_healthy: http_ok && socks_ok,
    }
}

pub struct TrojonHealthResult {
    pub http_proxy_ok: bool,
    pub socks5_ok: bool,
    pub all_healthy: bool,
}
