use std::fs;
use std::path::PathBuf;

fn gradle_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".gradle").join("gradle.properties")
}

pub fn get_current() -> crate::error::Result<String> {
    let path = gradle_path();
    if path.exists() {
        Ok(fs::read_to_string(&path)?)
    } else {
        Ok(String::new())
    }
}

pub fn apply(addr: &str) -> crate::error::Result<()> {
    let path = gradle_path();
    let mut content = if path.exists() { fs::read_to_string(&path)? } else { String::new() };

    let props = [
        "systemProp.http.proxyHost",
        "systemProp.http.proxyPort",
        "systemProp.https.proxyHost",
        "systemProp.https.proxyPort",
        "systemProp.http.nonProxyHosts",
    ];

    for prop in &props {
        if let Some(pos) = content.find(prop) {
            let line_start = content.rfind('\n').map(|p| p + 1).unwrap_or(0);
            let line_end = content[pos..].find('\n').map(|p| pos + p).unwrap_or(content.len());
            content = format!("{}{}{}", &content[..line_start], "", &content[line_end..]);
        }
    }

    let parts: Vec<&str> = addr.split(':').collect();
    let host = parts.first().copied().unwrap_or("127.0.0.1");
    let port = parts.get(1).copied().unwrap_or("11032");

    content.push_str(&format!(
        "\nsystemProp.http.proxyHost={}\nsystemProp.http.proxyPort={}\n",
        host, port
    ));
    content.push_str(&format!(
        "systemProp.https.proxyHost={}\nsystemProp.https.proxyPort={}\n",
        host, port
    ));
    content.push_str("systemProp.http.nonProxyHosts=localhost|127.*|10.*|192.168.*\n");

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(())
}

pub fn clear() -> crate::error::Result<()> {
    let path = gradle_path();
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        let filtered: String = content
            .lines()
            .filter(|line| !line.starts_with("systemProp.http.proxy") && !line.starts_with("systemProp.https.proxy") && !line.starts_with("systemProp.http.nonProxyHosts"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, filtered)?;
    }
    Ok(())
}

pub fn restore(backup: &str) -> crate::error::Result<()> {
    let path = gradle_path();
    if backup.is_empty() {
        if path.exists() {
            fs::remove_file(&path)?;
        }
    } else {
        fs::write(&path, backup)?;
    }
    Ok(())
}
