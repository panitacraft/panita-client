use super::*;
use crate::panita_api::tests::{
    EXPIRES, TOKEN, canned, closed_client, confirmed_json, created_json, credentials, failure_json,
    pending_json, server, session, user_json,
};
use crate::paths::AppPaths;
use uuid::Uuid;

const FAST: Duration = Duration::from_millis(5);
const CACHE: &str = "insts-cache.json";

fn paths() -> AppPaths {
    AppPaths::new(std::env::temp_dir().join(format!("panita-activation-{}", Uuid::new_v4())))
        .unwrap_or_else(|_| panic!("path"))
}

fn open(paths: &AppPaths) -> PrivateStore {
    PrivateStore::open(paths).unwrap_or_else(|_| panic!("store"))
}

fn controller_with(
    paths: &AppPaths,
    client: PanitaApiClient,
    key: Result<PanitaCredentials, ErrorCode>,
) -> ActivationController {
    ActivationController::with_parts(open(paths), client, key, FAST)
        .unwrap_or_else(|_| panic!("controller"))
}

fn controller(
    paths: &AppPaths,
    replies: Vec<crate::panita_api::tests::Canned>,
) -> (ActivationController, std::thread::JoinHandle<Vec<String>>) {
    let (client, task) = server(replies);
    (controller_with(paths, client, Ok(credentials())), task)
}

fn seed_session(paths: &AppPaths, token: &str) {
    assert!(panita_session::save(&open(paths), &session(token)).is_ok());
}

fn seed_cache(controller: &ActivationController) {
    assert!(
        controller
            .store
            .transaction(CACHE, |_| Ok((b"{}".to_vec(), ())))
            .is_ok()
    );
}

fn cache_exists(paths: &AppPaths) -> bool {
    paths.root().join(CACHE).exists()
}

fn view(controller: &ActivationController) -> ActivationView {
    controller.view().unwrap_or_else(|_| panic!("view"))
}

fn code<T>(result: Result<T, CoreError>) -> Option<ErrorCode> {
    result.err().map(|error| error.code())
}

fn requests(task: std::thread::JoinHandle<Vec<String>>) -> Vec<String> {
    task.join().unwrap_or_else(|_| panic!("fixture"))
}

