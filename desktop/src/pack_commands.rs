//! Desktop commands for modpack installations: the manual-file list, opening its
//! source and folder, picking a local file, and the disposable content cache. The
//! frontend only names an entry by its manifest path; every filesystem path is
//! derived here or in the core, and file contents are verified in the core.
use launcher_core::{
    error::ErrorCode,
    minecraft::download::Cancellation,
    modpack::{
        install::{import_pending, pending_files, refresh_pending},
        manual::{PendingFile, PendingReason},
        store,
    },
    paths::AppPaths,
};
use serde_json::{Value, json};
use tauri::Manager;
use uuid::Uuid;

use super::DesktopState;

fn paths(app: &tauri::AppHandle) -> Result<AppPaths, ErrorCode> {
    AppPaths::new(
        app.path()
            .app_data_dir()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    )
    .map_err(|error| error.code())
}

fn main_only(window: &tauri::WebviewWindow) -> Result<(), ErrorCode> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(ErrorCode::InvalidData)
    }
}

pub(super) fn files_json(files: &[PendingFile]) -> Value {
    Value::Array(
        files
            .iter()
            .map(|file| {
                json!({
                    "path": file.path, "name": file.name, "fileName": file.file_name(),
                    "folder": file.folder(), "projectId": file.project_id, "fileId": file.file_id,
                    "size": file.size,
                    "reason": if file.reason == PendingReason::Withdrawn { "withdrawn" } else { "manual" },
                })
            })
            .collect(),
    )
}

fn list(remaining: &[PendingFile]) -> Value {
    json!({"files": files_json(remaining), "complete": remaining.is_empty()})
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, launcher_core::error::CoreError> + Send + 'static,
) -> Result<T, ErrorCode> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .map_err(|error| error.code())
}

fn find(files: Vec<PendingFile>, path: &str) -> Result<PendingFile, ErrorCode> {
    files
        .into_iter()
        .find(|file| file.path == path)
        .ok_or(ErrorCode::InvalidData)
}

#[tauri::command]
pub(super) async fn pack_files(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
) -> Result<Value, ErrorCode> {
    main_only(&window)?;
    let paths = paths(&app)?;
    let files = blocking(move || pending_files(&paths, instance_id)).await?;
    Ok(list(&files))
}

/// Re-check the whole list against the player's folder. Local: no session, no network.
#[tauri::command]
pub(super) async fn verify_pack_files(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
) -> Result<Value, ErrorCode> {
    main_only(&window)?;
    let paths = paths(&app)?;
    let remaining =
        blocking(move || refresh_pending(&paths, instance_id, &Cancellation::default())).await?;
    Ok(list(&remaining))
}

#[tauri::command]
pub(super) async fn open_pack_source(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
    path: String,
) -> Result<bool, ErrorCode> {
    use tauri_plugin_opener::OpenerExt;
    main_only(&window)?;
    let paths = paths(&app)?;
    let file = find(
        blocking(move || pending_files(&paths, instance_id)).await?,
        &path,
    )?;
    // The address is built from numeric identifiers, never taken from the frontend.
    let url = file.source_url();
    tauri::async_runtime::spawn_blocking(move || {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|_| ErrorCode::StorageUnavailable)
    })
    .await
    .map_err(|_| ErrorCode::StorageUnavailable)??;
    Ok(true)
}

#[tauri::command]
pub(super) async fn open_pack_folder(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
    path: String,
) -> Result<bool, ErrorCode> {
    use tauri_plugin_opener::OpenerExt;
    main_only(&window)?;
    let paths = paths(&app)?;
    let directory = blocking(move || {
        let file = pending_files(&paths, instance_id)?
            .into_iter()
            .find(|file| file.path == path)
            .ok_or_else(|| launcher_core::error::CoreError::new(ErrorCode::InvalidData))?;
        let directory = paths.game_directory(instance_id)?.join(file.folder());
        std::fs::create_dir_all(&directory)
            .map_err(|_| launcher_core::error::CoreError::new(ErrorCode::StorageUnavailable))?;
        Ok(directory)
    })
    .await?;
    // The shell may keep waiting after the folder appears; do not block the dialog on it.
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
        Err(_) => Ok(true),
    }
}

/// Ask the player for a file with the native dialog, then verify and place it. The
/// dialog result never crosses IPC; a wrong file is rejected by size and SHA-1.
#[tauri::command]
pub(super) async fn import_pack_file(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
    path: String,
) -> Result<Value, ErrorCode> {
    use tauri_plugin_dialog::DialogExt;
    main_only(&window)?;
    let paths = paths(&app)?;
    let known = {
        let paths = paths.clone();
        find(
            blocking(move || pending_files(&paths, instance_id)).await?,
            &path,
        )?
    };
    let title = format!("Selecciona {}", known.file_name());
    let picked = {
        let (app, window) = (app.clone(), window.clone());
        tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_parent(&window)
                .set_title(title)
                .blocking_pick_file()
        })
        .await
        .map_err(|_| ErrorCode::StorageUnavailable)?
    };
    let Some(picked) = picked else {
        return Ok(json!({"cancelled": true}));
    };
    let selected = picked.into_path().map_err(|_| ErrorCode::InvalidData)?;
    let remaining = import_pending(
        &paths,
        instance_id,
        &known.path,
        &selected,
        &Cancellation::default(),
    )
    .await
    .map_err(|error| error.code())?;
    let mut value = list(&remaining);
    value["cancelled"] = json!(false);
    Ok(value)
}

#[tauri::command]
pub(super) async fn content_cache_size(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<u64, ErrorCode> {
    main_only(&window)?;
    let paths = paths(&app)?;
    blocking(move || store::size(&paths, &Cancellation::default())).await
}

/// Never while a pack is being installed: its verified downloads are in this cache.
#[tauri::command]
pub(super) async fn clear_content_cache(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<store::ClearReport, ErrorCode> {
    main_only(&window)?;
    if state.plays.installing() {
        return Err(ErrorCode::OperationBusy);
    }
    let paths = paths(&app)?;
    blocking(move || store::clear(&paths, &Cancellation::default())).await
}
