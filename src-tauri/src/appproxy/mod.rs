pub mod gradle;
pub mod maven;
pub mod npm;
pub mod pip;
pub mod git;
pub mod docker;
pub mod goproxy;
pub mod curl;
pub mod detect;

use std::collections::HashMap;

const ALL_PRESETS: &[&str] = &["gradle", "maven", "npm", "pip", "git", "docker", "go", "curl"];

pub struct AppProxyRegistry {
    backups: HashMap<String, String>,
    applied_presets: Vec<String>,
}

impl Default for AppProxyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AppProxyRegistry {
    pub fn new() -> Self {
        Self {
            backups: HashMap::new(),
            applied_presets: Vec::new(),
        }
    }

    pub async fn apply_all(&mut self, presets: &[String], addr: &str) -> crate::error::Result<()> {
        for preset in presets {
            if let Err(e) = self.apply_preset(preset, addr).await {
                tracing::warn!("Failed to apply preset {}: {}", preset, e);
            } else {
                if !self.applied_presets.contains(preset) {
                    self.applied_presets.push(preset.clone());
                }
            }
        }
        Ok(())
    }

    pub async fn apply_preset(&mut self, name: &str, addr: &str) -> crate::error::Result<()> {
        match name {
            "gradle" => {
                let backup = gradle::get_current()?;
                self.backups.insert("gradle".into(), backup);
                gradle::apply(addr)?;
            }
            "maven" => {
                let backup = maven::get_current()?;
                self.backups.insert("maven".into(), backup);
                maven::apply(addr)?;
            }
            "npm" => {
                npm::apply(addr)?;
            }
            "pip" => {
                let backup = pip::get_current()?;
                self.backups.insert("pip".into(), backup);
                pip::apply(addr)?;
            }
            "git" => {
                git::apply(addr)?;
            }
            "docker" => {
                let backup = docker::get_current()?;
                self.backups.insert("docker".into(), backup);
                docker::apply(addr)?;
            }
            "go" => {
                goproxy::apply(addr)?;
            }
            "curl" => {
                let backup = curl::get_current()?;
                self.backups.insert("curl".into(), backup);
                curl::apply(addr)?;
            }
            _ => return Err(crate::error::HaioError::AppProxy(format!("Unknown preset: {}", name))),
        }
        Ok(())
    }

    pub async fn clear_all(&mut self) -> crate::error::Result<()> {
        for name in ALL_PRESETS {
            if let Err(e) = self.clear_preset(name).await {
                tracing::warn!("Failed to clear preset {}: {}", name, e);
            }
        }
        self.backups.clear();
        self.applied_presets.clear();
        Ok(())
    }

    pub async fn clear_preset(&mut self, name: &str) -> crate::error::Result<()> {
        match name {
            "gradle" => {
                if let Some(backup) = self.backups.remove("gradle") {
                    gradle::restore(&backup)?;
                } else {
                    gradle::clear()?;
                }
            }
            "maven" => {
                if let Some(backup) = self.backups.remove("maven") {
                    maven::restore(&backup)?;
                } else {
                    maven::clear()?;
                }
            }
            "npm" => npm::clear()?,
            "pip" => {
                if let Some(backup) = self.backups.remove("pip") {
                    pip::restore(&backup)?;
                } else {
                    pip::clear()?;
                }
            }
            "git" => git::clear()?,
            "docker" => {
                if let Some(backup) = self.backups.remove("docker") {
                    docker::restore(&backup)?;
                } else {
                    docker::clear()?;
                }
            }
            "go" => goproxy::clear()?,
            "curl" => {
                if let Some(backup) = self.backups.remove("curl") {
                    curl::restore(&backup)?;
                } else {
                    curl::clear()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn detect_available(&self) -> Vec<String> {
        detect::detect_available().await
    }
}
