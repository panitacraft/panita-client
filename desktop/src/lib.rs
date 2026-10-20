use launcher_core::sessions::AccountController;
use std::sync::{Arc, Mutex};
use tauri::Manager;
mod account_commands;
#[cfg(any(target_os = "linux", test))]
mod linux_activation;
mod play_commands;
mod protocol;
mod settings_commands;

struct DesktopState {
    process: launcher_core::operation::ProcessSessionId,
    logs: launcher_core::logging::BoundedLog,
    accounts: Mutex<Option<Arc<AccountController>>>,
    orphaned_callback: bool,
    catalog_fetch: tokio::sync::Mutex<()>,
    plays: Arc<play_commands::PlayController>,
    window: Mutex<settings_commands::WindowPreferences>,
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
        let settings = GlobalSettings::load(&store).map_err(|error| error.code())?;
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
        settings_commands::restore_window(&app, &settings);
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
async fn catalog_snapshot(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<launcher_core::registry::CatalogOverview, launcher_core::error::ErrorCode> {
    // Serialize catalog refreshes without holding the account-state lock over network work.
    let _fetch = state.catalog_fetch.lock().await;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| launcher_core::error::ErrorCode::StorageUnavailable)?;
    let paths = launcher_core::paths::AppPaths::new(root).map_err(|error| error.code())?;
    launcher_core::registry::load_catalog(paths)
        .await
        .map_err(|error| error.code())
}

pub fn run() {
    let context = tauri::generate_context!();
    #[cfg(target_os = "linux")]
    if linux_activation::forward_from_environment(&context.config().identifier) {
        return;
    }
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
            catalog_fetch: tokio::sync::Mutex::new(()),
            plays: Arc::new(play_commands::PlayController::new(process)),
            window: Mutex::new(settings_commands::WindowPreferences::default()),
            orphaned_callback: std::env::args_os()
                .skip(1)
                .any(|arg| arg.to_string_lossy().starts_with("panitacraft:")),
        })
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            account_commands::handle_callback(app, args);
            play_commands::restore_window(app, cwd);
        }))
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            if window.label() == "main" {
                match event {
                    tauri::WindowEvent::Resized(_) => {
                        settings_commands::remember_window_size(window)
                    }
                    tauri::WindowEvent::CloseRequested { .. } => {
                        settings_commands::persist_window(window)
                    }
                    _ => (),
                }
            }
            if window.label() == "main"
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
                && play_commands::close_window(window)
            {
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            application_name,
            initialize,
            catalog_snapshot,
            settings_commands::settings_snapshot,
            settings_commands::save_global_settings,
            settings_commands::save_instance_settings,
            settings_commands::select_instance,
            account_commands::account_snapshot,
            account_commands::add_offline_account,
            account_commands::start_microsoft_login,
            account_commands::cancel_microsoft_login,
            account_commands::select_account,
            account_commands::renew_account,
            account_commands::retry_account_save,
            account_commands::remove_account,
            account_commands::validate_account,
            play_commands::play_snapshot,
            play_commands::begin_install,
            play_commands::begin_update,
            play_commands::begin_play,
            play_commands::confirm_play,
            play_commands::cancel_play,
            play_commands::stop_game
        ])
        .build(context);
    match result {
        Ok(app) => app.run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event
                && play_commands::prevent_close(app)
            {
                api.prevent_exit();
            }
        }),
        Err(_) => {
            eprintln!("Desktop startup failed");
            std::process::exit(1);
        }
    }
}
