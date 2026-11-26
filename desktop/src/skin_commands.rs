//! Desktop commands for the skin and cape manager. The frontend names a saved look by
//! its identifier and a cape by the service's identifier; it never supplies a path, and
//! the access token that talks to Minecraft Services never leaves the core. Every command
//! refuses an account that is not a Microsoft account, so hiding the entry point in the
//! interface is a convenience and not the boundary.
use launcher_core::{
    error::ErrorCode,
    skins::{self, ApplyRequest, Saved, SkinManager, SkinModel, SkinsView, TextureKind},
};
use tauri::{Emitter, Manager};
use uuid::Uuid;

use super::{
    DesktopState,
    account_commands::controller,
    pack_commands::{blocking, main_only, paths},
};

fn manager(app: &tauri::AppHandle) -> Result<SkinManager, ErrorCode> {
    Ok(SkinManager::new(paths(app)?))
}

/// Keep the library in step with the account after its session was acquired, renewed or
/// selected. Best effort and silent: the Skins page also synchronises when it opens.
pub(super) fn refresh_in_background(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<DesktopState>();
        let Ok(accounts) = controller(&state) else {
            return;
        };
        // Offline accounts have nothing to synchronise.
        if accounts.selected_microsoft_profile().is_err() {
            return;
        }
        let Ok(manager) = manager(&app) else {
            return;
        };
        let _gate = state.skins.lock().await;
        let result = manager.sync(&accounts).await.map_err(|error| error.code());
        state
            .logs
            .action("skins synchronised in background", &result);
        if let Ok(view) = result {
            let _ = app.emit("skins-changed", view);
        }
    });
}

/// What the library holds for the selected account, without any network use.
#[tauri::command]
pub(super) async fn skins_snapshot(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<SkinsView, ErrorCode> {
    main_only(&window)?;
    let profile = controller(&state)?
        .selected_microsoft_profile()
        .map_err(|error| error.code())?;
    let manager = manager(&app)?;
    blocking(move || manager.view(&profile)).await
}

/// Fetch the account's current skin and capes and keep their images.
#[tauri::command]
pub(super) async fn skins_sync(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<SkinsView, ErrorCode> {
    main_only(&window)?;
    let accounts = controller(&state)?;
    let manager = manager(&app)?;
    let _gate = state.skins.lock().await;
    let result = manager.sync(&accounts).await.map_err(|error| error.code());
    state.logs.action("skins synchronised", &result);
    result
}

/// Put a saved look and/or a cape on the account.
#[tauri::command]
pub(super) async fn skins_apply(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    request: ApplyRequest,
) -> Result<SkinsView, ErrorCode> {
    main_only(&window)?;
    let accounts = controller(&state)?;
    let manager = manager(&app)?;
    let _gate = state.skins.lock().await;
    let result = manager
        .apply(&accounts, request)
        .await
        .map_err(|error| error.code());
    state.logs.action("skin or cape applied", &result);
    result
}

/// Add a skin file, sent as base64, to the library as a new look.
#[tauri::command]
pub(super) async fn skins_add_look(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    name: String,
    model: SkinModel,
    cape: Option<String>,
    png: String,
) -> Result<Saved, ErrorCode> {
    main_only(&window)?;
    let profile = controller(&state)?
        .selected_microsoft_profile()
        .map_err(|error| error.code())?;
    let manager = manager(&app)?;
    let result = blocking(move || {
        let bytes = skins::decode_upload(&png)?;
        manager.add_look(&profile, &name, model, cape, &bytes)
    })
    .await;
    state.logs.action("skin added to the library", &result);
    result
}

/// Save the look the account wears now.
#[tauri::command]
pub(super) async fn skins_save_current(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    name: String,
) -> Result<Saved, ErrorCode> {
    main_only(&window)?;
    let profile = controller(&state)?
        .selected_microsoft_profile()
        .map_err(|error| error.code())?;
    let manager = manager(&app)?;
    let result = blocking(move || manager.save_current(&profile, &name)).await;
    state.logs.action("current look saved", &result);
    result
}

/// Rename a saved look, change its model or cape, or replace its skin file.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(super) async fn skins_edit_look(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    id: Uuid,
    name: String,
    model: SkinModel,
    cape: Option<String>,
    png: Option<String>,
) -> Result<SkinsView, ErrorCode> {
    main_only(&window)?;
    let profile = controller(&state)?
        .selected_microsoft_profile()
        .map_err(|error| error.code())?;
    let manager = manager(&app)?;
    let result = blocking(move || {
        let bytes = png.as_deref().map(skins::decode_upload).transpose()?;
        manager.edit_look(&profile, id, &name, model, cape, bytes.as_deref())
    })
    .await;
    state.logs.action("saved look edited", &result);
    result
}

/// Remove a saved look. The account's own skin is not touched.
#[tauri::command]
pub(super) async fn skins_delete_look(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    id: Uuid,
) -> Result<SkinsView, ErrorCode> {
    main_only(&window)?;
    let profile = controller(&state)?
        .selected_microsoft_profile()
        .map_err(|error| error.code())?;
    let manager = manager(&app)?;
    let result = blocking(move || manager.delete_look(&profile, id)).await;
    state.logs.action("saved look deleted", &result);
    result
}

/// The PNG of a cached skin or cape, as a raw binary response.
#[tauri::command]
pub(super) async fn skin_texture(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    kind: TextureKind,
    key: String,
) -> Result<tauri::ipc::Response, ErrorCode> {
    main_only(&window)?;
    controller(&state)?
        .selected_microsoft_profile()
        .map_err(|error| error.code())?;
    let manager = manager(&app)?;
    let bytes = blocking(move || manager.texture(kind, &key)).await?;
    Ok(tauri::ipc::Response::new(bytes))
}
