//! Explicit, retry-only erasure of one UUID-owned instance. The external intent
//! keeps partially erased data discoverable and prevents replacement installations.
use super::*;
use crate::minecraft::{
    directory_capability,
    download::{Cancellation, try_lock, try_lock_for_erasure},
    install::{InstalledLock, read_json},
    maintenance::validate_manifest,
};
use crate::storage::StoreTransaction;
use cap_fs_ext::{DirExt, MetadataExt};
use cap_std::fs::Dir;
use std::ffi::OsString;

const FILE: &str = "instance-erasures.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    schema: u32,
    #[serde(rename = "type")]
    kind: String,
    entries: Vec<Intent>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub record: InstallationRecord,
    dev: u64,
    ino: u64,
}
fn parse(bytes: Option<Vec<u8>>) -> Result<Vec<Intent>, CoreError> {
    let Some(bytes) = bytes else {
        return Ok(Vec::new());
    };
    let pending: Pending = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if pending.schema != 1 || pending.kind != "erase_instances" || pending.entries.len() > 1000 {
        return Err(invalid());
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for entry in &pending.entries {
        entry.record.validate()?;
        if entry.record.schema_version != 4
            || !ids.insert(entry.record.id)
            || !names.insert(entry.record.directory_name.clone())
        {
            return Err(invalid());
        }
    }
    Ok(pending.entries)
}
pub(super) fn pending(paths: &AppPaths) -> Result<Vec<Intent>, CoreError> {
    use std::io::Read;
    // Registry callers already hold the writer lock. Opening PrivateStore here
    // would reacquire it while cleaning temporaries and deadlock those callers.
    let directory = private_fs::PrivateDir::open(paths.root()).map_err(map_private_error)?;
    let mut file = match directory.open_file(FILE) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(map_private_error(error)),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid());
    }
    parse(Some(bytes))
}
fn save(transaction: &StoreTransaction<'_>, entries: Vec<Intent>) -> Result<(), CoreError> {
    let empty = entries.is_empty();
    transaction.write(
        FILE,
        &formatted_json(&Pending {
            schema: 1,
            kind: "erase_instances".into(),
            entries,
        })?,
    )?;
    if empty {
        transaction.remove_verified_file(FILE)?;
    }
    Ok(())
}
/// Called without taking the profile writer lock, including by queued installers.
pub(crate) fn blocks_root(root: &Path) -> Result<bool, CoreError> {
    let Some(parent) = root
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == "instances"))
    else {
        return Ok(false);
    };
    let paths = AppPaths::new(parent.parent().ok_or_else(invalid)?)?;
    Ok(pending(&paths)?.iter().any(|entry| {
        root.file_name().and_then(|name| name.to_str()) == entry.record.directory_name.as_deref()
    }))
}
fn names(directory: &Dir, budget: &mut usize) -> Result<Vec<OsString>, CoreError> {
    let mut result = Vec::new();
    for entry in directory.entries().map_err(io)? {
        *budget = budget.checked_sub(1).ok_or_else(invalid)?;
        result.push(entry.map_err(io)?.file_name());
    }
    Ok(result)
}
// Preflight and deletion both use retained handles and reject redirected entries.
fn walk(directory: &Dir, depth: usize, budget: &mut usize, erase: bool) -> Result<(), CoreError> {
    if depth > 64 {
        return Err(invalid());
    }
    for name in names(directory, budget)? {
        if depth == 0 && (name == ".installation.lock" || name == ".mutation.lock") {
            continue;
        }
        let metadata = directory.symlink_metadata(&name).map_err(io)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid());
        }
        if metadata.is_dir() {
            let child = directory.open_dir_nofollow(&name).map_err(io)?;
            let identity = child.dir_metadata().map_err(io)?;
            walk(&child, depth + 1, budget, erase)?;
            let current = directory.symlink_metadata(&name).map_err(io)?;
            if current.file_type().is_symlink()
                || current.dev() != identity.dev()
                || current.ino() != identity.ino()
            {
                return Err(invalid());
            }
            drop(child);
            if erase {
                directory.remove_dir(&name).map_err(io)?;
            }
        } else if metadata.is_file() {
            if erase {
                directory.remove_file(&name).map_err(io)?;
            }
        } else {
            return Err(invalid());
        }
    }
    Ok(())
}
impl InstanceRegistry {
    /// Requires a separately confirmed complete data erasure. Never runs at startup.
    pub fn erase_instance(&self, id: Uuid, cancel: &Cancellation) -> Result<(), CoreError> {
        cancel.check()?;
        if !pending(&self.paths)?
            .iter()
            .any(|entry| entry.record.id == id)
        {
            self.prepare_instance(id)?;
        }
        PrivateStore::open(&self.paths)?.with_lock(|transaction| {
            let mut intents = parse(transaction.read(FILE)?)?;
            let previous = intents.iter().find(|entry| entry.record.id == id).cloned();
            let record = match &previous {
                Some(intent) => intent.record.clone(),
                None => self
                    .scan()?
                    .into_iter()
                    .find(|(_, record)| record.id == id)
                    .map(|(_, record)| record)
                    .ok_or_else(invalid)?,
            };
            let name = record.directory_name.as_deref().ok_or_else(invalid)?;
            if record.schema_version != 4 || !valid_directory_name(name) {
                return Err(invalid());
            }
            let root = self.paths.root().join("instances").join(name);
            let profile =
                private_fs::PrivateDir::open(self.paths.root()).map_err(map_private_error)?;
            let parent_private = profile.open_dir("instances").map_err(map_private_error)?;
            let parent = directory_capability(&parent_private)?;
            match parent.symlink_metadata(name) {
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && previous.is_some() =>
                {
                    intents.retain(|entry| entry.record.id != id);
                    return save(transaction, intents);
                }
                Err(error) => return Err(io(error)),
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(invalid());
                }
                Ok(_) => (),
            }
            let private = private_fs::PrivateDir::open(&root).map_err(map_private_error)?;
            let directory = directory_capability(&private)?;
            let identity = directory.dir_metadata().map_err(io)?;
            if previous
                .as_ref()
                .is_some_and(|intent| intent.dev != identity.dev() || intent.ino != identity.ino())
            {
                return Err(invalid());
            }
            let lease = try_lock_for_erasure(&root)?
                .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
            let mutation = try_lock(&root, ".mutation.lock")?
                .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
            if previous.is_none() {
                for file in [
                    "transaction.json",
                    "install-journal.json",
                    "layout-migration.json",
                ] {
                    match directory.symlink_metadata(file) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                        _ => return Err(invalid()),
                    }
                }
                // Unknown or future metadata must not become permission to erase.
                if let Some(manifest) = read_json::<InstalledLock>(&root, "installed.json")? {
                    validate_manifest(&manifest, id)?;
                }
            }
            walk(&directory, 0, &mut 1_000_000, false)?;
            cancel.commit(|| {
                if previous.is_none() {
                    intents.push(Intent {
                        record: record.clone(),
                        dev: identity.dev(),
                        ino: identity.ino(),
                    });
                    save(transaction, intents.clone())?;
                }
                walk(&directory, 0, &mut 1_000_000, true)?;
                drop(mutation);
                drop(lease);
                for lock in [".mutation.lock", ".installation.lock"] {
                    let metadata = directory.symlink_metadata(lock).map_err(io)?;
                    if metadata.file_type().is_symlink() || !metadata.is_file() {
                        return Err(invalid());
                    }
                    directory.remove_file(lock).map_err(io)?;
                }
                let current = parent.symlink_metadata(name).map_err(io)?;
                if current.file_type().is_symlink()
                    || current.dev() != identity.dev()
                    || current.ino() != identity.ino()
                {
                    return Err(invalid());
                }
                drop(directory);
                drop(private);
                parent.remove_dir(name).map_err(io)?;
                intents.retain(|entry| entry.record.id != id);
                save(transaction, intents)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        paths: AppPaths,
        id: Uuid,
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let (paths, accounts, id) = crate::minecraft::play::tests::fixture();
            drop(accounts);
            crate::minecraft::play::tests::complete_installation(&paths, id);
            let root = paths
                .instance(id)
                .unwrap_or_else(|_| panic!("fixture instance"));
            Self { paths, id, root }
        }
        fn registry(&self) -> InstanceRegistry {
            InstanceRegistry::new(self.paths.clone())
        }
        fn intent(&self) {
            let record = self
                .registry()
                .get(self.id)
                .unwrap_or_else(|_| panic!("record"))
                .unwrap_or_else(|| panic!("owned"));
            let private =
                private_fs::PrivateDir::open(&self.root).unwrap_or_else(|_| panic!("private"));
            let identity = directory_capability(&private)
                .unwrap_or_else(|_| panic!("capability"))
                .dir_metadata()
                .unwrap_or_else(|_| panic!("identity"));
            PrivateStore::open(&self.paths)
                .unwrap_or_else(|_| panic!("store"))
                .with_lock(|transaction| {
                    save(
                        transaction,
                        vec![Intent {
                            record,
                            dev: identity.dev(),
                            ino: identity.ino(),
                        }],
                    )
                })
                .unwrap_or_else(|_| panic!("intent"));
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.paths.root());
        }
    }
    #[test]
    fn erase_removes_worlds_backups_and_metadata_but_preserves_siblings_and_shared_data() {
        let f = Fixture::new();
        let sibling = f.paths.root().join("instances/unowned-player-folder");
        std::fs::create_dir_all(&sibling).unwrap_or_else(|_| panic!("sibling"));
        std::fs::write(sibling.join("keep.txt"), b"other instance")
            .unwrap_or_else(|_| panic!("canary"));
        let shared = f.paths.root().join("shared-canary.txt");
        std::fs::write(&shared, b"shared data").unwrap_or_else(|_| panic!("shared"));
        std::fs::create_dir_all(f.root.join("minecraft/saves/world"))
            .unwrap_or_else(|_| panic!("world"));
        std::fs::write(f.root.join("minecraft/saves/world/level.dat"), b"world")
            .unwrap_or_else(|_| panic!("world"));
        std::fs::hard_link(&shared, f.root.join("minecraft/linked.txt"))
            .unwrap_or_else(|_| panic!("hard link"));
        f.registry()
            .erase_instance(f.id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("erase"));
        assert!(!f.root.exists());
        assert_eq!(
            std::fs::read(&shared).unwrap_or_else(|_| panic!("shared retained")),
            b"shared data"
        );
        assert_eq!(
            std::fs::read(sibling.join("keep.txt")).unwrap_or_else(|_| panic!("sibling retained")),
            b"other instance"
        );
        assert!(f.paths.accounts().exists());
        assert!(!f.paths.root().join(FILE).exists());
    }
    #[test]
    fn erase_refuses_busy_instances_and_precommit_cancellation() {
        let f = Fixture::new();
        let lease = try_lock(&f.root, ".installation.lock")
            .unwrap_or_else(|_| panic!("lock"))
            .unwrap_or_else(|| panic!("idle"));
        assert_eq!(
            f.registry()
                .erase_instance(f.id, &Cancellation::default())
                .err()
                .unwrap_or_else(|| panic!("busy"))
                .code(),
            ErrorCode::OperationBusy
        );
        drop(lease);
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(f.registry().erase_instance(f.id, &cancel).is_err());
        assert!(f.root.join("installed.json").exists());
        assert!(!f.paths.root().join(FILE).exists());
    }
    #[test]
    fn pending_erasure_blocks_normal_operations_and_only_explicit_retry_finishes_it() {
        let f = Fixture::new();
        f.intent();
        std::fs::remove_file(f.root.join("installed.json"))
            .unwrap_or_else(|_| panic!("simulate partial deletion"));
        let records = f
            .registry()
            .load()
            .unwrap_or_else(|_| panic!("discover pending instance"));
        assert_eq!(records[0].state, InstallationState::Failed);
        assert!(f.registry().prepare_instance(f.id).is_err());
        assert!(try_lock(&f.root, ".installation.lock").is_err());
        assert!(f.root.join("minecraft").exists());
        f.registry()
            .erase_instance(f.id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("explicit retry"));
        assert!(!f.root.exists());
        assert!(!f.paths.root().join(FILE).exists());
    }
    #[test]
    fn absent_completed_folder_stays_discoverable_until_explicit_marker_retirement() {
        let f = Fixture::new();
        f.intent();
        std::fs::remove_dir_all(&f.root)
            .unwrap_or_else(|_| panic!("simulate final removal before marker retirement"));
        assert_eq!(
            f.registry()
                .load()
                .unwrap_or_else(|_| panic!("discover intent"))
                .len(),
            1
        );
        assert!(!f.root.exists());
        f.registry()
            .erase_instance(f.id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("retire intent"));
        assert!(!f.root.exists());
        assert!(!f.paths.root().join(FILE).exists());
    }
    #[test]
    fn unknown_intents_transactions_and_future_manifests_are_preserved() {
        let f = Fixture::new();
        let bytes = b"{\"schema\":999,\"type\":\"erase_instances\",\"entries\":[]}";
        PrivateStore::open(&f.paths)
            .unwrap_or_else(|_| panic!("store"))
            .with_lock(|t| t.write(FILE, bytes))
            .unwrap_or_else(|_| panic!("future marker"));
        assert!(
            f.registry()
                .erase_instance(f.id, &Cancellation::default())
                .is_err()
        );
        assert_eq!(
            std::fs::read(f.paths.root().join(FILE)).unwrap_or_else(|_| panic!("retained")),
            bytes
        );
        std::fs::remove_file(f.paths.root().join(FILE)).unwrap_or_else(|_| panic!("fixture reset"));
        std::fs::write(f.root.join("transaction.json"), b"unknown intent")
            .unwrap_or_else(|_| panic!("transaction"));
        assert!(
            f.registry()
                .erase_instance(f.id, &Cancellation::default())
                .is_err()
        );
        assert!(f.root.join("installed.json").exists());
        std::fs::remove_file(f.root.join("transaction.json"))
            .unwrap_or_else(|_| panic!("fixture reset"));
        let mut manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(f.root.join("installed.json")).unwrap_or_else(|_| panic!("manifest")),
        )
        .unwrap_or_else(|_| panic!("JSON"));
        manifest["schema"] = serde_json::json!(999);
        private_fs::PrivateDir::open(&f.root)
            .unwrap_or_else(|_| panic!("private"))
            .write_atomic_durable(
                "installed.json",
                &serde_json::to_vec(&manifest).unwrap_or_else(|_| panic!("JSON")),
            )
            .unwrap_or_else(|_| panic!("future manifest"));
        assert!(
            f.registry()
                .erase_instance(f.id, &Cancellation::default())
                .is_err()
        );
        assert!(!f.paths.root().join(FILE).exists());
    }
    #[cfg(windows)]
    #[test]
    fn windows_locked_file_leaves_a_retryable_intent_and_reports_failure() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        let file = f.root.join("minecraft/locked.txt");
        std::fs::write(&file, b"locked player data").unwrap_or_else(|_| panic!("file"));
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&file)
            .unwrap_or_else(|_| panic!("deny deletion"));
        assert!(
            f.registry()
                .erase_instance(f.id, &Cancellation::default())
                .is_err()
        );
        assert!(
            !pending(&f.paths)
                .unwrap_or_else(|_| panic!("intent"))
                .is_empty()
        );
        assert_eq!(
            f.registry().load().unwrap_or_else(|_| panic!("discover"))[0].id,
            f.id
        );
        drop(held);
        f.registry()
            .erase_instance(f.id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("retry"));
        assert!(!f.root.exists());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_preflight_rejects_redirects_without_deleting_any_data() {
        let f = Fixture::new();
        let outside = f.paths.root().join("outside");
        std::fs::create_dir(&outside).unwrap_or_else(|_| panic!("outside"));
        std::fs::write(outside.join("keep"), b"keep").unwrap_or_else(|_| panic!("canary"));
        std::os::unix::fs::symlink(&outside, f.root.join("minecraft/redirect"))
            .unwrap_or_else(|_| panic!("symlink"));
        assert!(
            f.registry()
                .erase_instance(f.id, &Cancellation::default())
                .is_err()
        );
        assert!(f.root.join("installed.json").exists());
        assert!(outside.join("keep").exists());
        assert!(!f.paths.root().join(FILE).exists());
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    #[tokio::test]
    async fn asynchronous_installation_lock_rejects_pending_erasure() {
        let (paths, accounts, id) = crate::minecraft::play::tests::fixture();
        drop(accounts);
        let registry = InstanceRegistry::new(paths.clone());
        let root = registry
            .prepare_instance(id)
            .unwrap_or_else(|_| panic!("prepare"));
        let record = registry
            .get(id)
            .unwrap_or_else(|_| panic!("record"))
            .unwrap_or_else(|| panic!("owned"));
        let private = private_fs::PrivateDir::open(&root).unwrap_or_else(|_| panic!("private"));
        let identity = directory_capability(&private)
            .unwrap_or_else(|_| panic!("capability"))
            .dir_metadata()
            .unwrap_or_else(|_| panic!("identity"));
        PrivateStore::open(&paths)
            .unwrap_or_else(|_| panic!("store"))
            .with_lock(|t| {
                save(
                    t,
                    vec![Intent {
                        record: record.clone(),
                        dev: identity.dev(),
                        ino: identity.ino(),
                    }],
                )
            })
            .unwrap_or_else(|_| panic!("intent"));
        assert!(
            crate::minecraft::download::lock(&root, ".installation.lock", &Cancellation::default())
                .await
                .is_err()
        );
        assert!(registry.save(&record).is_err());
        let mut changed = record.last_catalog;
        changed.name = "Updated catalog title".into();
        assert!(registry.remember_catalog(&[changed]).is_ok());
        registry
            .erase_instance(id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("explicit erase"));
        std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
    }
    #[test]
    fn redirected_player_folder_is_rejected_before_publishing_erasure_intent() {
        let (paths, accounts, id) = crate::minecraft::play::tests::fixture();
        drop(accounts);
        let registry = InstanceRegistry::new(paths.clone());
        let root = registry
            .prepare_instance(id)
            .unwrap_or_else(|_| panic!("prepare"));
        let target = paths.root().join("outside");
        std::fs::create_dir(&target).unwrap_or_else(|_| panic!("outside"));
        std::fs::write(target.join("keep.txt"), b"outside player data")
            .unwrap_or_else(|_| panic!("canary"));
        let link = root.join("minecraft/redirect");
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(link.components().collect::<PathBuf>())
                .arg(target.components().collect::<PathBuf>())
                .output()
                .unwrap_or_else(|_| panic!("junction creation"));
            assert!(output.status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap_or_else(|_| panic!("symlink"));
        assert!(
            registry
                .erase_instance(id, &Cancellation::default())
                .is_err()
        );
        assert!(root.join("instance.json").exists());
        assert!(target.join("keep.txt").exists());
        assert!(!paths.root().join(FILE).exists());
        #[cfg(windows)]
        std::fs::remove_dir(&link).unwrap_or_else(|_| panic!("remove junction itself"));
        #[cfg(unix)]
        std::fs::remove_file(&link).unwrap_or_else(|_| panic!("remove symlink itself"));
        std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
    }
}
