use std::fs;
use std::path::PathBuf;

fn docker_config_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".docker").join("config.json")
}

pub fn get_current() -> crate::error::Result<String> {
    let path = docker_config_path();
    if path.exists() {
        Ok(fs::read_to_string(&path)?)
    } else {
        Ok(String::new())
    }
}

pub fn apply(addr: &str) -> crate::error::Result<()> {
    let path = docker_config_path();
    let mut config: serde_json::Value = if path.exists() {
        serde_json::from_str(&fs::read_to_string(&path)?)?
    } else {
        serde_json::json!({})
    };

    let mut proxies = config.get("proxies").cloned().unwrap_or(serde_json::json!({}));

    let default_proxy = serde_json::json!({
        "httpProxy": format!("http://{}", addr),
        "httpsProxy": format!("http://{}", addr),
        "noProxy": "localhost,127.0.0.1"
    });

    if let Some(obj) = proxies.as_object_mut() {
        obj.insert("default".into(), default_proxy);
    }

    config["proxies"] = proxies;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

pub fn clear() -> crate::error::Result<()> {
    let path = docker_config_path();
    if path.exists() {
        let mut config: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        if let Some(proxies) = config.get_mut("proxies") {
            if let Some(obj) = proxies.as_object_mut() {
                obj.remove("default");
                if obj.is_empty() {
                    config.as_object_mut().map(|o| o.remove("proxies"));
                }
            }
        }
        fs::write(&path, serde_json::to_string_pretty(&config)?)?;
    }
    Ok(())
}

pub fn restore(backup: &str) -> crate::error::Result<()> {
    let path = docker_config_path();
    if backup.is_empty() {
        if path.exists() {
            fs::remove_file(&path)?;
        }
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, backup)?;
    }
    Ok(())
}
