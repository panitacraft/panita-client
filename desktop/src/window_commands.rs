//! Window controls are restricted to their calling main window. Close requests
//! retain the shared persistence and game-supervision event handlers.
use launcher_core::error::ErrorCode;

#[tauri::command]
pub(super) fn main_window_control(
    window: tauri::WebviewWindow,
    action: String,
) -> Result<Option<bool>, ErrorCode> {
    if window.label() != "main" {
        return Err(ErrorCode::InvalidData);
    }
    let failed = |_| ErrorCode::ProcessControlFailed;
    match action.as_str() {
        "drag" => {
            window.start_dragging().map_err(failed)?;
            Ok(None)
        }
        "minimize" => {
            window.minimize().map_err(failed)?;
            Ok(None)
        }
        "close" => {
            window.close().map_err(failed)?;
            Ok(None)
        }
        "state" => window.is_maximized().map(Some).map_err(failed),
        "toggle_maximize" => {
            if window.is_maximized().map_err(failed)? {
                window.unmaximize().map_err(failed)?;
            } else {
                window.maximize().map_err(failed)?;
            }
            window.is_maximized().map(Some).map_err(failed)
        }
        _ => Err(ErrorCode::InvalidData),
    }
}
