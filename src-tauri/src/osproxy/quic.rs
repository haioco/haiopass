//! Best-effort blocking of outbound QUIC (UDP 443) while the bypass is active.
//!
//! Browsers and apps (notably Google services: YouTube, NotebookLM) try QUIC
//! over UDP 443 first. That traffic bypasses our HTTP CONNECT proxy (TCP only)
//! and dies at the filter, surfacing as ERR_QUIC_PROTOCOL_ERROR. Dropping
//! UDP 443 forces clients to fall back to TCP, which the proxy handles.
//!
//! All platforms require elevated privileges for this; failures are non-fatal
//! and logged by the caller — traffic still works, but Google services that
//! insist on QUIC may fail until privileges are available.

#[cfg(target_os = "windows")]
pub use windows_impl::{block, unblock};
#[cfg(target_os = "linux")]
pub use linux_impl::{block, unblock};
#[cfg(target_os = "macos")]
pub use macos_impl::{block, unblock};

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::process::Command;

    use crate::error::{HaioError, Result};

    const RULE_NAME: &str = "HaioBypass Block QUIC";

    fn run_netsh(args: &[&str]) -> Result<bool> {
        let out = Command::new("netsh").args(args).output()?;
        Ok(out.status.success())
    }

    pub fn block() -> Result<()> {
        // Delete-then-add keeps this idempotent across restarts.
        let _ = run_netsh(&[
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            &format!("name={}", RULE_NAME),
        ]);
        let added = run_netsh(&[
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={}", RULE_NAME),
            "dir=out",
            "action=block",
            "protocol=udp",
            "remoteport=443",
        ])?;
        if !added {
            return Err(HaioError::OsProxy(
                "Failed to add QUIC block firewall rule (administrator privileges required)"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn unblock() -> Result<()> {
        let _ = run_netsh(&[
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            &format!("name={}", RULE_NAME),
        ]);
        // Deletion is best-effort; treat command failure as success so we
        // never block disconnecting on a stale/missing rule.
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod linux_impl {
    use std::process::Command;

    use crate::error::{HaioError, Result};

    const RULE_SPEC: [&str; 6] = ["OUTPUT", "-p", "udp", "--dport", "443", "DROP"];

    fn run_iptables(args: &[&str]) -> std::io::Result<bool> {
        Command::new("iptables")
            .args(args)
            .output()
            .map(|o| o.status.success())
    }

    pub fn block() -> Result<()> {
        let spec: Vec<&str> = RULE_SPEC.to_vec();
        let mut check_args = vec!["-C"];
        check_args.extend_from_slice(&spec);
        if run_iptables(&check_args).unwrap_or(false) {
            return Ok(());
        }
        let mut insert_args = vec!["-I"];
        insert_args.extend_from_slice(&spec);
        if !run_iptables(&insert_args)? {
            return Err(HaioError::OsProxy(
                "Failed to add iptables QUIC block rule (root privileges required)".into(),
            ));
        }
        Ok(())
    }

    pub fn unblock() -> Result<()> {
        let spec: Vec<&str> = RULE_SPEC.to_vec();
        let mut delete_args = vec!["-D"];
        delete_args.extend_from_slice(&spec);
        // Ignore result — rule may not exist or we may lack privileges.
        let _ = run_iptables(&delete_args);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use std::process::Command;

    use crate::error::{HaioError, Result};

    const ANCHOR_NAME: &str = "haio-bypass";
    const ANCHOR_FILE: &str = "/etc/pf.anchors/haio-bypass";
    const PF_CONF: &str = "/etc/pf.conf";
    const ANCHOR_RULE: &str = "block drop out proto udp from any to any port 443";
    const LOAD_LINE_1: &str = "anchor \"haio-bypass\"";
    const LOAD_LINE_2: &str = "load anchor \"haio-bypass\" from \"/etc/pf.anchors/haio-bypass\"";

    pub fn block() -> Result<()> {
        std::fs::write(ANCHOR_FILE, format!("{}\n", ANCHOR_RULE))
            .map_err(|e| HaioError::OsProxy(format!("Cannot write {} (root required): {}", ANCHOR_FILE, e)))?;

        let conf = std::fs::read_to_string(PF_CONF).unwrap_or_default();
        if !conf.contains(&LOAD_LINE_2) {
            let updated = format!(
                "{}\n{}\n{}\n",
                conf.trim_end(),
                LOAD_LINE_1,
                LOAD_LINE_2
            );
            std::fs::write(PF_CONF, updated)
                .map_err(|e| HaioError::OsProxy(format!("Cannot update {} (root required): {}", PF_CONF, e)))?;
        }

        // Enable pf (ignoring "already enabled") and reload rules.
        let _ = Command::new("/sbin/pfctl").arg("-e").output();
        let out = Command::new("/sbin/pfctl").args(["-f", PF_CONF]).output()?;
        if !out.status.success() {
            return Err(HaioError::OsProxy(format!(
                "pfctl reload failed (root required): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    pub fn unblock() -> Result<()> {
        let conf = match std::fs::read_to_string(PF_CONF) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };
        let cleaned: String = conf
            .lines()
            .filter(|l| l.trim() != LOAD_LINE_1 && l.trim() != LOAD_LINE_2)
            .collect::<Vec<_>>()
            .join("\n");
        let changed = cleaned != conf.trim_end();
        let _ = std::fs::write(PF_CONF, format!("{}\n", cleaned));
        let _ = std::fs::remove_file(ANCHOR_FILE);
        if changed {
            let _ = Command::new("/sbin/pfctl").args(["-f", PF_CONF]).output();
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub fn block() -> crate::error::Result<()> {
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub fn unblock() -> crate::error::Result<()> {
    Ok(())
}
