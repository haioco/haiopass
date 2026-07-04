use std::process::Command;

pub fn get_current_proxy() -> crate::error::Result<String> {
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.system.proxy", "mode"])
        .output()?;
    let mode = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if mode == "'manual'" {
        let host: String = get_gsettings("org.gnome.system.proxy.http", "host")?;
        let port: String = get_gsettings("org.gnome.system.proxy.http", "port")?;
        if !host.is_empty() {
            return Ok(format!("{}:{}", host, port));
        }
    } else if mode == "'auto'" {
        let url: String = get_gsettings("org.gnome.system.proxy", "autoconfig-url")?;
        if !url.is_empty() {
            return Ok(url);
        }
    }
    Ok(String::new())
}

fn get_gsettings(schema: &str, key: &str) -> crate::error::Result<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().trim_matches('\'').to_string())
}

pub fn set_proxy(addr: &str) -> crate::error::Result<()> {
    let parts: Vec<&str> = addr.split(':').collect();
    let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(11032);

    let pac_url = format!("http://127.0.0.1:{}/pac.js", port);

    Command::new("gsettings").args([
        "set", "org.gnome.system.proxy", "mode", "'auto'"
    ]).output()?;
    Command::new("gsettings").args([
        "set", "org.gnome.system.proxy", "autoconfig-url", &format!("'{}'", pac_url)
    ]).output()?;
    Command::new("gsettings").args([
        "set", "org.gnome.system.proxy", "ignore-hosts",
        "['localhost', '127.0.0.0/8', '::1']"
    ]).output()?;

    Ok(())
}

pub fn clear_proxy() -> crate::error::Result<()> {
    Command::new("gsettings").args([
        "set", "org.gnome.system.proxy", "mode", "'none'"
    ]).output()?;
    Ok(())
}
