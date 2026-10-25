//! Runtime ownership requires a pre-spawn inventory and a known installation.
//! Merely finding a matching DLL in an old directory never authorizes adoption.
use super::*;
use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use sha1::{Digest, Sha1};
use std::{collections::BTreeMap, io::Read, path::PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Tracking {
    libraries: Vec<super::super::plan::PlannedArtifact>,
    baseline: BTreeSet<String>,
    observed: BTreeMap<String, FileIdentity>,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

fn identity(file: &cap_std::fs::File) -> Result<Option<FileIdentity>, CoreError> {
    let metadata = file.metadata().map_err(io)?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > 128 * 1024 * 1024 {
        return Ok(None);
    }
    let modified = metadata
        .modified()
        .map_err(io)?
        .into_std()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid())?;
    Ok(Some(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    }))
}

type NativeIdentity = (String, String, u64);
pub(super) type ArchiveCache = BTreeMap<String, Vec<NativeIdentity>>;

pub(super) fn validate(tracking: &Tracking) -> Result<(), CoreError> {
    if tracking.libraries.is_empty()
        || tracking.libraries.len() > 512
        || tracking.baseline.len() > 10_000
        || tracking.observed.len() > 10_000
    {
        return Err(invalid());
    }
    for path in &tracking.baseline {
        super::super::safe_path(path)?;
    }
    for (path, identity) in &tracking.observed {
        super::super::safe_path(path)?;
        if identity.size > 128 * 1024 * 1024 || identity.modified_nanos >= 1_000_000_000 {
            return Err(invalid());
        }
    }
    for library in &tracking.libraries {
        super::super::safe_path(&library.destination)?;
        library.artifact.validate()?;
        if library.executable || !library.destination.starts_with("libraries/") {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(in crate::minecraft) struct Capture {
    root: PathBuf,
    receipt: Generation,
    created: bool,
}

pub(in crate::minecraft) struct Observation {
    root: PathBuf,
    directory: String,
    id: Uuid,
    operation: Uuid,
}

fn association(receipt: &Generation, installed: &InstalledLock) -> bool {
    matches!(receipt.schema, 1 | 2)
        && (receipt.schema == 2 || receipt.runtime.is_none())
        && receipt.id == installed.instance_id
        && receipt.operation == installed.operation
        && installed
            .native_directory()
            .is_ok_and(|path| path == receipt.directory)
}

/// Only names are read on Play. Archive/file hashing is deferred to manual cleanup.
pub(in crate::minecraft) fn capture(
    paths: &AppPaths,
    id: Uuid,
) -> Result<Option<Capture>, CoreError> {
    let root = install::metadata_root(paths, id)?;
    let Some(installed) = read_json::<InstalledLock>(&root, "installed.json")? else {
        return Ok(None);
    };
    super::super::maintenance::validate_manifest(&installed, id)?;
    if installed.schema_version != 2 || installed.plan.version.classpath.is_empty() {
        return Ok(None);
    }
    let mut libraries = Vec::new();
    for artifact in &installed.plan.version.classpath {
        let planned = installed
            .plan
            .artifacts
            .iter()
            .find(|entry| entry.artifact == *artifact)
            .ok_or_else(invalid)?;
        if !planned.destination.starts_with("libraries/") {
            return Err(invalid());
        }
        libraries.push(planned.clone());
    }
    if libraries.len() > 512 {
        return Err(invalid());
    }
    let directory = installed.native_directory()?;
    let exists = root.join(&directory).try_exists().map_err(io)?;
    let existing = if exists {
        read_json::<Generation>(&root.join(&directory), RECEIPT)?
    } else {
        None
    };
    if let Some(receipt) = &existing {
        if let Some(tracking) = &receipt.runtime {
            validate(tracking)?;
        }
        if !association(receipt, &installed) {
            return Err(invalid());
        }
        if receipt.runtime.is_none()
            && serde_json::to_value(&receipt.files).map_err(|_| invalid())?
                != serde_json::to_value(&installed.native_files).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
    }
    let (private, _) =
        super::super::download::ensure_parent(&root, &format!("{directory}/{RECEIPT}"))?;
    let capability = super::super::world_backup::capability(&private)?;
    let baseline = inventory(&capability)?.into_iter().collect();
    let mut receipt = existing.unwrap_or(Generation {
        schema: 2,
        id,
        operation: installed.operation,
        directory,
        files: installed.native_files,
        runtime: None,
    });
    receipt.schema = 2;
    if let Some(tracking) = &mut receipt.runtime {
        if serde_json::to_value(&tracking.libraries).map_err(|_| invalid())?
            != serde_json::to_value(&libraries).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        tracking.baseline = baseline;
        // Names already present before this run are not newly created by it.
        // Keep prior observations only while their original identity still matches.
        tracking.observed.retain(|path, original| {
            open_regular(&capability, path)
                .ok()
                .and_then(|file| identity(&file).ok().flatten())
                .as_ref()
                == Some(original)
        });
    } else {
        receipt.runtime = Some(Tracking {
            libraries,
            baseline,
            observed: BTreeMap::new(),
        });
    }
    Ok(Some(Capture {
        root,
        receipt,
        created: !exists,
    }))
}

impl Capture {
    /// Publish only after successful process creation. Interrupted publication leaks
    /// storage conservatively; it must never invent provenance on a later scan.
    pub(in crate::minecraft) fn publish(self) -> Result<Observation, CoreError> {
        write_json(
            &self.root,
            &format!("{}/{RECEIPT}", self.receipt.directory),
            &self.receipt,
        )?;
        Ok(Observation {
            root: self.root,
            directory: self.receipt.directory,
            id: self.receipt.id,
            operation: self.receipt.operation,
        })
    }

    pub(in crate::minecraft) fn abort(self) {
        if !self.created {
            return;
        }
        // remove_dir only removes an empty directory, including across a race.
        if let Ok(private) = private_fs::PrivateDir::open(&self.root)
            && let Ok(capability) = super::super::world_backup::capability(&private)
        {
            let parts: Vec<_> = self.receipt.directory.split('/').collect();
            if let Ok(parent) = super::open_chain(&capability, &parts[..parts.len() - 1]) {
                let _ = parent.remove_dir(parts[parts.len() - 1]);
            }
        }
    }
}

fn open_regular(directory: &cap_std::fs::Dir, path: &str) -> Result<cap_std::fs::File, CoreError> {
    super::super::safe_path(path)?;
    let parts: Vec<_> = path.split('/').collect();
    let parent = super::open_chain(directory, &parts[..parts.len() - 1])?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    parent
        .open_with(parts[parts.len() - 1], &options)
        .map_err(io)
}

impl Observation {
    /// The supervisor calls this after confirmed child exit, before releasing its lease.
    pub(in crate::minecraft) fn finish(self) -> Result<(), CoreError> {
        let mut receipt: Generation =
            read_json(&self.root.join(&self.directory), RECEIPT)?.ok_or_else(invalid)?;
        if receipt.schema != 2
            || receipt.id != self.id
            || receipt.operation != self.operation
            || receipt.directory != self.directory
        {
            return Err(invalid());
        }
        let tracking = receipt.runtime.as_mut().ok_or_else(invalid)?;
        validate(tracking)?;
        let private = private_fs::PrivateDir::open(self.root.join(&self.directory))
            .map_err(map_private_error)?;
        let capability = super::super::world_backup::capability(&private)?;
        for path in inventory(&capability)? {
            if path == RECEIPT || tracking.baseline.contains(&path) {
                continue;
            }
            let name = path.rsplit('/').next().ok_or_else(invalid)?;
            if !native_name(name) && !name.ends_with(".x") {
                continue;
            }
            let Ok(file) = open_regular(&capability, &path) else {
                continue;
            };
            if let Some(identity) = identity(&file)? {
                tracking.observed.insert(path, identity);
            }
        }
        write_json(
            &self.root,
            &format!("{}/{RECEIPT}", self.directory),
            &receipt,
        )
    }
}

fn inventory(directory: &cap_std::fs::Dir) -> Result<Vec<String>, CoreError> {
    fn walk(
        directory: &cap_std::fs::Dir,
        prefix: &str,
        depth: usize,
        out: &mut Vec<String>,
    ) -> Result<(), CoreError> {
        if depth > 32 {
            return Err(invalid());
        }
        for entry in directory.entries().map_err(io)? {
            let entry = entry.map_err(io)?;
            let name = entry.file_name().into_string().map_err(|_| invalid())?;
            let path = format!("{prefix}{name}");
            super::super::safe_path(&path)?;
            out.push(path.clone());
            if out.len() > 10_000 {
                return Err(invalid());
            }
            if entry.file_type().map_err(io)?.is_dir() {
                let child = directory.open_dir_nofollow(&name).map_err(io)?;
                walk(&child, &format!("{path}/"), depth + 1, out)?;
            }
        }
        Ok(())
    }
    let mut result = Vec::new();
    walk(directory, "", 0, &mut result)?;
    Ok(result)
}

fn native_name(name: &str) -> bool {
    name.ends_with(".dll")
        || name.ends_with(".so")
        || name.contains(".so.")
        || name.ends_with(".dylib")
}

/// Build identities from fully verified archives, not from the extraction directory.
fn expected(
    paths: &AppPaths,
    tracking: &Tracking,
    cancel: &Cancellation,
) -> Result<Vec<NativeIdentity>, CoreError> {
    if tracking.libraries.len() > 512 || tracking.baseline.len() > 10_000 {
        return Err(invalid());
    }
    for path in &tracking.baseline {
        super::super::safe_path(path)?;
    }
    if tracking.observed.len() > 10_000 {
        return Err(invalid());
    }
    for path in tracking.observed.keys() {
        super::super::safe_path(path)?;
    }
    let mut result = Vec::new();
    let mut total = 0u64;
    for library in &tracking.libraries {
        cancel.check()?;
        library.artifact.validate()?;
        super::super::safe_path(&library.destination)?;
        if !library.destination.starts_with("libraries/") {
            return Err(invalid());
        }
        let (parent, leaf) =
            super::super::download::existing_parent(paths.root(), &library.destination)?
                .ok_or_else(invalid)?;
        let source = parent.open_file(&leaf).map_err(map_private_error)?;
        if !super::super::download::verified_open_file(&source, &library.artifact)? {
            return Err(invalid());
        }
        let file = super::super::download::native_file(&source)?;
        let mut archive = zip::ZipArchive::new(file).map_err(|_| invalid())?;
        if archive.len() > 100_000 {
            return Err(invalid());
        }
        for index in 0..archive.len() {
            cancel.check()?;
            let mut entry = archive.by_index(index).map_err(|_| invalid())?;
            let name = entry
                .name()
                .rsplit('/')
                .next()
                .ok_or_else(invalid)?
                .to_owned();
            if !native_name(&name) || entry.is_dir() {
                continue;
            }
            super::super::safe_path(entry.name())?;
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
            {
                return Err(invalid());
            }
            total = total.checked_add(entry.size()).ok_or_else(invalid)?;
            if entry.size() > 128 * 1024 * 1024 || total > 512 * 1024 * 1024 {
                return Err(invalid());
            }
            let mut hash = Sha1::new();
            let mut count = 0u64;
            let mut buffer = [0; 64 * 1024];
            loop {
                cancel.check()?;
                let size = entry.read(&mut buffer).map_err(io)?;
                if size == 0 {
                    break;
                }
                count += size as u64;
                if count > entry.size() {
                    return Err(invalid());
                }
                hash.update(&buffer[..size]);
            }
            if count != entry.size() {
                return Err(invalid());
            }
            result.push((name, format!("{:x}", hash.finalize()), count));
            if result.len() > 10_000 {
                return Err(invalid());
            }
        }
        source.verify().map_err(map_private_error)?;
    }
    Ok(result)
}

/// Adopt only newly observed regular single-link files in a recorded execution.
/// Persist the enlarged inventory before any deletion so interruption is repeatable.
pub(super) fn extend(
    paths: &AppPaths,
    root: &Path,
    receipt: &mut Generation,
    cancel: &Cancellation,
    cache: &mut ArchiveCache,
) -> Result<(), CoreError> {
    let Some(tracking) = &receipt.runtime else {
        return Ok(());
    };
    validate(tracking)?;
    if tracking.libraries.len() > 512 || tracking.baseline.len() > 10_000 {
        return Err(invalid());
    }
    for path in &tracking.baseline {
        super::super::safe_path(path)?;
    }
    let key = format!(
        "{:x}",
        Sha1::digest(serde_json::to_vec(&tracking.libraries).map_err(|_| invalid())?)
    );
    if !cache.contains_key(&key) {
        cache.insert(key.clone(), expected(paths, tracking, cancel)?);
    }
    let identities = cache.get(&key).ok_or_else(invalid)?;
    let private =
        private_fs::PrivateDir::open(root.join(&receipt.directory)).map_err(map_private_error)?;
    let capability = super::super::world_backup::capability(&private)?;
    let mut adopted = BTreeMap::new();
    let mut markers = Vec::new();
    for path in inventory(&capability)? {
        cancel.check()?;
        if path == RECEIPT
            || !tracking.observed.contains_key(&path)
            || receipt.files.iter().any(|file| file.path == path)
        {
            continue;
        }
        let parts: Vec<_> = path.split('/').collect();
        let parent = super::open_chain(&capability, &parts[..parts.len() - 1])?;
        let name = parts[parts.len() - 1];
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let Ok(mut file) = parent.open_with(name, &options) else {
            continue;
        };
        let metadata = file.metadata().map_err(io)?;
        if identity(&file)?.as_ref() != tracking.observed.get(&path) {
            continue;
        }
        if !metadata.is_file() || metadata.nlink() != 1 {
            continue;
        }
        if metadata.len() == 0 && name.ends_with(".x") {
            markers.push(path);
            continue;
        }
        if !native_name(name) || metadata.len() > 128 * 1024 * 1024 {
            continue;
        }
        let mut hash = Sha1::new();
        let mut reader = Read::by_ref(&mut file).take(metadata.len() + 1);
        let mut buffer = [0; 64 * 1024];
        loop {
            cancel.check()?;
            let count = reader.read(&mut buffer).map_err(io)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let sha1 = format!("{:x}", hash.finalize());
        let jna = jna_name(name);
        if identities.iter().any(|(leaf, hash, size)| {
            *hash == sha1
                && *size == metadata.len()
                && (leaf == name
                    || (jna
                        && matches!(
                            leaf.as_str(),
                            "jnidispatch.dll" | "libjnidispatch.so" | "libjnidispatch.jnilib"
                        )))
        }) {
            adopted.insert(
                path.clone(),
                super::super::natives::NativeFile {
                    path,
                    sha1,
                    size: metadata.len(),
                },
            );
        }
    }
    for path in markers {
        let dll = path.strip_suffix(".x").ok_or_else(invalid)?;
        if jna_name(dll.rsplit('/').next().ok_or_else(invalid)?) && adopted.contains_key(dll) {
            adopted.insert(
                path.clone(),
                super::super::natives::NativeFile {
                    path,
                    sha1: format!("{:x}", Sha1::digest([])),
                    size: 0,
                },
            );
        }
    }
    receipt.files.extend(adopted.into_values());
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for file in &receipt.files {
        total = total.checked_add(file.size).ok_or_else(invalid)?;
        if !names.insert(file.path.to_ascii_lowercase())
            || total > 512 * 1024 * 1024
            || receipt.files.len() > 10_000
        {
            return Err(invalid());
        }
    }
    cancel.check()?;
    write_json(root, &format!("{}/{RECEIPT}", receipt.directory), receipt)
}

fn jna_name(name: &str) -> bool {
    let digits = name.strip_prefix("jna").and_then(|name| {
        name.strip_suffix(".dll")
            .or_else(|| name.strip_suffix(".so"))
    });
    digits.is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}
