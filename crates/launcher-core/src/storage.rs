use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;

use private_fs::{Error as PrivateFsError, PrivateDir};

use crate::error::{CoreError, ErrorCode};
use crate::paths::AppPaths;

const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;
const LOCK_NAME: &str = ".mutation.lock";

#[cfg(windows)]
#[path = "storage_windows.rs"]
mod windows;

/// A private application-data root. All writers must use `transaction` so
/// separate launcher processes serialize read-modify-write operations.
pub struct PrivateStore {
    root: PrivateDir,
    root_path: std::path::PathBuf,
    lock_path: std::path::PathBuf,
}

/// Operations spanning documents share the same process-wide filesystem lock.
pub struct StoreTransaction<'a> {
    store: &'a PrivateStore,
}

impl StoreTransaction<'_> {
    pub fn read(&self, name: &'static str) -> Result<Option<Vec<u8>>, CoreError> {
        self.store.check_secret_namespace(name)?;
        read_file(&self.store.root, Path::new(name))
    }

    pub fn write(&self, name: &'static str, bytes: &[u8]) -> Result<(), CoreError> {
        self.store.check_secret_namespace(name)?;
        if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        self.store
            .root
            .write_atomic_durable(name, bytes)
            .map_err(map_private_error)
    }

    /// Cosmetic only: hide a data file in Explorer without changing its permissions.
    /// An atomic replacement publishes a new file, so callers repeat this after writing.
    #[cfg(windows)]
    pub(crate) fn hide(&self, name: &'static str) -> Result<(), CoreError> {
        self.store.check_secret_namespace(name)?;
        self.store
            .root
            .open_file(name)
            .and_then(|file| file.set_hidden())
            .map_err(map_private_error)
    }

    /// Caller holds the writer lock and validates that this document is retired.
    /// Resolve from the retained private root, rejecting redirects and hard links.
    pub(crate) fn remove_verified_file(&self, name: &'static str) -> Result<(), CoreError> {
        self.store.check_secret_namespace(name)?;
        if name.is_empty() || name.contains(['/', '\\']) {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let verified = match self.store.root.open_file(name) {
            Ok(file) => file,
            Err(PrivateFsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => return Err(map_private_error(error)),
        };
        #[cfg(windows)]
        let root = {
            use std::os::windows::io::AsHandle;
            File::from(
                self.store
                    .root
                    .as_handle()
                    .try_clone_to_owned()
                    .map_err(map_io_error)?,
            )
        };
        #[cfg(unix)]
        let root = {
            use std::os::fd::AsFd;
            File::from(
                self.store
                    .root
                    .as_fd()
                    .try_clone_to_owned()
                    .map_err(map_io_error)?,
            )
        };
        let directory = cap_std::fs::Dir::from_std_file(root);
        directory.remove_file(name).map_err(map_io_error)?;
        drop(verified);
        #[cfg(unix)]
        directory.into_std_file().sync_all().map_err(map_io_error)?;
        Ok(())
    }
}

impl PrivateStore {
    /// Open the whole application profile and migrate legacy Windows directory
    /// permissions. Instance metadata stores must use `open` instead.
    pub fn open_application(paths: &AppPaths) -> Result<Self, CoreError> {
        let store = Self::open(paths)?;
        #[cfg(windows)]
        windows::migrate(&store)?;
        Ok(store)
    }

    pub fn open(paths: &AppPaths) -> Result<Self, CoreError> {
        let root = PrivateDir::ensure(paths.root()).map_err(map_private_error)?;
        match root.open_data_file(LOCK_NAME) {
            Ok(_) => (),
            Err(PrivateFsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                match root.create(LOCK_NAME) {
                    Ok(_) => (),
                    Err(PrivateFsError::Io(error))
                        if error.kind() == std::io::ErrorKind::AlreadyExists =>
                    {
                        root.open_data_file(LOCK_NAME).map_err(map_private_error)?;
                    }
                    Err(error) => return Err(map_private_error(error)),
                }
            }
            Err(error) => return Err(map_private_error(error)),
        }
        #[cfg(windows)]
        root.open_data_file(LOCK_NAME)
            .and_then(|file| file.set_hidden())
            .map_err(map_private_error)?;
        #[cfg(windows)]
        {
            match root.open_data_file(".installation.lock") {
                Ok(file) => file.set_hidden().map_err(map_private_error)?,
                Err(PrivateFsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(map_private_error(error)),
            }
            match root.open_dir(".artifact-locks") {
                Ok(directory) => directory.set_hidden().map_err(map_private_error)?,
                Err(PrivateFsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(map_private_error(error)),
            }
        }
        let store = Self {
            root,
            root_path: paths.root().to_path_buf(),
            lock_path: paths.root().join(LOCK_NAME),
        };
        store.cleanup_stale_temps()?;
        Ok(store)
    }

    pub fn read(&self, name: &'static str) -> Result<Option<Vec<u8>>, CoreError> {
        self.check_secret_namespace(name)?;
        read_file(&self.root, Path::new(name))
    }

    fn check_secret_namespace(&self, name: &str) -> Result<(), CoreError> {
        if name.eq_ignore_ascii_case("accounts.json")
            || name.eq_ignore_ascii_case("panita-session.json")
        {
            self.root
                .verify_private_namespace()
                .map_err(map_private_error)?;
        }
        Ok(())
    }

    /// The closure sees the latest durable bytes while the interprocess lock
    /// is held. An error in the closure leaves the existing document intact.
    pub fn transaction<T>(
        &self,
        name: &'static str,
        mutate: impl FnOnce(Option<Vec<u8>>) -> Result<(Vec<u8>, T), CoreError>,
    ) -> Result<T, CoreError> {
        self.with_lock(|transaction| {
            let existing = transaction.read(name)?;
            let (next, result) = mutate(existing)?;
            transaction.write(name, &next)?;
            Ok(result)
        })
    }

    pub fn with_lock<T>(
        &self,
        operation: impl FnOnce(&StoreTransaction<'_>) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        let _lock = self.lock()?;
        operation(&StoreTransaction { store: self })
    }

    fn lock(&self) -> Result<File, CoreError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.lock_path)
            .map_err(map_io_error)?;
        private_fs::verify_data(&file).map_err(map_private_error)?;
        file.lock().map_err(map_io_error)?;
        private_fs::verify_data(&file).map_err(map_private_error)?;
        Ok(file)
    }

    /// Remove only abandoned staging files created by our atomic writer.
    /// The mutation lock prevents removal of a live writer's temporary file.
    fn cleanup_stale_temps(&self) -> Result<(), CoreError> {
        let _lock = self.lock()?;
        let entries = std::fs::read_dir(&self.root_path).map_err(map_io_error)?;
        for entry in entries {
            let entry = entry.map_err(map_io_error)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !is_atomic_temp_name(name) {
                continue;
            }
            self.root.open_file(name).map_err(map_private_error)?;
            std::fs::remove_file(entry.path()).map_err(map_io_error)?;
        }
        Ok(())
    }
}

fn is_atomic_temp_name(name: &str) -> bool {
    let Some(hex) = name
        .strip_prefix(".private-fs-")
        .and_then(|rest| rest.strip_suffix(".tmp"))
    else {
        return false;
    };
    hex.len() == 32 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_file(root: &PrivateDir, name: &Path) -> Result<Option<Vec<u8>>, CoreError> {
    let mut file = match root.open_file(name) {
        Ok(file) => file,
        Err(PrivateFsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(map_private_error(error)),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(map_io_error)?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(CoreError::new(ErrorCode::InvalidData));
    }
    file.verify().map_err(map_private_error)?;
    Ok(Some(bytes))
}

fn map_io_error(error: std::io::Error) -> CoreError {
    let code = match error.kind() {
        std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
        std::io::ErrorKind::StorageFull => ErrorCode::InsufficientSpace,
        _ => ErrorCode::StorageUnavailable,
    };
    CoreError::new(code)
}

pub(crate) fn map_private_error(error: PrivateFsError) -> CoreError {
    match error {
        PrivateFsError::Io(error) => map_io_error(error),
        PrivateFsError::AtomicCommit {
            may_have_published: true,
            ..
        } => CoreError::new(ErrorCode::StorageCommitUncertain),
        PrivateFsError::InsecureAcl
        | PrivateFsError::InsecurePermissions { .. }
        | PrivateFsError::OwnerMismatch
        | PrivateFsError::UntrustedCreationParent
        | PrivateFsError::WrongOwner { .. } => CoreError::new(ErrorCode::UnsafePermissions),
        _ => CoreError::new(ErrorCode::StorageUnavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_paths() -> AppPaths {
        let path = std::env::temp_dir().join(format!("panita-storage-{}", Uuid::new_v4()));
        AppPaths::new(path).unwrap_or_else(|_| panic!("absolute test path"))
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn credential_namespace_allows_readonly_public_modes_but_blocks_other_writers() {
        use std::os::unix::fs::PermissionsExt;
        let paths = test_paths();
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("private store"));
        for name in ["accounts.json", "panita-session.json"] {
            store
                .transaction(name, |_| Ok((b"synthetic-private-data".to_vec(), ())))
                .unwrap_or_else(|_| panic!("private fixture"));
        }
        std::fs::set_permissions(paths.root(), std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|_| panic!("read-only public parent"));
        for name in ["accounts.json", "panita-session.json"] {
            assert_eq!(
                store.read(name).ok().flatten().as_deref(),
                Some(b"synthetic-private-data".as_slice())
            );
            assert_eq!(
                std::fs::metadata(paths.root().join(name))
                    .unwrap_or_else(|_| panic!("file metadata"))
                    .permissions()
                    .mode()
                    & 0o7777,
                0o600
            );
        }
        std::fs::set_permissions(paths.root(), std::fs::Permissions::from_mode(0o777))
            .unwrap_or_else(|_| panic!("unsafe parent"));
        for name in ["accounts.json", "panita-session.json"] {
            assert_eq!(
                store.read(name).err().map(|error| error.code()),
                Some(ErrorCode::UnsafePermissions)
            );
            assert!(
                store
                    .transaction(name, |_| Ok((b"replacement".to_vec(), ())))
                    .is_err()
            );
            assert_eq!(
                std::fs::read(paths.root().join(name)).ok().as_deref(),
                Some(b"synthetic-private-data".as_slice())
            );
        }
        let data = PrivateDir::open_data(paths.root()).unwrap_or_else(|_| panic!("data access"));
        data.write_atomic_durable("ordinary.dat", b"ordinary bytes")
            .unwrap_or_else(|_| panic!("ordinary write"));
        drop(data);
        drop(store);
        std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
    }

    #[test]
    #[cfg(windows)]
    fn mutation_lock_is_hidden_and_remains_private_after_reopening() {
        use std::os::windows::fs::MetadataExt;
        let paths = test_paths();
        let root = PrivateDir::ensure(paths.root()).unwrap_or_else(|_| panic!("root"));
        root.create(".installation.lock")
            .unwrap_or_else(|_| panic!("installation lock"));
        root.ensure_dir(".artifact-locks")
            .unwrap_or_else(|_| panic!("artifact locks"));
        drop(root);
        for _ in 0..2 {
            let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store"));
            let lock = store.lock().unwrap_or_else(|_| panic!("lock"));
            assert_ne!(
                lock.metadata()
                    .unwrap_or_else(|_| panic!("metadata"))
                    .file_attributes()
                    & 2,
                0
            );
            assert!(private_fs::verify(&lock).is_ok());
            for path in [
                paths.root().join(".installation.lock"),
                paths.root().join(".artifact-locks"),
            ] {
                assert_ne!(
                    std::fs::metadata(path)
                        .unwrap_or_else(|_| panic!("metadata"))
                        .file_attributes()
                        & 2,
                    0
                );
            }
        }
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn restricted_atomic_replacement_preserves_last_valid_document() {
        let paths = test_paths();
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("private root"));
        let first = store.transaction("accounts.json", |previous| {
            assert!(previous.is_none());
            Ok((b"first".to_vec(), ()))
        });
        assert!(first.is_ok());
        let refused: Result<(), CoreError> = store.transaction("accounts.json", |_| {
            Err(CoreError::new(ErrorCode::InvalidData))
        });
        assert!(refused.is_err());
        assert_eq!(
            store.read("accounts.json").ok().flatten(),
            Some(b"first".to_vec())
        );
        let replaced = store.transaction("accounts.json", |previous| {
            assert_eq!(previous, Some(b"first".to_vec()));
            Ok((b"second".to_vec(), ()))
        });
        assert!(replaced.is_ok());
        assert_eq!(
            store.read("accounts.json").ok().flatten(),
            Some(b"second".to_vec())
        );
        let file = File::open(paths.accounts()).unwrap_or_else(|_| panic!("account file"));
        assert!(private_fs::verify(&file).is_ok());
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn existing_insecure_document_is_never_replaced() {
        let paths = test_paths();
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("private root"));
        assert!(std::fs::create_dir(paths.accounts()).is_ok());
        let result = store.transaction("accounts.json", |_| Ok((b"replacement".to_vec(), ())));
        assert!(result.is_err());
        assert!(paths.accounts().is_dir());
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn independent_stores_serialize_read_modify_write() {
        let paths = test_paths();
        let first = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("private root"));
        let second = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("same private root"));
        assert!(
            first
                .transaction("settings.json", |_| Ok((vec![0], ())))
                .is_ok()
        );
        std::thread::scope(|scope| {
            for store in [&first, &second] {
                scope.spawn(move || {
                    for _ in 0..10 {
                        let result = store.transaction("settings.json", |previous| {
                            let Some(bytes) = previous else {
                                return Err(CoreError::new(ErrorCode::InvalidData));
                            };
                            let Some(value) = bytes.first() else {
                                return Err(CoreError::new(ErrorCode::InvalidData));
                            };
                            Ok((vec![value + 1], ()))
                        });
                        assert!(result.is_ok());
                    }
                });
            }
        });
        assert_eq!(first.read("settings.json").ok().flatten(), Some(vec![20]));
        drop(first);
        drop(second);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn startup_cleans_only_owned_staging_names() {
        let paths = test_paths();
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("private root"));
        let stale = ".private-fs-00000000000000000000000000000000.tmp";
        assert!(store.root.create(stale).is_ok());
        assert!(store.root.create("keep.tmp").is_ok());
        drop(store);
        let reopened = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("reopen root"));
        assert!(reopened.root.open_file(stale).is_err());
        assert!(reopened.root.open_file("keep.tmp").is_ok());
        drop(reopened);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }
}
