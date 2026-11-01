//! Open backend-resolved folders without leaking AppImage libraries into host tools.
use launcher_core::error::ErrorCode;
use std::path::PathBuf;

/// A false result means the native shell is still waiting (Windows compatibility).
pub(super) async fn open(app: tauri::AppHandle, directory: PathBuf) -> Result<bool, ErrorCode> {
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        tauri::async_runtime::spawn_blocking(move || {
            super::linux_desktop::wait_for_opener(
                super::linux_desktop::system_opener(
                    directory.as_os_str(),
                    std::env::var_os("APPIMAGE").is_some(),
                ),
                std::time::Duration::from_secs(5),
            )
            .map(|()| true)
            .map_err(|_| ErrorCode::StorageUnavailable)
        })
        .await
        .map_err(|_| ErrorCode::StorageUnavailable)?
    }
    #[cfg(not(target_os = "linux"))]
    {
        use tauri_plugin_opener::OpenerExt;
        let opening = tauri::async_runtime::spawn_blocking(move || {
            app.opener()
                .open_path(directory.to_string_lossy(), None::<&str>)
                .map_err(|_| ErrorCode::StorageUnavailable)
        });
        match tokio::time::timeout(std::time::Duration::from_secs(5), opening).await {
            Ok(result) => {
                result.map_err(|_| ErrorCode::StorageUnavailable)??;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}
