//! Linux runtime executables are owner-only 0700, distinct from 0600 metadata.
use super::{Artifact, download::io, invalid, safe_path};
use crate::error::{CoreError, ErrorCode};
use private_fs::PrivateDir;
use rustix::fs::{self, FileType, Mode, OFlags};
use sha1::{Digest, Sha1};
use std::{fs::File, io::Read};

fn unsafe_permissions() -> CoreError {
    CoreError::new(ErrorCode::UnsafePermissions)
}

fn parent(directory: &PrivateDir) -> Result<(), CoreError> {
    let stat = fs::fstat(directory).map_err(|error| io(error.into()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_mode & 0o7777 != 0o700
    {
        return Err(unsafe_permissions());
    }
    Ok(())
}

fn verify(directory: &PrivateDir, file: &File, require_execute: bool) -> Result<(), CoreError> {
    parent(directory)?;
    let stat = fs::fstat(file).map_err(|error| io(error.into()))?;
    let mode = stat.st_mode & 0o7777;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_nlink != 1
        || !(mode == 0o700 || (!require_execute && mode == 0o600))
    {
        return Err(unsafe_permissions());
    }
    Ok(())
}

fn open(directory: &PrivateDir, name: &str) -> Result<Option<File>, CoreError> {
    safe_path(name)?;
    if name.contains('/') {
        return Err(invalid());
    }
    parent(directory)?;
    let fd = match fs::openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(io(error.into())),
    };
    let file = File::from(fd);
    verify(directory, &file, false)?;
    Ok(Some(file))
}

pub(super) fn matches(
    directory: &PrivateDir,
    name: &str,
    artifact: &Artifact,
) -> Result<bool, CoreError> {
    let Some(mut file) = open(directory, name)? else {
        return Ok(false);
    };
    if file.metadata().map_err(io)?.len() != artifact.size {
        return Ok(false);
    }
    let mut hash = Sha1::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        verify(directory, &file, false)?;
        let count = file.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    verify(directory, &file, false)?;
    Ok(format!("{:x}", hash.finalize()) == artifact.sha1)
}

pub(super) fn require(directory: &PrivateDir, name: &str) -> Result<(), CoreError> {
    let file = open(directory, name)?.ok_or_else(invalid)?;
    verify(directory, &file, true)
}

pub(super) fn enable(directory: &PrivateDir, name: &str) -> Result<(), CoreError> {
    let file = open(directory, name)?.ok_or_else(invalid)?;
    fs::fchmod(&file, Mode::from_raw_mode(0o700)).map_err(|error| io(error.into()))?;
    file.sync_all().map_err(io)?;
    verify(directory, &file, true)
}

/// Staging is protected 0600. Replace only a validated runtime artifact, then
/// synchronize its execute mode and parent before installation can be committed.
pub(super) fn publish(directory: &PrivateDir, staging: &str, name: &str) -> Result<(), CoreError> {
    let source = directory
        .open_file(staging)
        .map_err(crate::storage::map_private_error)?;
    let file = super::download::native_file(&source)?;
    // Validate any existing destination without following links or relaxing it.
    open(directory, name)?;
    parent(directory)?;
    fs::renameat(directory, staging, directory, name).map_err(|error| io(error.into()))?;
    let finish = || {
        fs::fchmod(&file, Mode::from_raw_mode(0o700)).map_err(|error| io(error.into()))?;
        file.sync_all().map_err(io)?;
        verify(directory, &file, true)?;
        fs::fsync(directory).map_err(|error| io(error.into()))?;
        let published = open(directory, name)?.ok_or_else(invalid)?;
        let original = fs::fstat(&file).map_err(|error| io(error.into()))?;
        let current = fs::fstat(&published).map_err(|error| io(error.into()))?;
        if (original.st_dev, original.st_ino) != (current.st_dev, current.st_ino) {
            return Err(invalid());
        }
        verify(directory, &published, true)
    };
    finish().map_err(|_: CoreError| CoreError::new(ErrorCode::StorageCommitUncertain))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn runtime_access_rejects_links_public_modes_and_missing_execute_permission() {
        let root =
            std::env::temp_dir().join(format!("panita-runtime-mode-{}", uuid::Uuid::new_v4()));
        let directory = PrivateDir::ensure(&root).unwrap_or_else(|_| panic!("private root"));
        directory
            .write_atomic_durable("java", b"runtime bytes")
            .unwrap_or_else(|_| panic!("fixture"));
        assert!(require(&directory, "java").is_err());
        enable(&directory, "java").unwrap_or_else(|_| panic!("enable"));
        assert!(require(&directory, "java").is_ok());
        symlink("java", root.join("redirect")).unwrap_or_else(|_| panic!("symlink"));
        assert!(open(&directory, "redirect").is_err());
        std::fs::hard_link(root.join("java"), root.join("alias"))
            .unwrap_or_else(|_| panic!("hard link"));
        assert!(open(&directory, "java").is_err());
        std::fs::remove_file(root.join("alias")).unwrap_or_else(|_| panic!("remove alias"));
        std::fs::set_permissions(root.join("java"), std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|_| panic!("mode fixture"));
        assert!(open(&directory, "java").is_err());
        std::fs::set_permissions(root.join("java"), std::fs::Permissions::from_mode(0o700))
            .unwrap_or_else(|_| panic!("mode fixture"));
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|_| panic!("parent mode fixture"));
        assert!(open(&directory, "java").is_err());
        std::fs::remove_dir_all(&root).unwrap_or_else(|_| panic!("cleanup"));
    }
}
