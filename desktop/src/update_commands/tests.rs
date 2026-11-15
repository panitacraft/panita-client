use super::*;
use tauri_plugin_updater::Error;

#[test]
fn updater_failures_map_to_fixed_codes_without_carrying_service_text() {
    assert_eq!(
        failure(&Error::Network("secret detail".into())),
        ErrorCode::NetworkUnavailable
    );
    assert_eq!(
        failure(&Error::ReleaseNotFound),
        ErrorCode::NetworkUnavailable
    );
    assert_eq!(
        failure(&Error::MissingSignedVersion),
        ErrorCode::ArtifactMismatch
    );
    assert_eq!(
        failure(&Error::SignedVersionMismatch {
            signed: "1.0.0".into(),
            announced: "1.0.1".into()
        }),
        ErrorCode::ArtifactMismatch
    );
    assert_eq!(
        failure(&Error::UnsupportedArch),
        ErrorCode::LauncherUpdateUnsupported
    );
    assert_eq!(
        failure(&Error::EmptyEndpoints),
        ErrorCode::LauncherUpdateFailed
    );
}

#[test]
fn a_build_without_a_package_stamp_updates_manually() {
    // Test and development binaries are not stamped by the bundler.
    assert_eq!(delivery(), Delivery::Manual);
}

#[test]
fn the_view_exposes_only_fixed_state_names() {
    let view = UpdateView {
        current_version: "0.2.0-beta.3".into(),
        channel: UpdateChannel::Beta,
        phase: Phase::Ready,
        pending_version: Some("0.2.0-beta.4".into()),
        delivery: Delivery::InPlace,
        release_url: None,
        progress: Some(100),
        apply_requested: true,
        error: Some(ErrorCode::LauncherUpdateFailed),
    };
    let json = serde_json::to_value(&view).unwrap_or_default();
    assert_eq!(json["currentVersion"], "0.2.0-beta.3");
    assert_eq!(json["channel"], "beta");
    assert_eq!(json["phase"], "ready");
    assert_eq!(json["delivery"], "in_place");
    assert_eq!(json["applyRequested"], true);
    assert_eq!(json["error"], "launcher_update_failed");
}

#[test]
fn an_unset_runtime_is_idle() {
    assert_eq!(Runtime::default().phase(), Phase::Idle);
}

#[test]
fn linux_update_relaunch_targets_the_updated_appimage_without_replaying_callbacks() {
    let image = std::path::PathBuf::from("/home/player/Panita Client 1.0.0.AppImage");
    let relaunch = LinuxRelaunch::new(
        image.clone(),
        [
            std::ffi::OsString::from("--background"),
            std::ffi::OsString::from("panitacraft://oauth/callback#code=private"),
            std::ffi::OsString::from("PANITACRAFT://oauth/callback#error=private"),
        ],
    );
    assert_eq!(relaunch.program, image);
    assert_eq!(relaunch.args, [std::ffi::OsString::from("--background")]);
}

#[test]
fn linux_update_relaunch_removes_old_desktop_activation_and_appimage_loader_state() {
    let relaunch = LinuxRelaunch::new(
        std::path::PathBuf::from("/opt/Panita.AppImage"),
        std::iter::empty(),
    );
    let command = relaunch.command();
    let removed = command
        .get_envs()
        .filter_map(|(name, value)| value.is_none().then(|| name.to_string_lossy().into_owned()))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        removed,
        LINUX_RELAUNCH_ENVIRONMENT
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    );
    // The command inherits the normal session (DISPLAY, WAYLAND_DISPLAY, HOME,
    // XDG_RUNTIME_DIR, and D-Bus); it clears only the explicit stale allowlist.
    assert!(!removed.contains("DISPLAY"));
    assert!(!removed.contains("WAYLAND_DISPLAY"));
    assert!(!removed.contains("XDG_RUNTIME_DIR"));
}
