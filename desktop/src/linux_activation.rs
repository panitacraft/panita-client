//! Preserve desktop activation across the Linux single-instance handoff.
//! The upstream relay sends argv and cwd, but not the launching environment.
#[cfg(target_os = "linux")]
use tauri::Manager;

const PREFIX: &str = "panita-activation-v1:";
const MAX_TOKEN_BYTES: usize = 4096;

#[cfg(target_os = "linux")]
pub(super) fn open_browser(app: &tauri::AppHandle, url: String) -> Result<(), ()> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let handle = app.clone();
    // Wry executes this inline on the UI thread, or dispatches it there when
    // called from a worker. GTK objects never leave that thread.
    app.run_on_main_thread(move || {
        use gtk::prelude::*;
        let result = (|| {
            let window = handle.get_webview_window("main").ok_or(())?;
            let native = window.gtk_window().map_err(|_| ())?;
            let context = native.display().app_launch_context().ok_or(())?;
            context.set_timestamp(gtk::current_event_time());
            // Let the default browser receive the active launcher's startup
            // context. A detached xdg-open process loses that user activation.
            // Errors may contain the authorization URL; expose only a code.
            gtk::gio::AppInfo::launch_default_for_uri(&url, Some(&context)).map_err(|_| ())
        })();
        let _ = send.send(result);
    })
    .map_err(|_| ())?;
    receive.recv().map_err(|_| ())?
}

fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= MAX_TOKEN_BYTES && !token.chars().any(char::is_control)
}

fn decode(context: &str) -> Option<&str> {
    context
        .strip_prefix(PREFIX)
        .filter(|token| valid_token(token))
}

#[cfg(target_os = "linux")]
pub(super) fn forward_from_environment(identifier: &str) -> bool {
    // Read before GTK consumes the startup environment. Never log these values.
    let token = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|token| valid_token(token));
    let Some(token) = token else { return false };
    let args: Vec<String> = std::env::args_os()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    forward(identifier, &args, &token)
}

#[cfg(target_os = "linux")]
fn forward(identifier: &str, args: &[String], token: &str) -> bool {
    if !valid_token(token) {
        return false;
    }
    let result = (|| -> zbus::Result<()> {
        let connection = zbus::blocking::connection::Builder::session()?
            .method_timeout(std::time::Duration::from_secs(2))
            .build()?;
        let name = format!("{identifier}.SingleInstance");
        let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
        // Use the existing relay's unused cwd slot, leaving callback argv intact.
        // No new D-Bus interface, URL parameter, frontend event or persistent file.
        connection.call_method(
            Some(name.as_str()),
            path.as_str(),
            Some("org.SingleInstance.DBus"),
            "ExecuteCallback",
            &(args, format!("{PREFIX}{token}")),
        )?;
        Ok(())
    })();
    // No owner or a failed relay: let the normal single-instance/startup path run.
    result.is_ok()
}

#[cfg(target_os = "linux")]
pub(super) fn present(window: tauri::WebviewWindow, context: &str) {
    let token = decode(context).map(str::to_owned);
    let main_window = window.clone();
    let _ = window.run_on_main_thread(move || {
        use gtk::prelude::*;
        if let Ok(native) = main_window.gtk_window() {
            if let Some(token) = token {
                if native.display().type_().name() == "GdkWaylandDisplay" {
                    if !native.is_visible() {
                        native.show();
                    }
                    if let Some(surface) = native.window() {
                        // On Wayland this consumes the compositor-issued activation
                        // token and restores/focuses the existing surface directly.
                        surface.set_startup_id(&token);
                        return;
                    }
                } else {
                    native.set_startup_id(&token);
                }
            }
            native.present();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_context_is_bounded_and_distinct_from_a_working_directory() {
        assert_eq!(decode("/home/player"), None);
        assert_eq!(decode(PREFIX), None);
        assert_eq!(
            decode(&format!("{PREFIX}fixture-token")),
            Some("fixture-token")
        );
        assert!(decode(&format!("{PREFIX}{}", "x".repeat(MAX_TOKEN_BYTES))).is_some());
        assert!(decode(&format!("{PREFIX}{}", "x".repeat(MAX_TOKEN_BYTES + 1))).is_none());
        assert!(decode(&format!("{PREFIX}fixture\ninvalid")).is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "run inside dbus-run-session"]
    fn relay_preserves_callback_arguments_and_transfers_activation_once() {
        struct Receiver(std::sync::mpsc::Sender<(Vec<String>, String)>);
        #[zbus::interface(name = "org.SingleInstance.DBus")]
        impl Receiver {
            fn execute_callback(&self, args: Vec<String>, context: String) {
                let _ = self.0.send((args, context));
            }
        }
        let identifier = format!("com.panitacraft.ActivationTest{}", std::process::id());
        let name = format!("{identifier}.SingleInstance");
        let path = format!("/{}", name.replace('.', "/"));
        let (tx, rx) = std::sync::mpsc::channel();
        let _owner = zbus::blocking::connection::Builder::session()
            .and_then(|builder| builder.name(name.as_str()))
            .and_then(|builder| builder.serve_at(path.as_str(), Receiver(tx)))
            .and_then(|builder| builder.build())
            .unwrap_or_else(|_| panic!("isolated relay owner"));
        let args = vec![
            "/usr/bin/panita-desktop".to_owned(),
            "panitacraft://oauth/callback?code=SYNTHETIC&state=SYNTHETIC".to_owned(),
        ];
        assert!(forward(&identifier, &args, "fixture-activation"));
        let (received, context) = rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap_or_else(|_| panic!("activation relay"));
        assert_eq!(received, args);
        assert_eq!(decode(&context), Some("fixture-activation"));
        assert!(rx.try_recv().is_err());
        drop(_owner);
        assert!(!forward(&identifier, &args, "fixture-activation"));
    }
}
