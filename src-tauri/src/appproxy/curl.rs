use std::fs;
use std::path::PathBuf;

fn curlrc_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".curlrc")
}

pub fn get_current() -> crate::error::Result<String> {
    let path = curlrc_path();
    if path.exists() {
        Ok(fs::read_to_string(&path)?)
    } else {
        Ok(String::new())
    }
}

pub fn apply(addr: &str) -> crate::error::Result<()> {
    let path = curlrc_path();
    let content = if path.exists() { fs::read_to_string(&path)? } else { String::new() };

    // Remove existing proxy lines
    let filtered: String = content
        .lines()
        .filter(|line| !line.starts_with("proxy"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut final_content = filtered.trim_end().to_string();
    final_content.push_str(&format!("\nproxy = http://{}\n", addr));

    fs::write(&path, final_content)?;
    Ok(())
}

pub fn clear() -> crate::error::Result<()> {
    let path = curlrc_path();
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        let filtered: String = content
            .lines()
            .filter(|line| !line.starts_with("proxy"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, filtered)?;
    }
    Ok(())
}

pub fn restore(backup: &str) -> crate::error::Result<()> {
    let path = curlrc_path();
    if backup.is_empty() {
        if path.exists() {
            fs::remove_file(&path)?;
        }
    } else {
        fs::write(&path, backup)?;
    }
    Ok(())
}
