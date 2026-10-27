use super::DesktopState;
use super::play_commands::PlayController;
use launcher_core::activation::ActivationController;
use launcher_core::error::ErrorCode;
use launcher_core::panita_api::PanitaCredentials;
use std::sync::Arc;
use tauri::Emitter;

pub(super) fn controller(state: &DesktopState) -> Result<Arc<ActivationController>, ErrorCode> {
    state
        .activation
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .clone()
        .ok_or(ErrorCode::StorageUnavailable)
}

pub(super) fn emit_activation(app: &tauri::AppHandle, activation: &ActivationController) {
    if let Ok(view) = activation.view() {
        let _ = app.emit("activation-changed", view);
    }
}

/// Inspect the result of a call made with `credentials`. The API closing the
/// session ends the activation: launches that have not started are cancelled and
/// the interface learns about it. A running game is a separate process and is
/// never touched.
pub(super) fn observe(
    app: &tauri::AppHandle,
    plays: &PlayController,
    activation: &ActivationController,
    credentials: &PanitaCredentials,
    code: ErrorCode,
) {
    if code == ErrorCode::SessionRejected && activation.note_rejection(credentials).unwrap_or(false)
    {
        plays.cancel_pending();
        plays.publish(app);
        emit_activation(app, activation);
    }
}
