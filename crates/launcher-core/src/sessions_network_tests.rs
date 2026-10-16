use super::*;
use crate::authentication::network_tests::{
    callback_404_fixture, dns_failure_client, tls_failure_client,
};
use std::sync::atomic::Ordering;

fn assert_preserved(controller: &AccountController, paths: &AppPaths, id: Uuid, before: &[u8]) {
    assert_eq!(std::fs::read(paths.accounts()).unwrap_or_default(), before);
    let view = controller.view().unwrap_or_else(|_| panic!("view"));
    assert_eq!(view.accounts.len(), 1);
    assert_eq!(view.selected_account_id, Some(id));
    assert_eq!(view.accounts[0].metadata.kind, AccountKind::Microsoft);
    assert!(!matches!(view.accounts[0].status, SessionStatus::Ready));
    assert_eq!(
        controller
            .repository
            .refresh_token(id)
            .unwrap_or_else(|_| panic!("preserved token"))
            .expose(),
        "fixture-refresh"
    );
}

#[tokio::test]
async fn isolated_dns_failure_preserves_accounts_and_offline_play_skips_authentication() {
    let (client, calls) = dns_failure_client();
    let (paths, controller, id) = saved_microsoft(client);
    let before = std::fs::read(paths.accounts()).unwrap_or_else(|_| panic!("synthetic file"));
    assert!(
        matches!(controller.renew_selected().await, Err(error) if error.code() == ErrorCode::NetworkUnavailable)
    );
    assert_preserved(&controller, &paths, id, &before);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // A fresh explicit Play attempt retries once; there is no background retry loop.
    assert!(
        matches!(controller.validate_play().await, Err(error) if error.code() == ErrorCode::NetworkUnavailable)
    );
    assert_preserved(&controller, &paths, id, &before);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let offline = controller
        .add_offline("FixtureOffline")
        .unwrap_or_else(|_| panic!("offline"));
    let profile = controller
        .validate_play()
        .await
        .unwrap_or_else(|_| panic!("offline validation"));
    assert_eq!(profile.id, offline.profile_id);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    drop(controller);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn isolated_tls_failure_preserves_account_and_blocks_microsoft_play() {
    let (client, task) = tls_failure_client();
    let (paths, controller, id) = saved_microsoft(client);
    let before = std::fs::read(paths.accounts()).unwrap_or_else(|_| panic!("synthetic file"));
    assert!(
        matches!(controller.validate_play().await, Err(error) if error.code() == ErrorCode::NetworkUnavailable)
    );
    assert_preserved(&controller, &paths, id, &before);
    assert!(!task.join().unwrap_or_else(|_| panic!("TLS server")));
    drop(controller);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn isolated_callback_404_cannot_complete_pending_login_or_replace_account() {
    let (client, _) = dns_failure_client();
    let (paths, controller, id) = saved_microsoft(client);
    let before = std::fs::read(paths.accounts()).unwrap_or_else(|_| panic!("synthetic file"));
    assert!(controller.begin_login(None).is_ok());
    let (url, task) = callback_404_fixture();
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_else(|_| panic!("client"))
        .get(format!("{url}#code=fixture-code&state=fixture-state"))
        .send()
        .await
        .unwrap_or_else(|_| panic!("HTTP fixture"));
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    assert!(
        !response
            .text()
            .await
            .unwrap_or_default()
            .contains("panitacraft://")
    );
    assert!(task.join().is_ok());
    assert!(
        controller
            .view()
            .unwrap_or_else(|_| panic!("view"))
            .login_pending
    );
    assert_preserved(&controller, &paths, id, &before);
    assert!(controller.cancel_login().is_ok());
    assert!(
        !controller
            .view()
            .unwrap_or_else(|_| panic!("view"))
            .login_pending
    );
    assert_preserved(&controller, &paths, id, &before);
    // A failed page does not prevent starting a fresh operation.
    assert!(controller.begin_login(None).is_ok());
    assert!(controller.cancel_login().is_ok());
    drop(controller);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}
