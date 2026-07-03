use std::fs;
use std::path::PathBuf;

fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("Library")
        .join("LaunchAgents")
        .join("ir.haio.bypass.plist")
}

pub fn enable() -> crate::error::Result<()> {
    let exe = std::env::current_exe()?;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ir.haio.bypass</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--minimized</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>"#,
        exe.to_string_lossy()
    );
    let path = plist_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, plist)?;
    Ok(())
}

pub fn disable() -> crate::error::Result<()> {
    let path = plist_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn is_enabled() -> bool {
    plist_path().exists()
}
