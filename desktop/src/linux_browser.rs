//! Linux browser opening with optional GTK activation and a system-opener fallback.
use launcher_core::logging::BrowserLaunchEvent;
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
pub(super) fn open_browser(app: &tauri::AppHandle, url: String) -> Result<(), ()> {
    use tauri::Manager;
    let record = |event| {
        let state = app.state::<super::DesktopState>();
        state.logs.record(launcher_core::logging::LogRecord {
            process: state.process,
            operation: None,
            event: launcher_core::logging::LogEvent::BrowserLaunch(event),
        });
    };
    // Called on a blocking worker: neither GTK dispatch nor the opener wait may
    // block the window's event loop. GTK objects stay on the main thread.
    let primary = open_with_context(app, url.clone());
    complete_launch(
        primary,
        || {
            wait_for_opener(
                system_opener(&url, std::env::var_os("APPIMAGE").is_some()),
                Duration::from_secs(15),
            )
        },
        record,
    )
}

#[cfg(target_os = "linux")]
fn open_with_context(app: &tauri::AppHandle, url: String) -> Result<(), BrowserLaunchEvent> {
    use gtk::prelude::*;
    use tauri::Manager;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let context = handle
            .get_webview_window("main")
            .and_then(|window| window.gtk_window().ok())
            .and_then(|native| native.display().app_launch_context());
        if let Some(context) = &context {
            context.set_timestamp(gtk::current_event_time());
        } else {
            let state = handle.state::<super::DesktopState>();
            state.logs.record(launcher_core::logging::LogRecord {
                process: state.process,
                operation: None,
                event: launcher_core::logging::LogEvent::BrowserLaunch(
                    BrowserLaunchEvent::ContextUnavailable,
                ),
            });
        }
        // GIO accepts a missing context. Native errors may contain the OAuth URL;
        // discard their text and record only a fixed stage.
        let result = gtk::gio::AppInfo::launch_default_for_uri(&url, context.as_ref())
            .map_err(|_| BrowserLaunchEvent::GioFailed);
        let _ = send.send(result);
    })
    .map_err(|_| BrowserLaunchEvent::DispatchFailed)?;
    receive
        .recv()
        .map_err(|_| BrowserLaunchEvent::DispatchFailed)?
}

fn complete_launch(
    primary: Result<(), BrowserLaunchEvent>,
    fallback: impl FnOnce() -> Result<(), BrowserLaunchEvent>,
    record: impl Fn(BrowserLaunchEvent),
) -> Result<(), ()> {
    if let Err(event) = primary {
        record(event);
        match fallback() {
            Ok(()) => record(BrowserLaunchEvent::FallbackSucceeded),
            Err(event) => {
                record(event);
                return Err(());
            }
        }
    }
    Ok(())
}

fn system_opener(url: &str, appimage: bool) -> Command {
    // Use the host utility, never a helper resolved through AppImage's PATH.
    // The URL is one argument, never shell syntax. Keep display/D-Bus/desktop
    // activation variables so the helper can reach the user's desktop session.
    let mut command = Command::new("/usr/bin/xdg-open");
    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if appimage {
        for key in [
            "LD_LIBRARY_PATH",
            "LD_PRELOAD",
            "GIO_EXTRA_MODULES",
            "GIO_MODULE_DIR",
            "GSETTINGS_SCHEMA_DIR",
            "GTK_PATH",
            "GTK_EXE_PREFIX",
            "GTK_DATA_PREFIX",
            "GDK_PIXBUF_MODULE_FILE",
            "GDK_PIXBUF_MODULEDIR",
        ] {
            command.env_remove(key);
        }
    }
    command
}

#[cfg(target_os = "linux")]
fn wait_for_opener(mut command: Command, timeout: Duration) -> Result<(), BrowserLaunchEvent> {
    let mut child = command
        .spawn()
        .map_err(|_| BrowserLaunchEvent::SystemOpenerFailed)?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(BrowserLaunchEvent::SystemOpenerFailed)
                };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            result => {
                // xdg-open can exec the browser itself. Never terminate it on
                // timeout: reap in the background once it eventually exits.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Err(if result.is_err() {
                    BrowserLaunchEvent::SystemOpenerFailed
                } else {
                    BrowserLaunchEvent::SystemOpenerTimedOut
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn fallback_runs_only_after_primary_failure_and_reports_the_outcome() {
        assert_eq!(
            complete_launch(
                Ok(()),
                || panic!("duplicate browser"),
                |_| panic!("no failure")
            ),
            Ok(())
        );
        for fallback in [
            Ok(()),
            Err(BrowserLaunchEvent::SystemOpenerFailed),
            Err(BrowserLaunchEvent::SystemOpenerTimedOut),
        ] {
            let events = RefCell::new(Vec::new());
            let result = complete_launch(
                Err(BrowserLaunchEvent::GioFailed),
                || fallback,
                |event| events.borrow_mut().push(event),
            );
            assert_eq!(result.is_ok(), fallback.is_ok());
            assert_eq!(
                *events.borrow(),
                [
                    BrowserLaunchEvent::GioFailed,
                    fallback
                        .err()
                        .unwrap_or(BrowserLaunchEvent::FallbackSucceeded)
                ]
            );
        }
    }

    #[test]
    fn oauth_url_is_a_single_literal_argument_and_environment_changes_are_child_local() {
        let url = "https://login.example/authorize?state=SYNTHETIC&redirect_uri=https%3A%2F%2Fexample.test";
        let original = std::env::var_os("LD_LIBRARY_PATH");
        let command = system_opener(url, true);
        assert_eq!(command.get_program(), "/usr/bin/xdg-open");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new(url)]
        );
        let env: Vec<_> = command.get_envs().collect();
        assert!(env.contains(&(std::ffi::OsStr::new("LD_LIBRARY_PATH"), None)));
        assert!(
            !env.iter().any(|(key, _)| *key == "DBUS_SESSION_BUS_ADDRESS"
                || *key == "DISPLAY"
                || *key == "WAYLAND_DISPLAY"
                || *key == "XDG_ACTIVATION_TOKEN")
        );
        assert_eq!(std::env::var_os("LD_LIBRARY_PATH"), original);
        assert_eq!(system_opener(url, false).get_envs().count(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn opener_exit_failure_spawn_failure_and_timeout_are_distinguished() {
        for (script, expected) in [
            ("exit 0", Ok(())),
            ("exit 3", Err(BrowserLaunchEvent::SystemOpenerFailed)),
            (
                "exec sleep 5",
                Err(BrowserLaunchEvent::SystemOpenerTimedOut),
            ),
        ] {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", script]);
            assert_eq!(
                wait_for_opener(command, Duration::from_millis(100)),
                expected
            );
        }
        assert_eq!(
            wait_for_opener(
                Command::new("/nonexistent/panita-opener"),
                Duration::from_millis(100)
            ),
            Err(BrowserLaunchEvent::SystemOpenerFailed)
        );
    }
}
