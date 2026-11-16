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
pub mod webview_check;
pub mod portal;
pub mod device;

use std::sync::Arc;
use tokio::sync::RwLock;
use tauri::Manager;
use tauri::Emitter;

pub struct AppState {
    pub config: Arc<RwLock<config::Store>>,
    pub proxy: Arc<RwLock<Option<proxy::server::ProxyServer>>>,
    /// Loopback SOCKS5 listener used for UDP-capable applications.
    pub socks5: Arc<RwLock<Option<proxy::socks5::Socks5Server>>>,
    pub trojan: Arc<RwLock<trojan::manager::TrojanManager>>,
    pub domains: Arc<RwLock<domains::store::DomainStore>>,
    pub os_proxy: Arc<RwLock<osproxy::OsProxy>>,
    pub app_proxy: Arc<RwLock<appproxy::AppProxyRegistry>>,
    pub interval_handle: Arc<RwLock<Option<tauri::async_runtime::JoinHandle<()>>>>,
    pub health_handle: Arc<RwLock<Option<tauri::async_runtime::JoinHandle<()>>>>,
    /// Portal (OTP / HaioGate / billing) client. Additive: if initialization
    /// fails the VPN keeps working and the HaioGate tab shows a retry state.
    pub portal: Arc<portal::PortalService>,
    /// Device identity: OS-keyring private key + enrolled certificate. Additive:
    /// legacy password configs never touch it.
    pub device: Arc<device::DeviceService>,
}

