pub mod app;
pub mod config;
pub mod trojan;
pub mod domains;
pub mod proxy;
pub mod osproxy;
pub mod appproxy;
pub mod autostart;
pub mod health;
pub mod tray;
pub mod updater;
pub mod error;

use std::sync::Arc;
use tokio::sync::RwLock;
use tauri::Manager;

pub struct AppState {
    pub config: Arc<RwLock<config::Store>>,
    pub proxy: Arc<RwLock<Option<proxy::server::ProxyServer>>>,
    pub trojan: Arc<RwLock<trojan::manager::TrojanManager>>,
    pub domains: Arc<RwLock<domains::store::DomainStore>>,
    pub os_proxy: Arc<RwLock<osproxy::OsProxy>>,
    pub app_proxy: Arc<RwLock<appproxy::AppProxyRegistry>>,
    pub interval_handle: Arc<RwLock<Option<tauri::async_runtime::JoinHandle<()>>>>,
    pub health_handle: Arc<RwLock<Option<tauri::async_runtime::JoinHandle<()>>>>,
}

pub fn run() {
    eprintln!("[HaioBypass] Starting application...");
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    // Register panic hook for crash sentinel
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Restore proxy on crash if sentinel exists
        let sentinel = config::Store::config_dir().join("proxy.sentinel");
        if sentinel.exists() {
            let _ = std::fs::remove_file(&sentinel);
            // Attempt to clear OS proxy
            let _ = std::process::Command::new("gsettings")
                .args(["set", "org.gnome.system.proxy", "mode", "'none'"])
                .output();
            // Remove stale QUIC block rule from previous session
            let _ = osproxy::quic::unblock();
        }
        prev_hook(info);
    }));

    let config = config::Store::load().unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        config::Store::new()
    });

    // Crash sentinel check — if sentinel exists from previous crash, restore proxy
    if config::Store::config_dir().join("proxy.sentinel").exists() {
        tracing::warn!("Crash sentinel found — restoring OS proxy and clearing presets");
        let _ = std::fs::remove_file(config::Store::config_dir().join("proxy.sentinel"));
        // Clear OS proxy (Linux gsettings, the most common desktop)
        let _ = std::process::Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy", "mode", "'none'"])
            .output();
        // Remove stale QUIC block rule left by the crashed session
        let _ = osproxy::quic::unblock();
    }

    let state = Arc::new(AppState {
        config: Arc::new(RwLock::new(config)),
        proxy: Arc::new(RwLock::new(None)),
        trojan: Arc::new(RwLock::new(trojan::manager::TrojanManager::new())),
        domains: Arc::new(RwLock::new(domains::store::DomainStore::new())),
        os_proxy: Arc::new(RwLock::new(osproxy::OsProxy::new())),
        app_proxy: Arc::new(RwLock::new(appproxy::AppProxyRegistry::new())),
        interval_handle: Arc::new(RwLock::new(None)),
        health_handle: Arc::new(RwLock::new(None)),
    });

    let app_state = state.clone();
    let window_state = state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            app::commands::enable_proxy,
            app::commands::disable_proxy,
            app::commands::get_status,
            app::commands::test_connection,
            app::commands::save_config,
            app::commands::delete_config,
            app::commands::get_state,
            app::commands::set_state,
            app::commands::install_and_start_trojan,
            app::commands::get_presets,
            app::commands::toggle_preset,
            app::commands::refresh_domains,
            app::commands::set_port,
            app::commands::set_autostart,
            app::commands::set_proxy_port,
            app::commands::check_for_updates,
            app::commands::quit_and_restore,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let state = state.clone();

            let tray_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                tray::setup_tray(&tray_handle).await;
            });

            let enable_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let config = state.config.read().await;
                let enabled = config.get().enabled;
                drop(config);
                if enabled {
                    let _ = crate::app::commands::enable_proxy(enable_handle).await;
                }
            });

            Ok(())
        })
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app_handle = window.app_handle().clone();
                let state = window_state.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::app::commands::quit_and_cleanup(state).await;
                    app_handle.exit(0);
                });
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
