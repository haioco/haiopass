use std::process::Command;

use crate::error::{HaioError, Result};

/// Run `networksetup` and fail on nonzero exit, surfacing stderr.
/// `networksetup` prints errors like `** Error: ...` to stderr and exits
/// nonzero — previously these were silently swallowed, which made the app
/// report "Connected" while no system proxy had been applied at all
/// (e.g. macOS 15 Sequoia requires root for proxy changes).
fn run(args: &[&str]) -> Result<String> {
    let out = Command::new("networksetup").args(args).output().map_err(|e| {
        HaioError::OsProxy(format!("failed to run networksetup {:?}: {}", args, e))
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(HaioError::OsProxy(format!(
            "networksetup {:?} failed: {}{}",
            args,
            stdout.trim(),
            stderr.trim()
        )));
    }
    Ok(stdout)
}

/// Shell-quote a single argument for embedding in an osascript
/// `do shell script` command string.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run a shell snippet with administrator privileges, prompting the user
/// once via the standard macOS password dialog. Returns Err if the user
/// cancels or the command fails.
fn run_elevated(snippet: &str) -> Result<String> {
    let script = format!("do shell script {} with administrator privileges", sh_quote(snippet));
    let out = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| HaioError::OsProxy(format!("failed to run osascript: {}", e)))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(HaioError::OsProxy(format!(
            "elevated networksetup failed (user cancelled or denied?): {}",
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn get_active_services() -> Result<Vec<String>> {
    let out = run(&["-listallnetworkservices"])?;
    Ok(out
        .lines()
        .skip(1) // skip header line ("An asterisk (*) denotes ...")
        .filter(|s| !s.trim().is_empty() && !s.starts_with('*'))
        .map(|s| s.trim().to_string())
        .collect())
}

pub fn get_current_proxy() -> crate::error::Result<String> {
    let services = get_active_services().unwrap_or_default();
    for service in &services {
        // Read-only query; a per-service failure just means that service
        // has no (or no readable) proxy config — skip it.
        if let Ok(out) = run(&["-getautoproxyurl", service]) {
            if out.contains("Enabled: Yes") {
                if let Some(url_line) = out.lines().find(|l| l.starts_with("URL:")) {
                    let url = url_line.trim_start_matches("URL:").trim();
                    if !url.is_empty() {
                        return Ok(url.to_string());
                    }
                }
            }
        }
    }
    Ok(String::new())
}

pub fn set_proxy(addr: &str) -> crate::error::Result<()> {
    // `apply()` passes "127.0.0.1:<port>"; `restore()` passes a previously
    // backed-up PAC url — accept both.
    let pac_url = if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_string()
    } else {
        let parts: Vec<&str> = addr.split(':').collect();
        let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(11032);
        format!("http://127.0.0.1:{}/pac.js", port)
    };
    let services = get_active_services()?;
    if services.is_empty() {
        return Err(HaioError::OsProxy(
            "no active network services found to apply the proxy to".into(),
        ));
    }

    // Try without elevation first (works on older macOS for admin users).
    let mut failed = Vec::new();
    for service in &services {
        let args: Vec<&str> = vec!["-setautoproxyurl", service, &pac_url];
        if run(&args).is_err() {
            failed.push(service.clone());
        }
    }

    // macOS 15+ (Sequoia) requires root for proxy changes: fall back to a
    // single admin-privilege prompt covering every remaining service.
    if !failed.is_empty() {
        let mut snippet = String::new();
        for service in &failed {
            snippet.push_str(&format!(
                "networksetup -setautoproxyurl {} {}; networksetup -setautoproxystate {} on; ",
                sh_quote(service),
                sh_quote(&pac_url),
                sh_quote(service),
            ));
        }
        run_elevated(snippet.trim_end())?;
    }

    // Verify the PAC actually landed on at least one service. Reading back
    // does not require elevation.
    for service in &services {
        if let Ok(out) = run(&["-getautoproxyurl", service]) {
            if out.contains("Enabled: Yes") && out.contains(&pac_url) {
                tracing::info!("PAC applied to network service {:?}", service);
                return Ok(());
            }
        }
    }
    Err(HaioError::OsProxy(format!(
        "PAC url {} was not applied to any network service — system proxy takeover failed",
        pac_url
    )))
}

pub fn clear_proxy() -> crate::error::Result<()> {
    let services = get_active_services().unwrap_or_default();
    let mut failed = Vec::new();
    for service in &services {
        if run(&["-setautoproxystate", service, "off"]).is_err() {
            failed.push(service.clone());
        }
    }
    if !failed.is_empty() {
        let mut snippet = String::new();
        for service in &failed {
            snippet.push_str(&format!(
                "networksetup -setautoproxystate {} off; ",
                sh_quote(service)
            ));
        }
        // Best-effort: never block disconnect on a cancelled/denied prompt —
        // a leftover PAC to a dead local port is annoying but recoverable on
        // next connect, whereas a stuck "disconnecting" state is worse.
        if let Err(e) = run_elevated(snippet.trim_end()) {
            tracing::warn!("could not disable auto-proxy on {:?}: {}", failed, e);
        }
    }
    Ok(())
}
