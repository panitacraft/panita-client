//! Public settings IPC and best-effort launcher window restoration.
use super::{DesktopState, account_commands};
use launcher_core::{
    error::ErrorCode,
    paths::AppPaths,
    registry::InstanceRegistry,
    settings::{GlobalSettings, InstanceSettings, LauncherWindow},
    storage::PrivateStore,
};
use serde_json::{Value, json};
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use uuid::Uuid;

pub(super) struct WindowPreferences {
    pub initialized: bool,
    normal: LauncherWindow,
    warning: Option<ErrorCode>,
}

impl Default for WindowPreferences {
    fn default() -> Self {
        Self {
            initialized: false,
            normal: LauncherWindow {
                width: 1100,
                height: 700,
                maximized: false,
            },
            warning: None,
        }
    }
}

fn store(app: &tauri::AppHandle) -> Result<(AppPaths, PrivateStore), ErrorCode> {
    let paths = AppPaths::new(
        app.path()
            .app_data_dir()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    )
    .map_err(|error| error.code())?;
    let store = PrivateStore::open(&paths).map_err(|error| error.code())?;
    Ok((paths, store))
}

fn snapshot(app: &tauri::AppHandle, instance_id: Option<Uuid>) -> Result<Value, ErrorCode> {
    let (paths, store) = store(app)?;
    let global = GlobalSettings::load(&store).map_err(|error| error.code())?;
    let instance = instance_id
        .map(|id| {
            let registry = InstanceRegistry::new(paths.clone());
            if id.is_nil() || registry.get(id).map_err(|error| error.code())?.is_none() {
                return Err(ErrorCode::InstanceMissing);
            }
            registry.load_settings(id).map_err(|error| error.code())
        })
        .transpose()?;
    let warning = app
        .state::<DesktopState>()
        .window
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .warning;
    Ok(
        json!({ "global": global, "instance": instance, "instanceId": instance_id, "windowWarning": warning, "trayAvailable": super::tray::available() }),
    )
}

#[tauri::command]
pub(super) fn save_tray_preferences(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    close_to_tray: bool,
    hide_on_game_start: bool,
    reopen_on_game_exit: bool,
) -> Result<(), ErrorCode> {
    account_commands::controller(&state)?;
    let (_, store) = store(&app)?;
    let saved = GlobalSettings::save_tray_preferences(
        &store,
        close_to_tray,
        hide_on_game_start,
        reopen_on_game_exit,
    )
    .map(|_| ())
    .map_err(|error| error.code());
    state.logs.action("tray preferences saved", &saved);
    if saved.is_ok() {
        state.logs.note(
            launcher_core::session_log::Level::Info,
            &format!(
                "tray preferences changed close_to_tray={close_to_tray}; hide_on_game_start={hide_on_game_start}; reopen_on_game_exit={reopen_on_game_exit}"
            ),
        );
    }
    saved
}

#[tauri::command]
pub(super) fn settings_snapshot(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Option<Uuid>,
) -> Result<Value, ErrorCode> {
    account_commands::controller(&state)?;
    snapshot(&app, instance_id)
}

