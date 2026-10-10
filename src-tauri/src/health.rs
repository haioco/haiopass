use std::time::Duration;

use tokio::net::TcpStream;

use crate::config::TrojanConfig;

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

/// Whether the configured Trojan server is reachable and its certificate
/// still validates. With the in-process client there is no local SOCKS port
/// to poll — the server itself is the thing that can be up or down.
pub async fn check_tunnel_health(config: &TrojanConfig) -> bool {
    crate::trojan::client::probe(config).await
}

pub async fn check_trojan_health(
    http_proxy_port: u16,
    trojan_config: Option<&TrojanConfig>,
) -> TrojanHealthResult {
    let http_ok = check_proxy_health(http_proxy_port, &[]).await.0;
    // With no credentials set the tunnel is "healthy by definition" — the
    // HTTP proxy is what matters, and the health monitor only runs while
    // the proxy is enabled.
    let tunnel_ok = match trojan_config {
        Some(cfg) => check_tunnel_health(cfg).await,
        None => false,
    };

    TrojanHealthResult {
        http_proxy_ok: http_ok,
        tunnel_ok,
        all_healthy: http_ok && tunnel_ok,
    }
}

pub struct TrojanHealthResult {
    pub http_proxy_ok: bool,
    pub tunnel_ok: bool,
    pub all_healthy: bool,
}
