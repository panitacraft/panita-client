use launcher_core::activation::ActivationController;
use launcher_core::sessions::AccountController;
use std::sync::{Arc, Mutex};
use tauri::Manager;
mod account_commands;
mod activation_commands;
mod hardware;
#[cfg(any(target_os = "linux", test))]
mod linux_activation;
#[cfg(any(target_os = "linux", test))]
mod linux_browser;
mod pack_commands;
mod play_commands;
mod protocol;
mod settings_commands;
#[cfg(windows)]
mod tray;
mod update_commands;
mod window_commands;
#[cfg(windows)]
mod windows_identity;
mod windows_maintenance;

struct DesktopState {
    process: launcher_core::operation::ProcessSessionId,
    logs: Arc<launcher_core::logging::BoundedLog>,
    accounts: Mutex<Option<Arc<AccountController>>>,
    activation: Mutex<Option<Arc<ActivationController>>>,
    orphaned_callback: bool,
    catalog_fetch: tokio::sync::Mutex<()>,
    plays: Arc<play_commands::PlayController>,
    launcher_update: Arc<update_commands::UpdateController>,
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

    // The update check does not depend on accounts, so a damaged account store cannot hide it.
    update_commands::start_check(app.clone(), state.launcher_update.clone());
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
        let store = PrivateStore::open_application(&paths).map_err(|error| error.code())?;
        let settings = GlobalSettings::load(&store).map_err(|error| error.code())?;
        // A damaged Panita session file does not fail initialization: it is reported
        // through the activation view and blocks activation instead.
        let activation = Arc::new(
            ActivationController::new(PrivateStore::open(&paths).map_err(|error| error.code())?)
                .map_err(|error| error.code())?
                .with_diagnostics(state.logs.clone(), state.process),
        );
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
        *state
            .activation
            .lock()
            .map_err(|_| launcher_core::error::ErrorCode::StorageUnavailable)? =
            Some(activation.clone());
        *slot = Some(accounts.clone());
        settings_commands::restore_window(&app, &settings);
        activation_commands::start_check(app.clone(), activation, state.plays.clone());
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
fn diagnostic_snapshot(
    state: tauri::State<'_, DesktopState>,
) -> launcher_core::logging::DiagnosticSnapshot {
    // Available even when initialization fails; no storage or authentication work.
    state.logs.view()
}

#[tauri::command]
async fn open_logs_folder(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<bool, launcher_core::error::ErrorCode> {
    use tauri_plugin_opener::OpenerExt;
    if window.label() != "main" {
        return Err(launcher_core::error::ErrorCode::InvalidData);
    }
    // Fixed location under the data root; the frontend supplies no path.
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| launcher_core::error::ErrorCode::StorageUnavailable)?
        .join("logs");
    std::fs::create_dir_all(&directory)
        .map_err(|_| launcher_core::error::ErrorCode::StorageUnavailable)?;
    let opening = tauri::async_runtime::spawn_blocking(move || {
        app.opener()
            .open_path(directory.to_string_lossy(), None::<&str>)
            .map_err(|_| launcher_core::error::ErrorCode::StorageUnavailable)
    });
    // Windows shell integration may keep waiting; report an indeterminate result as opened.
    match tokio::time::timeout(std::time::Duration::from_secs(5), opening).await {
        Ok(result) => {
            result.map_err(|_| launcher_core::error::ErrorCode::StorageUnavailable)??;
            Ok(true)
        }
        Err(_) => Ok(true),
    }
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
    // The catalog belongs to an activated member; restricted entries need the session.
    let activation = activation_commands::controller(&state)?;
    let credentials = activation
        .session_credentials()
        .map_err(|error| error.code())?;
    let result = launcher_core::registry::load_catalog(paths, Some(credentials.clone()))
        .await
        .map_err(|error| error.code());
    if let Err(code) = &result {
        activation_commands::observe(&app, &state.plays, &activation, &credentials, *code);
    }
    let warning = match &result {
        Err(code) => Some(*code),
        Ok(snapshot) => snapshot.cache_warning.or(snapshot.registry_warning),
    };
    if let Some(code) = warning {
        state.logs.record(launcher_core::logging::LogRecord {
            process: state.process,
            operation: None,
            event: launcher_core::logging::LogEvent::CatalogFailure(code),
        });
    }
    result
}

pub fn run() {
    let context = tauri::generate_context!();
    #[cfg(windows)]
    if windows_maintenance::requested(std::env::args_os().skip(1)) {
        std::process::exit(windows_maintenance::remove(&context.config().identifier));
    }
    #[cfg(windows)]
    let startup_gate = match windows_maintenance::StartupGate::enter() {
        Ok(gate) => gate,
        Err(()) => {
            windows_maintenance::show_error(
                "Hay otra operación de inicio o eliminación de datos en curso. Espera a que termine y vuelve a abrir Panita Client.",
            );
            return;
        }
    };
    #[cfg(windows)]
    if !windows_maintenance::startup_allowed() {
        windows_maintenance::show_error(
            "La eliminación de datos quedó incompleta o no se puede comprobar. Cierra Panita Client y vuelve a ejecutar el desinstalador para completarla. Los datos restantes no se abrirán.",
        );
        return;
    }
    #[cfg(windows)]
    windows_maintenance::retire_legacy_web_data();
    #[cfg(windows)]
    if !windows_identity::configure(&context.config().identifier) {
        // Cosmetic shell integration must not prevent account or game recovery.
        eprintln!("Windows application identity setup failed");
    }
    #[cfg(target_os = "linux")]
    if linux_activation::forward_from_environment(&context.config().identifier) {
        return;
    }
    let process = launcher_core::operation::ProcessSessionId::new();
    let logs = Arc::new(launcher_core::logging::BoundedLog::default());
    logs.record(launcher_core::logging::LogRecord {
        process,
        operation: None,
        event: launcher_core::logging::LogEvent::ApplicationStarted,
    });
    let plays = Arc::new(play_commands::PlayController::with_logs(
        process,
        logs.clone(),
    ));
    let result = tauri::Builder::default()
        .manage(DesktopState {
            process,
            logs: logs.clone(),
            accounts: Mutex::new(None),
            activation: Mutex::new(None),
            catalog_fetch: tokio::sync::Mutex::new(()),
            launcher_update: Arc::new(update_commands::UpdateController::new(
                plays.clone(),
                logs,
                process,
            )),
            plays,
            window: Mutex::new(settings_commands::WindowPreferences::default()),
            orphaned_callback: std::env::args_os()
                .skip(1)
                .any(|arg| arg.to_string_lossy().starts_with("panitacraft:")),
        })
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            account_commands::handle_callback(app, args);
            play_commands::restore_window(app, cwd);
        }))
        .setup(|app| {
            #[cfg(windows)]
            tray::install(app.handle());
            // Session log files are best effort: a failure keeps in-memory diagnostics.
            let state = app.state::<DesktopState>();
            if let Ok(root) = app.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&root);
                let mut header = vec![
                    format!("launcher version={}", env!("CARGO_PKG_VERSION")),
                    format!("platform arch={}", std::env::consts::ARCH),
                ];
                header.extend(hardware::lines(&root));
                state
                    .logs
                    .attach_file(&root.join("logs"), state.process, &header);
            }
            let logs = state.logs.clone();
            launcher_core::network_log::install(move |level, text| logs.note(level, text));
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        // Used only from Rust commands; no frontend permission is granted to it.
        .plugin(tauri_plugin_dialog::init())
        // Used only from Rust; the webview is granted no updater permission.
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            window_commands::main_window_control,
            initialize,
            diagnostic_snapshot,
            open_logs_folder,
            catalog_snapshot,
            activation_commands::activation_snapshot,
            activation_commands::start_activation,
            activation_commands::cancel_activation,
            activation_commands::retry_activation_save,
            activation_commands::refresh_activation,
            activation_commands::sign_out_activation,
            activation_commands::open_panita_bot,
            activation_commands::open_terms_of_use,
            update_commands::launcher_update_status,
            update_commands::check_launcher_update,
            update_commands::set_update_channel,
            update_commands::apply_launcher_update,
            update_commands::cancel_launcher_update_apply,
            update_commands::open_launcher_release_page,
            settings_commands::settings_snapshot,
            settings_commands::save_global_settings,
            settings_commands::save_instance_settings,
            settings_commands::save_tray_preferences,
            settings_commands::select_instance,
            account_commands::account_snapshot,
            account_commands::add_offline_account,
            account_commands::start_microsoft_login,
            account_commands::cancel_microsoft_login,
            account_commands::microsoft_login_link,
            account_commands::select_account,
            account_commands::renew_account,
            account_commands::retry_account_save,
            account_commands::remove_account,
            account_commands::validate_account,
            play_commands::play_snapshot,
            play_commands::game_log_snapshot,
            play_commands::find_game_log,
            play_commands::open_game_log,
            play_commands::open_crash_reports,
            play_commands::begin_install,
            play_commands::begin_update,
            play_commands::begin_repair,
            play_commands::begin_uninstall,
            play_commands::clean_native_cache,
            pack_commands::pack_files,
            pack_commands::verify_pack_files,
            pack_commands::open_pack_source,
            pack_commands::open_pack_folder,
            pack_commands::open_curseforge_project,
            pack_commands::open_instance_folder,
            pack_commands::instance_folder_path,
            pack_commands::pack_version_names,
            pack_commands::import_pack_file,
            pack_commands::content_cache_size,
            pack_commands::clear_content_cache,
            play_commands::begin_play,
            play_commands::confirm_play,
            play_commands::cancel_play,
            play_commands::stop_game
        ])
        .build(context);
    #[cfg(windows)]
    drop(startup_gate);
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
