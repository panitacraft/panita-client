#![allow(clippy::expect_used)]

use super::*;

#[test]
fn maintenance_hint_requires_an_active_owned_manifest_but_not_healthy_cache_files() {
    let (paths, accounts, id) = fixture();
    drop(accounts);
    let root = install::metadata_root(&paths, id).expect("root");
    assert!(!has_installation_manifest(&paths, id).expect("incomplete"));
    complete_installation(&paths, id);
    assert!(has_installation_manifest(&paths, id).expect("installed"));
    let installed: InstalledLock = read_json(&root, "installed.json")
        .expect("read")
        .expect("manifest");
    std::fs::write(
        paths.root().join(&installed.plan.artifacts[0].destination),
        b"corrupt",
    )
    .expect("corrupt cache");
    assert!(has_installation_manifest(&paths, id).expect("repairable"));
    uninstall(&paths, id, &Cancellation::default()).expect("uninstall");
    assert!(!has_installation_manifest(&paths, id).expect("tombstone"));
    let mut foreign = installed;
    foreign.instance_id = Uuid::new_v4();
    write_json(&root, "installed.json", &foreign).expect("foreign manifest");
    assert!(has_installation_manifest(&paths, id).is_err());
    std::fs::remove_dir_all(paths.root()).expect("cleanup");
}
use crate::minecraft::{
    natives::NativeFile,
    play::tests::{complete_installation, fixture},
};

fn setup() -> (AppPaths, Uuid, std::path::PathBuf, InstalledLock) {
    let (paths, accounts, id) = fixture();
    drop(accounts);
    complete_installation(&paths, id);
    let root = install::metadata_root(&paths, id).expect("root");
    let installed = read_json(&root, "installed.json")
        .expect("read")
        .expect("manifest");
    (paths, id, root, installed)
}

fn native(root: &Path, installed: &mut InstalledLock, name: &str, bytes: &[u8]) {
    let directory =
        private_fs::PrivateDir::ensure(root.join(format!("natives-{}", installed.operation)))
            .expect("generation");
    directory.write_atomic_durable(name, bytes).expect("native");
    installed.native_files.push(NativeFile {
        path: name.into(),
        sha1: format!("{:x}", Sha1::digest(bytes)),
        size: bytes.len() as u64,
    });
}

fn intent(id: Uuid, previous: &InstalledLock, kind: Kind) -> Journal {
    Journal {
        schema_version: 3,
        instance_id: id,
        operation: Uuid::new_v4(),
        previous_operation: Some(previous.operation),
        kind,
        active: true,
        prepared: None,
        retained_files: 0,
    }
}

#[test]
fn uninstall_retains_hardlinked_files_and_independent_instance_data() {
    let (paths, id, root, mut installed) = setup();
    native(&root, &mut installed, "linked.dll", b"linked");
    write_json(&root, "installed.json", &installed).expect("manifest");
    let source = root.join(format!("natives-{}/linked.dll", installed.operation));
    let game = paths.game_directory(id).expect("game");
    std::fs::hard_link(&source, game.join("player-owned.bin")).expect("hard link");
    let registry = InstanceRegistry::new(paths.clone());
    let mut other = registry.get(id).expect("registry").expect("record");
    other.id = Uuid::new_v4();
    other.last_catalog.id = other.id;
    other.last_catalog.slug = "independent-instance".into();
    other.schema_version = 1;
    other.directory_name = None;
    registry.save(&other).expect("other instance");
    let other_root = paths.instance(other.id).expect("other root");
    let mut other_manifest = installed.clone();
    other_manifest.instance_id = other.id;
    other_manifest.native_files.clear();
    write_json(&other_root, "installed.json", &other_manifest).expect("other manifest");
    let before = std::fs::read(other_root.join("installed.json")).expect("before");
    let outcome = uninstall(&paths, id, &Cancellation::default()).expect("uninstall");
    assert_eq!(outcome.retained_files, 1);
    assert_eq!(std::fs::read(&source).expect("native retained"), b"linked");
    assert_eq!(
        std::fs::read(game.join("player-owned.bin")).expect("player file"),
        b"linked"
    );
    assert_eq!(
        std::fs::read(other_root.join("installed.json")).expect("after"),
        before
    );
    assert!(install::verify_installed(&paths, other.id).is_ok());
    std::fs::remove_dir_all(paths.root()).expect("cleanup");
}

