use super::*;
use std::sync::Arc;

const TARGET: &str = "26.3-update-fixture";

#[test]
fn updating_one_instance_retains_other_instance_files_settings_and_shared_cache_references() {
    let (paths, id, root, previous) = fixture();
    let registry = InstanceRegistry::new(paths.clone());
    let mut other = registry
        .get(id)
        .unwrap_or_else(|_| panic!("record"))
        .unwrap_or_else(|| panic!("record"));
    other.id = Uuid::new_v4();
    other.last_catalog.id = other.id;
    other.last_catalog.slug = "other-instance".into();
    other.directory_name = None;
    other.schema_version = 1;
    registry
        .save(&other)
        .unwrap_or_else(|_| panic!("other record"));
    let other_root = paths
        .instance(other.id)
        .unwrap_or_else(|_| panic!("other path"));
    let mut other_manifest = previous.clone();
    other_manifest.instance_id = other.id;
    write_json(&other_root, "installed.json", &other_manifest)
        .unwrap_or_else(|_| panic!("other manifest"));
    std::fs::create_dir_all(other_root.join("minecraft/saves/Other"))
        .unwrap_or_else(|_| panic!("other world"));
    std::fs::write(
        other_root.join("minecraft/saves/Other/level.dat"),
        b"independent world",
    )
    .unwrap_or_else(|_| panic!("other world data"));
    registry
        .save_settings(
            other.id,
            &crate::settings::InstanceSettings {
                ram_mib: Some(10240),
                ..Default::default()
            },
        )
        .unwrap_or_else(|_| panic!("other settings"));
    let before = std::fs::read(other_root.join("installed.json")).unwrap_or_default();
    let mut journal = staging(&paths, id, &previous);
    write_json(&root, "install-journal.json", &journal).unwrap_or_else(|_| panic!("intent"));
    publish(&paths, id, &root, &mut journal).unwrap_or_else(|_| panic!("publish"));
    assert_eq!(
        install::verify_installed(&paths, id)
            .ok()
            .map(|installed| installed.plan.version.id),
        Some(TARGET.into())
    );
    assert_eq!(
        install::verify_installed(&paths, other.id)
            .ok()
            .map(|installed| installed.plan.version.id),
        Some("26.3".into())
    );
    assert_eq!(
        std::fs::read(other_root.join("installed.json")).ok(),
        Some(before)
    );
    assert_eq!(
        std::fs::read(other_root.join("minecraft/saves/Other/level.dat"))
            .ok()
            .as_deref(),
        Some(b"independent world".as_slice())
    );
    assert_eq!(
        registry
            .load_settings(other.id)
            .ok()
            .and_then(|settings| settings.ram_mib),
        Some(10240)
    );
    preserved(&paths, id, &root);
}

fn fixture() -> (AppPaths, Uuid, std::path::PathBuf, InstalledLock) {
    let (paths, accounts, id) = crate::minecraft::play::tests::fixture();
    drop(accounts);
    crate::minecraft::play::tests::complete_installation(&paths, id);
    let root = paths.instance(id).unwrap_or_else(|_| panic!("instance"));
    std::fs::create_dir_all(root.join("minecraft/saves/Existing World"))
        .unwrap_or_else(|_| panic!("world"));
    std::fs::write(
        root.join("minecraft/saves/Existing World/level.dat"),
        b"original world",
    )
    .unwrap_or_else(|_| panic!("world data"));
    std::fs::write(root.join("minecraft/options.txt"), b"original options")
        .unwrap_or_else(|_| panic!("options"));
    let registry = InstanceRegistry::new(paths.clone());
    registry
        .save_settings(
            id,
            &crate::settings::InstanceSettings {
                ram_mib: Some(6144),
                ..Default::default()
            },
        )
        .unwrap_or_else(|_| panic!("settings"));
    let previous =
        install::verify_installed(&paths, id).unwrap_or_else(|error| panic!("previous: {error}"));
    (paths, id, root, previous)
}

