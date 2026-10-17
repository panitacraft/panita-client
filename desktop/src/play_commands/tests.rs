use super::*;

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
        assert_eq!(
            entry.as_object().unwrap_or_else(|| panic!("entry")).len(),
            7
        );
        assert!(entry.get("game").is_none());
        assert!(entry.get("pending").is_none());
        assert!(entry.get("args").is_none());
        assert!(entry.get("token").is_none());
    }
}