#[test]
fn uninstall_preserves_player_data_modified_files_identity_and_shared_caches() {
    let (paths, id, root, mut installed) = setup();
    native(&root, &mut installed, "owned.dll", b"verified");
    native(&root, &mut installed, "edited.dll", b"original");
    write_json(&root, "installed.json", &installed).expect("manifest");
    let generation = root.join(format!("natives-{}", installed.operation));
    std::fs::write(generation.join("edited.dll"), b"modified").expect("edit");
    std::fs::write(generation.join("unknown.dll"), b"unknown").expect("unknown");
    let game = paths.game_directory(id).expect("game");
    std::fs::create_dir_all(game.join("saves/world")).expect("world");
    std::fs::write(game.join("saves/world/level.dat"), b"world").expect("world");
    std::fs::write(game.join("options.txt"), b"settings").expect("options");
    let registry = InstanceRegistry::new(paths.clone());
    let before = registry.get(id).expect("registry").expect("record");
    let cache = paths.root().join(&installed.plan.artifacts[0].destination);
    let cached = std::fs::read(&cache).expect("cache");
    let outcome = uninstall(&paths, id, &Cancellation::default()).expect("uninstall");
    assert_eq!(outcome.retained_files, 1);
    assert!(!generation.join("owned.dll").exists());
    assert_eq!(
        std::fs::read(generation.join("edited.dll")).expect("edited"),
        b"modified"
    );
    assert_eq!(
        std::fs::read(generation.join("unknown.dll")).expect("unknown"),
        b"unknown"
    );
    assert_eq!(
        std::fs::read(game.join("saves/world/level.dat")).expect("world"),
        b"world"
    );
    assert_eq!(
        std::fs::read(game.join("options.txt")).expect("options"),
        b"settings"
    );
    assert_eq!(std::fs::read(cache).expect("cache"), cached);
    let after = registry.get(id).expect("registry").expect("record");
    assert_eq!(after.state, InstallationState::NotInstalled);
    assert_eq!(after.directory_name, before.directory_name);
    assert_eq!(
        serde_json::to_value(after.settings).expect("settings"),
        serde_json::to_value(before.settings).expect("settings")
    );
    let tombstone: InstalledLock = read_json(&root, "installed.json")
        .expect("read")
        .expect("tombstone");
    assert_eq!(tombstone.schema_version, 3);
    assert_eq!(tombstone.operation, installed.operation);
    assert_eq!(tombstone.plan.version.id, installed.plan.version.id);
    assert!(install::verify_installed(&paths, id).is_err());
    assert!(uninstall(&paths, id, &Cancellation::default()).is_ok());
    std::fs::remove_dir_all(paths.root()).expect("cleanup");
}

#[test]
fn uninstall_recovery_rolls_forward_before_and_after_tombstone_publication() {
    for published in [false, true] {
        let (paths, id, root, mut installed) = setup();
        native(&root, &mut installed, "owned.dll", b"verified");
        write_json(&root, "installed.json", &installed).expect("manifest");
        let journal = intent(id, &installed, Kind::Uninstall);
        write_json(&root, "install-journal.json", &journal).expect("intent");
        if published {
            assert_eq!(natives(&root, &installed, true).expect("remove"), 0);
            installed.schema_version = 3;
            write_json(&root, "installed.json", &installed).expect("tombstone");
        }
        install::recover_locked(&paths, id, &root).expect("recover");
        install::recover_locked(&paths, id, &root).expect("idempotent");
        assert_eq!(
            InstanceRegistry::new(paths.clone())
                .get(id)
                .expect("registry")
                .expect("record")
                .state,
            InstallationState::NotInstalled
        );
        assert!(
            !root
                .join(format!("natives-{}/owned.dll", installed.operation))
                .exists()
        );
        assert!(
            !read_json::<Journal>(&root, "install-journal.json")
                .expect("read")
                .expect("journal")
                .active
        );
        std::fs::remove_dir_all(paths.root()).expect("cleanup");
    }
}

