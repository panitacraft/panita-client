//! Host desktop helpers with an AppImage-safe child environment.
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum OpenError {
    Failed,
    TimedOut,
}

pub(super) fn system_opener(target: &std::ffi::OsStr, appimage: bool) -> Command {
    // Use the host utility, never a helper resolved through AppImage's PATH.
    // The target is one argument, never shell syntax. Keep display/D-Bus/desktop
    // activation variables so the helper can reach the user's desktop session.
    let mut command = Command::new("/usr/bin/xdg-open");
    command
        .arg(target)
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
pub(super) fn wait_for_opener(mut command: Command, timeout: Duration) -> Result<(), OpenError> {
    let mut child = command.spawn().map_err(|_| OpenError::Failed)?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(OpenError::Failed)
                };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            result => {
                // xdg-open can exec the target application itself. Never terminate it on
                // timeout: reap in the background once it eventually exits.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Err(if result.is_err() {
                    OpenError::Failed
                } else {
                    OpenError::TimedOut
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_paths_remain_literal_and_appimage_cleanup_preserves_desktop_context() {
        let path =
            std::ffi::OsStr::new("/home/player/Panita Client/instances/pack & friends/#world");
        let command = system_opener(path, true);
        assert_eq!(command.get_args().collect::<Vec<_>>(), [path]);
        let env: Vec<_> = command.get_envs().collect();
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
            assert!(env.contains(&(std::ffi::OsStr::new(key), None)), "{key}");
        }
        for key in [
            "DBUS_SESSION_BUS_ADDRESS",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_ACTIVATION_TOKEN",
            "HOME",
            "XDG_DATA_DIRS",
        ] {
            assert!(!env.iter().any(|(name, _)| *name == key), "{key}");
        }
        assert_eq!(system_opener(path, false).get_envs().count(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_folder_paths_are_not_replaced_or_reencoded() {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::OsStr::from_bytes(b"/home/player/pack-\xff");
        assert_eq!(
            system_opener(path, true).get_args().collect::<Vec<_>>(),
            [path]
        );
    }

    #[test]
    fn oauth_url_is_a_single_literal_argument_and_environment_changes_are_child_local() {
        let url = "https://login.example/authorize?state=SYNTHETIC&redirect_uri=https%3A%2F%2Fexample.test";
        let original = std::env::var_os("LD_LIBRARY_PATH");
        let command = system_opener(std::ffi::OsStr::new(url), true);
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
        assert_eq!(
            system_opener(std::ffi::OsStr::new(url), false)
                .get_envs()
                .count(),
            0
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn opener_exit_failure_spawn_failure_and_timeout_are_distinguished() {
        for (script, expected) in [
            ("exit 0", Ok(())),
            ("exit 3", Err(OpenError::Failed)),
            ("exec sleep 5", Err(OpenError::TimedOut)),
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
            Err(OpenError::Failed)
        );
    }
}