fn finish(paths: &AppPaths, controller: ActivationController) {
    drop(controller);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

fn ticket(controller: &ActivationController) -> Ticket {
    match &controller.lock().unwrap_or_else(|_| panic!("lock")).phase {
        Phase::Pending(pending) => pending.ticket,
        _ => panic!("a pending login"),
    }
}

fn stored(paths: &AppPaths) -> Option<PanitaSession> {
    panita_session::load(&open(paths)).unwrap_or_else(|_| panic!("load"))
}

#[tokio::test]
async fn activation_persists_before_reporting_active() {
    let paths = paths();
    let (controller, task) = controller(
        &paths,
        vec![
            canned(201, created_json()),
            canned(200, pending_json()),
            canned(200, confirmed_json(TOKEN)),
        ],
    );
    assert_eq!(view(&controller).state, ActivationState::NoSession);
    assert!(
        controller
            .begin("javithor360", &DeviceInfo::default())
            .await
            .is_ok()
    );
    let waiting = view(&controller);
    assert_eq!(waiting.state, ActivationState::Pending);
    assert_eq!(waiting.serial.as_deref(), Some("5V3J-A5TD-YE4L"));
    assert!(waiting.expires_at.is_some() && waiting.user.is_none());
    assert!(stored(&paths).is_none());
    assert!(controller.await_confirmation().await.is_ok());
    let active = view(&controller);
    assert_eq!(active.state, ActivationState::Active);
    assert!(active.persisted && active.error.is_none());
    assert_eq!(active.user.map(|user| user.id), Some(42));
    assert!(active.serial.is_none());
    let saved = stored(&paths).unwrap_or_else(|| panic!("saved session"));
    assert_eq!(saved.token().expose(), TOKEN);
    assert_eq!(controller.identity().ok().flatten(), Some(42));
    let seen = requests(task);
    assert_eq!(seen.len(), 3);
    assert!(
        seen.iter()
            .all(|request| !request.contains("authorization:"))
    );
    finish(&paths, controller);
}

#[tokio::test]
async fn failed_requests_leave_the_gate_open_with_the_reason() {
    let paths = paths();
    let (controller, task) = controller(
        &paths,
        vec![
            canned(401, failure_json("unknown_account")),
            canned(429, failure_json("rate_limited")).header("Retry-After", "42"),
            canned(201, created_json()),
        ],
    );
    let device = DeviceInfo::default();
    assert_eq!(
        code(controller.begin("nobody", &device).await),
        Some(ErrorCode::ActivationUnknownAccount)
    );
    let first = view(&controller);
    assert_eq!(first.state, ActivationState::NoSession);
    assert_eq!(first.error, Some(ErrorCode::ActivationUnknownAccount));
    assert_eq!(
        code(controller.begin("javithor360", &device).await),
        Some(ErrorCode::ActivationRateLimited)
    );
    let limited = view(&controller);
    assert_eq!(limited.retry_after, Some(42));
    assert!(controller.begin("javithor360", &device).await.is_ok());
    let pending = view(&controller);
    assert_eq!(pending.state, ActivationState::Pending);
    assert!(pending.error.is_none() && pending.retry_after.is_none());
    requests(task);
    finish(&paths, controller);
}

#[tokio::test]
async fn only_one_activation_runs_at_a_time() {
    let paths = paths();
    let (controller, task) = controller(&paths, vec![canned(201, created_json())]);
    let device = DeviceInfo::default();
    assert_eq!(
        code(controller.await_confirmation().await),
        Some(ErrorCode::OperationCancelled)
    );
    assert!(controller.begin("javithor360", &device).await.is_ok());
    assert_eq!(
        code(controller.begin("javithor360", &device).await),
        Some(ErrorCode::OperationBusy)
    );
    assert_eq!(view(&controller).state, ActivationState::Pending);
    requests(task);
    finish(&paths, controller);
}

#[tokio::test]
async fn transient_poll_failures_keep_waiting() {
    let paths = paths();
    let (controller, task) = controller(
        &paths,
        vec![
            canned(201, created_json()),
            canned(503, ""),
            canned(200, pending_json()),
            canned(200, confirmed_json(TOKEN)),
        ],
    );
    assert!(
        controller
            .begin("javithor360", &DeviceInfo::default())
            .await
            .is_ok()
    );
    assert!(controller.await_confirmation().await.is_ok());
    let active = view(&controller);
    assert_eq!(active.state, ActivationState::Active);
    assert!(active.error.is_none());
    requests(task);
    finish(&paths, controller);
}

#[tokio::test]
async fn a_spent_request_fails_the_login() {
    let paths = paths();
    let (controller, task) = controller(
        &paths,
        vec![
            canned(201, created_json()),
            canned(410, failure_json("consumed")),
        ],
    );
    assert!(
        controller
            .begin("javithor360", &DeviceInfo::default())
            .await
            .is_ok()
    );
    assert_eq!(
        code(controller.await_confirmation().await),
        Some(ErrorCode::ActivationConsumed)
    );
    let spent = view(&controller);
    assert_eq!(spent.state, ActivationState::NoSession);
    assert_eq!(spent.error, Some(ErrorCode::ActivationConsumed));
    assert!(stored(&paths).is_none());
    requests(task);
    finish(&paths, controller);
}

#[tokio::test]
async fn an_expired_serial_ends_the_login_without_polling() {
    let paths = paths();
    let (controller, task) = controller(
        &paths,
        vec![canned(
            201,
            created_json().replace(EXPIRES, "2020-01-01T00:00:00.000Z"),
        )],
    );
    assert!(
        controller
            .begin("javithor360", &DeviceInfo::default())
            .await
            .is_ok()
    );
    assert_eq!(
        code(controller.await_confirmation().await),
        Some(ErrorCode::ActivationExpired)
    );
    let ended = view(&controller);
    assert_eq!(ended.state, ActivationState::NoSession);
    assert_eq!(ended.error, Some(ErrorCode::ActivationExpired));
    assert_eq!(requests(task).len(), 1);
    finish(&paths, controller);
}

#[tokio::test]
async fn cancelling_stops_the_wait_and_a_late_confirmation_is_dropped() {
    let paths = paths();
    let (controller, task) = controller(
        &paths,
        vec![
            canned(201, created_json()),
            canned(200, pending_json()),
            canned(200, pending_json()),
        ],
    );
    assert!(
        controller
            .begin("javithor360", &DeviceInfo::default())
            .await
            .is_ok()
    );
    let (waited, ()) = tokio::join!(controller.await_confirmation(), async {
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(controller.cancel().is_ok());
    });
    assert_eq!(code(waited), Some(ErrorCode::OperationCancelled));
    let cancelled = view(&controller);
    assert_eq!(cancelled.state, ActivationState::NoSession);
    assert!(cancelled.error.is_none() && cancelled.serial.is_none());
    requests(task);

    // A confirmation that arrives for a replaced or cancelled login changes nothing.
    let (second, task) = self::controller(&paths, vec![canned(201, created_json())]);
    assert!(
        second
            .begin("javithor360", &DeviceInfo::default())
            .await
            .is_ok()
    );
    let stale = ticket(&second);
    assert!(second.cancel().is_ok());
    assert_eq!(
        code(second.complete_login(stale, session(TOKEN))),
        Some(ErrorCode::OperationCancelled)
    );
    assert_eq!(view(&second).state, ActivationState::NoSession);
    assert!(stored(&paths).is_none());
    requests(task);
    drop(second);
    finish(&paths, controller);
}

#[tokio::test]
async fn a_failed_save_keeps_a_memory_only_session_that_can_be_saved_later() {
    let paths = paths();
    let (controller, task) = controller(
        &paths,
        vec![
            canned(201, created_json()),
            canned(200, confirmed_json(TOKEN)),
        ],
    );
    assert!(
        controller
            .begin("javithor360", &DeviceInfo::default())
            .await
            .is_ok()
    );
    // Something that is not our file occupies the path.
    assert!(std::fs::create_dir(paths.panita_session()).is_ok());
    assert!(controller.await_confirmation().await.is_ok());
    let memory_only = view(&controller);
    assert_eq!(memory_only.state, ActivationState::Active);
    assert!(!memory_only.persisted && memory_only.error.is_some());
    assert!(controller.retry_save().is_err());
    assert!(!view(&controller).persisted);
    assert!(std::fs::remove_dir(paths.panita_session()).is_ok());
    assert!(controller.retry_save().is_ok());
    let saved = view(&controller);
    assert!(saved.persisted && saved.error.is_none());
    assert_eq!(
        stored(&paths).map(|session| session.token().expose().to_owned()),
        Some(TOKEN.to_owned())
    );
    requests(task);
    finish(&paths, controller);
}

#[tokio::test]
async fn a_saved_session_is_active_at_startup_without_any_request() {
    let paths = paths();
    seed_session(&paths, TOKEN);
    let (controller, task) = controller(&paths, Vec::new());
    let started = view(&controller);
    assert_eq!(started.state, ActivationState::Active);
    assert!(started.persisted && started.error.is_none());
    assert!(controller.session_credentials().is_ok());
    assert!(controller.online_credentials().is_ok());
    assert!(requests(task).is_empty());
    finish(&paths, controller);
}

#[tokio::test]
async fn a_damaged_session_file_is_preserved_and_blocks_activation() {
    let paths = paths();
    let store = open(&paths);
    assert!(
        store
            .transaction("panita-session.json", |_| Ok((b"not json".to_vec(), ())))
            .is_ok()
    );
    drop(store);
    let (controller, task) = controller(&paths, Vec::new());
    let blocked = view(&controller);
    assert_eq!(blocked.state, ActivationState::NoSession);
    assert_eq!(blocked.error, Some(ErrorCode::ActivationFileInvalid));
    assert_eq!(
        code(
            controller
                .begin("javithor360", &DeviceInfo::default())
                .await
        ),
        Some(ErrorCode::ActivationFileInvalid)
    );
    assert_eq!(
        std::fs::read(paths.panita_session()).ok(),
        Some(b"not json".to_vec())
    );
    assert!(requests(task).is_empty());
    finish(&paths, controller);
}

#[tokio::test]
async fn a_missing_launcher_key_blocks_activation_and_authenticated_calls() {
    let paths = paths();
    seed_session(&paths, TOKEN);
    let (client, task) = server(Vec::new());
    let controller = controller_with(&paths, client, Err(ErrorCode::ActivationUnavailable));
    assert_eq!(
        code(controller.session_credentials()),
        Some(ErrorCode::ActivationUnavailable)
    );
    assert_eq!(
        code(controller.refresh().await),
        Some(ErrorCode::ActivationUnavailable)
    );
    let other = self::paths();
    let (client, other_task) = server(Vec::new());
    let blocked = controller_with(&other, client, Err(ErrorCode::ActivationUnavailable));
    assert_eq!(
        code(blocked.begin("javithor360", &DeviceInfo::default()).await),
        Some(ErrorCode::ActivationUnavailable)
    );
    assert!(requests(task).is_empty() && requests(other_task).is_empty());
    finish(&other, blocked);
    finish(&paths, controller);
}

#[tokio::test]
async fn refresh_updates_the_snapshot_and_the_file() {
    let paths = paths();
    seed_session(&paths, TOKEN);
    let renamed = user_json().replace("\"ign\":\"javithor360\"", "\"ign\":\"renamed\"");
    let (controller, task) = controller(
        &paths,
        vec![canned(200, format!(r#"{{"data":{renamed}}}"#))],
    );
    assert_eq!(
        controller.refresh().await.ok(),
        Some(RefreshOutcome::Verified)
    );
    let verified = view(&controller);
    assert_eq!(verified.state, ActivationState::Active);
    assert_eq!(
        verified.user.and_then(|user| user.ign).as_deref(),
        Some("renamed")
    );
    assert!(verified.verified_at.is_some_and(|time| time > 1));
    let saved = stored(&paths).unwrap_or_else(|| panic!("saved"));
    assert_eq!(saved.user.ign.as_deref(), Some("renamed"));
    assert!(saved.verified_at > 1);
    let seen = requests(task);
    assert!(seen[0].starts_with("get /v1/auth/me "));
    assert!(seen[0].contains(&format!("authorization: bearer {TOKEN}")));
    finish(&paths, controller);
}

#[tokio::test]
async fn unreachable_or_failing_services_never_end_the_activation() {
    let paths = paths();
    seed_session(&paths, TOKEN);
    let (controller, task) = controller(
        &paths,
        vec![
            canned(503, ""),
            canned(401, failure_json("invalid_key")),
            canned(200, format!(r#"{{"data":{}}}"#, user_json())),
        ],
    );
    seed_cache(&controller);
    assert_eq!(
        controller.refresh().await.ok(),
        Some(RefreshOutcome::Offline)
    );
    let offline = view(&controller);
    assert_eq!(offline.state, ActivationState::ActiveOffline);
    assert_eq!(offline.error, Some(ErrorCode::ServiceUnavailable));
    assert!(offline.user.is_some() && stored(&paths).is_some());
    // Installed instances still launch; installing needs a verified session.
    assert!(controller.session_credentials().is_ok());
    assert_eq!(
        code(controller.online_credentials()),
        Some(ErrorCode::NetworkUnavailable)
    );
    // A refused launcher key is not a closed session.
    assert_eq!(
        controller.refresh().await.ok(),
        Some(RefreshOutcome::Offline)
    );
    assert_eq!(view(&controller).error, Some(ErrorCode::ApiKeyRejected));
    assert!(stored(&paths).is_some() && cache_exists(&paths));
    // Recovery clears the notice.
    assert_eq!(
        controller.refresh().await.ok(),
        Some(RefreshOutcome::Verified)
    );
    let recovered = view(&controller);
    assert_eq!(recovered.state, ActivationState::Active);
    assert!(recovered.error.is_none());
    assert!(controller.online_credentials().is_ok());
    requests(task);
    finish(&paths, controller);

    let paths = self::paths();
    seed_session(&paths, TOKEN);
    let down = controller_with(&paths, closed_client(), Ok(credentials()));
    assert_eq!(down.refresh().await.ok(), Some(RefreshOutcome::Offline));
    let state = view(&down);
    assert_eq!(state.state, ActivationState::ActiveOffline);
    assert_eq!(state.error, Some(ErrorCode::NetworkUnavailable));
    assert!(stored(&paths).is_some());
    finish(&paths, down);
}

#[tokio::test]
async fn invalid_token_ends_the_activation_and_removes_every_local_copy() {
    let paths = paths();
    seed_session(&paths, TOKEN);
    let (controller, task) = controller(&paths, vec![canned(401, failure_json("invalid_token"))]);
    seed_cache(&controller);
    assert_eq!(
        controller.refresh().await.ok(),
        Some(RefreshOutcome::Rejected)
    );
    let ended = view(&controller);
    assert_eq!(ended.state, ActivationState::NoSession);
    assert_eq!(ended.error, Some(ErrorCode::SessionRejected));
    assert!(ended.user.is_none());
    assert!(!paths.panita_session().exists() && !cache_exists(&paths));
    assert_eq!(
        code(controller.session_credentials()),
        Some(ErrorCode::ActivationRequired)
    );
    assert_eq!(controller.identity().ok().flatten(), None);
    requests(task);
    finish(&paths, controller);
}

#[tokio::test]
async fn late_results_and_stale_rejections_never_touch_a_newer_state() {
    let paths = paths();
    seed_session(&paths, TOKEN);
    let (controller, task) = controller(&paths, vec![canned(204, "")]);
    let (checked, generation) = {
        let state = controller.lock().unwrap_or_else(|_| panic!("lock"));
        let Phase::Active { session, .. } = &state.phase else {
            panic!("active");
        };
        (session.clone(), state.generation)
    };
    let mut changed = checked.user.clone();
    changed.ign = Some("Late".to_owned());
    // A rejection reported for a session that was already replaced.
    let stale = credentials().with_session(&session("ps_replaced-session"));
    assert_eq!(controller.note_rejection(&stale).ok(), Some(false));
    assert_eq!(view(&controller).state, ActivationState::Active);
    assert!(stored(&paths).is_some());
    // A sign-out invalidates a refresh that was already on the wire.
    assert!(controller.sign_out().await.is_ok());
    assert_eq!(
        code(controller.record_verified(&checked, generation, changed)),
        Some(ErrorCode::OperationCancelled)
    );
    assert!(
        controller
            .record_offline(&checked, generation, ErrorCode::NetworkUnavailable)
            .is_ok()
    );
    let after = view(&controller);
    assert_eq!(after.state, ActivationState::NoSession);
    assert!(after.user.is_none() && after.error.is_none());
    assert!(!paths.panita_session().exists());
    requests(task);
    finish(&paths, controller);
}

#[tokio::test]
async fn a_rejection_of_the_current_session_signs_it_out_everywhere_locally() {
    let paths = paths();
    seed_session(&paths, TOKEN);
    let (controller, task) = controller(&paths, Vec::new());
    seed_cache(&controller);
    let credentials = controller
        .session_credentials()
        .unwrap_or_else(|_| panic!("credentials"));
    assert_eq!(controller.note_rejection(&credentials).ok(), Some(true));
    assert_eq!(view(&controller).state, ActivationState::NoSession);
    assert_eq!(view(&controller).error, Some(ErrorCode::SessionRejected));
    assert!(!paths.panita_session().exists() && !cache_exists(&paths));
    assert_eq!(controller.note_rejection(&credentials).ok(), Some(false));
    assert!(requests(task).is_empty());
    finish(&paths, controller);
}

#[tokio::test]
async fn sign_out_closes_the_session_and_always_clears_locally() {
    for (reply, closed) in [
        (canned(204, ""), true),
        (canned(401, failure_json("invalid_token")), true),
        (canned(500, ""), false),
    ] {
        let paths = paths();
        seed_session(&paths, TOKEN);
        let (controller, task) = controller(&paths, vec![reply]);
        seed_cache(&controller);
        let outcome = controller.sign_out().await;
        assert_eq!(
            outcome.ok(),
            Some(SignOutOutcome {
                server_closed: closed
            })
        );
        let ended = view(&controller);
        assert_eq!(ended.state, ActivationState::NoSession);
        assert!(ended.error.is_none());
        assert!(!paths.panita_session().exists() && !cache_exists(&paths));
        let seen = requests(task);
        assert!(seen[0].starts_with("delete /v1/auth/session "));
        assert!(seen[0].contains(&format!("authorization: bearer {TOKEN}")));
        finish(&paths, controller);
    }

    // Offline sign-out clears the local copy and says the server was not reached.
    let paths = paths();
    seed_session(&paths, TOKEN);
    let offline = controller_with(&paths, closed_client(), Ok(credentials()));
    assert_eq!(
        offline.sign_out().await.ok(),
        Some(SignOutOutcome {
            server_closed: false
        })
    );
    assert!(!paths.panita_session().exists());
    finish(&paths, offline);
}

#[tokio::test]
async fn sign_out_cancels_a_pending_login_and_reports_incomplete_cleanup() {
    let paths = paths();
    let (controller, task) = controller(&paths, vec![canned(201, created_json())]);
    assert!(
        controller
            .begin("javithor360", &DeviceInfo::default())
            .await
            .is_ok()
    );
    assert_eq!(
        controller.sign_out().await.ok(),
        Some(SignOutOutcome {
            server_closed: true
        })
    );
    assert_eq!(view(&controller).state, ActivationState::NoSession);
    requests(task);
    finish(&paths, controller);

    // If the file cannot be removed, memory is still cleared and the error is shown.
    let paths = self::paths();
    seed_session(&paths, TOKEN);
    let (controller, task) = self::controller(&paths, vec![canned(204, "")]);
    assert!(
        controller
            .store
            .transaction("panita-session.json", |_| Ok((b"damaged".to_vec(), ())))
            .is_ok()
    );
    assert_eq!(
        code(controller.sign_out().await),
        Some(ErrorCode::ActivationFileInvalid)
    );
    let stuck = view(&controller);
    assert_eq!(stuck.state, ActivationState::NoSession);
    assert_eq!(stuck.error, Some(ErrorCode::ActivationFileInvalid));
    assert!(stuck.user.is_none());
    assert_eq!(
        std::fs::read(paths.panita_session()).ok(),
        Some(b"damaged".to_vec())
    );
    requests(task);
    finish(&paths, controller);
}

#[tokio::test]
async fn failures_are_recorded_as_codes_only() {
    let paths = paths();
    let (client, task) = server(vec![canned(401, failure_json("unknown_account"))]);
    let logs = Arc::new(BoundedLog::default());
    let controller = controller_with(&paths, client, Ok(credentials()))
        .with_diagnostics(logs.clone(), ProcessSessionId::new());
    assert_eq!(
        code(
            controller
                .begin("secret-member-name", &DeviceInfo::default())
                .await
        ),
        Some(ErrorCode::ActivationUnknownAccount)
    );
    seed_session(&paths, TOKEN);
    assert!(controller.cancel().is_ok());
    // A session activated by another process is adopted by the next begin.
    assert_eq!(
        code(
            controller
                .begin("secret-member-name", &DeviceInfo::default())
                .await
        ),
        Some(ErrorCode::OperationBusy)
    );
    let credentials = controller
        .session_credentials()
        .unwrap_or_else(|_| panic!("credentials"));
    assert_eq!(controller.note_rejection(&credentials).ok(), Some(true));
    let events: Vec<_> = logs
        .snapshot()
        .into_iter()
        .map(|record| record.event)
        .collect();
    assert_eq!(
        events,
        [
            LogEvent::ActivationFailure(ErrorCode::ActivationUnknownAccount),
            LogEvent::ActivationFailure(ErrorCode::SessionRejected),
        ]
    );
    let text = format!("{:?}", logs.snapshot());
    assert!(!text.contains("secret-member-name") && !text.contains(TOKEN));
    requests(task);
    finish(&paths, controller);
}
