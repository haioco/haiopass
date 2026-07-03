use std::fs;
use std::path::PathBuf;

fn maven_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".m2").join("settings.xml")
}

pub fn get_current() -> crate::error::Result<String> {
    let path = maven_path();
    if path.exists() {
        Ok(fs::read_to_string(&path)?)
    } else {
        Ok(String::new())
    }
}

pub fn apply(addr: &str) -> crate::error::Result<()> {
    let path = maven_path();
    let content = if path.exists() { fs::read_to_string(&path)? } else {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<settings xmlns="http://maven.apache.org/SETTINGS/1.0.0"
          xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
          xsi:schemaLocation="http://maven.apache.org/SETTINGS/1.0.0 http://maven.apache.org/xsd/settings-1.0.0.xsd">
  <proxies>
  </proxies>
</settings>"#.to_string()
    };

    let parts: Vec<&str> = addr.split(':').collect();
    let host = parts.first().copied().unwrap_or("127.0.0.1");
    let port = parts.get(1).copied().unwrap_or("11032");

    let proxy_xml = format!(
        r#"    <proxy>
      <id>haio-http</id>
      <active>true</active>
      <protocol>http</protocol>
      <host>{}</host>
      <port>{}</port>
      <nonProxyHosts>localhost|127.*</nonProxyHosts>
    </proxy>
    <proxy>
      <id>haio-https</id>
      <active>true</active>
      <protocol>https</protocol>
      <host>{}</host>
      <port>{}</port>
      <nonProxyHosts>localhost|127.*</nonProxyHosts>
    </proxy>"#,
        host, port, host, port
    );

    let content = remove_haio_proxies(&content);

    let content = if content.contains("<proxies>") {
        content.replace("<proxies>", &format!("<proxies>\n{}", proxy_xml))
    } else {
        content
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(())
}

pub fn clear() -> crate::error::Result<()> {
    let path = maven_path();
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        let content = remove_haio_proxies(&content);
        fs::write(&path, content)?;
    }
    Ok(())
}

fn remove_haio_proxies(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut filtered = Vec::new();
    let mut skip = false;
    for line in &lines {
        if line.contains("<id>haio-http</id>") || line.contains("<id>haio-https</id>") {
            skip = true;
        }
        if !skip {
            filtered.push(*line);
        }
        if line.contains("</proxy>") {
            skip = false;
        }
    }
    filtered.join("\n")
}

pub fn restore(backup: &str) -> crate::error::Result<()> {
    let path = maven_path();
    if backup.is_empty() {
        if path.exists() {
            fs::remove_file(&path)?;
        }
    } else {
        fs::write(&path, backup)?;
    }
    Ok(())
}
