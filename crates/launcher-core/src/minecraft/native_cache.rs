//! Disposable native directories, confined to one idle instance.
use super::{
    download::{Cancellation, io, try_lock},
    install::{self, InstalledLock, read_json},
    invalid,
};
use crate::{
    error::{CoreError, ErrorCode},
    paths::AppPaths,
    storage::map_private_error,
};
use cap_fs_ext::{DirExt, MetadataExt};
use serde::Serialize;
use std::{
    ffi::{OsStr, OsString},
    path::Path,
};
use uuid::Uuid;

#[cfg(test)]
mod tests;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub cleaned_generations: u32,
    pub retained_generations: u32,
    pub untracked_generations: u32,
    pub details: Vec<CleanupDetail>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupDetail {
    pub directory: String,
    pub reason: &'static str,
}

impl CleanupReport {
    fn retain(&mut self, directory: String, reason: &'static str) {
        self.retained_generations += 1;
        self.details.push(CleanupDetail { directory, reason });
    }
}

pub fn cleanup(
    paths: &AppPaths,
    id: Uuid,
    cancel: &Cancellation,
) -> Result<CleanupReport, CoreError> {
    cancel.check()?;
    let root = install::metadata_root(paths, id)?;
    let _lease = try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    if let Some(report) = super::maintenance::retry_uninstall_cleanup(paths, id, &root)? {
        return Ok(report);
    }
    install::recover_locked(paths, id, &root)?;
    let installed = read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
    super::maintenance::validate_manifest(&installed, id)?;
    cleanup_locked(&root, &installed, cancel)
}

/// Caller owns the instance operation lease and its authoritative installed pointer.
pub(super) fn cleanup_locked(
    root: &Path,
    installed: &InstalledLock,
    cancel: &Cancellation,
) -> Result<CleanupReport, CoreError> {
    let active = if installed.schema_version == 2 {
        Some(installed.native_directory()?)
    } else {
        None
    };
    purge(root, active.as_deref(), cancel)
}

/// A committed update remains usable if the OS prevents native cleanup.
pub(super) fn after_update(root: &Path, installed: &InstalledLock) -> CleanupReport {
    cleanup_locked(root, installed, &Cancellation::default()).unwrap_or_else(|_| {
        let mut report = CleanupReport::default();
        report.retain("natives".into(), "accessFailure");
        report
    })
}

pub(super) fn remove_all(root: &Path) -> Result<CleanupReport, CoreError> {
    purge(root, None, &Cancellation::default())
}

fn uuid_name(name: &str) -> bool {
    Uuid::parse_str(name).is_ok_and(|id| !id.is_nil() && id.to_string() == name)
}

fn names(directory: &cap_std::fs::Dir) -> Result<Vec<OsString>, CoreError> {
    let mut names = Vec::new();
    for entry in directory.entries().map_err(io)? {
        names.push(entry.map_err(io)?.file_name());
        if names.len() > 100_000 {
            return Err(invalid());
        }
    }
    Ok(names)
}

fn safe_directory(parent: &cap_std::fs::Dir, name: &OsStr) -> Result<cap_std::fs::Dir, CoreError> {
    let metadata = parent.symlink_metadata(name).map_err(io)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    parent.open_dir_nofollow(name).map_err(io)
}

/// No-follow, bounded handle-relative traversal. An outside hard-link alias keeps
/// its contents when its disposable native-directory name is unlinked.
fn remove_tree(
    parent: &cap_std::fs::Dir,
    name: &OsStr,
    cancel: &Cancellation,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), CoreError> {
    cancel.check()?;
    if depth > 64 {
        return Err(invalid());
    }
    let metadata = parent.symlink_metadata(name).map_err(io)?;
    if metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    if metadata.is_file() {
        return parent.remove_file(name).map_err(io);
    }
    let directory = safe_directory(parent, name)?;
    let identity = directory.dir_metadata().map_err(io)?;
    for child in names(&directory)? {
        cancel.check()?;
        *remaining = remaining.checked_sub(1).ok_or_else(invalid)?;
        let metadata = directory.symlink_metadata(&child).map_err(io)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid());
        }
        if metadata.is_dir() {
            remove_tree(&directory, &child, cancel, depth + 1, remaining)?;
        } else if metadata.is_file() {
            directory.remove_file(&child).map_err(io)?;
        } else {
            return Err(invalid());
        }
    }
    let current = parent.symlink_metadata(name).map_err(io)?;
    if current.file_type().is_symlink()
        || current.dev() != identity.dev()
        || current.ino() != identity.ino()
    {
        return Err(invalid());
    }
    drop(directory);
    parent.remove_dir(name).map_err(io)
}

fn purge(
    root: &Path,
    active: Option<&str>,
    cancel: &Cancellation,
) -> Result<CleanupReport, CoreError> {
    cancel.check()?;
    let private = private_fs::PrivateDir::open(root).map_err(map_private_error)?;
    let capability = super::world_backup::capability(&private)?;
    let mut candidates = Vec::new();
    for name in names(&capability)? {
        if name
            .to_str()
            .is_some_and(|name| name.strip_prefix("natives-").is_some_and(uuid_name))
        {
            candidates.push((false, name));
        }
    }
    let mut report = CleanupReport::default();
    let container = match capability.symlink_metadata("natives") {
        Ok(_) => match safe_directory(&capability, OsStr::new("natives")) {
            Ok(directory) => Some(directory),
            Err(error) => {
                report.retain(
                    "natives".into(),
                    if error.code() == ErrorCode::InvalidData {
                        "unsafePath"
                    } else {
                        "accessFailure"
                    },
                );
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            report.retain("natives".into(), "accessFailure");
            None
        }
    };
    if let Some(container) = &container {
        for name in names(container)? {
            candidates.push((true, name));
        }
    }
    for (nested, name) in candidates {
        cancel.check()?;
        let directory = if nested {
            format!("natives/{}", name.to_string_lossy())
        } else {
            name.to_string_lossy().into_owned()
        };
        if active == Some(directory.as_str()) {
            report.details.push(CleanupDetail {
                directory,
                reason: "active",
            });
            continue;
        }
        let parent = if nested {
            container.as_ref().ok_or_else(invalid)?
        } else {
            &capability
        };
        match remove_tree(parent, &name, cancel, 0, &mut 100_000) {
            Ok(()) => report.cleaned_generations += 1,
            Err(error) => {
                cancel.check()?;
                report.retain(
                    directory,
                    if error.code() == ErrorCode::InvalidData {
                        "unsafePath"
                    } else {
                        "accessFailure"
                    },
                );
            }
        }
    }
    if let Some(container) = container {
        let empty = names(&container)?.is_empty();
        drop(container);
        if empty
            && let Err(error) = capability.remove_dir("natives")
            && error.kind() != std::io::ErrorKind::NotFound
        {
            report.retain("natives".into(), "accessFailure");
        }
    }
    Ok(report)
}
