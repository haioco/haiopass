const TASK_NAME: &str = "HaioBypass";

pub fn enable() -> crate::error::Result<()> {
    let exe = std::env::current_exe().map_err(|e| crate::error::HaioError::Io(e))?;
    let exe_str = exe.to_string_lossy().to_string();

    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            KEY_WRITE,
        ) {
            key.set_value(TASK_NAME, &exe_str)?;
            return Ok(());
        }
    }

    Err(crate::error::HaioError::Other(
        "Failed to enable autostart (registry)".into(),
    ))
}

pub fn disable() -> crate::error::Result<()> {
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
