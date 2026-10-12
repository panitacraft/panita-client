use super::{DesktopState, protocol};
use launcher_core::accounts::RemovalOutcome;
use launcher_core::error::ErrorCode;
use launcher_core::sessions::{AccountController, AccountsView};
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

fn controller(state: &DesktopState) -> Result<Arc<AccountController>, ErrorCode> {
    state
        .accounts
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .clone()
        .ok_or(ErrorCode::StorageUnavailable)
}

fn emit_accounts(app: &tauri::AppHandle, accounts: &AccountController) {
    if let Ok(view) = accounts.view() {
        let _ = app.emit("accounts-changed", view);
    }
}

pub(super) fn start_maintenance(app: tauri::AppHandle, accounts: Arc<AccountController>) {
    tauri::async_runtime::spawn(async move {
        let _ = accounts.renew_selected().await;
        emit_accounts(&app, &accounts);
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut ticks = 0;
        loop {
            timer.tick().await;
            if accounts.expire_login().unwrap_or(false) {
                emit_accounts(&app, &accounts);
            }
            ticks += 1;
            if ticks >= 30 {
                ticks = 0;
                if accounts.view().is_ok_and(|view| !view.login_pending) {
                    let _ = accounts.maintain_selected().await;
                    emit_accounts(&app, &accounts);
                }
            }
        }
    });
}

#[tauri::command]
pub(super) fn account_snapshot(
    state: tauri::State<'_, DesktopState>,
) -> Result<AccountsView, ErrorCode> {
    controller(&state)?.view().map_err(|e| e.code())
}

#[tauri::command]
pub(super) fn add_offline_account(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    username: String,
) -> Result<(), ErrorCode> {
    let accounts = controller(&state)?;
    accounts.add_offline(&username).map_err(|e| e.code())?;
    emit_accounts(&app, &accounts);
    Ok(())
}

#[tauri::command]
pub(super) fn start_microsoft_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    account_id: Option<String>,
) -> Result<String, ErrorCode> {
    let intended = account_id
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| ErrorCode::InvalidData)?;
    let accounts = controller(&state)?;
    protocol::register(&app)?;
    let launch = accounts.begin_login(intended).map_err(|e| e.code())?;
    if app
        .opener()
        .open_url(launch.browser_url, None::<&str>)
        .is_err()
    {
        let _ = accounts.cancel_login();
        return Err(ErrorCode::BrowserUnavailable);
    }
    emit_accounts(&app, &accounts);
    Ok(launch.operation_id.to_string())
}

#[tauri::command]
pub(super) fn cancel_microsoft_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), ErrorCode> {
    let accounts = controller(&state)?;
    accounts.cancel_login().map_err(|e| e.code())?;
    emit_accounts(&app, &accounts);
    Ok(())
}

#[tauri::command]
pub(super) async fn select_account(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    account_id: String,
) -> Result<(), ErrorCode> {
    let id = Uuid::parse_str(&account_id).map_err(|_| ErrorCode::InvalidData)?;
    let accounts = controller(&state)?;
    accounts.select(id).map_err(|e| e.code())?;
    emit_accounts(&app, &accounts);
    let result = accounts.renew_selected().await.map_err(|e| e.code());
    emit_accounts(&app, &accounts);
    result
}

#[tauri::command]
pub(super) async fn renew_account(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), ErrorCode> {
    let accounts = controller(&state)?;
    let result = accounts.retry_selected().await.map_err(|e| e.code());
    emit_accounts(&app, &accounts);
    result
}

#[tauri::command]
pub(super) fn retry_account_save(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    account_id: String,
) -> Result<(), ErrorCode> {
    let accounts = controller(&state)?;
    let id = Uuid::parse_str(&account_id).map_err(|_| ErrorCode::InvalidData)?;
    accounts.retry_save(id).map_err(|e| e.code())?;
    emit_accounts(&app, &accounts);
    Ok(())
}

#[tauri::command]
pub(super) fn remove_account(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    account_id: String,
) -> Result<RemovalOutcome, ErrorCode> {
    let id = Uuid::parse_str(&account_id).map_err(|_| ErrorCode::InvalidData)?;
    let accounts = controller(&state)?;
    let result = accounts.remove(id).map_err(|e| e.code())?;
    emit_accounts(&app, &accounts);
    Ok(result)
}

#[tauri::command]
pub(super) async fn validate_account(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), ErrorCode> {
    let accounts = controller(&state)?;
    let result = accounts
        .validate_play()
        .await
        .map_err(|e| e.code())
        .map(|_| ());
    emit_accounts(&app, &accounts);
    result
}

pub(super) fn handle_callback(app: &tauri::AppHandle, args: Vec<String>) {
    if args.len() != 2 || !args[1].starts_with("panitacraft:") {
        return;
    }
    // Raw arguments are never logged or emitted. No active operation means rejection.
    let Ok(accounts) = controller(&app.state::<DesktopState>()) else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = accounts.callback(&args[1]).await;
        emit_accounts(&app, &accounts);
    });
}
