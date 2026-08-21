use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::sync::Notify;
use crate::config::TrojanConfig;
use crate::trojan::config_writer;

// Neutral runtime name to reduce AV heuristics. The extracted binary on disk
// is named "haio-proxy" (or "haio-proxy.exe" on Windows) regardless of upstream.
const TROJAN_BINARY: &str = "haio-proxy";
const MAX_RESTART_ATTEMPTS: u8 = 3;
const RESTART_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const WATCHDOG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

pub struct TrojanManager {
    child: Option<Child>,
    binary_path: PathBuf,
    config_path: PathBuf,
    log_path: PathBuf,
    saved_config: Option<TrojanConfig>,
    saved_port: Option<u16>,
    watchdog_handle: Option<tokio::task::JoinHandle<()>>,
    restart_notify: Arc<Notify>,
}

impl Default for TrojanManager {
    fn default() -> Self {
        Self::new()
    }
}

use std::sync::Arc;

impl TrojanManager {
    pub fn new() -> Self {
        let config_dir = super::super::config::Store::config_dir();
        let ext = if cfg!(windows) { ".exe" } else { "" };
        Self {
            child: None,
            binary_path: config_dir.join(format!("{}{}", TROJAN_BINARY, ext)),
            config_path: config_dir.join("config.json"),
            log_path: config_dir.join("haio-proxy.log"),
            saved_config: None,
            saved_port: None,
            watchdog_handle: None,
            restart_notify: Arc::new(Notify::new()),
        }
    }

