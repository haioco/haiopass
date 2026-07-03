use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, Command};
use crate::config::TrojanConfig;
use crate::trojan::config_writer;

const TROJAN_BINARY: &str = "trojan-go";

pub struct TrojanManager {
    child: Option<Child>,
    binary_path: PathBuf,
    config_path: PathBuf,
    log_path: PathBuf,
    stopped: bool,
}

impl Default for TrojanManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TrojanManager {
    pub fn new() -> Self {
        let config_dir = super::super::config::Store::config_dir();
        let ext = if cfg!(windows) { ".exe" } else { "" };
        Self {
            child: None,
            binary_path: config_dir.join(format!("{}{}", TROJAN_BINARY, ext)),
            config_path: config_dir.join("config.json"),
            log_path: config_dir.join("trojan.log"),
            stopped: false,
        }
    }

    pub async fn ensure_binary(&mut self) -> crate::error::Result<()> {
        if self.binary_path.exists() {
            return Ok(());
        }
        super::bundled::extract_bundled(&self.binary_path)?;
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.binary_path, std::fs::Permissions::from_mode(0o755))?;
        }
        tracing::info!("Extracted trojan-go to {}", self.binary_path.display());
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
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let child = cmd.spawn().map_err(|e| crate::error::HaioError::Trojan(format!("Failed to start: {}", e)))?;
        tracing::info!("Started trojan-go pid {:?}", child.id());
        self.child = Some(child);
        self.stopped = false;
        Ok(())
    }

    pub async fn stop(&mut self) -> crate::error::Result<()> {
        self.stopped = true;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            tracing::info!("Stopped trojan-go");
        }
        Ok(())
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
}
