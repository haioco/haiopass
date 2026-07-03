use std::process::Command;

const TASK_NAME: &str = "HaioBypass";

pub fn enable() -> crate::error::Result<()> {
    let exe = std::env::current_exe().map_err(|e| crate::error::HaioError::Io(e))?;
    let exe_str = exe.to_string_lossy().to_string();

    // Remove old task if exists
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .output();

    // Create new task
    Command::new("schtasks")
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

    Ok(())
}

pub fn disable() -> crate::error::Result<()> {
    Command::new("schtasks")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .output()
        .map_err(|e| crate::error::HaioError::Io(e))?;
    Ok(())
}

pub fn is_enabled() -> bool {
    Command::new("schtasks")
        .args(["/Query", "/TN", TASK_NAME])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
