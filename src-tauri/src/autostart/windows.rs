use std::process::Command;

const TASK_NAME: &str = "HaioBypass";

pub fn enable() -> crate::error::Result<()> {
    let exe = std::env::current_exe().map_err(|e| crate::error::HaioError::Io(e))?;
    let exe_str = exe.to_string_lossy().to_string();

    // Remove old task if exists
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .output();

    // Try Scheduled Task first (Win10+ with elevation). On Win7 non-admin
    // /RL HIGHEST may fail → fallback to registry HKCU Run (works without admin).
    let output = Command::new("schtasks")
        .args([
            "/Create",
            "/TN", TASK_NAME,
            "/TR", &format!("\"{}\"", exe_str),
            "/SC", "ONLOGON",
            "/RL", "HIGHEST",
            "/F",
        ])
        .output()
        .map_err(|e| crate::error::HaioError::Io(e))?;

    if output.status.success() {
        return Ok(());
    }

    // Fallback: HKCU\Software\Microsoft\Windows\CurrentVersion\Run (Win7-compatible, no admin)
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            KEY_WRITE,
        ) {
            let _ = key.set_value(TASK_NAME, &exe_str);
            // Verify it stuck
            if key.get_value::<String, _>(TASK_NAME).is_ok() {
                return Ok(());
            }
        }
    }

    // If both failed, surface original schtasks error
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(crate::error::HaioError::Other(format!(
        "Failed to enable autostart (schtasks + registry): {}",
        if stderr.is_empty() {
            "unknown error"
        } else {
            &stderr
        }
    )))
}

pub fn disable() -> crate::error::Result<()> {
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .output();

    // Also remove registry fallback if present
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_WRITE)
        {
            let _ = key.delete_value(TASK_NAME);
        }
    }
    Ok(())
}

pub fn is_enabled() -> bool {
    // Check schtasks first
    if Command::new("schtasks")
        .args(["/Query", "/TN", TASK_NAME])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return true;
    }
    // Fallback registry check (Win7 non-admin)
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_READ)
        {
            if key.get_value::<String, _>(TASK_NAME).is_ok() {
                return true;
            }
        }
    }
    false
}
