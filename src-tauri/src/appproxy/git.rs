use std::process::Command;

pub fn apply(addr: &str) -> crate::error::Result<()> {
    Command::new("git")
        .args(["config", "--global", "http.proxy", &format!("http://{}", addr)])
        .output()?;
    Ok(())
}

pub fn clear() -> crate::error::Result<()> {
    Command::new("git")
        .args(["config", "--global", "--unset", "http.proxy"])
        .output()?;
    Ok(())
}

pub fn restore(_backup: &str) -> crate::error::Result<()> {
    clear()
}
