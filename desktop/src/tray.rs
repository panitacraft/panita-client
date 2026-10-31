//! Windows notification-area icon: reopen or quit the launcher while its window is hidden.
//! The behavior is opt-in through three global settings; without the icon, none applies.
use super::{DesktopState, play_commands};
use launcher_core::{paths::AppPaths, settings::GlobalSettings, storage::PrivateStore};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

static INSTALLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TrayMode {
    pub close_to_tray: bool,
    pub hide_on_game_start: bool,
    pub reopen_on_game_exit: bool,
}

/// Whether the notification-area icon exists, so its settings can apply.
pub(super) fn available() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// The stored preferences, or all off when the icon is absent or the settings cannot be read.
pub(super) fn mode(app: &tauri::AppHandle) -> TrayMode {
    if !INSTALLED.load(Ordering::Acquire) {
        return TrayMode::default();
    }
    let loaded = app
        .path()
        .app_data_dir()
        .ok()
        .and_then(|root| AppPaths::new(root).ok())
        .and_then(|paths| PrivateStore::open(&paths).ok())
        .and_then(|store| GlobalSettings::load(&store).ok());
    loaded.map_or_else(TrayMode::default, |settings| TrayMode {
        close_to_tray: settings.launcher.close_to_tray,
        hide_on_game_start: settings.launcher.hide_on_game_start,
        reopen_on_game_exit: settings.launcher.reopen_on_game_exit,
    })
}

/// Hide the window once a game has started, when the player chose that.
pub(super) fn hide_after_game_start(app: &tauri::AppHandle) {
    let tray = mode(app);
    if !tray.hide_on_game_start {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("main") {
            handle
                .state::<DesktopState>()
                .plays
                .hide_for_game(|| window.hide().is_ok(), tray);
        }
    });
}

/// Create the icon. On Linux the indicator library is loaded at runtime and panics when it is
/// missing, so a failure there must leave the launcher running without the tray feature.
#[cfg(any(windows, target_os = "linux"))]
pub(super) fn install(app: &tauri::AppHandle) {
    #[cfg(target_os = "linux")]
    {
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(app)));
        if attempt.unwrap_or(false) {
            INSTALLED.store(true, Ordering::Release);
        }
    }
    #[cfg(windows)]
    if build(app) {
        INSTALLED.store(true, Ordering::Release);
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn build(app: &tauri::AppHandle) -> bool {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    let Some(icon) = app.default_window_icon().cloned() else {
        return false;
    };
    let Ok(open) = MenuItem::with_id(app, "tray-open", "Abrir Panita Client", true, None::<&str>)
    else {
        return false;
    };
    let Ok(quit) = MenuItem::with_id(app, "tray-quit", "Salir", true, None::<&str>) else {
        return false;
    };
    let Ok(menu) = Menu::with_items(app, &[&open, &quit]) else {
        return false;
    };
    let built = TrayIconBuilder::with_id("panita-client")
        .icon(icon)
        .tooltip("Panita Client")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-open" => play_commands::restore_window(app, String::new()),
            "tray-quit" => quit_launcher(app),
            _ => (),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                play_commands::restore_window(tray.app_handle(), String::new());
            }
        })
        .build(app);
    built.is_ok()
}

/// Quit the launcher. Running games are separate processes and keep running; an installation
/// or launch still being prepared is not interrupted, so the window comes back to say so.
#[cfg(any(windows, target_os = "linux"))]
fn quit_launcher(app: &tauri::AppHandle) {
    let plays = app.state::<DesktopState>().plays.clone();
    if plays.begin_tray_exit() {
        app.exit(0);
        return;
    }
    play_commands::restore_window(app, String::new());
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        plays.report_close_blocked();
        plays.publish(&handle);
    });
}