#[tauri::command]
pub(super) fn save_global_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    settings: GlobalSettings,
) -> Result<(), ErrorCode> {
    account_commands::controller(&state)?;
    let (_, store) = store(&app)?;
    let previous = GlobalSettings::load(&store).ok();
    let saved = settings
        .save_launch_settings(&store)
        .map_err(|error| error.code());
    state.logs.action("global settings saved", &saved);
    saved?;
    if let Some(previous) = previous {
        let changes = launcher_core::session_log::changes(&previous, &settings, &["launcher"]);
        if !changes.is_empty() {
            state.logs.note(
                launcher_core::session_log::Level::Info,
                &format!("global settings changed {}", changes.join("; ")),
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub(super) fn save_instance_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    settings: InstanceSettings,
) -> Result<(), ErrorCode> {
    account_commands::controller(&state)?;
    let (paths, store) = store(&app)?;
    let global = GlobalSettings::load(&store).map_err(|error| error.code())?;
    settings
        .validate_launch(&global)
        .map_err(|error| error.code())?;
    let registry = InstanceRegistry::new(paths);
    let previous = registry.load_settings(instance_id).ok();
    let saved = registry
        .save_settings(instance_id, &settings)
        .map_err(|error| error.code());
    state
        .logs
        .action(&format!("instance {instance_id} settings saved"), &saved);
    if saved.is_ok()
        && let Some(previous) = previous
    {
        let changes = launcher_core::session_log::changes(&previous, &settings, &[]);
        if !changes.is_empty() {
            state.logs.note(
                launcher_core::session_log::Level::Info,
                &format!(
                    "instance {instance_id} settings changed {}",
                    changes.join("; ")
                ),
            );
        }
    }
    saved
}

#[tauri::command]
pub(super) fn select_instance(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
) -> Result<(), ErrorCode> {
    account_commands::controller(&state)?;
    let (_, store) = store(&app)?;
    GlobalSettings::select_instance(&store, Some(instance_id)).map_err(|error| error.code())?;
    Ok(())
}

pub(super) fn restore_window(app: &tauri::AppHandle, settings: &GlobalSettings) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<DesktopState>();
    let Ok(mut memory) = state.window.lock() else {
        return;
    };
    if memory.initialized {
        return;
    }
    if let Some(saved) = &settings.launcher.window {
        memory.normal = saved.clone();
    }
    // Window calls can dispatch resize events. Never hold their memory lock
    // while asking the platform to restore the window.
    drop(memory);
    let warning = if let Some(saved) = &settings.launcher.window {
        let result = window
            .set_size(tauri::LogicalSize::new(saved.width, saved.height.max(700)))
            .and_then(|()| {
                if saved.maximized {
                    window.maximize()
                } else {
                    Ok(())
                }
            });
        result.err().map(|_| ErrorCode::StorageUnavailable)
    } else {
        None
    };
    if let Ok(mut memory) = state.window.lock() {
        memory.warning = warning;
        memory.initialized = true;
    }
}

fn capture(
    window: &tauri::Window,
    memory: &Mutex<WindowPreferences>,
) -> Result<Option<LauncherWindow>, ErrorCode> {
    if !memory
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .initialized
    {
        return Ok(None);
    }
    if window
        .is_minimized()
        .map_err(|_| ErrorCode::StorageUnavailable)?
    {
        return Ok(None);
    }
    let maximized = window
        .is_maximized()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let normal = if !maximized {
        let scale = window
            .scale_factor()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let size = window
            .inner_size()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .to_logical::<u32>(scale);
        Some(LauncherWindow {
            width: size.width,
            height: size.height,
            maximized: false,
        })
    } else {
        None
    };
    memory
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .observe(normal, maximized)
}

impl WindowPreferences {
    fn observe(
        &mut self,
        normal: Option<LauncherWindow>,
        maximized: bool,
    ) -> Result<Option<LauncherWindow>, ErrorCode> {
        if !self.initialized {
            return Ok(None);
        }
        if let Some(normal) = normal {
            normal.validate().map_err(|error| error.code())?;
            self.normal = normal;
        }
        self.normal.maximized = maximized;
        Ok(Some(self.normal.clone()))
    }
}

pub(super) fn remember_window_size(window: &tauri::Window) {
    let _ = capture(window, &window.state::<DesktopState>().window);
}

/// Save on normal close/hide; resize events only retain normal dimensions in memory.
/// Persisting minimized/hidden state would make the next launch appear unavailable.
pub(super) fn persist_window(window: &tauri::Window) {
    let app = window.app_handle();
    let state = app.state::<DesktopState>();
    let result = (|| {
        let saved = capture(window, &state.window)?.or_else(|| {
            state
                .window
                .lock()
                .ok()
                .filter(|memory| memory.initialized)
                .map(|memory| memory.normal.clone())
        });
        if let Some(saved) = saved {
            let (_, store) = store(app)?;
            GlobalSettings::save_launcher_window(&store, saved).map_err(|error| error.code())?;
        }
        Ok::<_, ErrorCode>(())
    })();
    if let Err(code) = result {
        if let Ok(mut memory) = state.window.lock() {
            memory.warning = Some(code);
        }
        state.logs.record(launcher_core::logging::LogRecord {
            process: state.process,
            operation: None,
            event: launcher_core::logging::LogEvent::StorageFailure(code),
        });
        let _ = app.emit("settings-warning", code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximized_observations_keep_normal_dimensions_and_uninitialized_events_are_ignored() {
        let mut memory = WindowPreferences::default();
        let normal = LauncherWindow {
            width: 960,
            height: 600,
            maximized: false,
        };
        assert_eq!(memory.observe(Some(normal.clone()), false).ok(), Some(None));
        memory.initialized = true;
        assert_eq!(
            memory.observe(Some(normal.clone()), false).ok(),
            Some(Some(normal.clone()))
        );
        assert_eq!(
            memory.observe(None, true).ok(),
            Some(Some(LauncherWindow {
                maximized: true,
                ..normal.clone()
            }))
        );
        assert_eq!(
            memory.observe(Some(normal.clone()), false).ok(),
            Some(Some(normal))
        );
        let before = memory.normal.clone();
        assert!(
            memory
                .observe(
                    Some(LauncherWindow {
                        width: 0,
                        height: 0,
                        maximized: false
                    }),
                    false
                )
                .is_err()
        );
        assert_eq!(memory.normal, before);
    }
}
