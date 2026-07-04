use winreg::enums::*;
use winreg::RegKey;

const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

pub fn get_current_proxy() -> crate::error::Result<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey_with_flags(INTERNET_SETTINGS, KEY_READ)?;
    let enable: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enable == 1 {
        let server: String = key.get_value("ProxyServer").unwrap_or_default();
        Ok(server)
    } else {
        let auto_url: String = key.get_value("AutoConfigURL").unwrap_or_default();
        if !auto_url.is_empty() {
            Ok(auto_url)
        } else {
            Ok(String::new())
        }
    }
}

pub fn set_proxy(addr: &str) -> crate::error::Result<()> {
    let parts: Vec<&str> = addr.split(':').collect();
    let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(11032);
    let pac_url = format!("http://127.0.0.1:{}/pac.js", port);

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey_with_flags(INTERNET_SETTINGS, KEY_WRITE)?;
    // Manual proxy for Android Studio / Gradle / JVM tools
    key.set_value("ProxyEnable", &1u32)?;
    key.set_value("ProxyServer", &addr.to_string())?;
    key.set_value("ProxyOverride", &"localhost;127.*;10.*;192.168.*;<local>".to_string())?;
    // PAC auto-config for browsers (fixes WebSocket wss:// proxying)
    key.set_value("AutoConfigURL", &pac_url)?;
    broadcast_settings_change();
    Ok(())
}

pub fn clear_proxy() -> crate::error::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey_with_flags(INTERNET_SETTINGS, KEY_WRITE)?;
    key.set_value("ProxyEnable", &0u32)?;
    broadcast_settings_change();
    Ok(())
}

extern "system" {
    fn InternetSetOptionW(
        hInternet: *mut std::ffi::c_void,
        dwOption: u32,
        lpBuffer: *mut std::ffi::c_void,
        dwBufferLength: u32,
    ) -> i32;
}

fn broadcast_settings_change() {
    unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            39, // INTERNET_OPTION_SETTINGS_CHANGED
            std::ptr::null_mut(),
            0,
        );
        InternetSetOptionW(
            std::ptr::null_mut(),
            37, // INTERNET_OPTION_REFRESH
            std::ptr::null_mut(),
            0,
        );
    }
}
