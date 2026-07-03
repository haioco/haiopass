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
    let port = config.get().proxy_port;
    let http_port = config.get().http_proxy_port;
    let presets = config.get().enabled_presets.clone();
    let cached = config.get().cached_domains.clone();
    drop(config);

    // 1. Determine domains (cache → fallback)
    let domains = if cached.is_empty() {
        domains::fallback::FALLBACK_DOMAINS.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    } else {
        cached
    };

    // 2. Start trojan-go first — wait for SOCKS5 port
    {
        let tc = {
            let config = state.config.read().await;
            config.get().trojan_config.clone()
        };
        if let Some(tc) = tc {
            let mut trojan = state.trojan.write().await;
            trojan.start(tc, port).await.map_err(|e| e.to_string())?;
        }
    }

    // Emit trojan status
    {
        let mut trojan = state.trojan.write().await;
        let (running, pid) = trojan.status();
        crate::app::events::emit_trojan_status(&app_handle, running, pid).await;
    }

    // Wait for SOCKS5 port to become available
    wait_for_port(port).await?;

    // 3. Start the local HTTP proxy
    {
        let mut proxy = state.proxy.write().await;
        if proxy.is_none() {
            let router = proxy::router::DomainRouter::new(domains.clone(), port);
            let server = proxy::server::ProxyServer::new(http_port, router);
            *proxy = Some(server);
        }
        if let Some(s) = proxy.as_mut() {
            s.set_domains(domains.clone()).await;
            s.start().await.map_err(|e| e.to_string())?;
        }
    }

    // 4. Apply OS proxy
    {
        let mut os_proxy = state.os_proxy.write().await;
        os_proxy.backup().await.map_err(|e| e.to_string())?;
        os_proxy.apply(format!("127.0.0.1:{}", http_port))
            .await
            .map_err(|e| e.to_string())?;
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

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn disable_proxy(
    app_handle: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let state = app_handle.state::<std::sync::Arc<AppState>>();
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

    // Stop trojan-go
    {
        let mut trojan = state.trojan.write().await;
        trojan.stop().await.map_err(|e| e.to_string())?;
    }
    crate::app::events::emit_trojan_status(&app_handle, false, None).await;

    // Remove sentinel
    remove_sentinel();

    // Update state
    {
        let mut config = state.config.write().await;
        config.get_mut().enabled = false;
        config.save().map_err(|e| e.to_string())?;
    }

    let _ = app_handle.emit("status:update", serde_json::json!({"enabled": false}));

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
pub async fn get_status(
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
        "autostart": s.autostart,
        "domainSourceUrl": crate::domains::fallback::DOMAINS_URLS[0],
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
        .ok_or("Invalid trojan:// URL")?;

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

#[tauri::command]
pub async fn install_and_start_trojan(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    let tc = {
        let config = state.config.read().await;
        config.get().trojan_config.clone()
    };
    let tc = tc.ok_or("No trojan config saved. Save a config first.")?;
    let port = {
        let config = state.config.read().await;
        config.get().proxy_port
    };

    {
        let mut trojan = state.trojan.write().await;
        trojan.ensure_binary().await.map_err(|e| e.to_string())?;
        trojan.start(tc, port).await.map_err(|e| e.to_string())?;
    }

    let (running, pid) = {
        let mut trojan = state.trojan.write().await;
        trojan.status()
    };
    crate::app::events::emit_trojan_status(&app_handle, running, pid).await;

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
    _state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "available": false }))
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

    let mut trojan = state.trojan.write().await;
    trojan.stop().await.map_err(|e| e.to_string())?;

    remove_sentinel();

    Ok(())
}

// --- Helper functions ---

async fn wait_for_port(port: u16) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", port);
    for i in 0..15 {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::net::TcpStream::connect(&addr),
        ).await;
        if let Ok(Ok(_)) = result {
            return Ok(());
        }
        tracing::warn!("wait_for_port: attempt {}/15 failed for {}", i + 1, port);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(format!("Timed out waiting for port {} after 15 attempts", port))
}

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

            // Get port config
            let (http_port, socks_port, enabled) = {
                let config = state.config.read().await;
                (
                    config.get().http_proxy_port,
                    config.get().proxy_port,
                    config.get().enabled,
                )
            };

            if !enabled {
                break;
            }

            let health = crate::health::check_trojan_health(http_port, socks_port).await;

            if health.all_healthy {
                if consecutive_failures > 0 {
                    tracing::info!("Health check recovered after {} failures", consecutive_failures);
                }
                consecutive_failures = 0;
                let _ = app_handle.emit("health:check", serde_json::json!({
                    "ok": true,
                    "httpPort": http_port,
                    "socksPort": socks_port,
                }));
            } else {
                consecutive_failures += 1;
                tracing::warn!(
                    "Health check failed (http={}, socks={}, failures={})",
                    health.http_proxy_ok,
                    health.socks5_ok,
                    consecutive_failures
                );

                let _ = app_handle.emit("health:check", serde_json::json!({
                    "ok": false,
                    "httpPort": http_port,
                    "socksPort": socks_port,
                    "consecutiveFailures": consecutive_failures,
                }));

                // If SOCKS5 is down but trojan manager says it should be running,
                // try to signal the watchdog to restart
                if !health.socks5_ok && consecutive_failures >= 2 {
                    let mut trojan = state.trojan.write().await;
                    let (running, _) = trojan.status();
                    if running {
                        // Process says it's running but port is down — it's probably hung
                        tracing::warn!("Trojan-go process alive but SOCKS5 port unresponsive, stopping to trigger restart");
                        let _ = trojan.stop().await;
                    }

                    // If HTTP proxy is also down, restart the whole proxy stack
                    if !health.http_proxy_ok && consecutive_failures >= 3 {
                        tracing::warn!("Full proxy stack down, attempting restart");
                        drop(trojan);
                        // The enable_proxy command would need to be called from frontend,
                        // but we can try to restart just the trojan part
                        let tc = {
                            let config = state.config.read().await;
                            config.get().trojan_config.clone()
                        };
                        let port = {
                            let config = state.config.read().await;
                            config.get().proxy_port
                        };
                        if let Some(tc) = tc {
                            let mut trojan = state.trojan.write().await;
                            if let Err(e) = trojan.start(tc, port).await {
                                tracing::error!("Auto-restart trojan failed: {}", e);
                            }
                        }
                    }
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

            // Update config
            let mut config = state.config.write().await;
            config.get_mut().cached_domains = domains;
            config.get_mut().last_fetch = Some(chrono::Utc::now().timestamp());
            config.get_mut().using_fallback = false;
            config.get_mut().using_cache = false;
            config.get_mut().last_fetch_error = None;
            let _ = config.save();
            drop(config);

            let _ = app_handle.emit("domains:updated", serde_json::json!({
                "count": state.config.read().await.get().cached_domains.len(),
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
