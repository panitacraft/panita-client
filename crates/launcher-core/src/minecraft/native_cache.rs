//! Ownership-based native cleanup, independent of reusable shared caches and player data.
use super::{
    download::{Cancellation, io, try_lock},
    install::{self, InstalledLock, read_json, write_json},
    invalid,
};
use crate::{
    error::{CoreError, ErrorCode},
    paths::AppPaths,
    storage::map_private_error,
};
use cap_fs_ext::DirExt;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};
use uuid::Uuid;

const RECEIPT: &str = ".generation.json";
#[cfg(test)]
mod tests;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Generation {
    schema: u32,
    id: Uuid,
    operation: Uuid,
    directory: String,
    files: Vec<super::natives::NativeFile>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub cleaned_generations: u32,
    pub retained_generations: u32,
    pub untracked_generations: u32,
}

/// Record exact launcher-managed files, never infer ownership from directory names.
/// No directory is created for a generation without extracted files.
pub(super) fn record(root: &Path, installed: &InstalledLock) -> Result<(), CoreError> {
    if installed.native_files.is_empty() {
        return Ok(());
    }
    super::maintenance::validate_manifest(installed, installed.instance_id)?;
    if installed
        .native_files
        .iter()
        .any(|file| file.path == RECEIPT)
    {
        return Err(invalid());
    }
    let directory = installed.native_directory()?;
    let path = root.join(&directory);
    // A missing old generation has no files to reclaim and must not be recreated.
    if !path.try_exists().map_err(io)? {
        return Ok(());
    }
    let expected = Generation {
        schema: 1,
        id: installed.instance_id,
        operation: installed.operation,
        directory,
        files: installed.native_files.clone(),
    };
    match read_json::<Generation>(&path, RECEIPT)? {
        Some(existing)
            if serde_json::to_value(&existing).map_err(|_| invalid())?
                != serde_json::to_value(&expected).map_err(|_| invalid())? =>
        {
            Err(invalid())
        }
        Some(_) => Ok(()),
        None => write_json(
            root,
            &format!("{}/{RECEIPT}", expected.directory),
            &expected,
        ),
    }
}

/// Explicit idle-only cleanup. Recovery must settle pending transactions first.
pub fn cleanup(
    paths: &AppPaths,
    id: Uuid,
    cancel: &Cancellation,
) -> Result<CleanupReport, CoreError> {
    cancel.check()?;
    let root = install::metadata_root(paths, id)?;
    let _lease = try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    install::recover_locked(paths, id, &root)?;
    let installed = read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
    super::maintenance::validate_manifest(&installed, id)?;
    record(&root, &installed)?;
    cleanup_locked(&root, &installed, cancel)
}

fn uuid_name(name: &str) -> bool {
    Uuid::parse_str(name).is_ok_and(|id| !id.is_nil() && id.to_string() == name)
}

fn children(directory: &cap_std::fs::Dir) -> Result<Vec<String>, CoreError> {
    let mut names = Vec::new();
    for entry in directory.entries().map_err(io)? {
        let entry = entry.map_err(io)?;
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_owned());
        }
        if names.len() > 10_000 {
            return Err(invalid());
        }
    }
    Ok(names)
}

pub(super) fn cleanup_locked(
    root: &Path,
    installed: &InstalledLock,
    cancel: &Cancellation,
) -> Result<CleanupReport, CoreError> {
    let private = private_fs::PrivateDir::open(root).map_err(map_private_error)?;
    let capability = super::world_backup::capability(&private)?;
    let mut candidates = Vec::new();
    for name in children(&capability)? {
        if name.strip_prefix("natives-").is_some_and(uuid_name) {
            candidates.push(name);
        }
    }
    match capability.open_dir_nofollow("natives") {
        Ok(container) => {
            for name in children(&container)? {
                if uuid_name(&name) {
                    candidates.push(format!("natives/{name}"));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(io(error)),
    }
    let active = if installed.schema_version == 2 {
        Some(installed.native_directory()?)
    } else {
        None
    };
    let mut report = CleanupReport::default();
    for directory in candidates {
        cancel.check()?;
        if active.as_deref() == Some(&directory) {
            continue;
        }
        let path = root.join(&directory);
        let receipt = match read_json::<Generation>(&path, RECEIPT) {
            Ok(Some(receipt)) => receipt,
            Ok(None) => {
                report.untracked_generations += 1;
                continue;
            }
            Err(_) => {
                report.retained_generations += 1;
                continue;
            }
        };
        let mut generation = installed.clone();
        generation.operation = receipt.operation;
        generation.native_directory = if directory.starts_with("natives/") {
            Some(directory.clone())
        } else {
            None
        };
        generation.native_files = receipt.files;
        if receipt.schema != 1
            || receipt.id != installed.instance_id
            || receipt.directory != directory
            || generation
                .native_files
                .iter()
                .any(|file| file.path == RECEIPT)
            || generation.native_directory()? != directory
            || super::maintenance::validate_manifest(&generation, installed.instance_id).is_err()
        {
            report.retained_generations += 1;
            continue;
        }
        match clean_generation(root, &capability, &directory, &generation) {
            Ok(true) => report.cleaned_generations += 1,
            _ => report.retained_generations += 1,
        }
    }
    Ok(report)
}

fn open_chain(root: &cap_std::fs::Dir, parts: &[&str]) -> Result<cap_std::fs::Dir, CoreError> {
    let mut directory = root.try_clone().map_err(io)?;
    for part in parts {
        directory = directory.open_dir_nofollow(part).map_err(io)?;
    }
    Ok(directory)
}

fn clean_generation(
    root: &Path,
    capability: &cap_std::fs::Dir,
    relative: &str,
    installed: &InstalledLock,
) -> Result<bool, CoreError> {
    // Reuse the uninstall hash/identity checks: edited, linked and unknown files stay.
    let retained = super::maintenance::natives(root, installed, true)?;
    let parts: Vec<_> = relative.split('/').collect();
    let parent = open_chain(capability, &parts[..parts.len() - 1])?;
    let leaf = parts.last().ok_or_else(invalid)?;
    let generation = parent.open_dir_nofollow(leaf).map_err(io)?;
    let mut directories = BTreeSet::new();
    for file in &installed.native_files {
        let mut parts: Vec<_> = file.path.split('/').collect();
        parts.pop();
        while !parts.is_empty() {
            directories.insert(parts.join("/"));
            parts.pop();
        }
    }
    let mut directories: Vec<_> = directories.into_iter().collect();
    directories.sort_by_key(|path| std::cmp::Reverse(path.split('/').count()));
    for path in directories {
        let parts: Vec<_> = path.split('/').collect();
        if let Ok(parent) = open_chain(&generation, &parts[..parts.len() - 1]) {
            let leaf = parts.last().ok_or_else(invalid)?;
            if parent.open_dir_nofollow(leaf).is_ok() {
                let _ = parent.remove_dir(leaf);
            }
        }
    }
    if retained != 0 || children(&generation)?.iter().any(|name| name != RECEIPT) {
        return Ok(false);
    }
    let private = private_fs::PrivateDir::open(root.join(relative)).map_err(map_private_error)?;
    private.open_file(RECEIPT).map_err(map_private_error)?;
    generation.remove_file(RECEIPT).map_err(io)?;
    drop(private);
    drop(generation);
    parent.remove_dir(leaf).map_err(io)?;
    Ok(true)
}
