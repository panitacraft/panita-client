//! Explicit whole-profile erasure. The production entry point resolves only the
//! current user's authoritative Windows profile; there is no path argument or
//! environment override. Caller excludes all launchers and startup until done.
use crate::error::{CoreError, ErrorCode};
use cap_fs_ext::{DirExt, MetadataExt};
use cap_std::fs::Dir;
use std::{
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
};

const MARKER: &str = ".data-removal.json";
const INTENT: &[u8] = b"{\n  \"schema\": 1,\n  \"type\": \"remove_all_launcher_data\"\n}\n";
fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidData)
}
fn io(_: std::io::Error) -> CoreError {
    CoreError::new(ErrorCode::StorageUnavailable)
}

#[cfg(windows)]
pub fn current_root() -> Result<PathBuf, CoreError> {
    winsafe::SHGetKnownFolderPath(
        &winsafe::co::KNOWNFOLDERID::RoamingAppData,
        winsafe::co::KF::DEFAULT,
        None,
    )
    .map(|path| PathBuf::from(path).join("PanitaClient"))
    .map_err(|_| CoreError::new(ErrorCode::StorageUnavailable))
}

/// Requires explicit uninstaller confirmation and exclusive process coordination.
#[cfg(windows)]
pub fn remove_current_profile() -> Result<(), CoreError> {
    remove_profile(&current_root()?)
}

pub fn pending(root: &Path) -> Result<bool, CoreError> {
    match std::fs::symlink_metadata(root.join(MARKER)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io(error)),
    }
}

fn capability(directory: &private_fs::PrivateDir) -> Result<Dir, CoreError> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        Ok(Dir::from_std_file(std::fs::File::from(
            directory.as_handle().try_clone_to_owned().map_err(io)?,
        )))
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        Ok(Dir::from_std_file(std::fs::File::from(
            directory.as_fd().try_clone_to_owned().map_err(io)?,
        )))
    }
}
fn lock_handle(file: &private_fs::PrivateFile) -> Result<std::fs::File, CoreError> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        Ok(std::fs::File::from(
            file.as_handle().try_clone_to_owned().map_err(io)?,
        ))
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        Ok(std::fs::File::from(
            file.as_fd().try_clone_to_owned().map_err(io)?,
        ))
    }
}
fn names(directory: &Dir, remaining: &mut usize) -> Result<Vec<OsString>, CoreError> {
    let mut result = Vec::new();
    for entry in directory.entries().map_err(io)? {
        *remaining = remaining.checked_sub(1).ok_or_else(invalid)?;
        result.push(entry.map_err(io)?.file_name());
    }
    Ok(result)
}

