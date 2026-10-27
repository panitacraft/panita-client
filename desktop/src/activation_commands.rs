use super::DesktopState;
use super::play_commands::PlayController;
use launcher_core::activation::{ActivationController, ActivationView, RefreshOutcome};
use launcher_core::error::ErrorCode;
use launcher_core::panita_api::{DeviceInfo, PanitaCredentials};
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

/// A session the API closed ends the activation: launches that have not started
/// are cancelled and the interface learns about it. A running game is a separate
/// process and is never touched.
fn session_ended(
    app: &tauri::AppHandle,
    plays: &PlayController,
    activation: &ActivationController,
) {
    plays.cancel_pending();
    plays.publish(app);
    emit_activation(app, activation);
}

/// Inspect the result of a call made with `credentials`.
pub(super) fn observe(
    app: &tauri::AppHandle,
    plays: &PlayController,
    activation: &ActivationController,
    credentials: &PanitaCredentials,
    code: ErrorCode,
) {
    if code == ErrorCode::SessionRejected && activation.note_rejection(credentials).unwrap_or(false)
    {
        session_ended(app, plays, activation);
    }
}

/// Publish the result of a session check made by `refresh`.
pub(super) fn after_refresh(
    app: &tauri::AppHandle,
    plays: &PlayController,
    activation: &ActivationController,
    outcome: RefreshOutcome,
) {
    if outcome == RefreshOutcome::Rejected {
        session_ended(app, plays, activation);
    } else {
        emit_activation(app, activation);
    }
}

/// Verify a saved session in the background at startup. Offline, nothing changes.
pub(super) fn start_check(
    app: tauri::AppHandle,
    activation: Arc<ActivationController>,
    plays: Arc<PlayController>,
) {
    tauri::async_runtime::spawn(async move {
        if let Ok(outcome) = activation.refresh().await {
            after_refresh(&app, &plays, &activation, outcome);
        }
    });
}

#[tauri::command]
pub(super) fn activation_snapshot(
    state: tauri::State<'_, DesktopState>,
) -> Result<ActivationView, ErrorCode> {
    controller(&state)?.view().map_err(|error| error.code())
}

/// Ask for a serial, then keep polling in the background until the member confirms
/// it in Discord, it expires or the login is cancelled. Progress arrives as
/// `activation-changed` events.
#[tauri::command]
pub(super) async fn start_activation(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    identifier: String,
) -> Result<(), ErrorCode> {
    let activation = controller(&state)?;
    let begun = activation.begin(&identifier, &DeviceInfo::detect()).await;
    emit_activation(&app, &activation);
    begun.map_err(|error| error.code())?;
    tauri::async_runtime::spawn(async move {
        let _ = activation.await_confirmation().await;
        emit_activation(&app, &activation);
    });
    Ok(())
}

#[tauri::command]
pub(super) fn cancel_activation(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), ErrorCode> {
    let activation = controller(&state)?;
    activation.cancel().map_err(|error| error.code())?;
    emit_activation(&app, &activation);
    Ok(())
}

#[tauri::command]
pub(super) fn retry_activation_save(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), ErrorCode> {
    let activation = controller(&state)?;
    let saved = activation.retry_save();
    emit_activation(&app, &activation);
    saved.map_err(|error| error.code())
}

/// Check the session now. The interface calls this for its explicit retry and when
/// the window regains focus while the session is unverified.
#[tauri::command]
pub(super) async fn refresh_activation(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), ErrorCode> {
    let activation = controller(&state)?;
    let outcome = activation.refresh().await.map_err(|error| error.code())?;
    after_refresh(&app, &state.plays, &activation, outcome);
    Ok(())
}

/// Resolves to `{ "serverClosed": bool }`. False means the API could not be reached:
/// the session then stays open on the server until the member closes it with the bot.
#[tauri::command]
pub(super) async fn sign_out_activation(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<serde_json::Value, ErrorCode> {
    let activation = controller(&state)?;
    // Launches that have not started must not outlive the session they would use.
    state.plays.cancel_pending();
    state.plays.publish(&app);
    let outcome = activation.sign_out().await;
    emit_activation(&app, &activation);
    let outcome = outcome.map_err(|error| error.code())?;
    Ok(serde_json::json!({ "serverClosed": outcome.server_closed }))
}
