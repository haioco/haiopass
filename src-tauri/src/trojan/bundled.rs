use std::path::Path;

// Binaries are downloaded by scripts/fetch-trojan.sh from upstream trojan-go
// releases and rebranded as haio-proxy-* to reduce AV false-positives.
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../resources/trojan-go/haio-proxy-windows-amd64.exe");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../resources/trojan-go/haio-proxy-linux-amd64");
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../resources/trojan-go/haio-proxy-darwin-arm64");
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../resources/trojan-go/haio-proxy-darwin-amd64");

pub fn extract_bundled(dest: &Path) -> crate::error::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Write to a temp file and rename so overwriting an existing (possibly
    // locked) binary is atomic on all platforms.
    let tmp = dest.with_file_name(format!("{}.tmp", dest.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()));
    std::fs::write(&tmp, EMBEDDED_BYTES)?;
    if dest.exists() {
        let _ = std::fs::remove_file(dest);
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}
