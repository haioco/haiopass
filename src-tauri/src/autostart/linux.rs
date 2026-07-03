use std::fs;
use std::path::PathBuf;

fn desktop_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config")
        .join("autostart")
        .join("haio-bypass.desktop")
}

pub fn enable() -> crate::error::Result<()> {
    let exe = std::env::current_exe()?;
    let content = format!(
        r#"[Desktop Entry]
Type=Application
Name=HaioBypass
Exec={} --minimized
Hidden=false
NoDisplay=false
X-GNOME-Autostart-enabled=true
"#,
        exe.to_string_lossy()
    );
    let path = desktop_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(())
}

pub fn disable() -> crate::error::Result<()> {
    let path = desktop_file_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn is_enabled() -> bool {
    desktop_file_path().exists()
}
