//! Size and player-initiated clearing of the disposable `content/` cache. Instances
//! hold their own copies, so clearing only means later installs download again.
//! It never touches instances, worlds, accounts, settings or the vanilla caches.
use std::ffi::{OsStr, OsString};

use cap_fs_ext::{DirExt, MetadataExt};
use cap_std::fs::Dir;
use serde::Serialize;

use super::{content::shard_lock, invalid};
use crate::{
    error::CoreError,
    minecraft::{
        directory_capability,
        download::{Cancellation, io, try_lock},
    },
    paths::AppPaths,
    storage::map_private_error,
};

const MAX_ENTRIES: usize = 100_000;
const ALGORITHMS: [&str; 3] = ["sha1", "sha256", "sha512"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearReport {
    pub removed_entries: u64,
    pub removed_bytes: u64,
    pub retained_entries: u64,
    pub details: Vec<ClearDetail>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearDetail {
    pub directory: String,
    pub reason: &'static str,
}

impl ClearReport {
    fn retain(&mut self, directory: &str, reason: &'static str, count: u64) {
        self.retained_entries += count;
        if !self
            .details
            .iter()
            .any(|detail| detail.directory == directory && detail.reason == reason)
        {
            self.details.push(ClearDetail {
                directory: directory.to_owned(),
                reason,
            });
        }
    }
}

fn open_content(paths: &AppPaths) -> Result<Option<Dir>, CoreError> {
    match private_fs::PrivateDir::open(paths.root().join("content")) {
        Ok(private) => Ok(Some(directory_capability(&private)?)),
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(map_private_error(error)),
    }
}

fn names(directory: &Dir, budget: &mut usize) -> Result<Vec<OsString>, CoreError> {
    let mut names = Vec::new();
    for entry in directory.entries().map_err(io)? {
        names.push(entry.map_err(io)?.file_name());
        *budget = budget.checked_sub(1).ok_or_else(invalid)?;
    }
    Ok(names)
}

fn subdirectory(parent: &Dir, name: &OsStr) -> Result<Option<Dir>, CoreError> {
    let metadata = parent.symlink_metadata(name).map_err(io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(None);
    }
    parent.open_dir_nofollow(name).map(Some).map_err(io)
}

/// Total bytes of cache entries, ignoring anything that is not a regular file.
pub fn size(paths: &AppPaths, cancel: &Cancellation) -> Result<u64, CoreError> {
    let Some(content) = open_content(paths)? else {
        return Ok(0);
    };
    let (mut budget, mut total) = (MAX_ENTRIES, 0u64);
    for algorithm in names(&content, &mut budget)? {
        let Some(algorithm) = subdirectory(&content, &algorithm)? else {
            continue;
        };
        for shard in names(&algorithm, &mut budget)? {
            let Some(shard) = subdirectory(&algorithm, &shard)? else {
                continue;
            };
            for name in names(&shard, &mut budget)? {
                cancel.check()?;
                let metadata = shard.symlink_metadata(&name).map_err(io)?;
                if metadata.is_file() && !metadata.file_type().is_symlink() {
                    total = total.saturating_add(metadata.len());
                }
            }
        }
    }
    Ok(total)
}

/// Remove cache entries that no operation is using. A shard whose writer lock is
/// held, and any link or unusual node, is kept and reported rather than forced.
pub fn clear(paths: &AppPaths, cancel: &Cancellation) -> Result<ClearReport, CoreError> {
    let mut report = ClearReport::default();
    let Some(content) = open_content(paths)? else {
        return Ok(report);
    };
    let mut budget = MAX_ENTRIES;
    for algorithm_name in names(&content, &mut budget)? {
        cancel.check()?;
        let label = algorithm_name.to_string_lossy().into_owned();
        let Some(algorithm) = subdirectory(&content, &algorithm_name)? else {
            report.retain(&label, "unsafePath", 1);
            continue;
        };
        if !ALGORITHMS.contains(&label.as_str()) {
            report.retain(&label, "unknown", 1);
            continue;
        }
        for shard_name in names(&algorithm, &mut budget)? {
            cancel.check()?;
            let directory = format!("content/{label}/{}", shard_name.to_string_lossy());
            let Some(shard) = subdirectory(&algorithm, &shard_name)? else {
                report.retain(&directory, "unsafePath", 1);
                continue;
            };
            let files = names(&shard, &mut budget)?;
            // Holding the shard lock keeps a concurrent writer out while it is emptied.
            let Some(_guard) = try_lock(paths.root(), &shard_lock(&directory))? else {
                report.retain(&directory, "inUse", files.len() as u64);
                continue;
            };
            for name in files {
                cancel.check()?;
                let metadata = shard.symlink_metadata(&name).map_err(io)?;
                if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.nlink() != 1
                {
                    report.retain(&directory, "unsafePath", 1);
                    continue;
                }
                match shard.remove_file(&name) {
                    Ok(()) => {
                        report.removed_entries += 1;
                        report.removed_bytes = report.removed_bytes.saturating_add(metadata.len());
                    }
                    Err(_) => report.retain(&directory, "accessFailure", 1),
                }
            }
            drop(shard);
            let _ = algorithm.remove_dir(&shard_name);
        }
        drop(algorithm);
        let _ = content.remove_dir(&algorithm_name);
    }
    drop(content);
    if let Ok(root) = private_fs::PrivateDir::open(paths.root())
        && let Ok(root) = directory_capability(&root)
    {
        let _ = root.remove_dir("content");
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
