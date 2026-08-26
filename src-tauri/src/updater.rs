use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

pub struct PendingUpdate {
    pub version: String,
    pub notes: Option<String>,
}

fn updater(app: &AppHandle) -> crate::error::Result<tauri_plugin_updater::Updater> {
    app.updater()
        .map_err(|e| crate::error::HaioError::Other(format!("Updater unavailable: {}", e)))
}

/// Check the updater endpoint for a newer release.
pub async fn check(app: &AppHandle) -> crate::error::Result<Option<PendingUpdate>> {
    let update = updater(app)?
        .check()
        .await
        .map_err(|e| crate::error::HaioError::Other(format!("Update check failed: {}", e)))?;

    Ok(update.map(|u| PendingUpdate {
        version: u.version.clone(),
        notes: u.body.clone(),
    }))
}

/// Download and install the pending update. The caller must restart the
/// app afterwards (`AppHandle::restart()`).
pub async fn download_and_install(app: &AppHandle) -> crate::error::Result<()> {
    let update = updater(app)?
        .check()
        .await
        .map_err(|e| crate::error::HaioError::Other(format!("Update check failed: {}", e)))?
        .ok_or_else(|| {
            crate::error::HaioError::Other("No update available to install".into())
        })?;

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| crate::error::HaioError::Other(format!("Update install failed: {}", e)))
}