// Preflight before unlinking anything; repeat checks during deletion. Retained
// directory handles confine traversal. Refuse links and unusual filesystem nodes.
fn walk(
    directory: &Dir,
    depth: usize,
    remaining: &mut usize,
    erase: bool,
) -> Result<(), CoreError> {
    if depth > 64 {
        return Err(invalid());
    }
    for name in names(directory, remaining)? {
        if depth == 0 && (name == MARKER || name == ".mutation.lock") {
            continue;
        }
        let metadata = directory.symlink_metadata(&name).map_err(io)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid());
        }
        if metadata.is_dir() {
            let child = directory.open_dir_nofollow(&name).map_err(io)?;
            let identity = child.dir_metadata().map_err(io)?;
            walk(&child, depth + 1, remaining, erase)?;
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

// Private path-taking implementation exists for isolated synthetic tests only;
// production callers can invoke only remove_current_profile(), above.
fn remove_profile(root: &Path) -> Result<(), CoreError> {
    if !root.is_absolute() || root.file_name().is_none() {
        return Err(invalid());
    }
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io(error)),
        Ok(metadata) if metadata.file_type().is_symlink() => return Err(invalid()),
        Ok(_) => {}
    }
    let private = private_fs::PrivateDir::open(root).map_err(crate::storage::map_private_error)?;
    let directory = capability(&private)?;
    let identity = directory.dir_metadata().map_err(io)?;
    let parent = Dir::open_ambient_dir(
        root.parent().ok_or_else(invalid)?,
        cap_std::ambient_authority(),
    )
    .map_err(io)?;
    let name = root.file_name().ok_or_else(invalid)?;
    let private_lock = match private.open_data_file(".mutation.lock") {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            private
                .create(".mutation.lock")
                .map_err(crate::storage::map_private_error)?
        }
        Err(error) => return Err(crate::storage::map_private_error(error)),
    };
    let lock = lock_handle(&private_lock)?;
    #[cfg(windows)]
    private_lock
        .set_hidden()
        .map_err(crate::storage::map_private_error)?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| CoreError::new(ErrorCode::OperationBusy))?;
    if pending(root)? {
        let mut bytes = Vec::new();
        private
            .open_file(MARKER)
            .map_err(crate::storage::map_private_error)?
            .take(128)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes != INTENT {
            return Err(invalid());
        }
    }
    walk(&directory, 0, &mut 1_000_000, false)?;
    private
        .write_atomic_durable(MARKER, INTENT)
        .map_err(crate::storage::map_private_error)?;
    #[cfg(windows)]
    private
        .open_file(MARKER)
        .and_then(|file| file.set_hidden())
        .map_err(crate::storage::map_private_error)?;
    walk(&directory, 0, &mut 1_000_000, true)?;
    drop(lock);
    drop(private_lock);
    directory.remove_file(".mutation.lock").map_err(io)?;
    directory.remove_file(MARKER).map_err(io)?;
    let current = parent.symlink_metadata(name).map_err(io)?;
    if current.file_type().is_symlink()
        || current.dev() != identity.dev()
        || current.ino() != identity.ino()
    {
        return Err(invalid());
    }
    drop(directory);
    drop(private);
    parent.remove_dir(name).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("panita-erasure-test-{}", uuid::Uuid::new_v4()));
            private_fs::PrivateDir::ensure(&path)
                .unwrap_or_else(|_| panic!("private synthetic fixture"));
            Self(path)
        }
        fn write(&self, name: &str, bytes: &[u8]) {
            private_fs::PrivateDir::open(&self.0)
                .unwrap_or_else(|_| panic!("synthetic fixture operation"))
                .write_atomic_durable(name, bytes)
                .unwrap_or_else(|_| panic!("synthetic fixture operation"));
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn erases_whole_profile_and_preserves_sibling() {
        let fixture = Fixture::new();
        let sibling = Fixture::new();
        fixture.write(
            "accounts.json",
            b"synthetic invalid account data, never parsed",
        );
        sibling.write("keep", b"outside");
        std::fs::create_dir_all(fixture.0.join("instances/demo/minecraft/saves/world"))
            .unwrap_or_else(|_| panic!("synthetic fixture operation"));
        std::fs::write(
            fixture
                .0
                .join("instances/demo/minecraft/saves/world/level.dat"),
            b"synthetic world",
        )
        .unwrap_or_else(|_| panic!("synthetic fixture operation"));
        remove_profile(&fixture.0).unwrap_or_else(|_| panic!("synthetic fixture operation"));
        assert!(!fixture.0.exists());
        assert!(sibling.0.join("keep").exists());
        remove_profile(&fixture.0).unwrap_or_else(|_| panic!("synthetic fixture operation"));
    }
    #[test]
    fn refuses_busy_writer_before_removing_anything() {
        let fixture = Fixture::new();
        fixture.write(".mutation.lock", b"");
        fixture.write("keep", b"present");
        let private_lock = private_fs::PrivateDir::open(&fixture.0)
            .unwrap_or_else(|_| panic!("synthetic fixture operation"))
            .open_file(".mutation.lock")
            .unwrap_or_else(|_| panic!("synthetic fixture operation"));
        let lock =
            lock_handle(&private_lock).unwrap_or_else(|_| panic!("synthetic fixture operation"));
        lock.lock()
            .unwrap_or_else(|_| panic!("synthetic fixture operation"));
        assert_eq!(
            remove_profile(&fixture.0).err().map(|error| error.code()),
            Some(ErrorCode::OperationBusy)
        );
        assert!(fixture.0.join("keep").exists());
    }
    #[test]
    fn preserves_unknown_intent_and_retries_known_intent() {
        let fixture = Fixture::new();
        fixture.write(MARKER, b"future schema");
        fixture.write("keep", b"present");
        assert!(pending(&fixture.0).unwrap_or_else(|_| panic!("synthetic fixture operation")));
        assert!(remove_profile(&fixture.0).is_err());
        assert!(fixture.0.join("keep").exists());
        fixture.write(MARKER, INTENT);
        remove_profile(&fixture.0).unwrap_or_else(|_| panic!("synthetic fixture operation"));
        assert!(!fixture.0.exists());
    }
    #[cfg(windows)]
    #[test]
    fn locked_content_keeps_pending_intent_until_explicit_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let fixture = Fixture::new();
        fixture.write("locked", b"synthetic data");
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(fixture.0.join("locked"))
            .unwrap_or_else(|_| panic!("synthetic fixture operation"));
        assert!(remove_profile(&fixture.0).is_err());
        assert!(pending(&fixture.0).unwrap_or_else(|_| panic!("synthetic fixture operation")));
        assert!(fixture.0.join("locked").exists());
        use std::os::windows::fs::MetadataExt;
        assert_ne!(
            std::fs::metadata(fixture.0.join(MARKER))
                .unwrap_or_else(|_| panic!("synthetic fixture operation"))
                .file_attributes()
                & 2,
            0
        );
        assert_ne!(
            std::fs::metadata(fixture.0.join(".mutation.lock"))
                .unwrap_or_else(|_| panic!("synthetic fixture operation"))
                .file_attributes()
                & 2,
            0
        );
        drop(locked);
        remove_profile(&fixture.0).unwrap_or_else(|_| panic!("synthetic fixture operation"));
        assert!(!fixture.0.exists());
    }
    #[test]
    fn refuses_redirected_directory_without_touching_target() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        outside.write("keep", b"outside");
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(fixture.0.join("redirect"))
                .arg(&outside.0)
                .output()
                .unwrap_or_else(|_| panic!("synthetic fixture operation"));
            assert!(status.status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside.0, fixture.0.join("redirect"))
            .unwrap_or_else(|_| panic!("synthetic fixture operation"));
        assert!(remove_profile(&fixture.0).is_err());
        assert!(outside.0.join("keep").exists());
        assert!(!pending(&fixture.0).unwrap_or_else(|_| panic!("synthetic fixture operation")));
    }
}
