use super::*;
use crate::settings::GlobalSettings;

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
        resolved_source: last_catalog.source.clone(),
        last_catalog,
        state: InstallationState::Installed,
        managed_files: vec!["versions/client.jar".to_owned()],
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

#[test]
fn registry_rename_disable_removal_and_pin_changes_preserve_worlds() {
    let (paths, registry) = temporary();
    let id = Uuid::new_v4();
    let record = installed(id);
    assert!(registry.save(&record).is_ok());
    let worlds = paths.instance(id).join("saves");
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
    assert_ne!(paths.instance(a.id), paths.instance(b.id));
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
    assert!(!paths.instance(record.id).exists());
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
