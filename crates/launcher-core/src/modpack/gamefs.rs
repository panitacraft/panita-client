//! Capability-relative access to an instance's game directory. Files and folders
//! there are ordinary player data, so private-fs ownership rules do not apply;
//! instead every step refuses links, redirected entries and unusual nodes, and a
//! new file is created absent-only through a temporary name and a rename.
use std::io::{Read, Write};
use std::path::Path;

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use uuid::Uuid;

use sha1::{Digest as _, Sha1};
use sha2::Sha512;

use super::{
    Hashes,
    diff::Local,
    game_path, invalid,
    transfer::{Expected, Hashers},
};
use crate::{
    error::{CoreError, ErrorCode},
    minecraft::{
        directory_capability,
        download::{Cancellation, io},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Absent,
    File,
    /// A directory, link, hard-linked file or other node at a file destination.
    Unsafe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    Absent,
    Matches,
    Differs,
    Unsafe,
}

fn conflict() -> CoreError {
    CoreError::new(ErrorCode::PackConflict)
}

/// Resolve the parent directory of `relative` one component at a time, never
/// following links. `create` makes missing parents; otherwise absence is `None`.
fn walk(root: &Dir, relative: &str, create: bool) -> Result<Option<(Dir, String)>, CoreError> {
    game_path(relative)?;
    let mut dir = root.try_clone().map_err(io)?;
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return Ok(Some((dir, part.to_owned())));
        }
        match dir.symlink_metadata(part) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(conflict());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                dir.create_dir(part).map_err(io)?;
            }
            Err(error) => return Err(io(error)),
        }
        dir = dir.open_dir_nofollow(part).map_err(io)?;
    }
    Err(invalid())
}

/// A directory beside the game directory where files wait while an operation runs:
/// the transient set-aside area, or the permanent backups of files the player kept.
pub struct Stash {
    dir: Dir,
}

impl Stash {
    /// `instance` is the instance root; `area` and `operation` name the folder.
    pub fn open(instance: &Path, area: &str, operation: Uuid) -> Result<Self, CoreError> {
        let private = private_fs::PrivateDir::open(instance)
            .map_err(crate::storage::map_private_error)?
            .ensure_dir(area)
            .and_then(|area| area.ensure_dir(operation.to_string()))
            .map_err(crate::storage::map_private_error)?;
        Ok(Self {
            dir: directory_capability(&private)?,
        })
    }

    /// Open an existing stash without creating one; `None` when there is nothing.
    pub fn existing(
        instance: &Path,
        area: &str,
        operation: Uuid,
    ) -> Result<Option<Self>, CoreError> {
        let path = instance.join(area).join(operation.to_string());
        match private_fs::PrivateDir::open(&path) {
            Ok(private) => Ok(Some(Self {
                dir: directory_capability(&private)?,
            })),
            Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(crate::storage::map_private_error(error)),
        }
    }

