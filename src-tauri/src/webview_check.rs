//! Pre-flight WebView2 runtime check (Windows).
//!
//! Tauri creates its window on the OS webview. On Windows that is the WebView2
//! runtime. The auto-updater only replaces the app bundle, so on a machine where
//! the runtime is missing — or was installed for a *different* Windows account —
//! the webview never initialises and the user gets Tauri's raw English error box
//! with no way forward.
//!
//! We detect the runtime before any window exists and try to repair it silently,
//! falling back to a native (webview-free) dialog with the manual download link.

/// Official Microsoft Evergreen standalone installer (x64).
#[cfg(target_os = "windows")]
const EVERGREEN_X64: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

#[cfg(target_os = "windows")]
mod imp {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    /// WebView2 "Evergreen" runtime EdgeUpdate client GUID.
    const CLIENT_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    const RETRY_COOLDOWN: Duration = Duration::from_secs(24 * 60 * 60);

    /// Installed runtime version, looked up in HKLM (32/64 views) then HKCU.
    fn runtime_version() -> Option<String> {
        let paths = [
            (
                HKEY_LOCAL_MACHINE,
                format!(
                    r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{}",
                    CLIENT_GUID
                ),
            ),
            (
                HKEY_LOCAL_MACHINE,
                format!(
                    r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{}",
                    CLIENT_GUID
                ),
            ),
            (
                HKEY_CURRENT_USER,
                format!(
                    r"Software\Microsoft\EdgeUpdate\Clients\{}",
                    CLIENT_GUID
                ),
            ),
        ];

        for (root, path) in paths {
            let root = RegKey::predef(root);
            if let Ok(key) = root.open_subkey_with_flags(&path, KEY_READ) {
                if let Ok(pv) = key.get_value::<String, _>("pv") {
                    if !pv.trim().is_empty() {
                        return Some(pv);
                    }
                }
            }
        }
        None
    }

    /// A fixed runtime shipped next to the exe (win7 build) has no registry
    /// entry, so treat its presence as "available" and never trigger a repair.
    fn bundled_runtime_present() -> bool {
        let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
        else {
            return false;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return false;
        };
        entries.flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("WebView2FixedRuntime")
        })
    }

    /// Avoid re-running a ~150 MB download on every launch when it keeps failing.
    fn repair_cooling_down() -> bool {
        let marker = crate::config::Store::config_dir().join("webview2-repair-failed");
        match std::fs::metadata(&marker).and_then(|m| m.modified()) {
            Ok(modified) => modified.elapsed().map(|d| d < RETRY_COOLDOWN).unwrap_or(false),
            Err(_) => false,
        }
    }

    fn mark_repair_failed() {
        let dir = crate::config::Store::config_dir();
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(dir.join("webview2-repair-failed"), b"");
        }
    }

    fn clear_repair_marker() {
        let _ = std::fs::remove_file(
            crate::config::Store::config_dir().join("webview2-repair-failed"),
        );
    }

    fn installer_path() -> PathBuf {
        std::env::temp_dir()
            .join("haiobypass-webview2")
            .join("MicrosoftEdgeWebview2Setup.exe")
    }

    /// Download the Evergreen standalone installer over HTTPS.
    fn download_installer() -> Result<(), String> {
        if !EVERGREEN_X64.starts_with("https://") {
            return Err("refusing non-HTTPS installer URL".into());
        }

        let dest = installer_path();
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if dest.exists() {
            return Ok(());
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;

        let bytes = rt.block_on(async {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(900))
                .build()
                .map_err(|e| e.to_string())?;
            let resp = client
                .get(EVERGREEN_X64)
                .send()
                .await
                .map_err(|e| format!("download request failed: {}", e))?;
            if !resp.status().is_success() {
                return Err(format!("unexpected status {}", resp.status()));
            }
            resp.bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| format!("download body failed: {}", e))
        })?;

        if bytes.is_empty() {
            return Err("downloaded installer is empty".into());
        }

        let tmp = dest.with_extension("exe.part");
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Run the installer for the current user and wait for registration.
    fn run_installer() -> Result<(), String> {
        let exe = installer_path();
        let status = std::process::Command::new(&exe)
            .args(["/silent", "/install"])
            .status()
            .map_err(|e| format!("could not launch installer: {}", e))?;

        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            if runtime_version().is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs(2));
        }

        Err(format!("installer exited with {}", status))
    }

    /// Native message box — used when the webview cannot be trusted to render UI.
    /// Text goes through temp UTF-8 files so Persian survives the command line.
    fn native_dialog(title: &str, body: &str) {
        let dir = std::env::temp_dir().join("haiobypass-webview2");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let title_file = dir.join("dialog-title.txt");
        let body_file = dir.join("dialog-body.txt");
        if std::fs::write(&title_file, title.as_bytes()).is_err()
            || std::fs::write(&body_file, body.as_bytes()).is_err()
        {
            return;
        }

        let script = format!(
            "$t=[IO.File]::ReadAllText('{}',[Text.Encoding]::UTF8);\
             $b=[IO.File]::ReadAllText('{}',[Text.Encoding]::UTF8);\
             Add-Type -AssemblyName System.Windows.Forms;\
             [void][System.Windows.Forms.MessageBox]::Show($b,$t,'OK',0);",
            title_file.display(),
            body_file.display()
        );

        let _ = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-STA",
                "-Command",
                script.as_str(),
            ])
            .status();
    }

    pub fn preflight() {
        if let Some(v) = runtime_version() {
            tracing::info!("WebView2 runtime {} detected", v);
            return;
        }
        if bundled_runtime_present() {
            tracing::info!("Bundled WebView2 fixed runtime detected");
            return;
        }

        tracing::warn!("WebView2 runtime not found — attempting silent repair");

        if repair_cooling_down() {
            tracing::warn!("WebView2 repair skipped (recent failure)");
            return;
        }

        if let Err(e) = download_installer().and_then(|_| run_installer()) {
            tracing::error!("WebView2 repair failed: {}", e);
            mark_repair_failed();
            return;
        }

        if runtime_version().is_some() {
            tracing::info!("WebView2 runtime installed successfully");
            clear_repair_marker();
        } else {
            tracing::error!("WebView2 still missing after repair attempt");
            mark_repair_failed();
            native_dialog(
                "HaioBypass — WebView2",
                "این برنامه برای اجرا به WebView2 Runtime نیاز دارد و نصب خودکار آن ناموفق بود.\n\n\
                 لطفاً با دسترسی Administrator این فایل را نصب کنید:\n\
                 https://go.microsoft.com/fwlink/p/?LinkId=2124703\n\n\
                 This app requires the WebView2 Runtime. The automatic installation failed.\n\
                 Please install it manually as Administrator using the link above.",
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    /// Linux uses WebKitGTK and macOS WKWebView — nothing to pre-flight.
    pub fn preflight() {
        tracing::info!("WebView pre-flight skipped (non-Windows)");
    }
}

/// Detect a missing WebView2 runtime and try to repair it before any window is
/// created. Safe to call on every platform and on every launch.
pub fn preflight() {
    imp::preflight()
}