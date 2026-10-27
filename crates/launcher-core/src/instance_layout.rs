//! Recoverable migration from .panita metadata and an inline game directory.
//! The caller holds the application root writer lock throughout preparation.
use crate::minecraft::{install::InstalledLock, invalid, safe_path};
use crate::{
    error::{CoreError, ErrorCode},
    registry::{InstallationRecord, InstallationState},
    storage::map_private_error,
};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

const INTENT: &str = "layout-migration.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveEntry {
    source: String,
    destination: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    #[serde(rename = "schema", alias = "schema_version")]
    schema_version: u32,
    record: InstallationRecord,
    game_entries: Vec<String>,
    native_moves: Vec<MoveEntry>,
    discard_files: Vec<String>,
    discard_dirs: Vec<String>,
}

fn io(error: std::io::Error) -> CoreError {
    crate::minecraft::download::io(error)
}

fn read(root: &Path, name: &str) -> Result<Option<Vec<u8>>, CoreError> {
    let directory = private_fs::PrivateDir::open(root).map_err(map_private_error)?;
    let mut file = match directory.open_file(name) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(map_private_error(error)),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(invalid());
    }
    Ok(Some(bytes))
}

fn publish(root: &Path, name: &str, value: &impl Serialize) -> Result<(), CoreError> {
    let bytes = crate::storage_json::pretty(value).map_err(|_| invalid())?;
    if let Some(existing) = read(root, name)? {
        let existing = if name == "installed.json" {
            let installed: InstalledLock =
                serde_json::from_slice(&existing).map_err(|_| invalid())?;
            serde_json::to_value(installed).map_err(|_| invalid())?
        } else {
            serde_json::from_slice::<serde_json::Value>(&existing).map_err(|_| invalid())?
        };
        if existing != serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| invalid())? {
            return Err(invalid());
        }
        return Ok(());
    }
    private_fs::PrivateDir::open(root)
        .map_err(map_private_error)?
        .write_atomic_durable(name, &bytes)
        .map_err(map_private_error)
}

fn names(root: &Path) -> Result<Vec<String>, CoreError> {
    names_with_privacy(root, true)
}

fn names_with_privacy(root: &Path, protected: bool) -> Result<Vec<String>, CoreError> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(root).map_err(io)? {
        let entry = entry.map_err(io)?;
        let name = entry.file_name().into_string().map_err(|_| invalid())?;
        safe_path(&name)?;
        if name.contains('/')
            || names
                .iter()
                .any(|other: &String| other.eq_ignore_ascii_case(&name))
        {
            return Err(invalid());
        }
        verify_entry(&entry.path())?;
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(io)?;
        if protected && metadata.is_dir() {
            private_fs::PrivateDir::open(entry.path()).map_err(map_private_error)?;
        } else if protected {
            private_fs::PrivateDir::open(root)
                .map_err(map_private_error)?
                .open_file(&name)
                .map_err(map_private_error)?;
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// Atomic no-replace is required for both files and directories.
fn move_entry(source: &Path, destination: &Path) -> Result<(), CoreError> {
    let from = source.try_exists().map_err(io)?;
    let to = destination.try_exists().map_err(io)?;
    match (from, to) {
        (false, true) => return verify_entry(destination), // The durable intent owns this destination.
        (true, false) => (),
        #[cfg(windows)]
        (true, true)
            if source.is_file() && same_file::is_same_file(source, destination).map_err(io)? =>
        {
            std::fs::remove_file(source).map_err(io)?;
            return verify_entry(destination);
        }
        _ => return Err(invalid()),
    }
    #[cfg(windows)]
    {
        if source.is_dir() {
            // Windows does not replace directories, including empty directories.
            std::fs::rename(source, destination).map_err(io)?;
        } else {
            // Link creation never replaces a destination. A process interruption
            // before unlinking is recovered only for the same physical file.
            std::fs::hard_link(source, destination).map_err(io)?;
            std::fs::remove_file(source).map_err(io)?;
        }
    }
    #[cfg(target_os = "linux")]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            source,
            rustix::fs::CWD,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| io(error.into()))?;
        sync(source.parent().ok_or_else(invalid)?)?;
        sync(destination.parent().ok_or_else(invalid)?)?;
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    Ok(())
}

fn verify_entry(path: &Path) -> Result<(), CoreError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io)?;
    if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
        return Err(CoreError::new(ErrorCode::UnsafePermissions));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(CoreError::new(ErrorCode::UnsafePermissions));
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyJournal {
    schema_version: u32,
    operation: uuid::Uuid,
    active: bool,
}

