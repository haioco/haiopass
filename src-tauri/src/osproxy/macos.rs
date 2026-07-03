use std::process::Command;

fn get_active_services() -> Vec<String> {
    let output = match Command::new("networksetup")
        .arg("-listallnetworkservices")
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip(1) // skip header
        .filter(|s| !s.starts_with('*'))
        .map(|s| s.trim().to_string())
        .collect()
}

pub fn get_current_proxy() -> crate::error::Result<String> {
    let services = get_active_services();
    for service in &services {
        let output = Command::new("networksetup")
            .args(["-getwebproxy", service])
            .output()?;
        let out = String::from_utf8_lossy(&output.stdout);
        if out.contains("Enabled: Yes") {
            let lines: Vec<&str> = out.lines().collect();
            if let Some(server_line) = lines.iter().find(|l| l.starts_with("Server:")) {
                let server = server_line.trim_start_matches("Server:").trim();
                if !server.is_empty() {
                    return Ok(server.to_string());
                }
            }
        }
    }
    Ok(String::new())
}

pub fn set_proxy(addr: &str) -> crate::error::Result<()> {
    let parts: Vec<&str> = addr.split(':').collect();
    let host = parts.first().copied().unwrap_or("127.0.0.1");
    let port = parts.get(1).copied().unwrap_or("11032");
    let services = get_active_services();
    for service in &services {
        Command::new("networksetup")
            .args(["-setwebproxy", service, host, port, "off"])
            .output()?;
        Command::new("networksetup")
            .args(["-setsecurewebproxy", service, host, port, "off"])
            .output()?;
    }
    Ok(())
}

pub fn clear_proxy() -> crate::error::Result<()> {
    let services = get_active_services();
    for service in &services {
        Command::new("networksetup")
            .args(["-setwebproxystate", service, "off"])
            .output()?;
        Command::new("networksetup")
            .args(["-setsecurewebproxystate", service, "off"])
            .output()?;
    }
    Ok(())
}
