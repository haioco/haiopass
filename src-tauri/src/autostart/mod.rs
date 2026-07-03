#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;

pub struct AutoStart;

impl AutoStart {
    pub fn enable() -> crate::error::Result<()> {
        #[cfg(target_os = "windows")]
        windows::enable()?;
        #[cfg(target_os = "linux")]
        linux::enable()?;
        #[cfg(target_os = "macos")]
        macos::enable()?;
        Ok(())
    }

    pub fn disable() -> crate::error::Result<()> {
        #[cfg(target_os = "windows")]
        windows::disable()?;
        #[cfg(target_os = "linux")]
        linux::disable()?;
        #[cfg(target_os = "macos")]
        macos::disable()?;
        Ok(())
    }

    pub fn is_enabled() -> bool {
        #[cfg(target_os = "windows")]
        { windows::is_enabled() }
        #[cfg(target_os = "linux")]
        { linux::is_enabled() }
        #[cfg(target_os = "macos")]
        { macos::is_enabled() }
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        { false }
    }
}
