//! Desktop commands for the per-instance screenshot gallery. The frontend names an
//! instance and a bare file name; the core resolves and validates every path through
//! no-follow handles, so nothing here accepts or returns a filesystem path.
use std::sync::Mutex;

use launcher_core::{
    error::ErrorCode,
    minecraft::screenshots::{self, ScreenshotList},
};
use uuid::Uuid;

use super::pack_commands::{blocking, main_only, paths};

/// One handle for the whole process. On Linux the copied image or text is served by this
/// handle, so dropping it would empty the clipboard; keeping it also avoids reconnecting
/// per copy.
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

fn with_clipboard(
    write: impl FnOnce(&mut arboard::Clipboard) -> Result<(), arboard::Error>,
) -> Result<(), ErrorCode> {
    let mut guard = CLIPBOARD
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    if guard.is_none() {
        *guard = Some(arboard::Clipboard::new().map_err(|_| ErrorCode::StorageUnavailable)?);
    }
    let clipboard = guard.as_mut().ok_or(ErrorCode::StorageUnavailable)?;
    write(clipboard).map_err(|_| {
        // A broken connection (for example a restarted display server) is retried afresh.
        *guard = None;
        ErrorCode::StorageUnavailable
    })
}

fn put_image(image: screenshots::Rgba) -> Result<(), ErrorCode> {
    with_clipboard(|clipboard| {
        clipboard.set_image(arboard::ImageData {
            width: image.width as usize,
            height: image.height as usize,
            bytes: image.pixels.into(),
        })
    })
}

/// Put plain text on the system clipboard.
pub(super) fn put_text(text: String) -> Result<(), ErrorCode> {
    with_clipboard(|clipboard| clipboard.set_text(text))
}

#[tauri::command]
pub(super) async fn list_screenshots(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
) -> Result<ScreenshotList, ErrorCode> {
    main_only(&window)?;
    let paths = paths(&app)?;
    blocking(move || screenshots::list(&paths, instance_id)).await
}

/// The validated image bytes, delivered as a raw binary response rather than JSON.
#[tauri::command]
pub(super) async fn screenshot_image(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
    name: String,
) -> Result<tauri::ipc::Response, ErrorCode> {
    main_only(&window)?;
    let paths = paths(&app)?;
    let bytes = blocking(move || screenshots::read(&paths, instance_id, &name)).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Open the screenshot in the system's default image viewer.
#[tauri::command]
pub(super) async fn open_screenshot(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
    name: String,
) -> Result<bool, ErrorCode> {
    main_only(&window)?;
    let paths = paths(&app)?;
    let file = blocking(move || screenshots::locate(&paths, instance_id, &name)).await?;
    super::folder_opener::open(app, file).await
}

/// Show the screenshot selected in the system file manager. When the file manager cannot
/// select it, open the folder that holds it instead.
#[tauri::command]
pub(super) async fn reveal_screenshot(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
    name: String,
) -> Result<bool, ErrorCode> {
    use tauri_plugin_opener::OpenerExt;
    main_only(&window)?;
    let paths = paths(&app)?;
    let file = blocking(move || screenshots::locate(&paths, instance_id, &name)).await?;
    let handle = app.clone();
    let target = file.clone();
    let revealed = tauri::async_runtime::spawn_blocking(move || {
        handle.opener().reveal_item_in_dir(target).is_ok()
    })
    .await
    .map_err(|_| ErrorCode::StorageUnavailable)?;
    if revealed {
        return Ok(true);
    }
    let folder = file
        .parent()
        .ok_or(ErrorCode::StorageUnavailable)?
        .to_owned();
    super::folder_opener::open(app, folder).await
}

/// Copy the screenshot's pixels to the system clipboard.
#[tauri::command]
pub(super) async fn copy_screenshot(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
    name: String,
) -> Result<bool, ErrorCode> {
    main_only(&window)?;
    let paths = paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let image =
            screenshots::read_rgba(&paths, instance_id, &name).map_err(|error| error.code())?;
        put_image(image)
    })
    .await
    .map_err(|_| ErrorCode::StorageUnavailable)??;
    Ok(true)
}

/// Move the screenshot to the system's recycle bin or trash so a mistake can be undone.
/// Nothing is erased permanently here. The file is validated just before it is moved.
#[tauri::command]
pub(super) async fn delete_screenshot(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
    name: String,
) -> Result<bool, ErrorCode> {
    main_only(&window)?;
    let paths = paths(&app)?;
    let file = blocking(move || screenshots::locate(&paths, instance_id, &name)).await?;
    tauri::async_runtime::spawn_blocking(move || {
        trash::delete(&file).map_err(|_| ErrorCode::StorageUnavailable)
    })
    .await
    .map_err(|_| ErrorCode::StorageUnavailable)??;
    Ok(true)
}
