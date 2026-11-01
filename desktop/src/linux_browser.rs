//! Linux browser opening with optional GTK activation and a system-opener fallback.
use launcher_core::logging::BrowserLaunchEvent;
#[cfg(target_os = "linux")]
use std::time::Duration;

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
            super::linux_desktop::wait_for_opener(
                super::linux_desktop::system_opener(
                    std::ffi::OsStr::new(&url),
                    std::env::var_os("APPIMAGE").is_some(),
                ),
                Duration::from_secs(15),
            )
            .map_err(|error| match error {
                super::linux_desktop::OpenError::Failed => BrowserLaunchEvent::SystemOpenerFailed,
                super::linux_desktop::OpenError::TimedOut => {
                    BrowserLaunchEvent::SystemOpenerTimedOut
                }
            })
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
}
