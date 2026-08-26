#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;

pub mod quic;

pub struct OsProxy {
    backup: Option<String>,
    applied: bool,
    quic_blocked: bool,
}

impl Default for OsProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl OsProxy {
    pub fn new() -> Self {
        Self { backup: None, applied: false, quic_blocked: false }
    }

    /// Best-effort QUIC (UDP 443) block so browsers fall back to TCP over
    /// the local proxy instead of failing with ERR_QUIC_PROTOCOL_ERROR.
    fn block_quic(&mut self) {
        match quic::block() {
            Ok(()) => {
                self.quic_blocked = true;
                tracing::info!("QUIC (UDP 443) blocked");
            }
            Err(e) => tracing::warn!(
                "Could not block QUIC (UDP 443); Google services using QUIC may fail: {}",
                e
            ),
        }
    }

    fn unblock_quic(&mut self) {
        if !self.quic_blocked {
            return;
        }
        if let Err(e) = quic::unblock() {
            tracing::warn!("Failed to remove QUIC block rule: {}", e);
        }
        self.quic_blocked = false;
    }

    /// Startup crash cleanup — removes a stale QUIC block rule left behind
    /// by a previous session. Safe to call unconditionally.
    pub fn cleanup_quic_rule() {
        if let Err(e) = quic::unblock() {
            tracing::warn!("QUIC rule cleanup failed: {}", e);
        }
    }

    pub async fn backup(&mut self) -> crate::error::Result<()> {
        #[cfg(target_os = "windows")]
        { self.backup = windows::get_current_proxy().ok(); }
        #[cfg(target_os = "linux")]
        { self.backup = linux::get_current_proxy().ok(); }
        #[cfg(target_os = "macos")]
        { self.backup = macos::get_current_proxy().ok(); }
        Ok(())
    }

    pub async fn apply(&mut self, addr: String) -> crate::error::Result<()> {
        #[cfg(target_os = "windows")]
        windows::set_proxy(&addr)?;
        #[cfg(target_os = "linux")]
        linux::set_proxy(&addr)?;
        #[cfg(target_os = "macos")]
        macos::set_proxy(&addr)?;
        self.applied = true;
        self.block_quic();
        Ok(())
    }

    pub async fn clear(&mut self) -> crate::error::Result<()> {
        self.unblock_quic();
        #[cfg(target_os = "windows")]
        windows::clear_proxy()?;
        #[cfg(target_os = "linux")]
        linux::clear_proxy()?;
        #[cfg(target_os = "macos")]
        macos::clear_proxy()?;
        self.applied = false;
        Ok(())
    }

    pub async fn restore(&mut self) -> crate::error::Result<()> {
        self.unblock_quic();
        if let Some(ref backup) = self.backup {
            if backup.is_empty() {
                self.clear().await?;
            } else {
                #[cfg(target_os = "windows")]
                windows::set_proxy(backup)?;
                #[cfg(target_os = "linux")]
                linux::set_proxy(backup)?;
                #[cfg(target_os = "macos")]
                macos::set_proxy(backup)?;
            }
        } else if self.applied {
            self.clear().await?;
        }
        Ok(())
    }

    pub fn is_applied(&self) -> bool {
        self.applied
    }
}
