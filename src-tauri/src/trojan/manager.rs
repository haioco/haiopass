//! Shared holder for the active tunnel configuration.
//!
//! With the in-process Trojan client (`client.rs`) there is no child process
//! to start, watch or kill. What the manager used to do — extract a binary,
//! spawn it, restart it — reduces to: which server credentials are live right
//! now. Holding them behind an `Arc<RwLock>` means every proxied connection
//! reads the current value, so a credential/server change takes effect on
//! the next dial with no restart and no reconnect.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::TrojanConfig;

/// The shared configuration handle consumed by the proxy server.
pub type SharedTrojanConfig = Arc<RwLock<Option<TrojanConfig>>>;

#[derive(Default)]
pub struct TrojanManager {
    config: SharedTrojanConfig,
}

impl TrojanManager {
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(None)),
        }
    }

    /// The handle the proxy server dials with. Cloning is cheap; the value
    /// behind it is whatever `set`/`clear` last installed.
    pub fn shared_config(&self) -> SharedTrojanConfig {
        self.config.clone()
    }

    /// Make `config` the live tunnel configuration for all new connections.
    pub async fn set(&self, config: TrojanConfig) {
        tracing::info!(
            "Tunnel configuration set (server {}:{})",
            config.server,
            config.port
        );
        *self.config.write().await = Some(config);
    }

    /// Deactivate the tunnel. New proxied connections will be refused until
    /// `set` is called again.
    pub async fn clear(&self) {
        *self.config.write().await = None;
        tracing::info!("Tunnel configuration cleared");
    }

    /// Whether a tunnel configuration is live.
    pub async fn is_connected(&self) -> bool {
        self.config.read().await.is_some()
    }
}
