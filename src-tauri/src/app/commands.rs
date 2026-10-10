use tauri::Manager;
use tauri::Emitter;
use crate::AppState;
use crate::domains;
use crate::proxy;
use crate::config;

#[tauri::command]
pub async fn enable_proxy(
    app_handle: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let state = app_handle.state::<std::sync::Arc<AppState>>();
    let config = state.config.read().await;
    let http_port = config.get().http_proxy_port;
    let presets = config.get().enabled_presets.clone();
    let cached = config.get().cached_domains.clone();
    let proxy_consent = config.get().proxy_consent;
    let block_quic = config.get().block_quic;
    drop(config);

    // 1. Determine domains (cache → fallback)
    let domains = if cached.is_empty() {
        domains::fallback::FALLBACK_DOMAINS.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    } else {
        cached
    };

    // 2. Activate the in-process tunnel — no binary to extract, no port to
    //    wait for. New proxied connections read this config as they dial.
    {
        let tc = {
            let config = state.config.read().await;
            config.get().trojan_config.clone()
        };
        if let Some(tc) = tc {
            let trojan = state.trojan.read().await;
            trojan.set(tc).await;
        }
    }

    // Emit tunnel status
    {
        let trojan = state.trojan.read().await;
        let running = trojan.is_connected().await;
        crate::app::events::emit_trojan_status(&app_handle, running).await;
    }

    // 3. Start the local HTTP proxy
    {
        let mut proxy = state.proxy.write().await;
        if proxy.is_none() {
            let router = proxy::router::DomainRouter::new(domains.clone());
            let shared_config = state.trojan.read().await.shared_config();
            let server = proxy::server::ProxyServer::new(http_port, router, shared_config);
            *proxy = Some(server);
        }
        if let Some(s) = proxy.as_mut() {
            s.set_domains(domains.clone()).await;
            s.start().await.map_err(|e| e.to_string())?;
        }
    }

    // 4. Apply OS proxy — only with the user's explicit one-time consent.
    //    The PAC/registry takeover is the system-intrusive part of the
    //    feature; without consent we stay a plain local proxy and ask the
    //    UI to explain what enabling the takeover would do.
    if proxy_consent {
        let mut os_proxy = state.os_proxy.write().await;
        os_proxy.backup().await.map_err(|e| e.to_string())?;
        os_proxy.apply(format!("127.0.0.1:{}", http_port))
            .await
            .map_err(|e| e.to_string())?;
    } else {
        let _ = app_handle.emit("consent:os-proxy", serde_json::json!({
            "httpPort": http_port,
        }));
    }

    // QUIC (UDP 443) block: an elevated firewall change, applied only when
    // the user explicitly opted in via settings — never implicitly.
    if block_quic {
        let mut os_proxy = state.os_proxy.write().await;
        os_proxy.block_quic();
    }

    // 5. Apply app proxy presets
    {
        let mut app_proxy = state.app_proxy.write().await;
        let proxy_addr = format!("127.0.0.1:{}", http_port);
        app_proxy.apply_all(&presets, &proxy_addr)
            .await
            .map_err(|e| e.to_string())?;
    }

    // 6. Write sentinel file
    write_sentinel().map_err(|e| e.to_string())?;

    // 7. Update state
    {
        let mut config = state.config.write().await;
        config.get_mut().enabled = true;
        config.get_mut().cached_domains = domains;
        config.save().map_err(|e| e.to_string())?;
    }

    // 8. Start 60-min refresh interval
    let arc_state: std::sync::Arc<AppState> = state.inner().clone();
    {
        let mut interval_handle = state.interval_handle.write().await;
        *interval_handle = Some(start_refresh_interval(arc_state.clone(), app_handle.clone()));
    }

    // 9. Start health monitor (every 30s)
    {
        let mut health_handle = state.health_handle.write().await;
        *health_handle = Some(start_health_monitor(arc_state.clone(), app_handle.clone()));
    }

    // 10. Do initial domain fetch
    refresh_domains_inner(state.inner().clone(), app_handle.clone()).await;

    // 10. Emit status
    let _ = app_handle.emit("status:update", serde_json::json!({"enabled": true}));
    crate::tray::update_status(&app_handle, true);

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn disable_proxy(
    app_handle: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let state = app_handle.state::<std::sync::Arc<AppState>>();
    // Mark disabled FIRST so health-monitor / watchdog see enabled==false
    // and do not restart the VPN right after we stop it.
    {
        let mut config = state.config.write().await;
        config.get_mut().enabled = false;
        let _ = config.save();
    }
    let _ = app_handle.emit("status:update", serde_json::json!({"enabled": false}));

    // Stop the background interval
    {
        let mut interval_handle = state.interval_handle.write().await;
        if let Some(handle) = interval_handle.take() {
            handle.abort();
        }
    }

    // Stop the health monitor
    {
        let mut health_handle = state.health_handle.write().await;
        if let Some(handle) = health_handle.take() {
            handle.abort();
        }
    }

    // Stop the local proxy
    {
        let mut proxy = state.proxy.write().await;
        if let Some(s) = proxy.as_mut() {
            s.stop().await.map_err(|e| e.to_string())?;
        }
        *proxy = None;
    }

    // Clear OS proxy
    {
        let mut os_proxy = state.os_proxy.write().await;
        os_proxy.clear().await.map_err(|e| e.to_string())?;
    }

    // Clear app proxy presets
    {
        let mut app_proxy = state.app_proxy.write().await;
        app_proxy.clear_all().await.map_err(|e| e.to_string())?;
    }

    // Deactivate the in-process tunnel
    {
        let trojan = state.trojan.read().await;
        trojan.clear().await;
    }
    crate::app::events::emit_trojan_status(&app_handle, false).await;

    // Remove sentinel
    remove_sentinel();

    // Update state
    {
        let mut config = state.config.write().await;
        config.get_mut().enabled = false;
        config.save().map_err(|e| e.to_string())?;
    }

    let _ = app_handle.emit("status:update", serde_json::json!({"enabled": false}));
    crate::tray::update_status(&app_handle, false);

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn get_status(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    let config = state.config.read().await;
    let s = config.get();
    Ok(serde_json::json!({
        "enabled": s.enabled,
        "domainCount": s.cached_domains.len(),
        "domains": s.cached_domains,
        "proxyPort": s.proxy_port,
        "httpProxyPort": s.http_proxy_port,
        "lastFetch": s.last_fetch,
        "usingFallback": s.using_fallback,
        "usingCache": s.using_cache,
        "lastFetchError": s.last_fetch_error,
        "enabledPresets": s.enabled_presets,
        "blockQuic": s.block_quic,
        "proxyConsent": s.proxy_consent,
        "autostart": s.autostart,
        "domainSourceUrl": crate::domains::fallback::DOMAINS_URLS[0],
        "version": app_handle.package_info().version.to_string(),
        "lastUpdateAt": s.last_update_at,
    }))
}

#[tauri::command]
pub async fn test_connection(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    let config = state.config.read().await;
    let http_port = config.get().http_proxy_port;
    let cached = config.get().cached_domains.clone();
    drop(config);

    let (ok, _) = crate::health::check_proxy_health(http_port, &cached).await;
    Ok(serde_json::json!({ "ok": ok, "port": http_port }))
}

#[tauri::command]
pub async fn save_config(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    trojan_url: String,
) -> Result<serde_json::Value, String> {
    let parsed = crate::config::trojan_url::parse_trojan_url(&trojan_url)
        .ok_or("Invalid access key")?;

    let mut config = state.config.write().await;
    config.get_mut().trojan_url = trojan_url;
    config.get_mut().trojan_config = Some(config::TrojanConfig {
        password: parsed.password,
        server: parsed.server,
        port: parsed.port,
        sni: parsed.sni,
    });
    config.save().map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "success": true,
        "server": config.get().trojan_config.as_ref().unwrap().server,
        "port": config.get().trojan_config.as_ref().unwrap().port,
    }))
}