fn staging(paths: &AppPaths, id: Uuid, previous: &InstalledLock) -> Journal {
    let mut prepared = previous.clone();
    prepared.operation = Uuid::new_v4();
    let old_client = prepared
        .plan
        .client_path()
        .unwrap_or_else(|_| panic!("old client"));
    prepared.plan.version.id = TARGET.into();
    let new_client = format!(
        "versions/{TARGET}/{TARGET}-{}.jar",
        prepared.plan.version.client.sha1
    );
    for file in &mut prepared.plan.artifacts {
        if file.destination == old_client {
            file.destination = new_client.clone();
        }
    }
    let (dir, leaf) = super::super::download::ensure_parent(paths.root(), &new_client)
        .unwrap_or_else(|_| panic!("target client parent"));
    dir.write_atomic_durable(
        leaf,
        &std::fs::read(paths.root().join(old_client)).unwrap_or_else(|_| panic!("client")),
    )
    .unwrap_or_else(|_| panic!("target client"));
    prepared
        .plan
        .validate()
        .unwrap_or_else(|error| panic!("target plan: {error}"));
    Journal {
        schema_version: 2,
        instance_id: id,
        operation: prepared.operation,
        previous_operation: previous.operation,
        from_version: previous.plan.version.id.clone(),
        to_version: TARGET.into(),
        active: true,
        backup_requested: false,
        backup: None,
        prepared: Some(prepared),
    }
}

fn preserved(paths: &AppPaths, id: Uuid, root: &Path) {
    assert_eq!(
        std::fs::read(root.join("minecraft/saves/Existing World/level.dat"))
            .ok()
            .as_deref(),
        Some(b"original world".as_slice())
    );
    assert_eq!(
        std::fs::read(root.join("minecraft/options.txt"))
            .ok()
            .as_deref(),
        Some(b"original options".as_slice())
    );
    assert_eq!(
        InstanceRegistry::new(paths.clone())
            .load_settings(id)
            .ok()
            .and_then(|settings| settings.ram_mib),
        Some(6144)
    );
}