fn journal(root: &Path) -> Result<Option<LegacyJournal>, CoreError> {
    let Some(bytes) = read(root, "install-journal.json")? else {
        return Ok(None);
    };
    let journal: LegacyJournal = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if journal.schema_version != 1 || journal.operation.is_nil() {
        return Err(invalid());
    }
    Ok(Some(journal))
}

fn sync(path: &Path) -> Result<(), CoreError> {
    #[cfg(unix)]
    std::fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(io)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn prepare_intent(root: &Path, mut record: InstallationRecord) -> Result<Intent, CoreError> {
    let legacy = root.join(".panita");
    if let Some(bytes) = read(&legacy, "settings.json")? {
        record.settings = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        record.settings.validate()?;
    }
    record.schema_version = 4;
    // Minecraft creates ordinary player files, not private-fs documents. Preserve
    // their ACLs; only launcher metadata requires private-file verification.
    let game_entries: Vec<_> = names_with_privacy(root, false)?
        .into_iter()
        .filter(|name| name != ".panita")
        .collect();
    if game_entries.iter().any(|name| reserved(name)) {
        return Err(invalid());
    }
    let mut intent = Intent {
        schema_version: 1,
        record,
        game_entries,
        native_moves: Vec::new(),
        discard_files: Vec::new(),
        discard_dirs: Vec::new(),
    };
    for name in names(&legacy)? {
        match name.as_str() {
            "installation.json"
            | "settings.json"
            | "lock.json"
            | "install-journal.json"
            | "transaction.json"
            | "verification-cache.json"
            | "natives"
            | ".mutation.lock"
            | ".installation.lock" => {
                // Windows byte-range locks also prohibit reads through another
                // handle. The directory enumeration already verified these files.
                if !name.ends_with(".lock") {
                    read(&legacy, &name)?;
                }
                intent.discard_files.push(format!(".panita/{name}"));
            }
            "installations" => {
                let installations = legacy.join(&name);
                for generation in names(&installations)? {
                    if !crate::registry::valid_directory_name(&generation) {
                        return Err(invalid());
                    }
                    let relative = format!(".panita/installations/{generation}");
                    for item in names(&root.join(&relative))? {
                        match item.as_str() {
                            "plan.json" => {
                                read(&root.join(&relative), &item)?;
                                intent.discard_files.push(format!("{relative}/{item}"));
                            }
                            "natives" => {
                                if names(&root.join(&relative).join(&item))?.is_empty() {
                                    intent.discard_dirs.push(format!("{relative}/natives"));
                                } else {
                                    intent.native_moves.push(MoveEntry {
                                        source: format!("{relative}/natives"),
                                        destination: format!("natives-{generation}"),
                                    });
                                }
                            }
                            _ => return Err(invalid()),
                        }
                    }
                    intent.discard_dirs.push(relative);
                }
                intent.discard_dirs.push(".panita/installations".into());
            }
            _ => return Err(invalid()),
        }
    }
    Ok(intent)
}

pub(crate) fn prepare(
    root: &Path,
    mut record: InstallationRecord,
) -> Result<InstallationRecord, CoreError> {
    let legacy = root.join(".panita");
    if record.schema_version >= 3 && !legacy.try_exists().map_err(io)? {
        private_fs::PrivateDir::open(root.join("minecraft")).map_err(map_private_error)?;
        if record.schema_version == 3 {
            let _operation = crate::minecraft::download::try_lock(root, ".installation.lock")?
                .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
            if record.state == InstallationState::Running {
                return Err(CoreError::new(ErrorCode::OperationBusy));
            }
            record.schema_version = 4;
            let store = crate::storage::PrivateStore::open(&crate::paths::AppPaths::new(root)?)?;
            store.with_lock(|transaction| {
                transaction.write(
                    "instance.json",
                    &serde_json::to_vec_pretty(&record).map_err(|_| invalid())?,
                )
            })?;
        }
        return Ok(record);
    }
    if record.schema_version >= 3 && names(&legacy)?.is_empty() {
        std::fs::remove_dir(&legacy).map_err(io)?;
        sync(root)?;
        return Ok(record);
    }
    if record.state == InstallationState::Running {
        return Err(CoreError::new(ErrorCode::OperationBusy));
    }
    let legacy_guard = crate::minecraft::download::try_lock(&legacy, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    // Clean only abandoned atomic-write temporaries before enumerating owned metadata.
    drop(crate::storage::PrivateStore::open(
        &crate::paths::AppPaths::new(&legacy)?,
    )?);
    let mut intent: Intent = match read(&legacy, INTENT)? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid())?,
        None => {
            let intent = prepare_intent(root, record.clone())?;
            // Validate the installed manifest before moving any player files.
            installed(&legacy, record.id)?;
            journal(&legacy)?;
            publish(&legacy, INTENT, &intent)?;
            intent
        }
    };
    if intent.schema_version != 1
        || intent.record.id != record.id
        || !matches!(intent.record.schema_version, 3 | 4)
        || intent.record.directory_name != record.directory_name
        || intent.record.resolved_source != record.resolved_source
        || intent.game_entries.len() > 10_000
    {
        return Err(invalid());
    }
    intent.record.validate()?;
    for name in &intent.game_entries {
        safe_path(name)?;
        if name.contains('/') || name.eq_ignore_ascii_case(".panita") || reserved(name) {
            return Err(invalid());
        }
    }
    for entry in &intent.native_moves {
        safe_path(&entry.source)?;
        safe_path(&entry.destination)?;
        let parts: Vec<_> = entry.source.split('/').collect();
        if parts.len() != 4
            || parts[0] != ".panita"
            || parts[1] != "installations"
            || !crate::registry::valid_directory_name(parts[2])
            || parts[3] != "natives"
            || entry.destination != format!("natives-{}", parts[2])
        {
            return Err(invalid());
        }
    }
    for path in intent.discard_files.iter().chain(&intent.discard_dirs) {
        safe_path(path)?;
        let parts: Vec<_> = path.split('/').collect();
        let metadata = parts.len() == 2
            && parts[0] == ".panita"
            && matches!(
                parts[1],
                "installation.json"
                    | "settings.json"
                    | "lock.json"
                    | "install-journal.json"
                    | ".mutation.lock"
                    | ".installation.lock"
            );
        let generation = parts.len() >= 3
            && parts[0] == ".panita"
            && parts[1] == "installations"
            && crate::registry::valid_directory_name(parts[2])
            && (parts.len() == 3
                || (parts.len() == 4 && matches!(parts[3], "plan.json" | "natives")));
        if !metadata && !generation && path != ".panita/installations" {
            return Err(invalid());
        }
    }
    private_fs::PrivateDir::open(root)
        .map_err(map_private_error)?
        .ensure_dir("minecraft")
        .map_err(map_private_error)?;
    for name in &intent.game_entries {
        move_entry(&root.join(name), &root.join("minecraft").join(name))?;
    }
    let _flat_guard = crate::minecraft::download::try_lock(root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    for entry in &intent.native_moves {
        move_entry(&root.join(&entry.source), &root.join(&entry.destination))?;
    }
    if let Some(installed) = installed(&legacy, record.id)? {
        publish(root, "installed.json", &installed)?;
    }
    if let Some(journal) = journal(&legacy)? {
        publish(root, "install-journal.json", &journal)?;
    }
    // Catalog availability can be refreshed between interrupted migration runs.
    // Before legacy record removal it remains the metadata writer's source;
    // afterwards the committed descriptor is authoritative. Never restore an
    // older enabled state or reset already imported/user-edited overrides.
    let mut descriptor = match read(root, "instance.json")? {
        Some(bytes) => {
            let current: InstallationRecord =
                serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            current.validate()?;
            if !matches!(current.schema_version, 3 | 4)
                || current.id != record.id
                || current.directory_name != record.directory_name
                || current.resolved_source != record.resolved_source
            {
                return Err(invalid());
            }
            current
        }
        None => {
            let mut current = intent.record.clone();
            if let Some(bytes) = read(&legacy, "settings.json")? {
                current.settings = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                current.settings.validate()?;
            }
            current
        }
    };
    descriptor.last_catalog = record.last_catalog;
    descriptor.state = record.state;
    descriptor.managed_files = record.managed_files;
    descriptor.validate()?;
    intent.record = descriptor.clone();
    private_fs::PrivateDir::open(&legacy)
        .map_err(map_private_error)?
        .write_atomic_durable(
            INTENT,
            &crate::storage_json::pretty(&intent).map_err(|_| invalid())?,
        )
        .map_err(map_private_error)?;
    private_fs::PrivateDir::open(root)
        .map_err(map_private_error)?
        .write_atomic_durable(
            "instance.json",
            &serde_json::to_vec_pretty(&descriptor).map_err(|_| invalid())?,
        )
        .map_err(map_private_error)?;
    drop(legacy_guard);
    for relative in &intent.discard_files {
        let path = root.join(relative);
        if path.try_exists().map_err(io)? {
            std::fs::remove_file(path).map_err(io)?;
        }
    }
    let mut dirs: Vec<PathBuf> = intent
        .discard_dirs
        .iter()
        .map(|relative| root.join(relative))
        .collect();
    dirs.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in dirs {
        if path.try_exists().map_err(io)? {
            std::fs::remove_dir(path).map_err(io)?;
        }
    }
    if names(&legacy)? != [INTENT] {
        return Err(invalid());
    }
    sync(&legacy)?;
    std::fs::remove_file(legacy.join(INTENT)).map_err(io)?;
    sync(&legacy)?;
    std::fs::remove_dir(&legacy).map_err(io)?;
    sync(root)?;
    Ok(descriptor)
}

fn reserved(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "minecraft"
            | "instance.json"
            | "installed.json"
            | "install-journal.json"
            | ".installation.lock"
            | ".mutation.lock"
    ) || name.to_ascii_lowercase().starts_with("natives-")
}

