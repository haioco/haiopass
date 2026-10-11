use std::sync::OnceLock;
use tauri::{
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState},
    menu::{Menu, MenuItem},
    Manager,
};

pub const TRAY_ID: &str = "haio-tray";

static STATUS_ITEM: OnceLock<MenuItem<tauri::Wry>> = OnceLock::new();

/// Reflect proxy state in the tray: a visible indicator of "the system
/// proxy is currently modified" while the bypass is active.
pub fn update_status(app_handle: &tauri::AppHandle, enabled: bool) {
    let tooltip = if enabled {
        "HaioBypass — Proxy: ON (system proxy redirected)"
    } else {
        "HaioBypass — Proxy: OFF"
    };
    if let Some(tray) = app_handle.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip));
    }
    if let Some(item) = STATUS_ITEM.get() {
        let _ = item.set_text(if enabled { "Status: ON" } else { "Status: OFF" });
    }
}

pub async fn setup_tray(app_handle: &tauri::AppHandle) {
    let toggle = MenuItem::with_id(app_handle, "toggle", "Toggle Proxy", true, None::<&str>).unwrap();
    let status = MenuItem::with_id(app_handle, "status", "Status: OFF", false, None::<&str>).unwrap();
    let open = MenuItem::with_id(app_handle, "open", "Open Window", true, None::<&str>).unwrap();
    let quit = MenuItem::with_id(app_handle, "quit", "Quit", true, None::<&str>).unwrap();

    let _ = STATUS_ITEM.set(status.clone());

    let menu = Menu::with_items(app_handle, &[&toggle, &status, &open, &quit]).unwrap();

    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(app_handle.default_window_icon().unwrap().clone())
        .menu(&menu)
        .tooltip("HaioBypass — Proxy: OFF")
        .on_menu_event(move |app, event| {
            match event.id.as_ref() {
                "toggle" => {
                    let app_handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app_handle.state::<std::sync::Arc<crate::AppState>>();
                        let enabled = state.config.read().await.get().enabled;
                        if enabled {
                            let _ = crate::app::commands::disable_proxy(app_handle.clone()).await;
                        } else {
                            let _ = crate::app::commands::enable_proxy(app_handle.clone()).await;
                        }
                    });
                }
                "open" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "quit" => {
                    let app_handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app_handle.state::<std::sync::Arc<crate::AppState>>();
                        let _ = crate::app::commands::quit_and_restore(state).await;
                        app_handle.exit(0);
                    });
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app_handle)
        .unwrap();
}
