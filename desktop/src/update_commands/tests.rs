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