fn installed(root: &Path, id: uuid::Uuid) -> Result<Option<InstalledLock>, CoreError> {
    let Some(bytes) = read(root, "lock.json")? else {
        return Ok(None);
    };
    let mut installed: InstalledLock = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if installed.schema_version != 1 || installed.instance_id != id || installed.operation.is_nil()
    {
        return Err(invalid());
    }
    installed.plan.validate()?;
    installed.schema_version = 2;
    Ok(Some(installed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::{CatalogInstance, CatalogSource},
        paths::AppPaths,
        registry::InstanceRegistry,
        settings::InstanceSettings,
        storage::PrivateStore,
    };

    fn legacy() -> (AppPaths, PathBuf, InstallationRecord) {
        let paths = AppPaths::new(
            std::env::temp_dir().join(format!("panita-layout-{}", uuid::Uuid::new_v4())),
        )
        .unwrap_or_else(|_| panic!("paths"));
        let id = uuid::Uuid::new_v4();
        let catalog = CatalogInstance {
            id,
            slug: "layout-test".into(),
            name: "Prueba".into(),
            description: "Prueba".into(),
            minecraft_version: Some("1.21.1".into()),
            enabled: true,
            restricted: false,
            min_launcher_version: None,
            icon_url: None,
            banner_url: None,
            sort_order: 0,
            updated_at: "2026-10-17T00:00:00Z".into(),
            source: CatalogSource::Vanilla {
                minecraft_version: "1.21.1".into(),
            },
        };
        let record = InstallationRecord {
            schema_version: 2,
            id,
            directory_name: Some("layout-test".into()),
            last_catalog: catalog.clone(),
            resolved_source: catalog.source,
            state: InstallationState::Installed,
            managed_files: Vec::new(),
            settings: InstanceSettings::default(),
        };
        let root = private_fs::PrivateDir::ensure(paths.root()).unwrap_or_else(|_| panic!("root"));
        let directory = root
            .ensure_dir("instances")
            .unwrap_or_else(|_| panic!("instances"))
            .ensure_dir("layout-test")
            .unwrap_or_else(|_| panic!("instance"));
        directory
            .ensure_dir(".panita")
            .unwrap_or_else(|_| panic!("legacy"));
        let root = paths.root().join("instances/layout-test");
        let store = PrivateStore::open(
            &AppPaths::new(root.join(".panita")).unwrap_or_else(|_| panic!("legacy paths")),
        )
        .unwrap_or_else(|_| panic!("store"));
        store
            .transaction("installation.json", |_| {
                Ok((serde_json::to_vec(&record).map_err(|_| invalid())?, ()))
            })
            .unwrap_or_else(|_| panic!("record"));
        std::fs::write(root.join("options.txt"), b"player options")
            .unwrap_or_else(|_| panic!("game-created options"));
        (paths, root, record)
    }

    fn intent(root: &Path, record: InstallationRecord) -> Intent {
        let guard =
            crate::minecraft::download::try_lock(&root.join(".panita"), ".installation.lock")
                .unwrap_or_else(|_| panic!("lock"));
        let intent = prepare_intent(root, record).unwrap_or_else(|error| panic!("intent: {error}"));
        publish(&root.join(".panita"), INTENT, &intent).unwrap_or_else(|_| panic!("publish"));
        drop(guard);
        intent
    }

    #[test]
    fn resumes_game_moves_and_committed_metadata_without_resetting_settings() {
        for committed in [false, true] {
            let (paths, root, record) = legacy();
            let settings = InstanceSettings {
                ram_mib: Some(6144),
                ..InstanceSettings::default()
            };
            publish(&root.join(".panita"), "settings.json", &settings)
                .unwrap_or_else(|_| panic!("settings"));
            let intent = intent(&root, record.clone());
            private_fs::PrivateDir::open(&root)
                .unwrap_or_else(|_| panic!("instance"))
                .create_dir("minecraft")
                .unwrap_or_else(|_| panic!("game"));
            move_entry(
                &root.join("options.txt"),
                &root.join("minecraft/options.txt"),
            )
            .unwrap_or_else(|_| panic!("first move"));
            if committed {
                publish(&root, "instance.json", &intent.record)
                    .unwrap_or_else(|_| panic!("descriptor"));
            }
            let registry = InstanceRegistry::new(paths.clone());
            let mut disabled = record.last_catalog.clone();
            disabled.enabled = false;
            registry
                .remember_catalog(&[disabled])
                .unwrap_or_else(|_| panic!("availability update"));
            assert_eq!(
                registry.prepare_instance(record.id).ok(),
                Some(root.clone())
            );
            assert_eq!(registry.load_settings(record.id).ok(), Some(settings));
            assert!(
                registry
                    .get(record.id)
                    .ok()
                    .flatten()
                    .is_some_and(|record| !record.last_catalog.enabled)
            );
            assert_eq!(
                std::fs::read(root.join("minecraft/options.txt"))
                    .ok()
                    .as_deref(),
                Some(b"player options".as_slice())
            );
            assert!(!root.join(".panita").exists());
            assert_eq!(registry.prepare_instance(record.id).ok(), Some(root));
        }
    }

    #[test]
    fn refuses_reserved_names_conflicting_moves_and_corrupt_settings_without_deletion() {
        for conflict in ["minecraft", "instance.json", "settings"] {
            let (paths, root, record) = legacy();
            let directory = private_fs::PrivateDir::open(&root).unwrap_or_else(|_| panic!("root"));
            if conflict == "settings" {
                private_fs::PrivateDir::open(root.join(".panita"))
                    .unwrap_or_else(|_| panic!("legacy"))
                    .write_atomic_durable("settings.json", b"{broken")
                    .unwrap_or_else(|_| panic!("corrupt"));
            } else {
                directory
                    .write_atomic_durable(conflict, b"unowned")
                    .unwrap_or_else(|_| panic!("collision"));
            }
            assert!(
                InstanceRegistry::new(paths)
                    .prepare_instance(record.id)
                    .is_err()
            );
            assert_eq!(
                std::fs::read(root.join("options.txt")).ok().as_deref(),
                Some(b"player options".as_slice())
            );
        }
        let (paths, root, record) = legacy();
        intent(&root, record.clone());
        let game = private_fs::PrivateDir::open(&root)
            .unwrap_or_else(|_| panic!("root"))
            .create_dir("minecraft")
            .unwrap_or_else(|_| panic!("game"));
        game.write_atomic_durable("options.txt", b"other data")
            .unwrap_or_else(|_| panic!("conflict"));
        assert!(
            InstanceRegistry::new(paths)
                .prepare_instance(record.id)
                .is_err()
        );
        assert_eq!(
            std::fs::read(root.join("options.txt")).ok().as_deref(),
            Some(b"player options".as_slice())
        );
        assert_eq!(
            std::fs::read(root.join("minecraft/options.txt"))
                .ok()
                .as_deref(),
            Some(b"other data".as_slice())
        );
    }

    #[test]
    fn malformed_intent_cannot_delete_unowned_metadata() {
        let (paths, root, record) = legacy();
        let mut intent = intent(&root, record.clone());
        intent.discard_files.push(".panita/world.dat".into());
        let directory =
            private_fs::PrivateDir::open(root.join(".panita")).unwrap_or_else(|_| panic!("legacy"));
        directory
            .write_atomic_durable("world.dat", b"preserve")
            .unwrap_or_else(|_| panic!("sentinel"));
        directory
            .write_atomic_durable(INTENT, &serde_json::to_vec(&intent).unwrap_or_default())
            .unwrap_or_else(|_| panic!("corrupt"));
        assert!(
            InstanceRegistry::new(paths)
                .prepare_instance(record.id)
                .is_err()
        );
        assert_eq!(
            std::fs::read(root.join(".panita/world.dat"))
                .ok()
                .as_deref(),
            Some(b"preserve".as_slice())
        );
    }

    #[cfg(windows)]
    #[test]
    fn interrupted_file_link_recovers_only_the_same_physical_file() {
        let (paths, root, record) = legacy();
        intent(&root, record.clone());
        private_fs::PrivateDir::open(&root)
            .unwrap_or_else(|_| panic!("root"))
            .create_dir("minecraft")
            .unwrap_or_else(|_| panic!("game"));
        std::fs::hard_link(root.join("options.txt"), root.join("minecraft/options.txt"))
            .unwrap_or_else(|_| panic!("link"));
        assert!(
            InstanceRegistry::new(paths)
                .prepare_instance(record.id)
                .is_ok()
        );
        assert!(!root.join("options.txt").exists());
        assert_eq!(
            std::fs::read(root.join("minecraft/options.txt"))
                .ok()
                .as_deref(),
            Some(b"player options".as_slice())
        );
    }
}
