//! Read-only world snapshots beneath retained directory capabilities.
use super::{
    download::{Cancellation, io},
    invalid,
};
use crate::{
    error::{CoreError, ErrorCode},
    storage::map_private_error,
};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Seek, Write},
    path::Path,
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupReceipt {
    pub operation: Uuid,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Identity {
    device: u64,
    inode: u64,
    length: u64,
    modified: cap_std::time::SystemTime,
    directory: bool,
}

fn unsafe_world() -> CoreError {
    CoreError::new(ErrorCode::WorldBackupUnsafe)
}

fn zip_error(error: zip::result::ZipError) -> CoreError {
    match error {
        zip::result::ZipError::Io(error) => io(error),
        _ => invalid(),
    }
}

fn identity(metadata: Metadata, directory: bool) -> Result<Identity, CoreError> {
    if (directory && !metadata.is_dir())
        || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
    {
        return Err(unsafe_world());
    }
    Ok(Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: if directory { 0 } else { metadata.len() },
        modified: metadata.modified().map_err(io)?,
        directory,
    })
}

fn component(name: &str) -> Result<(), CoreError> {
    let lower = name.to_lowercase();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
        || name.ends_with(['.', ' '])
        || lower == "accounts.json"
        || lower.starts_with("accounts.json.")
        || lower == "account-removals.json"
        || lower == ".panita"
        || lower.starts_with(".private-fs-")
    {
        return Err(unsafe_world());
    }
    Ok(())
}

fn source_file(dir: &Dir, name: &str) -> Result<cap_std::fs::File, CoreError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    dir.open_with(name, &options).map_err(io)
}

fn walk<W: Write + Seek>(
    dir: &Dir,
    prefix: &str,
    depth: usize,
    cancel: &Cancellation,
    found: &mut BTreeMap<String, Identity>,
    archive: &mut Option<zip::ZipWriter<W>>,
    expected: Option<&BTreeMap<String, Identity>>,
) -> Result<(), CoreError> {
    if depth > 64 {
        return Err(unsafe_world());
    }
    let mut names = std::collections::BTreeSet::new();
    for entry in dir.entries().map_err(io)? {
        cancel.check()?;
        let entry = entry.map_err(io)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| unsafe_world())?;
        component(&name)?;
        if !names.insert(name.to_lowercase()) || found.len() >= 100_000 {
            return Err(unsafe_world());
        }
        let relative = format!("{prefix}/{name}");
        let metadata = dir.symlink_metadata(&name).map_err(io)?;
        if metadata.file_type().is_symlink() {
            return Err(unsafe_world());
        }
        let directory = metadata.is_dir();
        if directory {
            let child = dir.open_dir_nofollow(&name).map_err(io)?;
            let current = identity(child.dir_metadata().map_err(io)?, true)?;
            if expected.is_some_and(|expected| expected.get(&relative) != Some(&current)) {
                return Err(unsafe_world());
            }
            found.insert(relative.clone(), current);
            if let Some(archive) = archive {
                archive
                    .add_directory(
                        format!("{relative}/"),
                        zip::write::SimpleFileOptions::default(),
                    )
                    .map_err(zip_error)?;
            }
            walk(
                &child,
                &relative,
                depth + 1,
                cancel,
                found,
                archive,
                expected,
            )?;
        } else {
            if !metadata.is_file() {
                return Err(unsafe_world());
            }
            let mut file = source_file(dir, &name)?;
            let current = identity(file.metadata().map_err(io)?, false)?;
            if expected.is_some_and(|expected| expected.get(&relative) != Some(&current)) {
                return Err(unsafe_world());
            }
            if let Some(archive) = archive {
                archive
                    .start_file(
                        &relative,
                        zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Stored)
                            .large_file(current.length >= u32::MAX as u64),
                    )
                    .map_err(zip_error)?;
                let mut remaining = current.length;
                let mut buffer = [0u8; 64 * 1024];
                while remaining > 0 {
                    cancel.check()?;
                    let limit = remaining.min(buffer.len() as u64) as usize;
                    let count = file.read(&mut buffer[..limit]).map_err(io)?;
                    if count == 0 {
                        return Err(unsafe_world());
                    }
                    archive.write_all(&buffer[..count]).map_err(io)?;
                    remaining -= count as u64;
                }
                if file.read(&mut buffer[..1]).map_err(io)? != 0
                    || identity(file.metadata().map_err(io)?, false)? != current
                {
                    return Err(unsafe_world());
                }
            }
            found.insert(relative, current);
        }
    }
    Ok(())
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