#[test]
fn interruption_at_each_commit_boundary_recovers_without_world_or_settings_changes() {
    for interrupted in [
        CommitStage::BeforePointer,
        CommitStage::PointerPublished,
        CommitStage::RecordPublished,
        CommitStage::JournalCompleted,
    ] {
        let (paths, id, root, previous) = fixture();
        let old_bytes =
            std::fs::read(root.join("installed.json")).unwrap_or_else(|_| panic!("old manifest"));
        let mut journal = staging(&paths, id, &previous);
        let guard = try_lock(&root, ".installation.lock")
            .unwrap_or_else(|_| panic!("lock"))
            .unwrap_or_else(|| panic!("lease"));
        write_json(&root, "install-journal.json", &journal).unwrap_or_else(|_| panic!("intent"));
        assert!(
            publish_steps(&paths, id, &root, &mut journal, |stage| {
                if stage == interrupted {
                    Err(CoreError::new(ErrorCode::StorageUnavailable))
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        recover_locked(&paths, id, &root).unwrap_or_else(|error| panic!("recovery: {error}"));
        recover_locked(&paths, id, &root).unwrap_or_else(|_| panic!("idempotent recovery"));
        let saved = read_json::<Journal>(&root, "install-journal.json")
            .unwrap_or_default()
            .unwrap_or_else(|| panic!("journal"));
        assert!(!saved.active && saved.prepared.is_none());
        drop(guard);
        let installed = install::verify_installed(&paths, id)
            .unwrap_or_else(|error| panic!("verified recovery: {error}"));
        if interrupted == CommitStage::BeforePointer {
            assert_eq!(installed.operation, previous.operation);
            assert_eq!(
                std::fs::read(root.join("installed.json")).ok(),
                Some(old_bytes)
            );
        } else {
            assert_eq!(installed.plan.version.id, TARGET);
            assert_eq!(
                InstanceRegistry::new(paths.clone())
                    .get(id)
                    .ok()
                    .flatten()
                    .map(|record| record.resolved_source),
                Some(CatalogSource::Vanilla {
                    minecraft_version: TARGET.into()
                })
            );
        }
        preserved(&paths, id, &root);
        assert!(
            paths
                .root()
                .join(previous.plan.client_path().unwrap_or_default())
                .is_file()
        );
    }
}

#[test]
fn cancellation_before_publication_retains_old_installation_and_completed_world_backup() {
    let (paths, id, root, previous) = fixture();
    let mut journal = staging(&paths, id, &previous);
    journal.backup_requested = true;
    let cancel = Cancellation::default();
    journal.backup =
        world_backup::create(&root, id, &journal.from_version, journal.operation, &cancel)
            .unwrap_or_else(|error| panic!("backup: {error}"));
    assert!(journal.backup.is_some());
    write_json(&root, "install-journal.json", &journal).unwrap_or_else(|_| panic!("intent"));
    cancel.cancel();
    assert_eq!(
        cancel
            .commit(|| publish(&paths, id, &root, &mut journal))
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::OperationCancelled)
    );
    recover_locked(&paths, id, &root).unwrap_or_else(|_| panic!("recovery"));
    assert_eq!(
        install::verify_installed(&paths, id)
            .ok()
            .map(|installed| installed.operation),
        Some(previous.operation)
    );
    assert!(
        root.join(format!("world-backups/{}/worlds.zip", journal.operation))
            .is_file()
    );
    preserved(&paths, id, &root);
}

#[test]
fn failed_preparation_and_foreign_or_corrupt_journals_never_replace_the_previous_manifest() {
    let (paths, id, root, previous) = fixture();
    let before = std::fs::read(root.join("installed.json")).unwrap_or_else(|_| panic!("manifest"));
    let mut journal = staging(&paths, id, &previous);
    journal.prepared = None;
    write_json(&root, "install-journal.json", &journal).unwrap_or_else(|_| panic!("start"));
    recover_locked(&paths, id, &root).unwrap_or_else(|_| panic!("abandon preparation"));
    journal.instance_id = Uuid::new_v4();
    write_json(&root, "install-journal.json", &journal).unwrap_or_else(|_| panic!("foreign"));
    let foreign = std::fs::read(root.join("install-journal.json")).unwrap_or_default();
    assert!(recover_locked(&paths, id, &root).is_err());
    assert_eq!(
        std::fs::read(root.join("install-journal.json")).ok(),
        Some(foreign)
    );
    private_fs::PrivateDir::open(&root)
        .unwrap_or_else(|_| panic!("root"))
        .write_atomic_durable("install-journal.json", b"{broken")
        .unwrap_or_else(|_| panic!("corrupt"));
    assert!(install::recover_locked(&paths, id, &root).is_err());
    assert_eq!(
        std::fs::read(root.join("installed.json")).ok(),
        Some(before)
    );
    preserved(&paths, id, &root);
}

#[test]
fn corrupt_staged_generation_after_publication_blocks_recovery_instead_of_adopting_it() {
    let (paths, id, root, previous) = fixture();
    let journal = staging(&paths, id, &previous);
    let prepared = journal
        .prepared
        .as_ref()
        .unwrap_or_else(|| panic!("prepared"));
    write_json(&root, "install-journal.json", &journal).unwrap_or_else(|_| panic!("intent"));
    write_json(&root, "installed.json", prepared).unwrap_or_else(|_| panic!("pointer"));
    let client = prepared.plan.client_path().unwrap_or_default();
    let (dir, leaf) = super::super::download::ensure_parent(paths.root(), &client)
        .unwrap_or_else(|_| panic!("parent"));
    dir.write_atomic_durable(leaf, b"corrupt client")
        .unwrap_or_else(|_| panic!("corrupt artifact"));
    assert_eq!(
        recover_locked(&paths, id, &root)
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert!(
        read_json::<Journal>(&root, "install-journal.json")
            .ok()
            .flatten()
            .is_some_and(|journal| journal.active)
    );
    preserved(&paths, id, &root);
}

#[tokio::test]
async fn updates_require_a_current_enabled_catalog_pin_and_do_not_fall_back_offline() {
    for (status, enabled, expected) in [
        (200, false, ErrorCode::InstanceDisabled),
        (404, true, ErrorCode::InstanceMissing),
        (503, true, ErrorCode::ServiceUnavailable),
        (200, true, ErrorCode::UpdateSelectionChanged),
    ] {
        let (paths, id, root, previous) = fixture();
        let mut record = crate::catalog::tests::record();
        record["enabled"] = serde_json::json!(enabled);
        let (client, server) = crate::catalog::tests::server(vec![(
            status,
            serde_json::json!({"data":record}).to_string(),
            false,
        )]);
        let result = update_with_catalog(
            &paths,
            id,
            &Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")),
            UpdateRequest {
                from_version: "26.3".into(),
                to_version: TARGET.into(),
                backup_worlds: true,
            },
            &Cancellation::default(),
            Arc::new(|_| {}),
            client,
        )
        .await;
        assert_eq!(result.err().map(|error| error.code()), Some(expected));
        assert!(server.join().is_ok());
        assert_eq!(
            read_json::<InstalledLock>(&root, "installed.json")
                .ok()
                .flatten()
                .map(|installed| installed.operation),
            Some(previous.operation)
        );
        assert!(!root.join("world-backups").exists());
        preserved(&paths, id, &root);
    }
}
