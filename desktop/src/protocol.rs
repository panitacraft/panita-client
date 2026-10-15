//! Register only the fixed callback scheme; raw callbacks stay in Rust arguments.
use launcher_core::error::ErrorCode;
#[cfg(target_os = "linux")]
use tauri::Manager;
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
pub fn register(app: &tauri::AppHandle) -> Result<(), ErrorCode> {
    let executable = app
        .env()
        .appimage
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_exe().map_err(|_| ErrorCode::CallbackRegistrationFailed)?);
    let directory = app
        .path()
        .data_dir()
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)?
        .join("applications");
    linux_protocol::register(&executable, &directory)
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn register(_app: &tauri::AppHandle) -> Result<(), ErrorCode> {
    Err(ErrorCode::CallbackRegistrationFailed)
}
