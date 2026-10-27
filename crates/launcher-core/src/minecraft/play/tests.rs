use super::*;
use crate::accounts::AccountRepository;
use crate::registry::InstallationRecord;

fn successful_auth_chain() -> Vec<(&'static str, u16, String)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expiry = chrono::DateTime::<chrono::Utc>::from_timestamp((now + 3600) as i64, 0)
        .unwrap_or_default()
        .to_rfc3339();
    vec![
        ("/token", 200, r#"{"access_token":"fixture-microsoft","refresh_token":"fixture-rotated","expires_in":3600,"token_type":"Bearer"}"#.into()),
        ("/xbox", 200, serde_json::json!({"Token":"fixture-xbox","NotAfter":expiry,"DisplayClaims":{"xui":[{"uhs":"1234"}]}}).to_string()),
        ("/xsts", 200, serde_json::json!({"Token":"fixture-xsts","NotAfter":expiry,"DisplayClaims":{"xui":[{"uhs":"1234"}]}}).to_string()),
        ("/minecraft", 200, r#"{"access_token":"fixture-minecraft","expires_in":3600,"token_type":"Bearer"}"#.into()),
        ("/profile", 200, fixture_profile()),
    ]
}

fn fixture_profile() -> String {
    r#"{"id":"5627dd98e6be3c21b8a8e92344183641","name":"PlayerOne"}"#.into()
}

fn microsoft_fixture(
    responses: Vec<(&'static str, u16, String)>,
) -> (
    AppPaths,
    Arc<AccountController>,
    Uuid,
    std::thread::JoinHandle<()>,
) {
    use crate::accounts::Secret;
    let (paths, offline, instance) = fixture();
    drop(offline);
    complete_installation(&paths, instance);
    let repository =
        AccountRepository::new(PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store")));
    repository
        .save_microsoft(
            Uuid::parse_str("5627dd98-e6be-3c21-b8a8-e92344183641")
                .unwrap_or_else(|_| panic!("profile")),
            "PlayerOne",
            &Secret::new("fixture-refresh".into()).unwrap_or_else(|_| panic!("refresh")),
            None,
        )
        .unwrap_or_else(|_| panic!("account"));
    let (client, server) = crate::authentication::tests::mock_client(responses);
    let accounts = Arc::new(
        AccountController::with_client(repository, ProcessSessionId::new(), client)
            .unwrap_or_else(|_| panic!("controller")),
    );
    // Save_microsoft selects the profile. No real accounts or endpoint overrides.
    (paths, accounts, instance, server)
}

async fn controlled_attempt(
    paths: &AppPaths,
    accounts: &Arc<AccountController>,
    instance: Uuid,
    status: u16,
) -> PendingPlay {
    let body = if status == 200 {
        serde_json::json!({"data":crate::catalog::tests::record()}).to_string()
    } else {
        "{}".into()
    };
    let (catalog, server) = crate::catalog::tests::server(vec![(status, body, false)]);
    let pending = prepare_with_catalog(
        paths.clone(),
        instance,
        accounts.clone(),
        Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")),
        ProcessSessionId::new(),
        Cancellation::default(),
        catalog,
    )
    .await
    .unwrap_or_else(|error| panic!("preflight: {error}"));
    assert!(server.join().is_ok());
    pending
}

async fn controlled_profile_rejection(final_status: u16, expected: ErrorCode) {
    let mut replies = successful_auth_chain();
    replies.push(("/profile", 401, r#"{"secret":"fixture-error-body"}"#.into()));
    replies.extend(successful_auth_chain());
    replies.push((
        "/profile",
        final_status,
        if final_status == 200 {
            fixture_profile()
        } else {
            "{}".into()
        },
    ));
    let (paths, accounts, instance, server) = microsoft_fixture(replies);
    assert!(accounts.renew_selected().await.is_ok());
    let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store"));
    let before = store
        .read("accounts.json")
        .unwrap_or_else(|_| panic!("before"));
    let game = paths
        .game_directory(instance)
        .unwrap_or_else(|_| panic!("game"));
    let marker = game.join("controlled-player-data.txt");
    assert!(std::fs::write(&marker, b"preserve player data").is_ok());
    let pending = controlled_attempt(&paths, &accounts, instance, 200).await;
    assert!(!pending.catalog_warning_required());
    let error = pending
        .start(None)
        .await
        .err()
        .unwrap_or_else(|| panic!("synthetic Java must never execute"));
    assert_eq!(error.code(), expected);
    assert!(!format!("{error:?}").contains("fixture-error-body"));
    // The strict server consumes exactly one rejected validation, one renewal
    // chain and one retry. Extra requests cannot turn this into a passing case.
    assert!(server.join().is_ok());
    assert!(
        before
            == store
                .read("accounts.json")
                .unwrap_or_else(|_| panic!("after"))
    );
    let view = serde_json::to_string(&accounts.view().unwrap_or_else(|_| panic!("view")))
        .unwrap_or_else(|_| panic!("view serialization"));
    for secret in [
        "fixture-refresh",
        "fixture-rotated",
        "fixture-minecraft",
        "fixture-microsoft",
        "fixture-xbox",
        "fixture-xsts",
    ] {
        assert!(!view.contains(secret));
    }
    assert_eq!(
        std::fs::read(&marker).unwrap_or_else(|_| panic!("marker")),
        b"preserve player data"
    );
    assert!(LaunchLease::acquire(&paths, instance).is_ok());
    drop(accounts);
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn controlled_persistent_microsoft_rejection_blocks_composed_play_after_one_renewal() {
    controlled_profile_rejection(401, ErrorCode::AuthenticationRequired).await;
}

#[tokio::test]
async fn controlled_microsoft_rejection_recovers_once_before_reaching_process_boundary() {
    controlled_profile_rejection(200, ErrorCode::ProcessStartFailed).await;
}

#[tokio::test]
async fn controlled_catalog_only_outage_requires_fresh_warning_and_still_validates_microsoft() {
    let mut replies = successful_auth_chain();
    replies.push(("/profile", 200, fixture_profile()));
    let (paths, accounts, instance, server) = microsoft_fixture(replies);
    assert!(accounts.renew_selected().await.is_ok());
    let first = controlled_attempt(&paths, &accounts, instance, 503).await;
    assert!(first.catalog_warning_required());
    let stale = first.operation_id();
    assert_eq!(
        first.start(None).await.err().map(|error| error.code()),
        Some(ErrorCode::CatalogConfirmationRequired)
    );
    let second = controlled_attempt(&paths, &accounts, instance, 503).await;
    assert_ne!(second.operation_id(), stale);
    assert_eq!(
        second
            .start(Some(stale))
            .await
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::CatalogConfirmationRequired)
    );
    let third = controlled_attempt(&paths, &accounts, instance, 503).await;
    let confirmation = third.operation_id();
    // Positive control: authentication stays available during catalog failure.
    // A valid warning reaches the invalid-Java sentinel, never a real game.
    assert_eq!(
        third
            .start(Some(confirmation))
            .await
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::ProcessStartFailed)
    );
    assert!(server.join().is_ok());
    assert!(LaunchLease::acquire(&paths, instance).is_ok());
    drop(accounts);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

fn local() -> CatalogInstance {
    CatalogInstance {
        id: Uuid::parse_str("62d7e99d-fb4e-4167-8732-ce3a334d4a58")
            .unwrap_or_else(|_| panic!("id")),
        slug: "vanilla-actual".into(),
        name: "Prueba".into(),
        description: "Prueba".into(),
        minecraft_version: Some("26.3".into()),
        enabled: true,
        restricted: false,
        min_launcher_version: None,
        icon_url: None,
        banner_url: None,
        sort_order: 10,
        updated_at: "2026-10-13T06:22:29Z".into(),
        source: CatalogSource::Vanilla {
            minecraft_version: "26.3".into(),
        },
    }
}

pub(crate) fn fixture() -> (AppPaths, Arc<AccountController>, Uuid) {
    let paths = AppPaths::new(std::env::temp_dir().join(format!("panita-play-{}", Uuid::new_v4())))
        .unwrap_or_else(|_| panic!("paths"));
    let catalog = local();
    let id = catalog.id;
    InstanceRegistry::new(paths.clone())
        .save(&InstallationRecord {
            schema_version: 1,
            id,
            directory_name: None,
            resolved_source: catalog.source.clone(),
            last_catalog: catalog,
            state: InstallationState::Installed,
            managed_files: vec![],
            settings: InstanceSettings::default(),
        })
        .unwrap_or_else(|_| panic!("registry"));
    let repository =
        AccountRepository::new(PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store")));
    let accounts = Arc::new(
        AccountController::new(repository, ProcessSessionId::new())
            .unwrap_or_else(|_| panic!("accounts")),
    );
    accounts
        .add_offline("Fixture_Player")
        .unwrap_or_else(|_| panic!("offline"));
    (paths, accounts, id)
}

/// A tiny integrity-valid installation with an intentionally non-executable Java
/// placeholder. Tests can exercise preflight without launching a game or shell.
pub(crate) fn complete_installation(paths: &AppPaths, id: Uuid) {
    use crate::minecraft::plan::{
        DocumentIdentity, InstallPlan, PlannedArtifact, asset_index_cache_path,
    };
    use crate::minecraft::{Artifact, VersionMetadata};
    use sha1::{Digest, Sha1};
    let platform = Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform"));
    let mut version =
        serde_json::from_slice::<VersionMetadata>(include_bytes!("../fixtures/26.3.json"))
            .unwrap_or_else(|_| panic!("metadata"))
            .resolve(&platform)
            .unwrap_or_else(|_| panic!("resolve"));
    let artifact = |bytes: &[u8]| Artifact {
        url: "https://piston-data.mojang.com/fixture".into(),
        sha1: format!("{:x}", Sha1::digest(bytes)),
        size: bytes.len() as u64,
        path: None,
    };
    let java = b"synthetic invalid Java executable";
    let client = b"synthetic client jar";
    let assets = br#"{"objects":{}}"#;
    let java_artifact = artifact(java);
    let runtime = serde_json::to_vec(&serde_json::json!({"files":{"bin/java.exe":{"type":"file","executable":true,"downloads":{"raw":java_artifact}}}})).unwrap_or_else(|_| panic!("runtime"));
    let runtime_artifact = artifact(&runtime);
    version.client = artifact(client);
    version.assets.artifact = artifact(assets);
    version.classpath.clear();
    version.natives.clear();
    version.logging = None;
    let runtime_root = "runtime/java-25-windows-x64".to_owned();
    let java_executable = format!("{runtime_root}/bin/java.exe");
    let client_path = format!("versions/26.3/26.3-{}.jar", version.client.sha1);
    let asset_path =
        asset_index_cache_path(&version.assets).unwrap_or_else(|_| panic!("asset path"));
    let runtime_path = format!("{runtime_root}/.runtime-manifest.json");
    let plan = InstallPlan {
        schema_version: 1,
        version,
        platform: "windows".into(),
        architecture: "amd64".into(),
        runtime_root,
        java_executable: java_executable.clone(),
        documents: vec![DocumentIdentity {
            source: runtime_artifact.clone(),
        }],
        artifacts: vec![
            PlannedArtifact {
                destination: java_executable.clone(),
                artifact: java_artifact,
                executable: true,
            },
            PlannedArtifact {
                destination: client_path.clone(),
                artifact: artifact(client),
                executable: false,
            },
            PlannedArtifact {
                destination: asset_path.clone(),
                artifact: artifact(assets),
                executable: false,
            },
            PlannedArtifact {
                destination: runtime_path.clone(),
                artifact: runtime_artifact,
                executable: false,
            },
        ],
    };
    let installed = InstalledLock {
        schema_version: 2,
        instance_id: id,
        operation: Uuid::new_v4(),
        plan,
        native_files: vec![],
        native_directory: None,
    };
    paths
        .game_directory(id)
        .unwrap_or_else(|_| panic!("prepare layout"));
    for (relative, bytes) in [
        (java_executable.clone(), java.as_slice()),
        (client_path, client.as_slice()),
        (asset_path, assets.as_slice()),
        (runtime_path, runtime.as_slice()),
    ] {
        let (directory, leaf) = super::super::download::ensure_parent(paths.root(), &relative)
            .unwrap_or_else(|_| panic!("parent"));
        directory
            .write_atomic_durable(&leaf, bytes)
            .unwrap_or_else(|_| panic!("write artifact"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            paths.root().join(java_executable),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap_or_else(|_| panic!("executable protection"));
    }
    let root =
        private_fs::PrivateDir::open(paths.instance(id).unwrap_or_else(|_| panic!("instance")))
            .unwrap_or_else(|_| panic!("instance directory"));
    root.write_atomic_durable(
        "installed.json",
        &serde_json::to_vec(&installed).unwrap_or_else(|_| panic!("manifest")),
    )
    .unwrap_or_else(|_| panic!("write manifest"));
}

#[tokio::test]
async fn complete_offline_attempt_requires_new_warning_confirmation_every_time() {
    let (paths, accounts, id) = fixture();
    complete_installation(&paths, id);
    let mut previous = None;
    for attempt in 0..3 {
        let (catalog, server) = crate::catalog::tests::server(vec![(503, "{}".into(), false)]);
        let pending = prepare_with_catalog(
            paths.clone(),
            id,
            accounts.clone(),
            Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")),
            ProcessSessionId::new(),
            Cancellation::default(),
            catalog,
        )
        .await
        .unwrap_or_else(|e| panic!("preflight: {e}"));
        assert!(server.join().is_ok());
        assert!(pending.catalog_warning_required());
        assert_eq!(
            LaunchLease::acquire(&paths, id).err().map(|e| e.code()),
            Some(ErrorCode::OperationBusy)
        );
        let current = pending.operation_id();
        let confirmation = if attempt == 2 {
            Some(current)
        } else {
            previous
        };
        let result = pending.start(confirmation).await;
        assert_eq!(
            result.err().map(|e| e.code()),
            Some(if attempt == 2 {
                ErrorCode::ProcessStartFailed
            } else {
                ErrorCode::CatalogConfirmationRequired
            })
        );
        previous = Some(current);
        assert!(LaunchLease::acquire(&paths, id).is_ok());
    }
    drop(accounts);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn account_switch_and_cancellation_after_preflight_prevent_spawn() {
    let (paths, accounts, id) = fixture();
    complete_installation(&paths, id);
    for cancel_attempt in [false, true] {
        let response = serde_json::json!({"data":crate::catalog::tests::record()});
        let (catalog, server) =
            crate::catalog::tests::server(vec![(200, response.to_string(), false)]);
        let cancel = Cancellation::default();
        let pending = prepare_with_catalog(
            paths.clone(),
            id,
            accounts.clone(),
            Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")),
            ProcessSessionId::new(),
            cancel.clone(),
            catalog,
        )
        .await
        .unwrap_or_else(|e| panic!("preflight: {e}"));
        assert!(server.join().is_ok());
        assert!(!pending.catalog_warning_required());
        if cancel_attempt {
            cancel.cancel();
        } else {
            let other = accounts
                .add_offline("Other_Player")
                .unwrap_or_else(|_| panic!("offline"));
            accounts
                .select(other.account_id)
                .unwrap_or_else(|_| panic!("switch"));
        }
        assert_eq!(
            pending.start(None).await.err().map(|e| e.code()),
            Some(ErrorCode::OperationCancelled)
        );
        assert!(LaunchLease::acquire(&paths, id).is_ok());
    }
    drop(accounts);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn catalog_outage_needs_warning_but_missing_disabled_and_bad_data_never_fall_back() {
    for code in [ErrorCode::NetworkUnavailable, ErrorCode::ServiceUnavailable] {
        assert_eq!(
            catalog_decision(&local(), Err(CoreError::new(code))).ok(),
            Some(true)
        );
    }
    for code in [
        ErrorCode::InstanceMissing,
        ErrorCode::InvalidData,
        ErrorCode::InstanceDisabled,
    ] {
        assert_eq!(
            catalog_decision(&local(), Err(CoreError::new(code)))
                .err()
                .map(|e| e.code()),
            Some(code)
        );
    }
    let mut disabled = local();
    disabled.enabled = false;
    assert_eq!(
        catalog_decision(
            &disabled,
            Err(CoreError::new(ErrorCode::NetworkUnavailable))
        )
        .err()
        .map(|e| e.code()),
        Some(ErrorCode::InstanceDisabled)
    );
    assert_eq!(
        catalog_decision(&local(), Ok(disabled.clone()))
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::InstanceDisabled)
    );
    assert_eq!(catalog_decision(&disabled, Ok(local())).ok(), Some(false));
    let mut unsupported = local();
    unsupported.source = CatalogSource::Unsupported {
        source_type: "future".into(),
    };
    assert!(
        catalog_decision(
            &unsupported,
            Err(CoreError::new(ErrorCode::NetworkUnavailable))
        )
        .is_err()
    );
    let mut updated_pin = local();
    updated_pin.source = CatalogSource::Vanilla {
        minecraft_version: "other-pin".into(),
    };
    assert_eq!(
        catalog_decision(&local(), Ok(updated_pin)).ok(),
        Some(false)
    );
}

#[tokio::test]
async fn explicit_disabled_detail_is_recorded_and_releases_launch_reservation() {
    let (paths, accounts, id) = fixture();
    let mut response = crate::catalog::tests::record();
    response["enabled"] = serde_json::json!(false);
    let (catalog, server) = crate::catalog::tests::server(vec![(
        200,
        serde_json::json!({"data":response}).to_string(),
        false,
    )]);
    let result = prepare_with_catalog(
        paths.clone(),
        id,
        accounts.clone(),
        Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")),
        ProcessSessionId::new(),
        Cancellation::default(),
        catalog,
    )
    .await;
    assert_eq!(
        result.err().map(|e| e.code()),
        Some(ErrorCode::InstanceDisabled)
    );
    assert!(server.join().is_ok());
    assert!(
        !InstanceRegistry::new(paths.clone())
            .get(id)
            .ok()
            .flatten()
            .unwrap_or_else(|| panic!("record"))
            .last_catalog
            .enabled
    );
    assert!(LaunchLease::acquire(&paths, id).is_ok());
    drop(accounts);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn missing_detail_never_uses_an_installed_fallback() {
    let (paths, accounts, id) = fixture();
    let (catalog, server) = crate::catalog::tests::server(vec![(404, "{}".into(), false)]);
    let result = prepare_with_catalog(
        paths.clone(),
        id,
        accounts.clone(),
        Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")),
        ProcessSessionId::new(),
        Cancellation::default(),
        catalog,
    )
    .await;
    assert_eq!(
        result.err().map(|e| e.code()),
        Some(ErrorCode::InstanceMissing)
    );
    assert!(server.join().is_ok());
    assert!(LaunchLease::acquire(&paths, id).is_ok());
    drop(accounts);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn outage_with_missing_files_never_returns_a_launch_or_confirmation_permit() {
    let (paths, accounts, id) = fixture();
    let (catalog, server) = crate::catalog::tests::server(vec![(503, "{}".into(), false)]);
    let result = prepare_with_catalog(
        paths.clone(),
        id,
        accounts.clone(),
        Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")),
        ProcessSessionId::new(),
        Cancellation::default(),
        catalog,
    )
    .await;
    assert_eq!(result.err().map(|e| e.code()), Some(ErrorCode::InvalidData));
    assert!(server.join().is_ok());
    assert!(LaunchLease::acquire(&paths, id).is_ok());
    drop(accounts);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn cancellation_prevents_the_process_creation_commit() {
    let cancel = Cancellation::default();
    cancel.cancel();
    let mut executed = false;
    let result = cancel.commit(|| {
        executed = true;
        Ok(())
    });
    assert_eq!(
        result.err().map(|e| e.code()),
        Some(ErrorCode::OperationCancelled)
    );
    assert!(!executed);
}

#[test]
fn cancelled_installation_audit_stops_before_reading_artifacts() {
    let (paths, accounts, id) = fixture();
    complete_installation(&paths, id);
    let lease = LaunchLease::acquire(&paths, id).unwrap_or_else(|_| panic!("lease"));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        lease
            .verify_installation_with_cancel(&cancel)
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::OperationCancelled)
    );
    drop(lease);
    drop(accounts);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn microsoft_service_outage_blocks_composed_play_without_transport_retry_or_spawn() {
    use crate::accounts::Secret;
    let (paths, offline_accounts, id) = fixture();
    drop(offline_accounts);
    complete_installation(&paths, id);
    let repository =
        AccountRepository::new(PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store")));
    let refresh = Secret::new("fixture-refresh".into()).unwrap_or_else(|_| panic!("refresh"));
    let account = repository
        .save_microsoft(Uuid::new_v4(), "Fixture_MS", &refresh, None)
        .unwrap_or_else(|_| panic!("microsoft account"));
    let (auth, auth_server) =
        crate::authentication::tests::mock_client(vec![("/token", 503, "{}".into())]);
    let accounts = Arc::new(
        AccountController::with_client(repository, ProcessSessionId::new(), auth)
            .unwrap_or_else(|_| panic!("accounts")),
    );
    accounts
        .select(account.account_id)
        .unwrap_or_else(|_| panic!("select"));
    let before = PrivateStore::open(&paths)
        .unwrap_or_else(|_| panic!("store"))
        .read("accounts.json")
        .unwrap_or_else(|_| panic!("before"));
    let response = serde_json::json!({"data":crate::catalog::tests::record()});
    let (catalog, catalog_server) =
        crate::catalog::tests::server(vec![(200, response.to_string(), false)]);
    let pending = prepare_with_catalog(
        paths.clone(),
        id,
        accounts.clone(),
        Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")),
        ProcessSessionId::new(),
        Cancellation::default(),
        catalog,
    )
    .await
    .unwrap_or_else(|e| panic!("preflight: {e}"));
    assert!(catalog_server.join().is_ok());
    // Reaching the synthetic Java executable would return ProcessStartFailed.
    assert_eq!(
        pending.start(None).await.err().map(|e| e.code()),
        Some(ErrorCode::ServiceUnavailable)
    );
    assert!(auth_server.join().is_ok());
    let after = PrivateStore::open(&paths)
        .unwrap_or_else(|_| panic!("store"))
        .read("accounts.json")
        .unwrap_or_else(|_| panic!("after"));
    assert!(before == after);
    assert!(LaunchLease::acquire(&paths, id).is_ok());
    drop(accounts);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}