#[tauri::command]
pub async fn delete_config(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    // Disconnect first so the saved config is not in use by a running proxy
    let enabled = { state.config.read().await.get().enabled };
    if enabled {
        disable_proxy(app_handle).await?;
    }

    let mut config = state.config.write().await;
    config.get_mut().trojan_url = String::new();
    config.get_mut().trojan_config = None;
    config.save().map_err(|e| e.to_string())?;

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn get_state(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    let config = state.config.read().await;
    serde_json::to_value(config.get()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_state(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    new_state: config::State,
) -> Result<serde_json::Value, String> {
    let mut config = state.config.write().await;
    config.set_state(new_state).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "success": true }))
}

/// Activate the in-process tunnel with the saved credentials. Kept as a
/// distinct command (previously `install_and_start_trojan`) so the UI's
/// "connect" affordance keeps working; there is nothing to install any more.
#[tauri::command]
pub async fn connect_trojan(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    let tc = {
        let config = state.config.read().await;
        config.get().trojan_config.clone()
    };
    let tc = tc.ok_or("No access key saved. Save a key first.")?;

    {
        let trojan = state.trojan.read().await;
        trojan.set(tc).await;
    }

    let running = {
        let trojan = state.trojan.read().await;
        trojan.is_connected().await
    };
    crate::app::events::emit_trojan_status(&app_handle, running).await;

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn get_presets(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    let config = state.config.read().await;
    let enabled = config.get().enabled_presets.clone();
    drop(config);

    let app_proxy = state.app_proxy.read().await;
    let available = app_proxy.detect_available().await;

    Ok(serde_json::json!({
        "enabled": enabled,
        "available": available,
    }))
}

#[tauri::command]
pub async fn toggle_preset(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    name: String,
    on: bool,
) -> Result<serde_json::Value, String> {
    let mut config = state.config.write().await;
    if on {
        if !config.get().enabled_presets.contains(&name) {
            config.get_mut().enabled_presets.push(name);
        }
    } else {
        config.get_mut().enabled_presets.retain(|p| p != &name);
    }
    config.save().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "success": true }))
}

