use tauri::Emitter;
use crate::AppState;

pub async fn emit_status_update(
    app_handle: &tauri::AppHandle,
    state: &std::sync::Arc<AppState>,
) -> Result<(), String> {
    let config = state.config.read().await;
    let s = config.get();
    let _ = app_handle.emit("status:update", serde_json::json!({
        "enabled": s.enabled,
        "domainCount": s.cached_domains.len(),
        "usingFallback": s.using_fallback,
        "usingCache": s.using_cache,
    }));
    Ok(())
}

pub async fn emit_trojan_status(
    app_handle: &tauri::AppHandle,
    running: bool,
    pid: Option<u32>,
) {
    let _ = app_handle.emit("trojan:status", serde_json::json!({
        "running": running,
        "pid": pid,
    }));
}

pub async fn emit_domains_updated(
    app_handle: &tauri::AppHandle,
    count: usize,
    fallback: bool,
) {
    let _ = app_handle.emit("domains:updated", serde_json::json!({
        "count": count,
        "usingFallback": fallback,
    }));
}

pub async fn emit_health_check(
    app_handle: &tauri::AppHandle,
    ok: bool,
    port: u16,
) {
    let _ = app_handle.emit("health:check", serde_json::json!({
        "ok": ok,
        "port": port,
    }));
}
