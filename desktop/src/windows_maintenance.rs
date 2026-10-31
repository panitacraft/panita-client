//! Serialize profile erasure against startup before Tauri opens any user data.
use launcher_core::data_removal;
use sha2::{Digest, Sha256};
use winsafe::prelude::Handle;
use winsafe::{HEVENT, HWND, co, guard::CloseHandleGuard};

pub(super) struct StartupGate(CloseHandleGuard<HEVENT>);
impl StartupGate {
    fn named(name: &str) -> Result<Self, ()> {
        let event = HEVENT::CreateEvent(None, false, true, Some(name)).map_err(|_| ())?;
        if event.WaitForSingleObject(Some(0)).map_err(|_| ())? != co::WAIT::OBJECT_0 {
            return Err(());
        }
        Ok(Self(event))
    }
    pub(super) fn enter() -> Result<Self, ()> {
        let root = data_removal::current_root().map_err(|_| ())?;
        let scope = format!(
            "{:x}",
            Sha256::digest(root.to_string_lossy().to_lowercase().as_bytes())
        );
        Self::named(&format!("Global\\PanitaClient-profile-{scope}"))
    }
}
impl Drop for StartupGate {
    fn drop(&mut self) {
        let _ = self.0.SetEvent();
    }
}

pub(super) fn show_error(text: &str) {
    let _ = HWND::NULL.MessageBox(text, "Panita Client", co::MB::OK | co::MB::ICONERROR);
}

pub(super) fn requested(args: impl Iterator<Item = std::ffi::OsString>) -> bool {
    args.eq([std::ffi::OsString::from("--remove-launcher-data")])
}

pub(super) fn remove(identifier: &str) -> i32 {
    let Ok(_gate) = StartupGate::enter() else {
        return 2;
    };
    // Tauri registers this window before startup releases the same gate. This
    // covers a hidden launcher still supervising games, without force-closing it.
    winsafe::SetLastError(co::ERROR::SUCCESS);
    match HWND::FindWindow(
        Some(winsafe::AtomStr::from_str(&format!("{identifier}-sic"))),
        Some(&format!("{identifier}-siw")),
    ) {
        Ok(None) => {}
        _ => return 2,
    }
    // End the member's launcher session on the API first; the local copy goes with the profile.
    if let Ok(root) = data_removal::current_root()
        && !data_removal::pending(&root).unwrap_or(true)
    {
        launcher_core::activation::close_stored_session_blocking(&root);
    }
    if data_removal::remove_current_profile().is_err() {
        return 3;
    }
    // The browser data is disposable; a failure is still reported so the uninstaller can retry.
    match data_removal::local_root().and_then(|local| data_removal::remove_web_data_in(&local)) {
        Ok(()) => 0,
        Err(_) => 3,
    }
}

/// Earlier builds kept the browser data in a folder named after the application identifier.
/// Retire it before the webview opens the new one; failures are retried on the next start.
pub(super) fn retire_legacy_web_data() {
    if let Ok(local) = data_removal::local_root() {
        data_removal::retire_legacy_web_data_in(&local);
    }
}

pub(super) fn startup_allowed() -> bool {
    matches!(
        data_removal::current_root().and_then(|root| data_removal::pending(&root)),
        Ok(false)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn erasure_requires_the_single_exact_argument() {
        fn args(values: &[&str]) -> impl Iterator<Item = std::ffi::OsString> {
            values.iter().map(std::ffi::OsString::from)
        }
        assert!(requested(args(&["--remove-launcher-data"])));
        assert!(!requested(args(&[])));
        assert!(!requested(args(&[
            "panitacraft://oauth/callback?--remove-launcher-data"
        ])));
        assert!(!requested(args(&["--remove-launcher-data", "C:\\other"])));
    }
    #[test]
    fn gate_excludes_other_startups_and_releases_on_drop() {
        let name = format!("Local\\PanitaClient-gate-test-{}", uuid::Uuid::new_v4());
        let first = StartupGate::named(&name).unwrap_or_else(|_| panic!("first gate"));
        assert!(StartupGate::named(&name).is_err());
        drop(first);
        assert!(StartupGate::named(&name).is_ok());
    }
}