    pub fn has(&self, relative: &str) -> Result<bool, CoreError> {
        let Some((dir, name)) = walk(&self.dir, relative, false)? else {
            return Ok(false);
        };
        match dir.symlink_metadata(&name) {
            Ok(metadata) => Ok(metadata.is_file() && !metadata.file_type().is_symlink()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(io(error)),
        }
    }

    /// True when the stash holds no file at any depth.
    pub fn is_empty(&self) -> Result<bool, CoreError> {
        fn empty(dir: &Dir) -> Result<bool, CoreError> {
            for entry in dir.entries().map_err(io)? {
                let entry = entry.map_err(io)?;
                if !entry.file_type().map_err(io)?.is_dir() {
                    return Ok(false);
                }
                let child = dir.open_dir_nofollow(entry.file_name()).map_err(io)?;
                if !empty(&child)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        empty(&self.dir)
    }

    /// Delete the whole stash folder. Used once an operation has finished.
    pub fn discard(instance: &Path, area: &str, operation: Uuid) -> Result<(), CoreError> {
        let path = instance.join(area).join(operation.to_string());
        if private_fs::PrivateDir::open(&path).is_err() {
            return Ok(());
        }
        let parent = private_fs::PrivateDir::open(instance.join(area))
            .map_err(crate::storage::map_private_error)?;
        let capability = directory_capability(&parent)?;
        capability
            .remove_dir_all(operation.to_string())
            .map_err(io)?;
        Ok(())
    }
}

pub struct GameDir {
    dir: Dir,
}

impl GameDir {
    /// Open `instances/{slug}/minecraft` as a retained directory capability.
    pub fn open(game_directory: &Path) -> Result<Self, CoreError> {
        let private = private_fs::PrivateDir::open(game_directory)
            .map_err(crate::storage::map_private_error)?;
        Ok(Self {
            dir: directory_capability(&private)?,
        })
    }

    fn parent(&self, relative: &str, create: bool) -> Result<Option<(Dir, String)>, CoreError> {
        walk(&self.dir, relative, create)
    }

    pub fn entry(&self, relative: &str) -> Result<Entry, CoreError> {
        let Some((dir, name)) = self.parent(relative, false)? else {
            return Ok(Entry::Absent);
        };
        match dir.symlink_metadata(&name) {
            Ok(metadata) => Ok(
                if !metadata.file_type().is_symlink() && metadata.is_file() && metadata.nlink() == 1
                {
                    Entry::File
                } else {
                    Entry::Unsafe
                },
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Entry::Absent),
            Err(error) => Err(io(error)),
        }
    }

    /// Size and change markers of the regular file at `relative`, for Play receipts.
    /// Equal markers mean the file was not touched since it was last hashed.
    pub fn stamp(&self, relative: &str) -> Result<Option<Vec<u64>>, CoreError> {
        let Some((dir, name)) = self.parent(relative, false)? else {
            return Ok(None);
        };
        let metadata = match dir.symlink_metadata(&name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.nlink() != 1 {
            return Ok(None);
        }
        let modified = metadata
            .modified()
            .map_err(io)?
            .into_std()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid())?;
        Ok(Some(vec![
            metadata.len(),
            modified.as_secs(),
            u64::from(modified.subsec_nanos()),
            metadata.dev(),
            metadata.ino(),
        ]))
    }

    /// Length of the regular file at `relative`, when there is one.
    pub fn size(&self, relative: &str) -> Result<Option<u64>, CoreError> {
        let Some((dir, name)) = self.parent(relative, false)? else {
            return Ok(None);
        };
        match dir.symlink_metadata(&name) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                Ok(Some(metadata.len()))
            }
            Ok(_) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io(error)),
        }
    }