#[test]
fn repair_recovery_keeps_old_pointer_or_finishes_exact_prepared_generation() {
    for published in [false, true] {
        let (paths, id, root, mut installed) = setup();
        native(&root, &mut installed, "edited.dll", b"original");
        write_json(&root, "installed.json", &installed).expect("manifest");
        let old_file = root.join(format!("natives-{}/edited.dll", installed.operation));
        std::fs::write(&old_file, b"modified").expect("edit");
        let mut journal = intent(id, &installed, Kind::Repair);
        let mut prepared = installed.clone();
        prepared.operation = journal.operation;
        prepared.native_files.clear();
        journal.prepared = Some(prepared.clone());
        write_json(&root, "install-journal.json", &journal).expect("intent");
        if published {
            write_json(&root, "installed.json", &prepared).expect("pointer");
        }
        install::recover_locked(&paths, id, &root).expect("recover");
        let current: InstalledLock = read_json(&root, "installed.json")
            .expect("read")
            .expect("manifest");
        assert_eq!(
            current.operation,
            if published {
                prepared.operation
            } else {
                installed.operation
            }
        );
        assert_eq!(current.plan.version.id, installed.plan.version.id);
        assert_eq!(std::fs::read(&old_file).expect("preserved"), b"modified");
        assert_eq!(
            InstanceRegistry::new(paths.clone())
                .get(id)
                .expect("registry")
                .expect("record")
                .state,
            if published {
                InstallationState::Installed
            } else {
                InstallationState::NeedsRepair
            }
        );
        std::fs::remove_dir_all(paths.root()).expect("cleanup");
    }
}

#[test]
fn maintenance_rejects_busy_cancelled_foreign_and_unsafe_requests_without_removal() {
    let (paths, id, root, mut installed) = setup();
    native(&root, &mut installed, "owned.dll", b"verified");
    write_json(&root, "installed.json", &installed).expect("manifest");
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert_eq!(
        uninstall(&paths, id, &cancelled)
            .err()
            .expect("cancelled")
            .code(),
        ErrorCode::OperationCancelled
    );
    let lease = try_lock(&root, ".installation.lock")
        .expect("lock")
        .expect("lease");
    assert_eq!(
        uninstall(&paths, id, &Cancellation::default())
            .err()
            .expect("busy")
            .code(),
        ErrorCode::OperationBusy
    );
    drop(lease);
    let mut journal = intent(Uuid::new_v4(), &installed, Kind::Uninstall);
    write_json(&root, "install-journal.json", &journal).expect("intent");
    assert!(uninstall(&paths, id, &Cancellation::default()).is_err());
    journal.instance_id = id;
    journal.previous_operation = Some(Uuid::new_v4());
    write_json(&root, "install-journal.json", &journal).expect("intent");
    assert!(uninstall(&paths, id, &Cancellation::default()).is_err());
    assert!(
        root.join(format!("natives-{}/owned.dll", installed.operation))
            .exists()
    );
    installed.native_files[0].path = "../minecraft/options.txt".into();
    assert!(validate_manifest(&installed, id).is_err());
    std::fs::remove_dir_all(paths.root()).expect("cleanup");
}

#[test]
fn broken_published_repair_can_be_recovered_by_explicit_uninstall() {
    let (paths, id, root, installed) = setup();
    let mut journal = intent(id, &installed, Kind::Repair);
    let mut prepared = installed.clone();
    prepared.operation = journal.operation;
    journal.prepared = Some(prepared.clone());
    write_json(&root, "install-journal.json", &journal).expect("intent");
    write_json(&root, "installed.json", &prepared).expect("pointer");
    std::fs::write(
        paths.root().join(&prepared.plan.artifacts[0].destination),
        b"corrupt",
    )
    .expect("corrupt");
    assert_eq!(
        install::recover_locked(&paths, id, &root)
            .expect_err("audit")
            .code(),
        ErrorCode::ArtifactMismatch
    );
    uninstall(&paths, id, &Cancellation::default()).expect("uninstall");
    assert_eq!(
        InstanceRegistry::new(paths.clone())
            .get(id)
            .expect("registry")
            .expect("record")
            .state,
        InstallationState::NotInstalled
    );
    std::fs::remove_dir_all(paths.root()).expect("cleanup");
}

#[tokio::test]
async fn repair_uses_recorded_plan_and_preserves_installation_when_java_probe_fails() {
    let (paths, id, root, installed) = setup();
    let (catalog, server) = crate::catalog::tests::server(vec![(
        200,
        serde_json::json!({"data":crate::catalog::tests::record()}).to_string(),
        false,
    )]);
    let result = repair_with_catalog(
        &paths,
        id,
        &Platform::x64("windows", "10").expect("platform"),
        &Cancellation::default(),
        std::sync::Arc::new(|_| {}),
        catalog,
    )
    .await;
    assert_eq!(
        result.err().expect("synthetic Java must fail").code(),
        ErrorCode::RuntimeUnsupported
    );
    server.join().expect("catalog");
    let current: InstalledLock = read_json(&root, "installed.json")
        .expect("read")
        .expect("manifest");
    assert_eq!(current.operation, installed.operation);
    assert!(install::verify_installed(&paths, id).is_ok());
    std::fs::remove_dir_all(paths.root()).expect("cleanup");
}
