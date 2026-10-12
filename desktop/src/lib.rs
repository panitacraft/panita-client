use tauri::Manager;

struct DesktopState {
    process: launcher_core::operation::ProcessSessionId,
    logs: launcher_core::logging::BoundedLog,
}

#[tauri::command]
fn initialize(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), launcher_core::error::ErrorCode> {
    use launcher_core::paths::AppPaths;
    use launcher_core::settings::GlobalSettings;
    use launcher_core::storage::PrivateStore;

    let result = (|| {
        let root = app
            .path()
            .app_data_dir()
            .map_err(|_| launcher_core::error::ErrorCode::StorageUnavailable)?;
        let paths = AppPaths::new(root).map_err(|error| error.code())?;
        let store = PrivateStore::open(&paths).map_err(|error| error.code())?;
        GlobalSettings::load(&store).map_err(|error| error.code())?;
        Ok(())
    })();
    if let Err(code) = result {
        state.logs.record(launcher_core::logging::LogRecord {
            process: state.process,
            operation: None,
            event: launcher_core::logging::LogEvent::StorageFailure(code),
        });
    }
    result
}

#[tauri::command]
fn application_name() -> &'static str {
    launcher_core::application_name()
}

#[tauri::command]
fn catalog_snapshot() -> Vec<launcher_core::catalog::CatalogInstance> {
    launcher_core::catalog::catalog_snapshot()
}

pub fn run() {
    let process = launcher_core::operation::ProcessSessionId::new();
    let logs = launcher_core::logging::BoundedLog::default();
    logs.record(launcher_core::logging::LogRecord {
        process,
        operation: None,
        event: launcher_core::logging::LogEvent::ApplicationStarted,
    });
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .manage(DesktopState { process, logs })
        .invoke_handler(tauri::generate_handler![
            application_name,
            initialize,
            catalog_snapshot
        ])
        .run(tauri::generate_context!());
    if result.is_err() {
        eprintln!("Desktop startup failed");
        std::process::exit(1);
    }
}