/// One-time consent for the OS-level proxy takeover (PAC URL / registry /
/// gsettings / networksetup). Granting it while the bypass is active
/// applies the takeover immediately, so the user sees the effect at once.
#[tauri::command]
pub async fn set_proxy_consent(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    consent: bool,
) -> Result<serde_json::Value, String> {
    let enabled = {
        let mut config = state.config.write().await;
        config.get_mut().proxy_consent = consent;
        let _ = config.save();
        config.get().enabled
    };

    if consent && enabled {
        let http_port = {
            let config = state.config.read().await;
            config.get().http_proxy_port
        };
        let mut os_proxy = state.os_proxy.write().await;
        os_proxy.backup().await.map_err(|e| e.to_string())?;
        os_proxy.apply(format!("127.0.0.1:{}", http_port))
            .await
            .map_err(|e| e.to_string())?;
    } else if !consent {
        // Revoking consent also undoes a live takeover.
        let mut os_proxy = state.os_proxy.write().await;
        os_proxy.clear().await.map_err(|e| e.to_string())?;
    }

    let _ = app_handle.emit("status:update", serde_json::json!({
        "enabled": enabled,
    }));

    Ok(serde_json::json!({ "success": true }))
}

/// Explicit opt-in for the QUIC (UDP 443) firewall block. This is the only
/// code path that creates the firewall rule — it needs elevated privileges
/// and is labelled as such in the UI.
#[tauri::command]
pub async fn set_quic_block(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    {
        let mut config = state.config.write().await;
        config.get_mut().block_quic = enabled;
        config.save().map_err(|e| e.to_string())?;
    }

    let bypass_active = { state.config.read().await.get().enabled };

    {
        let mut os_proxy = state.os_proxy.write().await;
        if enabled && bypass_active {
            os_proxy.block_quic();
        } else {
            os_proxy.force_unblock_quic();
        }
    }

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn refresh_domains(
    app_handle: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let state = app_handle.state::<std::sync::Arc<AppState>>();
    refresh_domains_inner(state.inner().clone(), app_handle).await;
    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn set_port(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    http_port: u16,
) -> Result<serde_json::Value, String> {
    if http_port < 1024 {
        return Err("Port must be between 1024 and 65535".into());
    }
    let mut config = state.config.write().await;
    config.get_mut().http_proxy_port = http_port;
    config.save().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn set_autostart(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    if enabled {
        crate::autostart::AutoStart::enable().map_err(|e| e.to_string())?;
    } else {
        crate::autostart::AutoStart::disable().map_err(|e| e.to_string())?;
    }
    let mut config = state.config.write().await;
    config.get_mut().autostart = enabled;
    config.save().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn set_proxy_port(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    proxy_port: u16,
) -> Result<serde_json::Value, String> {
    if proxy_port < 1024 {
        return Err("Port must be between 1024 and 65535".into());
    }
    let mut config = state.config.write().await;
    config.get_mut().proxy_port = proxy_port;
    config.save().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn check_for_updates(
    app_handle: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let current = app_handle.package_info().version.to_string();
    match crate::updater::check(&app_handle).await {
        Ok(Some(update)) => Ok(serde_json::json!({
            "available": true,
            "version": update.version,
            "notes": update.notes,
            "currentVersion": current,
        })),
        Ok(None) => Ok(serde_json::json!({
            "available": false,
            "currentVersion": current,
        })),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn install_update(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    // Record when the update is installed before the app restarts itself
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    {
        let mut config = state.config.write().await;
        config.get_mut().last_update_at = Some(now);
        let _ = config.save();
    }

    crate::updater::download_and_install(&app_handle)
        .await
        .map_err(|e| e.to_string())?;

    // Restart into the new version
    app_handle.restart();
    #[allow(unreachable_code)]
    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn quit_and_restore(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    quit_and_cleanup(state.inner().clone()).await?;
    Ok(serde_json::json!({ "success": true }))
}

pub async fn quit_and_cleanup(state: std::sync::Arc<AppState>) -> Result<(), String> {
    let mut os_proxy = state.os_proxy.write().await;
    os_proxy.restore().await.map_err(|e| e.to_string())?;

    let mut app_proxy = state.app_proxy.write().await;
    app_proxy.clear_all().await.map_err(|e| e.to_string())?;

    let trojan = state.trojan.read().await;
    trojan.clear().await;

    remove_sentinel();

    Ok(())
}

// --- Helper functions ---

fn write_sentinel() -> std::io::Result<()> {
    let path = config::Store::config_dir().join("proxy.sentinel");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, "1")
}

fn remove_sentinel() {
    let path = config::Store::config_dir().join("proxy.sentinel");
    let _ = std::fs::remove_file(path);
}

fn start_refresh_interval(
    state: std::sync::Arc<AppState>,
    app_handle: tauri::AppHandle,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        interval.tick().await; // consume immediate first tick
        loop {
            interval.tick().await;
            let _ = refresh_domains_inner(state.clone(), app_handle.clone()).await;
        }
    })
}

fn start_health_monitor(
    state: std::sync::Arc<AppState>,
    app_handle: tauri::AppHandle,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await; // consume immediate first tick

        let mut consecutive_failures: u32 = 0;

        loop {
            interval.tick().await;

            // Get current configuration
            let (http_port, enabled, trojan_config) = {
                let config = state.config.read().await;
                (
                    config.get().http_proxy_port,
                    config.get().enabled,
                    config.get().trojan_config.clone(),
                )
            };

            if !enabled {
                break;
            }

            let health =
                crate::health::check_trojan_health(http_port, trojan_config.as_ref()).await;

            if health.all_healthy {
                if consecutive_failures > 0 {
                    tracing::info!("Health check recovered after {} failures", consecutive_failures);
                }
                consecutive_failures = 0;
                let _ = app_handle.emit("health:check", serde_json::json!({
                    "ok": true,
                    "httpPort": http_port,
                }));
            } else {
                consecutive_failures += 1;
                tracing::warn!(
                    "Health check failed (http={}, tunnel={}, failures={})",
                    health.http_proxy_ok,
                    health.tunnel_ok,
                    consecutive_failures
                );

                let _ = app_handle.emit("health:check", serde_json::json!({
                    "ok": false,
                    "httpPort": http_port,
                    "tunnelOk": health.tunnel_ok,
                    "consecutiveFailures": consecutive_failures,
                }));

                // No silent auto-restart: with an in-process client there is
                // no process to respawn, and restart-on-failure is itself a
                // backdoor-shaped behaviour. Tell the UI and let the user
                // decide to reconnect.
                if consecutive_failures >= 2 {
                    let _ = app_handle.emit("health:failed", serde_json::json!({
                        "consecutiveFailures": consecutive_failures,
                    }));
                }
            }
        }
    })
}

pub async fn refresh_domains_inner(
    state: std::sync::Arc<AppState>,
    app_handle: tauri::AppHandle,
) {
    match domains::fetcher::fetch_domains().await {
        Ok(domains) => {
            tracing::info!("Fetched {} domains", domains.len());

            // Update router
            let proxy = state.proxy.read().await;
            if let Some(ref s) = *proxy {
                s.set_domains(domains.clone()).await;
            }
            drop(proxy);

            // Update config, keeping the diff so the user can see what a
            // remote list update actually changed.
            let (previous_len, added, removed) = {
                let mut config = state.config.write().await;
                let previous: std::collections::HashSet<String> =
                    config.get().cached_domains.iter().cloned().collect();
                let added = domains.iter().filter(|d| !previous.contains(*d)).count();
                let removed = previous
                    .iter()
                    .filter(|p| !domains.contains(*p))
                    .count();
                config.get_mut().cached_domains = domains;
                config.get_mut().last_fetch = Some(chrono::Utc::now().timestamp());
                config.get_mut().using_fallback = false;
                config.get_mut().using_cache = false;
                config.get_mut().last_fetch_error = None;
                let _ = config.save();
                (previous.len(), added, removed)
            };

            let _ = app_handle.emit("domains:updated", serde_json::json!({
                "count": state.config.read().await.get().cached_domains.len(),
                "previousCount": previous_len,
                "added": added,
                "removed": removed,
                "usingFallback": false,
            }));
        }
        Err(e) => {
            tracing::warn!("Domain fetch failed: {}", e);
            let mut config = state.config.write().await;
            config.get_mut().last_fetch_error = Some(e);
            if config.get().cached_domains.is_empty() {
                config.get_mut().using_fallback = true;
            } else {
                config.get_mut().using_cache = true;
            }
            let _ = config.save();
        }
    }
}
