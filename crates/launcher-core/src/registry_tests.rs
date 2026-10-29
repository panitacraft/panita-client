use super::*;
use crate::settings::GlobalSettings;

#[test]
fn installed_loader_hint_reports_the_recorded_version_and_omits_tombstones() {
    use crate::minecraft::{install, maintenance, play::tests};
    let (paths, accounts, id) = tests::fixture();
    drop(accounts);
    tests::complete_installation(&paths, id);
    let mut manifest = maintenance::installation_manifest(&paths, id)
        .unwrap_or_else(|_| panic!("manifest"))
        .unwrap_or_else(|| panic!("installed"));
    assert!(InstalledLoader::from_manifest(&manifest).is_none());
    let reference = crate::neoforge::generation::tests::published_for(
        &paths,
        &manifest.plan.version.id,
        &manifest.plan.version.java,
    )
    .unwrap_or_else(|_| panic!("loader"));
    manifest.plan.loader = Some(reference.into());
    manifest.plan.schema_version = 2;
    let root = paths.instance(id).unwrap_or_else(|_| panic!("root"));
    install::write_json(&root, "installed.json", &manifest).unwrap_or_else(|_| panic!("save"));
    let validated = maintenance::installation_manifest(&paths, id)
        .unwrap_or_else(|_| panic!("validated"))
        .unwrap_or_else(|| panic!("installed"));
    let hint = InstalledLoader::from_manifest(&validated).unwrap_or_else(|| panic!("hint"));
    assert_eq!(
        serde_json::to_value(hint).unwrap_or_else(|_| panic!("serialize")),
        serde_json::json!({"name":"neoforge","version":"21.1.264"})
    );
    manifest.schema_version = 3;
    assert!(InstalledLoader::from_manifest(&manifest).is_none());
    std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
}

fn entry(id: Uuid) -> CatalogInstance {
    CatalogInstance {
        id,
        slug: "vanilla-actual".to_owned(),
        name: "Nombre español".to_owned(),
        description: "Sin cambios".to_owned(),
        minecraft_version: Some("1.21.1".to_owned()),
        enabled: true,
        restricted: false,
        min_launcher_version: None,
        icon_url: None,
        banner_url: None,
        sort_order: 10,
        updated_at: "2026-10-16T00:00:00Z".to_owned(),
        source: CatalogSource::Vanilla {
            minecraft_version: "1.21.1".to_owned(),
        },
    }
}

fn installed(id: Uuid) -> InstallationRecord {
    let last_catalog = entry(id);
    InstallationRecord {
        schema_version: 1,
        id,
        directory_name: None,
        resolved_source: last_catalog.source.clone(),
        last_catalog,
        state: InstallationState::Installed,
        managed_files: vec!["versions/client.jar".to_owned()],
        settings: InstanceSettings::default(),
    }
}

fn snapshot(instances: Vec<CatalogInstance>, stale: bool) -> CatalogSnapshot {
    CatalogSnapshot {
        instances,
        stale,
        cache_warning: None,
    }
}

fn temporary() -> (AppPaths, InstanceRegistry) {
    let paths =
        AppPaths::new(std::env::temp_dir().join(format!("panita-registry-{}", Uuid::new_v4())))
            .unwrap_or_else(|_| panic!("paths"));
    // Match host initialization: the application root is protected before registry use.
    drop(PrivateStore::open(&paths).unwrap_or_else(|_| panic!("root")));
    let registry = InstanceRegistry::new(paths.clone());
    (paths, registry)
}

fn legacy(paths: &AppPaths, record: &InstallationRecord) -> PathBuf {
    let root = private_fs::PrivateDir::ensure(paths.root()).unwrap_or_else(|_| panic!("root"));
    let instances = root
        .ensure_dir("instances")
        .unwrap_or_else(|_| panic!("instances"));
    let instance = instances
        .ensure_dir(record.id.to_string())
        .unwrap_or_else(|_| panic!("instance"));
    instance
        .ensure_dir(".panita")
        .unwrap_or_else(|_| panic!("metadata"));
    let directory = paths.root().join("instances").join(record.id.to_string());
    PrivateStore::open(
        &AppPaths::new(directory.join(".panita")).unwrap_or_else(|_| panic!("paths")),
    )
    .unwrap_or_else(|_| panic!("store"))
    .transaction(LEGACY_RECORD, |_| Ok((formatted_json(record)?, ())))
    .unwrap_or_else(|_| panic!("legacy record"));
    directory
}

