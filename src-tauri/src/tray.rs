use tauri::{
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState},
    menu::{Menu, MenuItem},
    Manager,
};

pub async fn setup_tray(app_handle: &tauri::AppHandle) {
    let toggle = MenuItem::with_id(app_handle, "toggle", "Toggle Proxy", true, None::<&str>).unwrap();
    let status = MenuItem::with_id(app_handle, "status", "Status: OFF", false, None::<&str>).unwrap();
    let open = MenuItem::with_id(app_handle, "open", "Open Window", true, None::<&str>).unwrap();
    let quit = MenuItem::with_id(app_handle, "quit", "Quit", true, None::<&str>).unwrap();

    let menu = Menu::with_items(app_handle, &[&toggle, &status, &open, &quit]).unwrap();

    let _tray = TrayIconBuilder::new()
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
