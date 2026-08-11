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
    std::fs::write(dest, EMBEDDED_BYTES)?;
    Ok(())
}