pub fn run() {
    eprintln!("[HaioBypass] Starting application...");
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    // Must run before any window exists: a missing WebView2 runtime (typically
    // after an auto-update, or when it was installed for another Windows
    // account) makes the webview fail with an opaque error box.
    webview_check::preflight();

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

    // Remove dropped payloads from versions ≤ 2.0.3. The in-process client
    // extracts nothing; a stale haio-proxy.exe left in the config dir is
    // itself an antivirus flag.
    for legacy in ["haio-proxy", "haio-proxy.exe", "haio-proxy.log", "config.json"] {
        let path = config::Store::config_dir().join(legacy);
        if path.exists() {
            match std::fs::remove_file(&path) {
                Ok(()) => tracing::info!("Removed legacy artifact {}", path.display()),
                Err(e) => tracing::warn!(
                    "Could not remove legacy artifact {}: {}",
                    path.display(),
                    e
                ),
            }
        }
    }

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
        socks5: Arc::new(RwLock::new(None)),
        trojan: Arc::new(RwLock::new(trojan::manager::TrojanManager::new())),
        domains: Arc::new(RwLock::new(domains::store::DomainStore::new())),
        os_proxy: Arc::new(RwLock::new(osproxy::OsProxy::new())),
        app_proxy: Arc::new(RwLock::new(appproxy::AppProxyRegistry::new())),
        interval_handle: Arc::new(RwLock::new(None)),
        health_handle: Arc::new(RwLock::new(None)),
        portal: Arc::new(portal::PortalService::new()),
        device: Arc::new(device::DeviceService::new()),
    });

    let app_state = state.clone();
    let window_state = state.clone();

    let started = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            app::commands::enable_proxy,
            app::commands::disable_proxy,
            app::commands::get_status,
            app::commands::test_connection,
            app::commands::save_config,
            app::commands::delete_config,
            app::commands::get_state,
            app::commands::get_bypass_domains,
            app::commands::add_bypass_domain,
            app::commands::remove_bypass_domain,
            app::commands::set_bypass_domains,
            app::commands::connect_trojan,
            app::commands::get_presets,
            app::commands::toggle_preset,
            app::commands::refresh_domains,
            app::commands::set_port,
            app::commands::set_autostart,
            app::commands::set_minimize_to_tray,
            app::commands::set_proxy_consent,
            app::commands::set_quic_block,
            app::commands::set_proxy_port,
            app::commands::check_for_updates,
            app::commands::install_update,
            app::commands::quit_and_restore,
            portal::commands::portal_session,
            portal::commands::portal_request_otp,
            portal::commands::portal_login_otp,
            portal::commands::portal_logout,
            portal::commands::portal_me,
            portal::commands::portal_shahkar_verify,
            portal::commands::portal_list_packages,
            portal::commands::portal_list_accounts,
            portal::commands::portal_account_detail,
            portal::commands::portal_sync_quota,
            portal::commands::portal_start_buy,
            portal::commands::portal_start_renew,
            portal::commands::portal_buy_plan,
            portal::commands::portal_invoice,
            portal::commands::portal_invoice_pay,
            portal::commands::portal_list_orgs,
            portal::commands::portal_open_payment,
            portal::commands::portal_reconcile_pending,
            portal::commands::portal_pending_purchase,
            portal::commands::portal_cancel_pending,
            portal::commands::portal_apply_config,
            portal::commands::portal_add_member,
            portal::commands::portal_rotate_member,
            portal::commands::portal_revoke_member,
            portal::commands::portal_preview_invite,
            portal::commands::portal_enroll_invite,
            portal::commands::portal_primary_invite,
            portal::commands::portal_member_invite,
            portal::commands::portal_primary_enroll,
            portal::commands::portal_primary_reset_device,
            portal::commands::portal_member_reset_device,
            portal::commands::portal_list_invites,
            portal::commands::portal_cancel_invite,
            portal::commands::portal_device_identity,
            portal::commands::portal_forget_device_identity,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let state = state.clone();

            let tray_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                tray::setup_tray(&tray_handle).await;
            });

            let enable_handle = app_handle.clone();
            let portal_state = state.clone();
            let enable_state = state.clone();
            tauri::async_runtime::spawn(async move {
                let config = enable_state.config.read().await;
                let enabled = config.get().enabled;
                drop(config);
                if enabled {
                    let _ = crate::app::commands::enable_proxy(enable_handle).await;
                }
            });

            // Background update check on startup — emits update:available
            let update_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                match crate::updater::check(&update_handle).await {
                    Ok(Some(update)) => {
                        tracing::info!("Update available: v{}", update.version);
                        let _ = update_handle.emit(
                            "update:available",
                            serde_json::json!({
                                "version": update.version,
                                "notes": update.notes,
                            }),
                        );
                    }
                    Ok(None) => tracing::info!("App is up to date"),
                    Err(e) => tracing::warn!("Startup update check failed: {}", e),
                }
            });

            // Load enrolled device material into the TLS registry before any
            // dial. Failure is logged (a missing keyring entry after a reinstall
            // is expected); the health probe then reports the tunnel down and
            // the UI offers re-enrollment.
            let device_state = state.clone();
            tauri::async_runtime::spawn(async move {
                let meta = {
                    let config = device_state.config.read().await;
                    config.get().device_certificate.clone()
                };
                if let Some(meta) = meta {
                    if let Err(err) = device_state.device.commit(&meta) {
                        tracing::warn!(
                            "device identity {} could not be loaded: {}",
                            meta.identity_id,
                            err.code
                        );
                    }
                }
            });

            // Restore the portal session in the background: read the keyring,
            // validate /auth/me/, refresh once if needed, then tell the UI.
            // Never blocks launch, the proxy, or the updater.
            let portal_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let view = portal_state.portal.restore().await;
                let _ = portal_handle.emit("portal:session", view);
            });

            Ok(())
        })
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app_handle = window.app_handle().clone();
                let window = window.clone();
                let state = window_state.clone();
                tauri::async_runtime::spawn(async move {
                    let hide_to_tray = state.config.read().await.get().minimize_to_tray;
                    if hide_to_tray {
                        // Keep running in the tray — the bypass stays active;
                        // restoring the window is one tray click away.
                        let _ = window.hide();
                    } else {
                        let _ = crate::app::commands::quit_and_cleanup(state).await;
                        app_handle.exit(0);
                    }
                });
            }
        })
        .run(tauri::generate_context!());

    // When window creation fails — the usual cause on Windows being a WebView2
    // runtime the loader cannot use — `.expect()` aborted into an opaque state:
    // the process lingered behind the loader's own English message box with no
    // window and no way forward, which users reported as "the app won't run".
    // Explain it in Persian and exit cleanly instead of hanging.
    if let Err(e) = started {
        tracing::error!("HaioBypass could not create its window: {}", e);
        webview_check::show_native_dialog(
            "HaioBypass — WebView2",
            "برنامه نتوانست پنجره را نمایش دهد. لطفاً WebView2 Runtime را نصب کنید:\n\n\
             https://go.microsoft.com/fwlink/p/?LinkId=2124703\n\n\
             سپس برنامه را دوباره اجرا کنید.\n\n\
             HaioBypass could not create its window. Please install the WebView2\n\
             Runtime from the link above and start the app again.",
        );
        std::process::exit(1);
    }
}