#[test]
fn fresh_installation_uses_slug_and_persists_identity_across_catalog_renames() {
    let (paths, registry) = temporary();
    let record = installed(Uuid::new_v4());
    registry.save(&record).unwrap_or_else(|_| panic!("save"));
    let expected = paths.root().join("instances/vanilla-actual");
    assert_eq!(paths.instance(record.id).ok(), Some(expected.clone()));
    assert_eq!(
        paths.instance_settings(record.id).ok(),
        Some(expected.join("instance.json"))
    );
    let mut renamed = record.last_catalog.clone();
    renamed.slug = "renamed-catalog-slug".into();
    renamed.enabled = false;
    registry
        .remember_catalog(&[renamed])
        .unwrap_or_else(|_| panic!("catalog"));
    let records = InstanceRegistry::new(paths.clone())
        .load()
        .unwrap_or_else(|_| panic!("restart"));
    assert_eq!(records[0].schema_version, 4);
    assert_eq!(records[0].directory_name.as_deref(), Some("vanilla-actual"));
    assert_eq!(paths.instance(record.id).ok(), Some(expected));
    assert!(
        !paths
            .root()
            .join("instances")
            .join(record.id.to_string())
            .exists()
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn compact_descriptor_migration_is_locked_and_preserves_identity_settings_and_player_files() {
    let (paths, registry) = temporary();
    let record = installed(Uuid::new_v4());
    registry.save(&record).unwrap_or_else(|_| panic!("save"));
    let root = paths.instance(record.id).unwrap_or_else(|_| panic!("root"));
    let mut legacy = registry
        .get(record.id)
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("record"));
    legacy.schema_version = 3;
    legacy.managed_files = vec!["legacy-owned.txt".into()];
    let private = private_fs::PrivateDir::open(&root).unwrap_or_else(|_| panic!("private root"));
    private
        .write_atomic_durable(
            "instance.json",
            &formatted_json(&legacy).unwrap_or_default(),
        )
        .unwrap_or_else(|_| panic!("legacy"));
    let marker = root.join("minecraft/options.txt");
    std::fs::write(&marker, b"preserve").unwrap_or_else(|_| panic!("player marker"));
    let guard = crate::minecraft::download::try_lock(&root, ".installation.lock")
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("lock"));
    assert_eq!(registry.load().unwrap_or_default()[0].schema_version, 3);
    drop(guard);
    let migrated = registry.load().unwrap_or_default();
    assert_eq!(migrated[0].schema_version, 4);
    assert_eq!(migrated[0].settings, legacy.settings);
    assert_eq!(migrated[0].managed_files, legacy.managed_files);
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("instance.json")).unwrap_or_default())
            .unwrap_or_else(|_| panic!("json"));
    assert_eq!(value["schema"], 4);
    assert!(value["catalog"].get("id").is_none());
    assert!(value.get("directory_name").is_none());
    assert!(value.get("managed_files").is_none());
    let mut duplicate = value.clone();
    duplicate["catalog"]["id"] = record.id.to_string().into();
    assert!(serde_json::from_value::<InstallationRecord>(duplicate).is_err());
    let mut future = value;
    future["schema"] = 999.into();
    assert!(serde_json::from_value::<InstallationRecord>(future).is_err());
    assert_eq!(std::fs::read(marker).unwrap_or_default(), b"preserve");
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn legacy_migration_preserves_worlds_settings_and_operation_metadata() {
    let (paths, registry) = temporary();
    let record = installed(Uuid::new_v4());
    let old = legacy(&paths, &record);
    let files = [
        "saves/Test World/level.dat",
        "options.txt",
        ".panita/installations/generation/natives/library.dll",
    ];
    for file in files {
        let (directory, leaf) = crate::minecraft::download::ensure_parent(&old, file)
            .unwrap_or_else(|_| panic!("parent"));
        directory
            .write_atomic_durable(leaf, file.as_bytes())
            .unwrap_or_else(|_| panic!("sentinel"));
    }
    let settings = InstanceSettings {
        ram_mib: Some(6144),
        ..InstanceSettings::default()
    };
    PrivateStore::open(&AppPaths::new(old.join(".panita")).unwrap_or_else(|_| panic!("paths")))
        .unwrap_or_else(|_| panic!("store"))
        .transaction("settings.json", |_| Ok((formatted_json(&settings)?, ())))
        .unwrap_or_else(|_| panic!("settings"));
    let current = registry
        .prepare_instance(record.id)
        .unwrap_or_else(|_| panic!("migration"));
    assert_eq!(current, paths.root().join("instances/vanilla-actual"));
    assert!(!old.exists());
    for file in files {
        let relocated = if file.starts_with(".panita/") {
            "natives-generation/library.dll".to_owned()
        } else {
            format!("minecraft/{file}")
        };
        assert_eq!(
            std::fs::read(current.join(relocated)).ok().as_deref(),
            Some(file.as_bytes())
        );
    }
    assert_eq!(registry.load_settings(record.id).ok(), Some(settings));
    assert!(!current.join(".panita").exists());
    assert_eq!(registry.prepare_instance(record.id).ok(), Some(current));
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn pending_move_recovers_before_rename_and_after_rename() {
    let (paths, registry) = temporary();
    let mut record = installed(Uuid::new_v4());
    record.schema_version = 2;
    record.directory_name = Some("vanilla-actual".into());
    let old = legacy(&paths, &record);
    let destination = paths.root().join("instances/vanilla-actual");
    // Simulate exit after publishing the destination but before the rename.
    assert_eq!(paths.instance(record.id).ok(), Some(old.clone()));
    registry
        .load()
        .unwrap_or_else(|_| panic!("recover pending move"));
    assert!(!old.exists());
    // After a rename, the same persisted record already describes the final folder.
    assert_eq!(
        InstanceRegistry::new(paths.clone())
            .prepare_instance(record.id)
            .ok(),
        Some(destination)
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn migration_refuses_busy_or_running_instances_and_preserves_discoverability() {
    let (paths, registry) = temporary();
    let mut record = installed(Uuid::new_v4());
    let old = legacy(&paths, &record);
    let guard = crate::minecraft::download::try_lock(&old.join(".panita"), ".installation.lock")
        .unwrap_or_else(|_| panic!("lock"))
        .unwrap_or_else(|| panic!("available"));
    assert_eq!(
        registry
            .prepare_instance(record.id)
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::OperationBusy)
    );
    assert_eq!(registry.load().unwrap_or_default().len(), 1);
    assert_eq!(paths.instance(record.id).ok(), Some(old.clone()));
    drop(guard);
    record.state = InstallationState::Running;
    InstanceRegistry::store_at(&old)
        .unwrap_or_else(|_| panic!("store"))
        .transaction(LEGACY_RECORD, |_| Ok((formatted_json(&record)?, ())))
        .unwrap_or_else(|_| panic!("running record"));
    assert_eq!(
        registry
            .prepare_instance(record.id)
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::OperationBusy)
    );
    assert!(old.exists());
    // Completion can update metadata without moving a still-busy legacy folder.
    record.state = InstallationState::Installed;
    registry
        .save_installation(&record)
        .unwrap_or_else(|_| panic!("finish"));
    registry
        .prepare_instance(record.id)
        .unwrap_or_else(|_| panic!("idle migration"));
    assert!(!old.exists());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn case_collisions_and_windows_reserved_names_never_adopt_unowned_folders() {
    let (paths, registry) = temporary();
    let root = private_fs::PrivateDir::ensure(paths.root()).unwrap_or_else(|_| panic!("root"));
    let instances = root
        .ensure_dir("instances")
        .unwrap_or_else(|_| panic!("instances"));
    instances
        .ensure_dir("VANILLA-ACTUAL")
        .unwrap_or_else(|_| panic!("unowned folder"))
        .write_atomic_durable("owner.txt", b"unmanaged")
        .unwrap_or_else(|_| panic!("sentinel"));
    let a = installed(Uuid::new_v4());
    let b = installed(Uuid::new_v4());
    registry.save(&a).unwrap_or_else(|_| panic!("a"));
    registry.save(&b).unwrap_or_else(|_| panic!("b"));
    for record in [&a, &b] {
        assert_eq!(
            paths.instance(record.id).ok(),
            Some(
                paths
                    .root()
                    .join("instances")
                    .join(format!("vanilla-actual-{}", record.id))
            )
        );
    }
    let mut reserved = installed(Uuid::new_v4());
    reserved.last_catalog.slug = "con".into();
    registry
        .save(&reserved)
        .unwrap_or_else(|_| panic!("reserved name"));
    assert_eq!(
        paths.instance(reserved.id).ok(),
        Some(
            paths
                .root()
                .join("instances")
                .join(format!("con-{}", reserved.id))
        )
    );
    assert_eq!(
        std::fs::read(paths.root().join("instances/VANILLA-ACTUAL/owner.txt"))
            .ok()
            .as_deref(),
        Some(b"unmanaged".as_slice())
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn invalid_or_occupied_pending_destinations_preserve_both_directories() {
    let (paths, registry) = temporary();
    let mut record = installed(Uuid::new_v4());
    record.schema_version = 2;
    record.directory_name = Some("vanilla-actual".into());
    let old = legacy(&paths, &record);
    let instances = private_fs::PrivateDir::open(paths.root().join("instances"))
        .unwrap_or_else(|_| panic!("instances"));
    instances
        .ensure_dir("vanilla-actual")
        .unwrap_or_else(|_| panic!("occupied"))
        .write_atomic_durable("world.txt", b"other owner")
        .unwrap_or_else(|_| panic!("sentinel"));
    assert!(registry.prepare_instance(record.id).is_err());
    assert_eq!(paths.instance(record.id).ok(), Some(old.clone()));
    let (visible, warning) = registry
        .load_for_overview()
        .unwrap_or_else(|_| panic!("visible legacy instance"));
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, record.id);
    assert_eq!(warning, Some(ErrorCode::InvalidData));
    assert_eq!(
        std::fs::read(paths.root().join("instances/vanilla-actual/world.txt"))
            .ok()
            .as_deref(),
        Some(b"other owner".as_slice())
    );
    for name in [
        "../escape",
        "CON",
        "nul",
        "c:/escape",
        "bad.name",
        "lpt1",
        "bad\\path",
    ] {
        record.directory_name = Some(name.into());
        let store = InstanceRegistry::store_at(&old).unwrap_or_else(|_| panic!("store"));
        // Deliberately bypass validation to model corrupt disk metadata.
        store
            .transaction(LEGACY_RECORD, |_| Ok((formatted_json(&record)?, ())))
            .unwrap_or_else(|_| panic!("corrupt metadata"));
        let before = std::fs::read(old.join(".panita/installation.json")).unwrap_or_default();
        assert!(registry.load().is_err());
        assert_eq!(
            std::fs::read(old.join(".panita/installation.json")).ok(),
            Some(before)
        );
    }
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn legacy_directory_without_uuid_metadata_is_preserved_without_adoption() {
    let (paths, registry) = temporary();
    let catalog = entry(Uuid::new_v4());
    let root = private_fs::PrivateDir::ensure(paths.root()).unwrap_or_else(|_| panic!("root"));
    let instance = root
        .ensure_dir("instances")
        .unwrap_or_else(|_| panic!("instances"))
        .ensure_dir(catalog.id.to_string())
        .unwrap_or_else(|_| panic!("legacy"));
    instance
        .ensure_dir(".panita")
        .unwrap_or_else(|_| panic!("metadata"))
        .write_atomic_durable("settings.json", br#"{"schema_version":1,"ram_mib":2048}"#)
        .unwrap_or_else(|_| panic!("settings"));
    assert!(registry.ensure_catalog_instance(&catalog).is_err());
    let original = paths.root().join("instances").join(catalog.id.to_string());
    assert!(original.join(".panita/settings.json").exists());
    assert!(!original.join(".panita/installation.json").exists());
    assert!(!paths.root().join("instances/vanilla-actual").exists());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn duplicate_uuid_records_are_rejected_instead_of_selecting_a_folder() {
    let (paths, registry) = temporary();
    let record = installed(Uuid::new_v4());
    registry.save(&record).unwrap_or_else(|_| panic!("save"));
    legacy(&paths, &record);
    assert!(registry.load().is_err());
    assert!(paths.instance(record.id).is_err());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[cfg(any(windows, unix))]
#[test]
fn redirected_instance_directory_cannot_supply_ownership_metadata() {
    let (paths, registry) = temporary();
    let root = private_fs::PrivateDir::ensure(paths.root()).unwrap_or_else(|_| panic!("root"));
    root.ensure_dir("instances")
        .unwrap_or_else(|_| panic!("instances"));
    let external = root
        .ensure_dir("external")
        .unwrap_or_else(|_| panic!("external"));
    let mut record = installed(Uuid::new_v4());
    record.schema_version = 2;
    record.directory_name = Some("redirected".into());
    external
        .ensure_dir(".panita")
        .unwrap_or_else(|_| panic!("metadata"))
        .write_atomic_durable(LEGACY_RECORD, &formatted_json(&record).unwrap_or_default())
        .unwrap_or_else(|_| panic!("record"));
    let link = paths.root().join("instances").join("redirected");
    let target = paths.root().join("external");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap_or_else(|_| panic!("symlink"));
    #[cfg(windows)]
    assert!(
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap_or_else(|_| panic!("junction"))
            .success()
    );
    let before = std::fs::read(target.join(".panita/installation.json")).unwrap_or_default();
    assert!(registry.load().is_err());
    assert_eq!(
        std::fs::read(target.join(".panita/installation.json")).ok(),
        Some(before)
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn folder_child() {
    let Ok(root) = std::env::var("PANITA_FOLDER_TEST_ROOT") else {
        return;
    };
    let id = std::env::var("PANITA_FOLDER_TEST_ID")
        .ok()
        .and_then(|id| Uuid::parse_str(&id).ok())
        .unwrap_or_else(|| panic!("id"));
    let paths = AppPaths::new(root).unwrap_or_else(|_| panic!("paths"));
    InstanceRegistry::new(paths)
        .save(&installed(id))
        .unwrap_or_else(|_| panic!("save child"));
}

#[test]
fn independent_processes_allocate_same_slug_without_merging() {
    let (paths, registry) = temporary();
    let executable = std::env::current_exe().unwrap_or_else(|_| panic!("executable"));
    let ids = [Uuid::new_v4(), Uuid::new_v4()];
    let mut children = Vec::new();
    for id in ids {
        children.push(
            std::process::Command::new(&executable)
                .args(["--exact", "registry::tests::folder_child"])
                .env("PANITA_FOLDER_TEST_ROOT", paths.root())
                .env("PANITA_FOLDER_TEST_ID", id.to_string())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap_or_else(|_| panic!("spawn")),
        );
    }
    for mut child in children {
        assert!(child.wait().unwrap_or_else(|_| panic!("wait")).success());
    }
    assert_eq!(registry.load().unwrap_or_default().len(), 2);
    let a = paths.instance(ids[0]).unwrap_or_else(|_| panic!("a"));
    let b = paths.instance(ids[1]).unwrap_or_else(|_| panic!("b"));
    assert_ne!(a, b);
    assert!(a.ends_with("vanilla-actual") || b.ends_with("vanilla-actual"));
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn registry_rename_disable_removal_and_pin_changes_preserve_worlds() {
    let (paths, registry) = temporary();
    let id = Uuid::new_v4();
    let record = installed(id);
    assert!(registry.save(&record).is_ok());
    let worlds = paths
        .instance(id)
        .unwrap_or_else(|_| panic!("path"))
        .join("saves");
    assert!(std::fs::create_dir(&worlds).is_ok());
    assert!(std::fs::write(worlds.join("owner-world"), b"user data").is_ok());
    let mut remote = record.last_catalog.clone();
    remote.name = "Nombre nuevo".to_owned();
    remote.slug = "nuevo-slug".to_owned();
    remote.updated_at = "2026-10-17T00:00:00Z".to_owned();
    let local = registry.load().unwrap_or_else(|_| panic!("load"));
    let renamed = reconcile(
        Some(&snapshot(vec![remote.clone()], false)),
        &local,
        &Version::new(0, 1, 0),
    );
    assert_eq!(renamed[0].local_state, InstallationState::Installed);
    remote.enabled = false;
    assert!(registry.remember_catalog(&[remote.clone()]).is_ok());
    let remembered = registry.load().unwrap_or_else(|_| panic!("load"));
    assert!(!remembered[0].last_catalog.enabled);
    let missing = reconcile(
        Some(&snapshot(Vec::new(), false)),
        &remembered,
        &Version::new(0, 1, 0),
    );
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].availability, Availability::Disabled);
    let mut newer = remote.clone();
    newer.enabled = true;
    let stale = reconcile(
        Some(&snapshot(vec![newer.clone()], true)),
        &remembered,
        &Version::new(0, 1, 0),
    );
    assert_eq!(stale[0].availability, Availability::Disabled);
    newer.minecraft_version = Some("26.3".to_owned());
    newer.source = CatalogSource::Vanilla {
        minecraft_version: "26.3".to_owned(),
    };
    let update = reconcile(
        Some(&snapshot(vec![newer], false)),
        &remembered,
        &Version::new(0, 1, 0),
    );
    assert_eq!(update[0].local_state, InstallationState::UpdateAvailable);
    assert_eq!(update[0].installed_version.as_deref(), Some("1.21.1"));
    assert_eq!(
        std::fs::read(worlds.join("owner-world")).unwrap_or_default(),
        b"user data"
    );
    assert_eq!(remembered[0].resolved_source, record.resolved_source);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn registry_same_pin_has_independent_ids_settings_and_corruption_is_preserved() {
    let (paths, registry) = temporary();
    let a = installed(Uuid::new_v4());
    let b = installed(Uuid::new_v4());
    assert!(registry.save(&a).is_ok() && registry.save(&b).is_ok());
    assert_ne!(paths.instance(a.id).ok(), paths.instance(b.id).ok());
    let settings = InstanceSettings {
        ram_mib: Some(2048),
        ..InstanceSettings::default()
    };
    assert!(registry.save_settings(a.id, &settings).is_ok());
    assert_eq!(
        registry
            .load_settings(a.id)
            .unwrap_or_else(|_| panic!("settings"))
            .effective(&GlobalSettings::default())
            .ram_mib,
        2048
    );
    assert_eq!(
        registry
            .load_settings(b.id)
            .unwrap_or_else(|_| panic!("settings"))
            .effective(&GlobalSettings::default())
            .ram_mib,
        4096
    );
    let store = registry.store(a.id).unwrap_or_else(|_| panic!("store"));
    assert!(
        store
            .transaction(RECORD, |_| Ok((b"corrupt".to_vec(), ())))
            .is_ok()
    );
    assert!(registry.load().is_err());
    assert!(registry.save(&a).is_err());
    assert_eq!(
        store.read(RECORD).unwrap_or_default(),
        Some(b"corrupt".to_vec())
    );
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn registry_rejects_managed_path_escape_and_mismatched_identity_before_writing() {
    let (paths, registry) = temporary();
    let mut record = installed(Uuid::new_v4());
    for file in [
        "../world",
        "/absolute",
        "C:/outside",
        "versions\\escape",
        ".panita/settings.json",
        "a//b",
    ] {
        record.managed_files = vec![file.to_owned()];
        assert!(registry.save(&record).is_err());
    }
    assert!(paths.instance(record.id).is_err());
    record.managed_files.clear();
    record.last_catalog.id = Uuid::new_v4();
    assert!(registry.save(&record).is_err());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn registry_removed_and_offline_views_do_not_invent_installations() {
    let id = Uuid::new_v4();
    let local = vec![installed(id)];
    let removed = reconcile(
        Some(&snapshot(Vec::new(), false)),
        &local,
        &Version::new(0, 1, 0),
    );
    assert_eq!(removed[0].availability, Availability::Missing);
    let unverified = reconcile(None, &local, &Version::new(0, 1, 0));
    assert_eq!(unverified[0].availability, Availability::Unverified);
    let mut catalog = entry(id);
    catalog.min_launcher_version = Some("9.0.0".to_owned());
    let views = reconcile(
        Some(&snapshot(vec![catalog], false)),
        &[],
        &Version::new(0, 1, 0),
    );
    assert!(!views[0].compatible);
    assert_eq!(views[0].local_state, InstallationState::NotInstalled);
}
