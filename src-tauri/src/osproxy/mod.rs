#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;

pub struct OsProxy {
    backup: Option<String>,
    applied: bool,
}

impl Default for OsProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl OsProxy {
    pub fn new() -> Self {
        Self { backup: None, applied: false }
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
        Ok(())
    }

    pub async fn clear(&mut self) -> crate::error::Result<()> {
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
        if let Some(ref backup) = self.backup {
            #[cfg(target_os = "windows")]
            windows::set_proxy(backup)?;
            #[cfg(target_os = "linux")]
            linux::set_proxy(backup)?;
            #[cfg(target_os = "macos")]
            macos::set_proxy(backup)?;
        } else if self.applied {
            self.clear().await?;
        }
        Ok(())
    }

    pub fn is_applied(&self) -> bool {
        self.applied
    }
}