    /// Hash the existing file at `relative` against `expected` without following links.
    pub fn compare(
        &self,
        relative: &str,
        expected: &Expected,
        cancel: &Cancellation,
    ) -> Result<Content, CoreError> {
        match self.entry(relative)? {
            Entry::Absent => return Ok(Content::Absent),
            Entry::Unsafe => return Ok(Content::Unsafe),
            Entry::File => {}
        }
        let Some((dir, name)) = self.parent(relative, false)? else {
            return Ok(Content::Absent);
        };
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = dir.open_with(&name, &options).map_err(io)?;
        let metadata = file.metadata().map_err(io)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Ok(Content::Unsafe);
        }
        if metadata.len() != expected.size {
            return Ok(Content::Differs);
        }
        let mut hashers = Hashers::new(expected);
        let mut buffer = [0; 64 * 1024];
        loop {
            cancel.check()?;
            let count = file.read(&mut buffer).map_err(io)?;
            if count == 0 {
                break;
            }
            hashers.update(&buffer[..count]);
        }
        Ok(if hashers.matches(expected) {
            Content::Matches
        } else {
            Content::Differs
        })
    }

    /// Create a file that does not exist yet. `fill` writes its bytes; nothing is
    /// published unless it succeeds, and an existing entry is never replaced.
    pub fn write_new(
        &self,
        relative: &str,
        fill: impl FnOnce(&mut dyn Write) -> Result<(), CoreError>,
    ) -> Result<(), CoreError> {
        let (dir, name) = self.parent(relative, true)?.ok_or_else(invalid)?;
        match dir.symlink_metadata(&name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Err(conflict()),
            Err(error) => return Err(io(error)),
        }
        let temp = format!(".panita-{}.tmp", Uuid::new_v4().simple());
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut file = dir.open_with(&temp, &options).map_err(io)?;
        let written = fill(&mut file).and_then(|()| file.sync_all().map_err(io));
        drop(file);
        let published = written.and_then(|()| dir.rename(&temp, &dir, &name).map_err(io));
        if published.is_err() {
            let _ = dir.remove_file(&temp);
        }
        published
    }

    /// Hash the file at `relative` with both algorithms, without following links.
    pub fn fingerprint(&self, relative: &str, cancel: &Cancellation) -> Result<Local, CoreError> {
        match self.entry(relative)? {
            Entry::Absent => return Ok(Local::Absent),
            Entry::Unsafe => return Ok(Local::Unsafe),
            Entry::File => {}
        }
        let Some((dir, name)) = self.parent(relative, false)? else {
            return Ok(Local::Absent);
        };
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = dir.open_with(&name, &options).map_err(io)?;
        let metadata = file.metadata().map_err(io)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Ok(Local::Unsafe);
        }
        let (mut sha1, mut sha512) = (Sha1::new(), Sha512::new());
        let mut buffer = [0; 64 * 1024];
        loop {
            cancel.check()?;
            let count = file.read(&mut buffer).map_err(io)?;
            if count == 0 {
                break;
            }
            sha1.update(&buffer[..count]);
            sha512.update(&buffer[..count]);
        }
        Ok(Local::File(Hashes {
            sha1: Some(format!("{:x}", sha1.finalize())),
            sha512: Some(format!("{:x}", sha512.finalize())),
        }))
    }

    /// Move the regular file at `relative` into the stash under the same relative
    /// path. Returns false when there was nothing to move. Never replaces anything.
    pub fn stash(&self, relative: &str, stash: &Stash) -> Result<bool, CoreError> {
        if self.entry(relative)? == Entry::Absent {
            return Ok(false);
        }
        if self.entry(relative)? == Entry::Unsafe {
            return Err(conflict());
        }
        let (from, from_name) = self.parent(relative, false)?.ok_or_else(invalid)?;
        let (to, to_name) = walk(&stash.dir, relative, true)?.ok_or_else(invalid)?;
        match to.symlink_metadata(&to_name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Err(conflict()),
            Err(error) => return Err(io(error)),
        }
        from.rename(&from_name, &to, &to_name).map_err(io)?;
        Ok(true)
    }

    /// Move a stashed file back to its place. The destination must be absent.
    pub fn restore(&self, relative: &str, stash: &Stash) -> Result<bool, CoreError> {
        if !stash.has(relative)? {
            return Ok(false);
        }
        let (from, from_name) = walk(&stash.dir, relative, false)?.ok_or_else(invalid)?;
        let (to, to_name) = self.parent(relative, true)?.ok_or_else(invalid)?;
        match to.symlink_metadata(&to_name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Err(conflict()),
            Err(error) => return Err(io(error)),
        }
        from.rename(&from_name, &to, &to_name).map_err(io)?;
        Ok(true)
    }

    /// Remove a regular file this launcher created. Links and other nodes stay.
    pub fn remove_file(&self, relative: &str) -> Result<(), CoreError> {
        let Some((dir, name)) = self.parent(relative, false)? else {
            return Ok(());
        };
        match self.entry(relative)? {
            Entry::Absent => Ok(()),
            Entry::File => dir.remove_file(&name).map_err(io),
            Entry::Unsafe => Err(conflict()),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
