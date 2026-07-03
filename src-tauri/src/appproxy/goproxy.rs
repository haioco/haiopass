use std::fs;
use std::path::PathBuf;

fn goproxy_env_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".haiobypass").join("goproxy.env")
}

pub fn apply(addr: &str) -> crate::error::Result<()> {
    let path = goproxy_env_path();
    let content = format!(
        "# HaioBypass Go proxy — source this file before running go commands\n\
         export HTTP_PROXY=http://{}\n\
         export HTTPS_PROXY=http://{}\n\
         export NO_PROXY=localhost,127.0.0.1\n",
        addr, addr
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(())
}

pub fn clear() -> crate::error::Result<()> {
    let path = goproxy_env_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}
