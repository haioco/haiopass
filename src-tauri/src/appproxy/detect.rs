use std::process::Command;

pub async fn detect_available() -> Vec<String> {
    let mut available = Vec::new();

    if command_exists("gradle") || command_exists("gradlew") || std::path::Path::new(&format!("{}/.gradle", dirs::home_dir().unwrap_or_default().to_string_lossy())).exists() {
        available.push("gradle".into());
    }
    if command_exists("mvn") || std::path::Path::new(&format!("{}/.m2", dirs::home_dir().unwrap_or_default().to_string_lossy())).exists() {
        available.push("maven".into());
    }
    if command_exists("npm") {
        available.push("npm".into());
    }
    if command_exists("pip") || command_exists("pip3") {
        available.push("pip".into());
    }
    if command_exists("git") {
        available.push("git".into());
    }
    if command_exists("docker") {
        available.push("docker".into());
    }
    if command_exists("go") {
        available.push("go".into());
    }
    if std::path::Path::new(&format!("{}/.curlrc", dirs::home_dir().unwrap_or_default().to_string_lossy())).exists() || command_exists("curl") {
        available.push("curl".into());
    }

    available
}

fn command_exists(cmd: &str) -> bool {
    Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
