//! Layout of an mrpack archive: its verified index and its override layers. The
//! archive is verified byte-for-byte before anything here reads it. Overrides use a
//! policy of their own, separate from the custom configuration archives: mod jars may
//! ride along directly under `mods/`, while other executable or native types may not.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek};

use sha2::{Digest, Sha256};

use super::{
    Environment, MAX_DOCUMENT_BYTES, MrpackIndex,
    archive::{self, ArchiveEntry, Limits, Opened},
    content, game_path, invalid,
    transfer::Expected,
};
use crate::{
    error::{CoreError, ErrorCode},
    minecraft::download::Cancellation,
    paths::AppPaths,
};

const INDEX: &str = "modrinth.index.json";
const COMMON: &str = "overrides/";
const CLIENT: &str = "client-overrides/";

/// Types a pack may never place, with or without the launcher running them.
const EXECUTABLES: [&str; 9] = [
    "exe", "dll", "bat", "cmd", "ps1", "sh", "so", "dylib", "msi",
];
/// Folders whose files are immutable pack content, verified on every Play.
pub const CONTENT_ROOTS: [&str; 3] = ["mods", "resourcepacks", "shaderpacks"];

/// True when a pack may place this game path: no executable or native types, and a
/// jar only directly inside `mods/`.
pub fn placeable(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    match lower.rsplit_once('.') {
        Some((_, "jar")) => matches!(lower.split('/').collect::<Vec<_>>().as_slice(), ["mods", _]),
        Some((_, extension)) => !EXECUTABLES.contains(&extension),
        None => true,
    }
}

/// Immutable content rather than editable configuration.
pub fn is_content(path: &str) -> bool {
    path.split('/')
        .next()
        .is_some_and(|root| CONTENT_ROOTS.contains(&root.to_ascii_lowercase().as_str()))
}

/// Server-only overrides and unknown entries are never extracted.
fn rule(path: &str, directory: bool) -> Result<Option<String>, CoreError> {
    if path == INDEX {
        return if directory {
            Err(invalid())
        } else {
            Ok(Some(path.to_owned()))
        };
    }
    let Some(rest) = path
        .strip_prefix(COMMON)
        .or_else(|| path.strip_prefix(CLIENT))
    else {
        return Ok(None);
    };
    game_path(rest)?;
    if !directory && !placeable(rest) {
        return Err(invalid());
    }
    Ok(Some(rest.to_owned()))
}

/// Open the verified archive and split its entries into the index and raw overrides.
fn open(
    paths: &AppPaths,
    expected: &Expected,
    cancel: &Cancellation,
) -> Result<(Opened<File>, ArchiveEntry, Vec<ArchiveEntry>), CoreError> {
    let file = content::open_verified(paths, expected, cancel)?;
    let opened = archive::preflight_with(file, Limits::PACK, &rule)?;
    let mut index = None;
    let mut overrides = Vec::new();
    for entry in &opened.entries {
        if entry.source == INDEX {
            if index.replace(entry.clone()).is_some() {
                return Err(invalid());
            }
        } else {
            overrides.push(entry.clone());
        }
    }
    Ok((opened, index.ok_or_else(invalid)?, overrides))
}

fn read_index<R: Read + Seek>(
    opened: &mut Opened<R>,
    entry: &ArchiveEntry,
    cancel: &Cancellation,
) -> Result<(MrpackIndex, String), CoreError> {
    if entry.size > MAX_DOCUMENT_BYTES as u64 {
        return Err(invalid());
    }
    let mut document = Vec::new();
    let mut budget = MAX_DOCUMENT_BYTES as u64;
    opened.stream(entry, &mut budget, cancel, Some(&mut document))?;
    let index = MrpackIndex::parse(&document)?;
    Ok((index, format!("{:x}", Sha256::digest(&document))))
}

/// Apply the layer precedence: client overrides win over common ones for the same
/// path. A path the index downloads, or one that is both file and folder, rejects
/// the whole pack instead of choosing a winner.
fn resolve(
    overrides: Vec<ArchiveEntry>,
    index: &MrpackIndex,
) -> Result<Vec<ArchiveEntry>, CoreError> {
    let mut chosen: BTreeMap<String, ArchiveEntry> = BTreeMap::new();
    for entry in overrides {
        let key = entry.path.to_ascii_lowercase();
        let client = entry.source.starts_with(CLIENT);
        match chosen.get(&key) {
            Some(existing) if existing.source.starts_with(CLIENT) && !client => {}
            _ => {
                chosen.insert(key, entry);
            }
        }
    }
    let downloaded: BTreeSet<String> = index
        .files
        .iter()
        .map(|file| file.path.to_ascii_lowercase())
        .collect();
    if chosen.keys().any(|path| downloaded.contains(path)) {
        return Err(invalid());
    }
    let all: BTreeSet<String> = chosen.keys().chain(downloaded.iter()).cloned().collect();
    for path in &all {
        for (end, _) in path.match_indices('/') {
            if all.contains(&path[..end]) {
                return Err(invalid());
            }
        }
    }
    Ok(chosen.into_values().collect())
}

/// Everything the installer needs from a verified mrpack before the instance lock.
#[derive(Clone, Debug)]
pub struct Inspected {
    pub index: MrpackIndex,
    pub index_sha256: String,
    pub overrides: usize,
}

/// Read the index and validate its overrides. Index files must be placeable too.
pub fn inspect(
    paths: &AppPaths,
    expected: &Expected,
    cancel: &Cancellation,
) -> Result<Inspected, CoreError> {
    let (mut opened, entry, overrides) = open(paths, expected, cancel)?;
    let (index, index_sha256) = read_index(&mut opened, &entry, cancel)?;
    if index
        .files
        .iter()
        .filter(|file| file.env.client != Environment::Unsupported)
        .any(|file| !placeable(&file.path))
    {
        return Err(invalid());
    }
    let overrides = resolve(overrides, &index)?.len();
    Ok(Inspected {
        index,
        index_sha256,
        overrides,
    })
}

/// Re-open the verified archive for extraction. Its entries are exactly the resolved
/// overrides, and its index must still be the one the plan was built from.
pub fn layer(
    paths: &AppPaths,
    expected: &Expected,
    index: &MrpackIndex,
    index_sha256: &str,
    cancel: &Cancellation,
) -> Result<Opened<File>, CoreError> {
    let (mut opened, entry, overrides) = open(paths, expected, cancel)?;
    let (current, digest) = read_index(&mut opened, &entry, cancel)?;
    if &current != index || digest != index_sha256 {
        return Err(CoreError::new(ErrorCode::ArtifactMismatch));
    }
    opened.entries = resolve(overrides, &current)?;
    Ok(opened)
}

#[cfg(test)]
mod tests;
