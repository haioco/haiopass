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

#[cfg(target_os = "windows")]
mod imp {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    /// Official Microsoft Evergreen standalone installer (x64).
    const EVERGREEN_X64: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
    /// WebView2 "Evergreen" runtime EdgeUpdate client GUID.
    const CLIENT_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    /// The executable the WebView2 loader actually needs on disk.
    const RUNTIME_EXE: &str = "msedgewebview2.exe";
    /// The WebView2 loader checks this env var BEFORE the registry: when set,
    /// the folder it names is the only runtime the loader will ever use.
    const WEBVIEW2_RUNTIME_ENV: &str = "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER";
    const RETRY_COOLDOWN: Duration = Duration::from_secs(24 * 60 * 60);

    /// Full EdgeUpdate registration for the runtime: (version, install location).
    fn runtime_registration() -> Option<(String, String)> {
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
            let Ok(key) = root.open_subkey_with_flags(&path, KEY_READ) else {
                continue;
            };
            let pv = key
                .get_value::<String, _>("pv")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let location = key
                .get_value::<String, _>("location")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if let (Some(pv), Some(location)) = (pv, location) {
                return Some((pv, location));
            }
        }
        None
    }

    /// A registered runtime only counts if `msedgewebview2.exe` is really on disk.
    ///
    /// Trusting the registry `pv` value alone is what made this pre-flight
    /// useless: on a machine whose registration was stale or whose bundled
    /// fixed runtime was missing, it logged "runtime detected" and returned —
    /// and the very next step failed inside the WebView2 loader, which shows its
    /// own English "Could not find the WebView2 Runtime" dialog with no way out.
    fn runtime_is_usable() -> bool {
        let Some((pv, location)) = runtime_registration() else {
            return false;
        };
        let base = Path::new(&location);
        if base.join(&pv).join(RUNTIME_EXE).is_file() {
            return true;
        }
        // Some registrations omit the exact version subdir; accept any version
        // directory that actually carries the runtime executable.
        std::fs::read_dir(base)
            .map(|entries| entries.flatten().any(|e| e.path().join(RUNTIME_EXE).is_file()))
            .unwrap_or(false)
    }

    /// Clear a stale WEBVIEW2_BROWSER_EXECUTABLE_FOLDER env var that points at
    /// an unusable runtime — an inherited (machine/user-level) value makes the
    /// loader skip the system runtime too.
    fn ensure_webview_env() {
        if let Ok(value) = std::env::var(WEBVIEW2_RUNTIME_ENV) {
            if !Path::new(&value).join(RUNTIME_EXE).is_file() {
                tracing::warn!(
                    "Clearing stale {} (points at unusable {})",
                    WEBVIEW2_RUNTIME_ENV,
                    value
                );
                std::env::remove_var(WEBVIEW2_RUNTIME_ENV);
            }
        }
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

    /// Temp working dir for the WebView2 repair path. Product-branded so its
    /// contents are attributable when users or AV inspect temp.
    const REPAIR_DIR_NAME: &str = "HaioBypass-WebView2-Repair";

    fn repair_dir() -> PathBuf {
        std::env::temp_dir().join(REPAIR_DIR_NAME)
    }

    fn installer_path() -> PathBuf {
        repair_dir().join("MicrosoftEdgeWebview2Setup.exe")
    }

    /// Verify the downloaded installer's Authenticode signature: status
    /// `Valid` AND signer organization `Microsoft Corporation`, before it is
    /// ever executed. A hash pin cannot work here — Microsoft re-releases the
    /// Evergreen bootstrapper periodically — so the publisher identity is the
    /// stable invariant. Uses PowerShell's Get-AuthenticodeSignature
    /// (WinVerifyTrust) instead of executing anything downloaded.
    fn verify_installer_signature(exe: &Path) -> Result<(), String> {
        let script = format!(
            "$sig = Get-AuthenticodeSignature -FilePath '{}'; \
             if ($sig.Status -ne 'Valid') {{ exit 3 }} \
             if ($sig.SignerCertificate.Subject -notlike '*Microsoft Corporation*') {{ exit 4 }} \
             exit 0",
            exe.display()
        );
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()
            .map_err(|e| format!("signature check failed to start: {}", e))?;
        match status.code() {
            Some(0) => Ok(()),
            Some(3) => Err("installer signature is not valid".into()),
            Some(4) => Err("installer is not signed by Microsoft Corporation".into()),
            Some(code) => Err(format!("signature check failed (exit {})", code)),
            None => Err("signature check was terminated".into()),
        }
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

    /// Run the installer for the current user and wait for a *usable* runtime.
    /// The downloaded file must first prove it is a validly-signed Microsoft
    /// binary — download-execute from temp without that check is exactly the
    /// dropper shape behavioural engines score.
    fn run_installer() -> Result<(), String> {
        let exe = installer_path();
        verify_installer_signature(&exe)?;

        let status = std::process::Command::new(&exe)
            .args(["/silent", "/install"])
            .status()
            .map_err(|e| format!("could not launch installer: {}", e))?;

        // Wait for the runtime executable to actually appear, not merely for the
        // registration to be written — a registered version with no binaries is
        // the exact state the loader refuses to start from.
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            if runtime_is_usable() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs(2));
        }

        Err(format!("installer exited with {}", status))
    }

    /// Native message box — used when the webview cannot be trusted to render UI.
    /// Text goes through temp UTF-8 files so Persian survives the command line.
    fn native_dialog(title: &str, body: &str) {
        let dir = repair_dir();
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

    /// Shown when no usable WebView2 runtime could be found or repaired. Kept in
    /// one place so both the pre-flight and the window-creation failure path
    /// give the user the same actionable Persian guidance instead of the
    /// WebView2 loader's opaque English message box.
    fn show_runtime_error_dialog() {
        native_dialog(
            "HaioBypass — WebView2",
            "این برنامه برای اجرا به WebView2 Runtime نیاز دارد و نصب خودکار آن ناموفق بود.\n\n\
             لطفاً با دسترسی Administrator این فایل را نصب کنید:\n\
             https://go.microsoft.com/fwlink/p/?LinkId=2124703\n\n\
             سپس برنامه را دوباره اجرا کنید.\n\n\
             This app requires the WebView2 Runtime. The automatic installation failed.\n\
             Please install it manually as Administrator using the link above, then\n\
             start the app again.",
        );
    }

    pub fn preflight() {
        ensure_webview_env();

        if runtime_is_usable() {
            match runtime_registration() {
                Some((pv, _)) => tracing::info!("WebView2 runtime {} verified on disk", pv),
                None => tracing::info!("WebView2 runtime verified on disk"),
            }
            return;
        }

        tracing::warn!("WebView2 runtime missing or unusable — attempting silent repair");

        if repair_cooling_down() {
            tracing::warn!("WebView2 repair skipped (recent failure)");
        } else if let Err(e) = download_installer().and_then(|_| run_installer()) {
            tracing::error!("WebView2 repair failed: {}", e);
            mark_repair_failed();
        } else if runtime_is_usable() {
            tracing::info!("WebView2 runtime installed successfully");
            clear_repair_marker();
            return;
        } else {
            tracing::error!("WebView2 still unusable after repair attempt");
            mark_repair_failed();
        }

        // Always explain, even when the repair was skipped: staying silent here
        // is what left users staring at the loader's English error box.
        show_runtime_error_dialog();
    }

    pub fn show_native_dialog(title: &str, body: &str) {
        native_dialog(title, body)
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    /// Linux uses WebKitGTK and macOS WKWebView — nothing to pre-flight.
    pub fn preflight() {
        tracing::info!("WebView pre-flight skipped (non-Windows)");
    }

    pub fn show_native_dialog(title: &str, body: &str) {
        tracing::error!("{}: {}", title, body);
    }
}

/// Detect a missing WebView2 runtime and try to repair it before any window is
/// created. Safe to call on every platform and on every launch.
///
/// This verifies that a runtime is actually usable — a registered version
/// number is not enough. Anything less reports success and then lets the
/// WebView2 loader fail with an untranslatable English error box.
pub fn preflight() {
    imp::preflight()
}

/// Show a native, webview-free message box.
///
/// Used when the window could not be created at all, i.e. exactly when the
/// webview cannot be trusted to render any UI.
pub fn show_native_dialog(title: &str, body: &str) {
    imp::show_native_dialog(title, body)
}
