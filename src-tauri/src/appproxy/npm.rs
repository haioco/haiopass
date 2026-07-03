use std::process::Command;

pub fn apply(addr: &str) -> crate::error::Result<()> {
    Command::new("npm")
        .args(["config", "set", "proxy", &format!("http://{}", addr)])
        .output()?;
    Command::new("npm")
        .args(["config", "set", "https-proxy", &format!("http://{}", addr)])
        .output()?;
    Ok(())
}

pub fn clear() -> crate::error::Result<()> {
    Command::new("npm")
        .args(["config", "delete", "proxy"])
        .output()?;
    Command::new("npm")
        .args(["config", "delete", "https-proxy"])
        .output()?;
    Ok(())
}