/// Caller retains the instance operation lock. Never opens launcher/account data.
/// Partial directories are preserved without a completion receipt after failure.
pub(super) fn create(
    root: &Path,
    operation: Uuid,
    cancel: &Cancellation,
) -> Result<Option<BackupReceipt>, CoreError> {
    cancel.check()?;
    if operation.is_nil() {
        return Err(invalid());
    }
    let private = private_fs::PrivateDir::open(root).map_err(map_private_error)?;
    let game = capability(&private)?
        .open_dir_nofollow("minecraft")
        .map_err(io)?;
    match game.symlink_metadata("saves") {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io(error)),
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err(unsafe_world());
        }
        Ok(_) => (),
    }
    let saves = game.open_dir_nofollow("saves").map_err(io)?;
    let root_identity = identity(saves.dir_metadata().map_err(io)?, true)?;
    let mut before = BTreeMap::new();
    walk::<std::io::Cursor<Vec<u8>>>(&saves, "saves", 0, cancel, &mut before, &mut None, None)?;
    let files = before.values().filter(|entry| !entry.directory).count() as u64;
    if before.is_empty() {
        return Ok(None);
    }
    let bytes = before.values().try_fold(0u64, |total, entry| {
        total.checked_add(entry.length).ok_or_else(invalid)
    })?;
    let required = bytes
        .checked_add(before.len() as u64 * 512)
        .and_then(|size| size.checked_add(64 * 1024 * 1024))
        .ok_or_else(invalid)?;
    if fs2::available_space(root).map_err(io)? < required {
        return Err(CoreError::new(ErrorCode::InsufficientSpace));
    }
    let backups = private
        .ensure_dir("world-backups")
        .map_err(map_private_error)?;
    let destination = backups
        .create_dir(operation.to_string())
        .map_err(map_private_error)?;
    let output = destination
        .atomic_durable("worlds.zip")
        .map_err(map_private_error)?;
    let mut archive = Some(zip::ZipWriter::new(output));
    let mut copied = BTreeMap::new();
    walk(
        &saves,
        "saves",
        0,
        cancel,
        &mut copied,
        &mut archive,
        Some(&before),
    )?;
    let mut after = BTreeMap::new();
    walk::<std::io::Cursor<Vec<u8>>>(&saves, "saves", 0, cancel, &mut after, &mut None, None)?;
    let current_saves = game.open_dir_nofollow("saves").map_err(io)?;
    if copied != before
        || after != before
        || identity(current_saves.dir_metadata().map_err(io)?, true)? != root_identity
    {
        return Err(unsafe_world());
    }
    cancel.check()?;
    archive
        .take()
        .ok_or_else(invalid)?
        .finish()
        .map_err(zip_error)?
        .commit()
        .map_err(map_private_error)?;
    // A published archive is not a completed backup until every entry has been
    // read back successfully, including ZIP CRC checks, before the receipt.
    let file = destination
        .open_file("worlds.zip")
        .map_err(map_private_error)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    if archive.len() != before.len() {
        return Err(unsafe_world());
    }
    let mut buffer = [0u8; 64 * 1024];
    for index in 0..archive.len() {
        cancel.check()?;
        let mut file = archive.by_index(index).map_err(zip_error)?;
        let key = file.name().trim_end_matches('/');
        let original = before.get(key).ok_or_else(unsafe_world)?;
        if file.is_dir() != original.directory || file.size() != original.length {
            return Err(unsafe_world());
        }
        loop {
            cancel.check()?;
            if file.read(&mut buffer).map_err(io)? == 0 {
                break;
            }
        }
    }
    let receipt = BackupReceipt {
        operation,
        files,
        bytes,
    };
    destination
        .write_atomic_durable(
            "backup.json",
            &serde_json::to_vec(&receipt).map_err(|_| invalid())?,
        )
        .map_err(map_private_error)?;
    Ok(Some(receipt))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("panita-world-backup-{}", Uuid::new_v4()));
        let private =
            private_fs::PrivateDir::ensure(&root).unwrap_or_else(|_| panic!("private root"));
        private
            .create_dir("minecraft")
            .unwrap_or_else(|_| panic!("game root"));
        root
    }

    #[test]
    fn backup_preserves_unicode_worlds_and_does_not_include_other_instance_or_account_files() {
        let root = fixture();
        let world = root.join("minecraft/saves/Mundo de José");
        std::fs::create_dir_all(world.join("empty folder")).unwrap_or_else(|_| panic!("world"));
        std::fs::write(world.join("level.dat"), b"world fixture")
            .unwrap_or_else(|_| panic!("world data"));
        std::fs::write(root.join("minecraft/options.txt"), b"keep options")
            .unwrap_or_else(|_| panic!("options"));
        std::fs::write(
            root.join("accounts.json"),
            b"excluded synthetic account marker",
        )
        .unwrap_or_else(|_| panic!("excluded fixture"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&world, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|_| panic!("ordinary game directory permissions"));
        }
        let operation = Uuid::new_v4();
        let receipt = create(&root, operation, &Cancellation::default())
            .unwrap_or_else(|error| panic!("backup: {error}"));
        assert!(receipt.is_some_and(|receipt| receipt.files == 1 && receipt.bytes == 13));
        let mut archive = zip::ZipArchive::new(
            std::fs::File::open(root.join(format!("world-backups/{operation}/worlds.zip")))
                .unwrap_or_else(|_| panic!("archive")),
        )
        .unwrap_or_else(|_| panic!("zip"));
        assert_eq!(archive.len(), 3);
        let mut bytes = Vec::new();
        archive
            .by_name("saves/Mundo de José/level.dat")
            .unwrap_or_else(|_| panic!("world entry"))
            .read_to_end(&mut bytes)
            .unwrap_or_else(|_| panic!("read"));
        assert_eq!(bytes, b"world fixture");
        assert_eq!(
            std::fs::read(world.join("level.dat")).ok().as_deref(),
            Some(b"world fixture".as_slice())
        );
        assert_eq!(
            std::fs::read(root.join("minecraft/options.txt"))
                .ok()
                .as_deref(),
            Some(b"keep options".as_slice())
        );
        assert!(archive.by_name("accounts.json").is_err());
        assert!(create(&root, operation, &Cancellation::default()).is_err());
    }

    #[test]
    fn missing_worlds_and_cancellation_do_not_create_empty_backup_directories() {
        let root = fixture();
        assert!(
            create(&root, Uuid::new_v4(), &Cancellation::default())
                .ok()
                .flatten()
                .is_none()
        );
        assert!(!root.join("world-backups").exists());
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            create(&root, Uuid::new_v4(), &cancel)
                .err()
                .map(|error| error.code()),
            Some(ErrorCode::OperationCancelled)
        );
        assert!(!root.join("world-backups").exists());
    }

    #[test]
    fn hard_links_and_credential_named_files_are_rejected_without_publishing_a_backup() {
        let root = fixture();
        let world = root.join("minecraft/saves/Test");
        std::fs::create_dir_all(&world).unwrap_or_else(|_| panic!("world"));
        std::fs::write(root.join("outside-fixture"), b"must not be copied")
            .unwrap_or_else(|_| panic!("source"));
        std::fs::hard_link(root.join("outside-fixture"), world.join("level.dat"))
            .unwrap_or_else(|_| panic!("link"));
        assert_eq!(
            create(&root, Uuid::new_v4(), &Cancellation::default())
                .err()
                .map(|error| error.code()),
            Some(ErrorCode::WorldBackupUnsafe)
        );
        std::fs::remove_file(world.join("level.dat"))
            .unwrap_or_else(|_| panic!("remove fixture link"));
        std::fs::write(world.join("accounts.json"), b"excluded fixture")
            .unwrap_or_else(|_| panic!("credential-named fixture"));
        assert_eq!(
            create(&root, Uuid::new_v4(), &Cancellation::default())
                .err()
                .map(|error| error.code()),
            Some(ErrorCode::WorldBackupUnsafe)
        );
        assert!(!root.join("world-backups").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_world_directories_are_rejected() {
        let root = fixture();
        std::fs::create_dir(root.join("minecraft/saves")).unwrap_or_else(|_| panic!("saves"));
        std::os::unix::fs::symlink(&root, root.join("minecraft/saves/redirect"))
            .unwrap_or_else(|_| panic!("redirect"));
        assert_eq!(
            create(&root, Uuid::new_v4(), &Cancellation::default())
                .err()
                .map(|error| error.code()),
            Some(ErrorCode::WorldBackupUnsafe)
        );
    }
}
