use std::fs;
use std::path::PathBuf;

fn pip_conf_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    { dirs::home_dir().unwrap_or_default().join("AppData").join("Roaming").join("pip").join("pip.ini") }
    #[cfg(not(target_os = "windows"))]
    { dirs::home_dir().unwrap_or_default().join(".config").join("pip").join("pip.conf") }
}

pub fn get_current() -> crate::error::Result<String> {
    let path = pip_conf_path();
    if path.exists() {
        Ok(fs::read_to_string(&path)?)
    } else {
        Ok(String::new())
    }
}

pub fn apply(addr: &str) -> crate::error::Result<()> {
    let path = pip_conf_path();
    let content = if path.exists() { fs::read_to_string(&path)? } else {
        "[global]\n".to_string()
    };

    let filtered: String = content
        .lines()
        .filter(|line| !line.starts_with("proxy") && !line.starts_with("http_proxy"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut final_content = filtered.trim_end().to_string();
    final_content.push_str(&format!("\nproxy=http://{}\n", addr));

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, final_content)?;
    Ok(())
}

pub fn clear() -> crate::error::Result<()> {
    let path = pip_conf_path();
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        let filtered: String = content
            .lines()
            .filter(|line| !line.starts_with("proxy") && !line.starts_with("http_proxy"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, filtered)?;
    }
    Ok(())
}

pub fn restore(backup: &str) -> crate::error::Result<()> {
    let path = pip_conf_path();
    if backup.is_empty() {
        if path.exists() {
            fs::remove_file(&path)?;
        }
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, backup)?;
    }
    Ok(())
}
