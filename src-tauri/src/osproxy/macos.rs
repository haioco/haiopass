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
            .args(["-getautoproxyurl", service])
            .output()?;
        let out = String::from_utf8_lossy(&output.stdout);
        if out.contains("Enabled: Yes") {
            let lines: Vec<&str> = out.lines().collect();
            if let Some(url_line) = lines.iter().find(|l| l.starts_with("URL:")) {
                let url = url_line.trim_start_matches("URL:").trim();
                if !url.is_empty() {
                    return Ok(url.to_string());
                }
            }
        }
    }
    Ok(String::new())
}

pub fn set_proxy(addr: &str) -> crate::error::Result<()> {
    let parts: Vec<&str> = addr.split(':').collect();
    let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(11032);
    let pac_url = format!("http://127.0.0.1:{}/pac.js", port);
    let services = get_active_services();
    for service in &services {
        Command::new("networksetup")
            .args(["-setautoproxyurl", service, &pac_url])
            .output()?;
    }
    Ok(())
}

pub fn clear_proxy() -> crate::error::Result<()> {
    let services = get_active_services();
    for service in &services {
        Command::new("networksetup")
            .args(["-setautoproxystate", service, "off"])
            .output()?;
    }
    Ok(())
}
