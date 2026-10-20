use super::*;

fn installation(controller: &PlayController, instance: Uuid, request: Uuid) -> Cancellation {
    controller
        .reserve_operation(instance, request, true)
        .unwrap_or_else(|_| panic!("reservation"))
}

#[test]
fn installation_blocks_duplicate_play_and_close_until_worker_finishes() {
    let controller = PlayController::new(ProcessSessionId::new());
    let instance = Uuid::new_v4();
    let request = Uuid::new_v4();
    let cancel = installation(&controller, instance, request);
    assert_eq!(
        controller.reserve(instance, Uuid::new_v4()).err(),
        Some(ErrorCode::OperationBusy)
    );
    assert!(controller.close_window(|| panic!("installation must not hide")));
    assert!(controller.cancel(instance, request).is_ok());
    assert!(cancel.check().is_err());
    assert!(!controller.allow_close());
    controller.fail(instance, request, ErrorCode::OperationCancelled);
    assert!(controller.allow_close());
}

#[test]
fn account_change_keeps_installation_but_hiding_cancels_it_and_waits() {
    let controller = PlayController::new(ProcessSessionId::new());
    let instance = Uuid::new_v4();
    let request = Uuid::new_v4();
    let cancel = installation(&controller, instance, request);
    controller.cancel_pending();
    assert!(cancel.check().is_ok());
    let game = Uuid::new_v4();
    assert!(controller.reserve(game, request).is_ok());
    controller.update(game, request, |entry| entry.phase = Phase::Running);
    assert!(controller.close_window(|| true));
    assert!(cancel.check().is_err());
    controller.update(game, request, |entry| entry.phase = Phase::Exited);
    assert!(!controller.finish_background());
    controller.fail(instance, request, ErrorCode::OperationCancelled);
    assert!(controller.finish_background());
}

#[test]
fn installation_progress_is_bounded_scoped_and_terminal_results_are_authoritative() {
    use launcher_core::minecraft::download::DownloadProgress;
    let controller = PlayController::new(ProcessSessionId::new());
    let instance = Uuid::new_v4();
    let request = Uuid::new_v4();
    let _cancel = installation(&controller, instance, request);
    let progress = DownloadProgress {
        bytes: 10,
        total: 10,
        reused: false,
    };
    assert!(!controller.installation_progress(instance, Uuid::new_v4(), progress));
    controller.installation_progress(
        instance,
        request,
        DownloadProgress {
            bytes: 5,
            ..progress
        },
    );
    controller.installation_progress(instance, request, progress);
    controller.installation_progress(
        instance,
        request,
        DownloadProgress {
            reused: true,
            ..progress
        },
    );
    let snapshot = controller.snapshot().unwrap_or_else(|_| panic!("snapshot"));
    assert_eq!(snapshot["instances"][0]["receivedFiles"], json!(1));
    assert_eq!(snapshot["instances"][0]["reusedFiles"], json!(1));
    assert_eq!(snapshot["instances"][0]["operation"], json!("install"));
    // Cancellation after publication cannot undo a completed manifest.
    assert!(controller.cancel(instance, request).is_ok());
    controller.update(instance, request, |entry| entry.phase = Phase::Installed);
    let completed = controller.snapshot().unwrap_or_else(|_| panic!("snapshot"));
    assert!(!controller.installation_progress(instance, request, progress));
    assert_eq!(controller.snapshot().ok(), Some(completed));
    assert!(controller.reserve(instance, Uuid::new_v4()).is_ok());
    assert!(!controller.installation_progress(instance, request, progress));
}

