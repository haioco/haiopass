use std::path::Path;

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../resources/trojan-go/trojan-go-windows-amd64.exe");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../resources/trojan-go/trojan-go-linux-amd64");
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../resources/trojan-go/trojan-go-darwin-arm64");
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../resources/trojan-go/trojan-go-darwin-amd64");

pub fn extract_bundled(dest: &Path) -> crate::error::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, EMBEDDED_BYTES)?;
    Ok(())
}
