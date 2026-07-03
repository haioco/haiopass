use std::path::Path;
use crate::config::TrojanConfig;

pub fn write_config(
    path: &Path,
    config: &TrojanConfig,
    local_port: u16,
) -> crate::error::Result<()> {
    let cfg = serde_json::json!({
        "run_type": "client",
        "local_addr": "127.0.0.1",
        "local_port": local_port,
        "remote_addr": config.server,
        "remote_port": config.port,
        "password": [config.password],
        "log_level": 1,
        "ssl": {
            "verify": true,
            "verify_hostname": true,
            "sni": config.sni,
            "alpn": ["h2", "http/1.1"],
            "reuse_session": true,
            "session_ticket": false,
            "curves": ""
        },
        "tcp": {
            "prefer_ipv4": false,
            "no_delay": true,
            "keep_alive": true,
            "reuse_port": false,
            "fast_open": false,
            "fast_open_qlen": 20
        }
    });

    let json = serde_json::to_string_pretty(&cfg)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &json)?;
    Ok(())
}