#[test]
fn hidden_launcher_keeps_all_games_and_exits_only_after_last_exit() {
    let controller = PlayController::new(ProcessSessionId::new());
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let request = Uuid::new_v4();
    for instance in [first, second] {
        assert!(controller.reserve(instance, request).is_ok());
        controller.update(instance, request, |entry| entry.phase = Phase::Running);
    }
    assert!(controller.close_window(|| true));
    assert!(!controller.finish_background());
    assert_eq!(
        controller.reserve(Uuid::new_v4(), Uuid::new_v4()).err(),
        Some(ErrorCode::OperationCancelled)
    );
    controller.update(first, request, |entry| entry.phase = Phase::Exited);
    assert!(!controller.finish_background());
    controller.update(second, request, |entry| entry.phase = Phase::Exited);
    assert!(controller.finish_background());
    assert!(!controller.finish_background());
    assert!(!controller.restore());
}

#[test]
fn reopening_preserves_request_and_game_controls_and_prevents_hidden_exit() {
    let controller = PlayController::new(ProcessSessionId::new());
    let instance = Uuid::new_v4();
    let request = Uuid::new_v4();
    assert!(controller.reserve(instance, request).is_ok());
    controller.update(instance, request, |entry| entry.phase = Phase::Stopping);
    assert!(controller.close_window(|| true));
    assert!(controller.restore());
    let snapshot = controller.snapshot().unwrap_or_else(|_| panic!("snapshot"));
    assert_eq!(snapshot["instances"][0]["requestId"], json!(request));
    assert_eq!(snapshot["instances"][0]["phase"], json!("stopping"));
    controller.update(instance, request, |entry| entry.phase = Phase::Exited);
    assert!(!controller.finish_background());
    assert!(controller.reserve(instance, Uuid::new_v4()).is_ok());
}

#[test]
fn hiding_cancels_pending_work_and_waits_for_its_completion() {
    let controller = PlayController::new(ProcessSessionId::new());
    let running = Uuid::new_v4();
    let pending = Uuid::new_v4();
    let request = Uuid::new_v4();
    assert!(controller.reserve(running, request).is_ok());
    controller.update(running, request, |entry| entry.phase = Phase::Running);
    let cancel = controller
        .reserve(pending, request)
        .unwrap_or_else(|_| panic!("reservation"));
    assert!(controller.close_window(|| true));
    assert!(cancel.check().is_err());
    controller.update(running, request, |entry| entry.phase = Phase::Exited);
    assert!(!controller.finish_background());
    controller.fail(pending, request, ErrorCode::OperationCancelled);
    assert!(controller.finish_background());
}

#[test]
fn failed_hide_keeps_window_lifecycle_visible() {
    let controller = PlayController::new(ProcessSessionId::new());
    let instance = Uuid::new_v4();
    let request = Uuid::new_v4();
    assert!(controller.reserve(instance, request).is_ok());
    controller.update(instance, request, |entry| entry.phase = Phase::Running);
    assert!(controller.close_window(|| false));
    assert_eq!(
        controller.snapshot().unwrap_or_else(|_| panic!("snapshot"))["closeBlocked"],
        json!(true)
    );
    controller.update(instance, request, |entry| entry.phase = Phase::Exited);
    assert!(!controller.finish_background());
    assert!(!controller.close_window(|| panic!("idle close must not hide")));
}

#[test]
fn restoration_after_last_exit_wins_over_queued_idle_check() {
    let controller = PlayController::new(ProcessSessionId::new());
    let instance = Uuid::new_v4();
    let request = Uuid::new_v4();
    assert!(controller.reserve(instance, request).is_ok());
    controller.update(instance, request, |entry| entry.phase = Phase::Running);
    assert!(controller.close_window(|| true));
    controller.update(instance, request, |entry| entry.phase = Phase::Exited);
    assert!(controller.restore());
    assert!(!controller.finish_background());
}

#[test]
fn visible_or_minimized_idle_launcher_does_not_exit_automatically() {
    let controller = PlayController::new(ProcessSessionId::new());
    assert!(!controller.finish_background());
    let instance = Uuid::new_v4();
    let request = Uuid::new_v4();
    assert!(controller.reserve(instance, request).is_ok());
    assert!(controller.close_window(|| panic!("preparation must stay visible")));
    controller.fail(instance, request, ErrorCode::OperationCancelled);
    assert!(!controller.finish_background());
}

