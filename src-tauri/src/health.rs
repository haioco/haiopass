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
    // Run both probes concurrently: the local port check is cheap, but the
    // TLS probe can take up to a few seconds on a bad link. Sequential checks
    // made the UI wait for the sum of both timeouts.
    let http_probe = check_proxy_health(http_proxy_port, &[]);
    let tunnel_probe = async {
        match trojan_config {
            Some(cfg) => check_tunnel_health(cfg).await,
            // No credentials means we cannot prove the tunnel works.
            None => false,
        }
    };
    let ((http_ok, _), tunnel_ok) = tokio::join!(http_probe, tunnel_probe);

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