    pub async fn ensure_binary(&mut self) -> crate::error::Result<()> {
        // Always re-extract the bundled binary and overwrite any cached copy.
        // This guarantees upgrades replace stale/corrupt binaries (e.g. a wrong-arch
        // binary left behind by an earlier broken release) instead of silently
        // keeping the broken file and failing with ERROR_EXE_MACHINE_TYPE_MISMATCH.
        super::bundled::extract_bundled(&self.binary_path)?;
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&self.binary_path, std::fs::Permissions::from_mode(0o755)).await?;
        }
        tracing::info!("Ensured trojan-go binary at {}", self.binary_path.display());
        Ok(())
    }

    pub async fn start(&mut self, config: TrojanConfig, local_port: u16) -> crate::error::Result<()> {
        if let Some(ref mut child) = self.child {
            if child.try_wait().ok().flatten().is_none() {
                tracing::info!("Trojan already running (pid {:?})", child.id());
                return Ok(());
            }
        }

        self.ensure_binary().await?;
        config_writer::write_config(&self.config_path, &config, local_port)?;

        let log_file = std::fs::File::create(&self.log_path)
            .map_err(|e| crate::error::HaioError::Trojan(format!("Failed to create log file: {}", e)))?;

        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("--config").arg(&self.config_path);
        cmd.stdout(Stdio::from(log_file.try_clone()
            .map_err(|e| crate::error::HaioError::Trojan(format!("Failed to clone log file: {}", e)))?));
        cmd.stderr(Stdio::from(log_file));

        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let child = cmd.spawn().map_err(|e| crate::error::HaioError::Trojan(format!("Failed to start: {}", e)))?;
        tracing::info!("Started trojan-go pid {:?}", child.id());
        self.child = Some(child);

        // Save config for restarts
        self.saved_config = Some(config);
        self.saved_port = Some(local_port);

        // Start watchdog
        self.start_watchdog();

        Ok(())
    }

    pub async fn stop(&mut self) -> crate::error::Result<()> {
        // Cancel watchdog first
        self.stop_watchdog();

        // Clear saved config to prevent watchdog from restarting
        self.saved_config = None;
        self.saved_port = None;

        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
            tracing::info!("Stopped trojan-go");
        }
        // Watchdog spawns orphan processes not tracked in self.child.
        // Ensure any stray haio-proxy instance is killed on disconnect,
        // otherwise VPN appears to stay connected after UI shows disconnected.
        Self::kill_stray_processes().await;
        Ok(())
    }

    async fn kill_stray_processes() {
        // Best-effort: kill any lingering haio-proxy process by image name.
        // This covers watchdog-orphaned children that are not in self.child.
        #[cfg(windows)]
        {
            let _ = tokio::process::Command::new("taskkill")
                .args(["/F", "/IM", "haio-proxy.exe", "/T"])
                .output()
                .await;
        }
        #[cfg(not(windows))]
        {
            // pkill -9 by exact name; ignore errors if not found
            let _ = tokio::process::Command::new("pkill")
                .args(["-9", "-x", "haio-proxy"])
                .output()
                .await;
            // Fallback: pkill -f for full path match
            let _ = tokio::process::Command::new("pkill")
                .args(["-9", "-f", "haio-proxy"])
                .output()
                .await;
        }
        // Give OS time to release the SOCKS port
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    pub fn status(&mut self) -> (bool, Option<u32>) {
        match &mut self.child {
            Some(c) => {
                match c.try_wait() {
                    Ok(Some(_)) => (false, None),
                    Ok(None) => (true, c.id()),
                    Err(_) => (false, None),
                }
            }
            None => (false, None),
        }
    }

    fn start_watchdog(&mut self) {
        self.stop_watchdog();

        let binary_path = self.binary_path.clone();
        let config_path = self.config_path.clone();
        let log_path = self.log_path.clone();
        let saved_config = self.saved_config.clone();
        let saved_port = self.saved_port;
        let notify = self.restart_notify.clone();

        if saved_config.is_none() || saved_port.is_none() {
            return;
        }

        let handle = tokio::spawn(async move {
            let mut restart_count: u8 = 0;

            loop {
                tokio::time::sleep(WATCHDOG_INTERVAL).await;

                // Check if restart was requested (after a crash)
                tokio::select! {
                    _ = notify.notified() => {
                        // Reset restart count on explicit notification
                        restart_count = 0;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(0)) => {}
                }

                // Check if process is alive
                // We can't check directly from outside, so we check the port
                let config = match &saved_config {
                    Some(c) => c,
                    None => break,
                };
                let port = match saved_port {
                    Some(p) => p,
                    None => break,
                };

                // Quick port check
                let addr = format!("127.0.0.1:{}", port);
                let alive = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    tokio::net::TcpStream::connect(&addr),
                )
                .await
                .ok()
                .and_then(|r| r.ok())
                .is_some();

                if alive {
                    restart_count = 0;
                    continue;
                }

                // Process is dead, attempt restart
                if restart_count >= MAX_RESTART_ATTEMPTS {
                    tracing::error!(
                        "Trojan-go crashed and failed to restart after {} attempts",
                        MAX_RESTART_ATTEMPTS
                    );
                    break;
                }

                restart_count += 1;
                tracing::warn!(
                    "Trojan-go appears down, restarting (attempt {}/{})",
                    restart_count,
                    MAX_RESTART_ATTEMPTS
                );

                tokio::time::sleep(RESTART_INTERVAL).await;

                // Re-extract binary, overwriting stale/corrupt copies
                if let Err(e) = super::bundled::extract_bundled(&binary_path) {
                    tracing::error!("Failed to re-extract trojan-go binary: {}", e);
                    continue;
                }
                #[cfg(not(windows))]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = tokio::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755)).await;
                }

                // Rewrite config (in case it was deleted)
                if let Err(e) = config_writer::write_config(&config_path, config, port) {
                    tracing::error!("Failed to rewrite trojan config: {}", e);
                    continue;
                }

                // Spawn new process
                let log_file = match std::fs::File::create(&log_path) {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!("Failed to create log file: {}", e);
                        continue;
                    }
                };

                let mut cmd = Command::new(&binary_path);
                cmd.arg("--config").arg(&config_path);
                if let Ok(f) = log_file.try_clone() {
                    cmd.stdout(Stdio::from(f));
                }
                cmd.stderr(Stdio::from(log_file));

                #[cfg(windows)]
                {
                    cmd.creation_flags(0x08000000);
                }

                match cmd.spawn() {
                    Ok(_child) => {
                        tracing::info!("Watchdog restarted trojan-go (pid {:?})", _child.id());
                        // We can't store the child in this task, but the port check will verify it
                    }
                    Err(e) => {
                        tracing::error!("Watchdog failed to restart trojan-go: {}", e);
                    }
                }
            }
        });

        self.watchdog_handle = Some(handle);
    }

    fn stop_watchdog(&mut self) {
        if let Some(handle) = self.watchdog_handle.take() {
            handle.abort();
        }
    }
}