#[test]
fn desktop_reservation_rejects_duplicates_and_stale_updates() {
    let controller = PlayController::new(ProcessSessionId::new());
    let instance = Uuid::new_v4();
    let old = Uuid::new_v4();
    assert!(controller.reserve(instance, old).is_ok());
    assert_eq!(
        controller.reserve(instance, Uuid::new_v4()).err(),
        Some(ErrorCode::OperationBusy)
    );
    controller.fail(instance, old, ErrorCode::OperationCancelled);
    let current = Uuid::new_v4();
    assert!(controller.reserve(instance, current).is_ok());
    controller.fail(instance, old, ErrorCode::NetworkUnavailable);
    assert_eq!(
        controller
            .snapshot()
            .ok()
            .map(|snapshot| snapshot["instances"][0]["phase"].clone()),
        Some(json!("preparing"))
    );
    assert_eq!(
        controller.cancel(instance, old).err(),
        Some(ErrorCode::OperationCancelled)
    );
}

#[test]
fn normal_exit_is_blocked_until_pending_work_finishes_and_serialized_with_new_requests() {
    let controller = PlayController::new(ProcessSessionId::new());
    let instance = Uuid::new_v4();
    let request = Uuid::new_v4();
    assert!(controller.reserve(instance, request).is_ok());
    assert!(!controller.allow_close());
    assert_eq!(
        controller
            .snapshot()
            .ok()
            .map(|snapshot| snapshot["closeBlocked"].clone()),
        Some(json!(true))
    );
    assert!(controller.cancel(instance, request).is_ok());
    assert!(!controller.allow_close());
    controller.fail(instance, request, ErrorCode::OperationCancelled);
    assert_eq!(
        controller
            .snapshot()
            .ok()
            .map(|snapshot| snapshot["closeBlocked"].clone()),
        Some(json!(false))
    );
    assert!(controller.allow_close());
    assert_eq!(
        controller.reserve(instance, Uuid::new_v4()).err(),
        Some(ErrorCode::OperationCancelled)
    );
}

#[test]
fn running_games_remain_owned_when_pending_accounts_are_cancelled() {
    let controller = PlayController::new(ProcessSessionId::new());
    let instance = Uuid::new_v4();
    let request = Uuid::new_v4();
    assert!(controller.reserve(instance, request).is_ok());
    controller.update(instance, request, |entry| entry.phase = Phase::Running);
    controller.cancel_pending();
    assert_eq!(
        controller
            .snapshot()
            .ok()
            .map(|snapshot| snapshot["instances"][0]["phase"].clone()),
        Some(json!("running"))
    );
    assert!(!controller.allow_close());
}

#[test]
fn snapshots_exclude_backend_ownership_and_bound_terminal_history() {
    let controller = PlayController::new(ProcessSessionId::new());
    for _ in 0..100 {
        let instance = Uuid::new_v4();
        let request = Uuid::new_v4();
        assert!(controller.reserve(instance, request).is_ok());
        controller.fail(instance, request, ErrorCode::InstanceMissing);
    }
    let snapshot = controller.snapshot().unwrap_or_else(|_| panic!("snapshot"));
    assert!(
        snapshot["instances"]
            .as_array()
            .unwrap_or_else(|| panic!("instances"))
            .len()
            <= 64
    );
    for entry in snapshot["instances"]
        .as_array()
        .unwrap_or_else(|| panic!("instances"))
    {
        let keys: Vec<_> = entry
            .as_object()
            .unwrap_or_else(|| panic!("entry"))
            .keys()
            .map(String::as_str)
            .collect();
        let mut expected = vec![
            "instanceId",
            "requestId",
            "phase",
            "confirmationId",
            "error",
            "exitCode",
            "exitSuccess",
            "operation",
            "receivedFiles",
            "reusedFiles",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert!(entry.get("game").is_none());
        assert!(entry.get("pending").is_none());
        assert!(entry.get("args").is_none());
        assert!(entry.get("token").is_none());
    }
}
