use launcher_core::sessions::AccountController;
use std::sync::{Arc, Mutex};
use tauri::Manager;
mod account_commands;
mod protocol;

struct DesktopState {
    process: launcher_core::operation::ProcessSessionId,
    logs: launcher_core::logging::BoundedLog,
    accounts: Mutex<Option<Arc<AccountController>>>,
    orphaned_callback: bool,
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
        let mut slot = state
            .accounts
            .lock()
            .map_err(|_| launcher_core::error::ErrorCode::StorageUnavailable)?;
        if slot.is_some() {
            return Ok(());
        }
        let root = app
            .path()
            .app_data_dir()
            .map_err(|_| launcher_core::error::ErrorCode::StorageUnavailable)?;
        let paths = AppPaths::new(root).map_err(|error| error.code())?;
        let store = PrivateStore::open(&paths).map_err(|error| error.code())?;
        GlobalSettings::load(&store).map_err(|error| error.code())?;
        let accounts = Arc::new(
            AccountController::new(
                launcher_core::accounts::AccountRepository::new(store),
                state.process,
            )
            .map_err(|e| e.code())?,
        );
        if state.orphaned_callback {
            accounts.report_orphaned_callback().map_err(|e| e.code())?;
        }
        *slot = Some(accounts.clone());
        account_commands::start_maintenance(app.clone(), accounts);
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
        .manage(DesktopState {
            process,
            logs,
            accounts: Mutex::new(None),
            orphaned_callback: std::env::args_os()
                .skip(1)
                .any(|arg| arg.to_string_lossy().starts_with("panitacraft:")),
        })
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            account_commands::handle_callback(app, args);
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "linux")]
                {
                    let main_window = window.clone();
                    let _ = window.run_on_main_thread(move || {
                        use gtk::prelude::GtkWindowExt;
                        if let Ok(native_window) = main_window.gtk_window() {
                            // Tao skips focus while its queued unminimize is pending.
                            // Present the native window without that stale-state guard.
                            native_window.present();
                        }
                    });
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            application_name,
            initialize,
            catalog_snapshot,
            account_commands::account_snapshot,
            account_commands::add_offline_account,
            account_commands::start_microsoft_login,
            account_commands::cancel_microsoft_login,
            account_commands::select_account,
            account_commands::renew_account,
            account_commands::retry_account_save,
            account_commands::remove_account,
            account_commands::validate_account
        ])
        .run(tauri::generate_context!());
    if result.is_err() {
        eprintln!("Desktop startup failed");
        std::process::exit(1);
    }
}
