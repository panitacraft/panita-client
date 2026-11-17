//! Register only the fixed callback scheme; raw callbacks stay in Rust arguments.
use launcher_core::error::ErrorCode;
#[cfg(target_os = "linux")]
use tauri::Manager;
#[cfg(any(target_os = "linux", test))]
#[path = "linux_game_identity.rs"]
mod linux_game_identity;
#[cfg(any(target_os = "linux", test))]
#[path = "linux_protocol.rs"]
mod linux_protocol;

#[cfg(windows)]
pub fn register(_app: &tauri::AppHandle) -> Result<(), ErrorCode> {
    use windows_registry::{CLASSES_ROOT, CURRENT_USER};
    let exe = std::env::current_exe().map_err(|_| ErrorCode::CallbackRegistrationFailed)?;
    let exe = exe.to_str().ok_or(ErrorCode::CallbackRegistrationFailed)?;
    let command = format!("\"{exe}\" \"%1\"");
    let base = "Software\\Classes\\panitacraft";
    if let Ok(existing) = CURRENT_USER.open(base) {
        let owned = existing.get_string("PanitaClientOwner").ok().as_deref()
            == Some("com.panitacraft.client");
        let same_command = CURRENT_USER
            .open(format!("{base}\\shell\\open\\command"))
            .and_then(|key| key.get_string(""))
            .ok()
            .as_deref()
            == Some(command.as_str());
        if !owned && !same_command {
            return Err(ErrorCode::CallbackRegistrationFailed);
        }
    }
    let result = (|| -> windows_registry::Result<()> {
        let key = CURRENT_USER.create(base)?;
        key.set_string("", "URL:Panita Client OAuth callback")?;
        key.set_string("URL Protocol", "")?;
        key.set_string("PanitaClientOwner", "com.panitacraft.client")?;
        CURRENT_USER
            .create(format!("{base}\\shell\\open\\command"))?
            .set_string("", &command)?;
        Ok(())
    })();
    result.map_err(|_| ErrorCode::CallbackRegistrationFailed)?;
    let effective = CLASSES_ROOT
        .open("panitacraft\\shell\\open\\command")
        .and_then(|key| key.get_string(""))
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)?;
    if effective != command {
        return Err(ErrorCode::CallbackRegistrationFailed);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_paths(
    app: &tauri::AppHandle,
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), ErrorCode> {
    let executable = app
        .env()
        .appimage
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_exe().map_err(|_| ErrorCode::CallbackRegistrationFailed)?);
    let data = app
        .path()
        .data_dir()
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)?;
    let directory = data.join("applications");
    let icons = data.join("icons/hicolor/512x512/apps");
    Ok((executable, directory, icons))
}

#[cfg(target_os = "linux")]
pub struct PendingDesktopIntegration {
    executable: std::path::PathBuf,
    directory: std::path::PathBuf,
    changed: bool,
}

/// Publish the canonical launcher identity before GTK maps the first window.
/// This phase intentionally performs local, idempotent file operations only.
#[cfg(target_os = "linux")]
pub fn prepare_desktop_integration(
    app: &tauri::AppHandle,
) -> Result<PendingDesktopIntegration, ErrorCode> {
    let (executable, directory, icons) = linux_paths(app)?;
    let mut changed = linux_protocol::prepare(&executable, &directory, &icons)
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)?;
    // The shared Minecraft window identity is best effort and must never block
    // the launcher: without it the game window only keeps a generic icon.
    if let (Ok(escaped), Some(hicolor)) = (
        linux_protocol::escaped_executable(&executable),
        icons.ancestors().nth(2),
    ) {
        changed |= linux_game_identity::publish(&escaped, &directory, hicolor).unwrap_or(false);
    }
    Ok(PendingDesktopIntegration {
        executable,
        directory,
        changed,
    })
}

/// Refresh shell caches and migrate a legacy MIME default off the startup path.
#[cfg(target_os = "linux")]
pub fn finish_desktop_integration(pending: PendingDesktopIntegration) -> Result<(), ErrorCode> {
    linux_protocol::finish_migration(&pending.executable, &pending.directory, pending.changed)
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)
}

#[cfg(target_os = "linux")]
pub fn register(app: &tauri::AppHandle) -> Result<(), ErrorCode> {
    let (executable, directory, icons) = linux_paths(app)?;
    linux_protocol::register(&executable, &directory, &icons)
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn register(_app: &tauri::AppHandle) -> Result<(), ErrorCode> {
    Err(ErrorCode::CallbackRegistrationFailed)
}
