//! Register only the fixed callback scheme; raw callbacks stay in Rust arguments.
use launcher_core::error::ErrorCode;
#[cfg(target_os = "linux")]
use tauri::Manager;

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
    let executable = executable
        .to_str()
        .ok_or(ErrorCode::CallbackRegistrationFailed)?;
    if executable.contains(['\n', '\r', '\0']) {
        return Err(ErrorCode::CallbackRegistrationFailed);
    }
    let escaped = executable
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    let directory = app
        .path()
        .data_dir()
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)?
        .join("applications");
    std::fs::create_dir_all(&directory).map_err(|_| ErrorCode::CallbackRegistrationFailed)?;
    let name = "com.panitacraft.client.desktop";
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Panita Client\nExec=\"{escaped}\" %u\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/panitacraft;\n"
    );
    std::fs::write(directory.join(name), entry)
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)?;
    let status = std::process::Command::new("xdg-mime")
        .args(["default", name, "x-scheme-handler/panitacraft"])
        .output()
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)?;
    if !status.status.success() {
        return Err(ErrorCode::CallbackRegistrationFailed);
    }
    let query = std::process::Command::new("xdg-mime")
        .args(["query", "default", "x-scheme-handler/panitacraft"])
        .output()
        .map_err(|_| ErrorCode::CallbackRegistrationFailed)?;
    if !query.status.success() || String::from_utf8_lossy(&query.stdout).trim() != name {
        return Err(ErrorCode::CallbackRegistrationFailed);
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn register(_app: &tauri::AppHandle) -> Result<(), ErrorCode> {
    Err(ErrorCode::CallbackRegistrationFailed)
}
