pub mod trojan_url;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;

const CONFIG_DIR: &str = ".haiobypass";
const CONFIG_FILE: &str = "state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrojanConfig {
    pub password: String,
    pub server: String,
    pub port: u16,
    pub sni: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_socks_port")]
    pub proxy_port: u16,
    #[serde(default = "default_http_port")]
    pub http_proxy_port: u16,
    #[serde(default)]
    pub trojan_url: String,
    #[serde(default)]
    pub trojan_config: Option<TrojanConfig>,
    #[serde(default)]
    pub cached_domains: Vec<String>,
    #[serde(default)]
    pub last_fetch: Option<i64>,
    #[serde(default)]
    pub using_fallback: bool,
    #[serde(default)]
    pub using_cache: bool,
    #[serde(default)]
    pub last_fetch_error: Option<String>,
    #[serde(default)]
    pub enabled_presets: Vec<String>,
    #[serde(default = "default_autostart")]
    pub autostart: bool,
    #[serde(default = "default_minimize_to_tray")]
    pub minimize_to_tray: bool,
    #[serde(default)]
    pub last_update_at: Option<i64>,
}

fn default_enabled() -> bool { false }
fn default_socks_port() -> u16 { 11031 }
fn default_http_port() -> u16 { 11032 }
fn default_autostart() -> bool { false }
fn default_minimize_to_tray() -> bool { true }

impl Default for State {
    fn default() -> Self {
        Self {
            enabled: false,
            proxy_port: 11031,
            http_proxy_port: 11032,
            trojan_url: String::new(),
            trojan_config: None,
            cached_domains: Vec::new(),
            last_fetch: None,
            using_fallback: false,
            using_cache: false,
            last_fetch_error: None,
            enabled_presets: vec!["gradle".into()],
            autostart: false,
            minimize_to_tray: true,
            last_update_at: None,
        }
    }
}

pub struct Store {
    state: State,
    path: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        Self {
            state: State::default(),
            path: Self::config_dir().join(CONFIG_FILE),
        }
    }

    pub fn config_dir() -> PathBuf {
        let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join(CONFIG_DIR)
    }

    pub fn load() -> crate::error::Result<Self> {
        let path = Self::config_dir().join(CONFIG_FILE);
        if path.exists() {
            let data = fs::read_to_string(&path)?;
            let mut state: State = serde_json::from_str(&data)?;
            if state.proxy_port == 10808 {
                state.proxy_port = 11031;
            }
            if state.http_proxy_port == 10809 {
                state.http_proxy_port = 11032;
            }
            Ok(Self { state, path })
        } else {
            Ok(Self {
                state: State::default(),
                path,
            })
        }
    }

    pub fn save(&mut self) -> crate::error::Result<()> {
        let dir = self.path.parent().unwrap();
        fs::create_dir_all(dir)?;
        let data = serde_json::to_string_pretty(&self.state)?;
        fs::write(&self.path, data)?;
        Ok(())
    }

    pub fn get(&self) -> &State {
        &self.state
    }

    pub fn get_mut(&mut self) -> &mut State {
        &mut self.state
    }

    pub fn set_state(&mut self, new_state: State) -> crate::error::Result<()> {
        self.state = new_state;
        self.save()
    }
}
