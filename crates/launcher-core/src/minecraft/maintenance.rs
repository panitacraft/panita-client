//! Exact-plan repair and data-preserving uninstall under the instance writer lease.
use super::{
    Platform,
    download::{Cancellation, io, try_lock},
    install::{self, InstalledLock, Progress, read_json, write_json},
    invalid,
};
use crate::{
    catalog::{CatalogClient, CatalogSource},
    error::{CoreError, ErrorCode},
    paths::AppPaths,
    registry::{InstallationState, InstanceRegistry},
    storage::map_private_error,
};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{io::Read, path::Path};
use uuid::Uuid;

pub struct MaintenanceOutcome {
    pub retained_files: u32,
}

/// Presentation hint only: a recorded active plan may still need artifact repair.
/// Missing/tombstoned plans must not expose operations that require that plan.
pub(crate) fn has_installation_manifest(paths: &AppPaths, id: Uuid) -> Result<bool, CoreError> {
    let root = paths.instance(id)?;
    let Some(installed) = read_json::<InstalledLock>(&root, "installed.json")? else {
        return Ok(false);
    };
    validate_manifest(&installed, id)?;
    Ok(installed.schema_version == 2)
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Repair,
    Uninstall,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    instance_id: Uuid,
    operation: Uuid,
    previous_operation: Option<Uuid>,
    kind: Kind,
    active: bool,
    prepared: Option<InstalledLock>,
    retained_files: u32,
}

pub(super) fn validate_manifest(manifest: &InstalledLock, id: Uuid) -> Result<(), CoreError> {
    if !matches!(manifest.schema_version, 2 | 3)
        || manifest.instance_id != id
        || id.is_nil()
        || manifest.operation.is_nil()
        || manifest.native_files.len() > 10_000
    {
        return Err(invalid());
    }
    manifest.plan.validate()?;
    let mut seen = std::collections::HashSet::new();
    let mut total = 0u64;
    for file in &manifest.native_files {
        super::safe_path(&file.path)?;
        if !super::valid_hash(&file.sha1) {
            return Err(invalid());
        }
        total = total.checked_add(file.size).ok_or_else(invalid)?;
        if file.size > 128 * 1024 * 1024
            || total > 512 * 1024 * 1024
            || !seen.insert(file.path.to_ascii_lowercase())
        {
            return Err(invalid());
        }
    }
    Ok(())
}

impl Journal {
    fn validate(&self, id: Uuid) -> Result<(), CoreError> {
        if self.schema_version != 3
            || self.instance_id != id
            || id.is_nil()
            || self.operation.is_nil()
            || self
                .previous_operation
                .is_some_and(|previous| previous.is_nil() || previous == self.operation)
            || self.retained_files > 10_000
            || (!self.active && self.prepared.is_some())
            || (self.kind == Kind::Repair && self.previous_operation.is_none())
            || (self.kind == Kind::Uninstall && self.prepared.is_some())
        {
            return Err(invalid());
        }
        if let Some(prepared) = &self.prepared {
            validate_manifest(prepared, id)?;
            if prepared.schema_version != 2 || prepared.operation != self.operation {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

pub(super) fn journal_inactive(value: serde_json::Value, id: Uuid) -> bool {
    serde_json::from_value::<Journal>(value)
        .is_ok_and(|journal| journal.validate(id).is_ok() && !journal.active)
}

// A recognized published generation can be repaired or removed even when its
// artifact audit prevents ordinary recovery. Never supersede an unknown intent.
fn recover_for_maintenance(paths: &AppPaths, id: Uuid, root: &Path) -> Result<(), CoreError> {
    match install::recover_locked(paths, id, root) {
        Ok(()) => return Ok(()),
        Err(error) if error.code() == ErrorCode::ArtifactMismatch => (),
        Err(error) => return Err(error),
    }
    let value =
        read_json::<serde_json::Value>(root, "install-journal.json")?.ok_or_else(invalid)?;
    let installed = read_json::<InstalledLock>(root, "installed.json")?.ok_or_else(invalid)?;
    validate_manifest(&installed, id)?;
    if value["schema_version"] == 2 {
        super::update::validate_for_uninstall(root, id)?;
    } else {
        let journal: Journal = serde_json::from_value(value).map_err(|_| invalid())?;
        journal.validate(id)?;
        let prepared = journal.prepared.as_ref().ok_or_else(invalid)?;
        if journal.kind != Kind::Repair
            || !journal.active
            || installed.operation != journal.operation
            || serde_json::to_value(prepared).map_err(|_| invalid())?
                != serde_json::to_value(&installed).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
    }
    save_record(
        paths,
        id,
        InstallationState::NeedsRepair,
        Some(&installed.plan.version.id),
    )
}

fn save_record(
    paths: &AppPaths,
    id: Uuid,
    state: InstallationState,
    version: Option<&str>,
) -> Result<(), CoreError> {
    let registry = InstanceRegistry::new(paths.clone());
    let mut record = registry.get(id)?.ok_or_else(invalid)?;
    record.state = state;
    if let Some(version) = version {
        record.resolved_source = CatalogSource::Vanilla {
            minecraft_version: version.to_owned(),
        };
    }
    registry.save_installation(&record)
}

pub(super) fn recover_locked(paths: &AppPaths, id: Uuid, root: &Path) -> Result<bool, CoreError> {
    let Some(value) = read_json::<serde_json::Value>(root, "install-journal.json")? else {
        return Ok(false);
    };
    if value["schema_version"] != 3 {
        return Ok(false);
    }
    let mut journal: Journal = serde_json::from_value(value).map_err(|_| invalid())?;
    journal.validate(id)?;
    if !journal.active {
        return Ok(true);
    }
    if journal.kind == Kind::Uninstall {
        finish_uninstall(paths, id, root, &mut journal)?;
        return Ok(true);
    }
    let installed = read_json::<InstalledLock>(root, "installed.json")?.ok_or_else(invalid)?;
    validate_manifest(&installed, id)?;
    if installed.schema_version != 2 {
        return Err(invalid());
    }
    if Some(installed.operation) == journal.previous_operation {
        let state =
            match install::verify_manifest(paths, id, root, &installed, &Cancellation::default()) {
                Ok(()) => InstallationState::Installed,
                Err(error) if error.code() == ErrorCode::ArtifactMismatch => {
                    InstallationState::NeedsRepair
                }
                Err(error) => return Err(error),
            };
        save_record(paths, id, state, Some(&installed.plan.version.id))?;
    } else if installed.operation == journal.operation {
        let prepared = journal.prepared.as_ref().ok_or_else(invalid)?;
        if serde_json::to_value(prepared).map_err(|_| invalid())?
            != serde_json::to_value(&installed).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        install::verify_manifest(paths, id, root, &installed, &Cancellation::default())?;
        save_record(
            paths,
            id,
            InstallationState::Installed,
            Some(&installed.plan.version.id),
        )?;
    } else {
        return Err(invalid());
    }
    complete(root, &mut journal)?;
    Ok(true)
}

fn complete(root: &Path, journal: &mut Journal) -> Result<(), CoreError> {
    journal.active = false;
    journal.prepared = None;
    write_json(root, "install-journal.json", journal)
}

/// Remove only unchanged, manifest-owned native files. Unknown entries and all
/// directories remain. Ordinary edits, links and unverifiable files are retained.
fn natives(root: &Path, installed: &InstalledLock, remove: bool) -> Result<u32, CoreError> {
    if installed.native_files.is_empty() {
        return Ok(0);
    }
    let directory = private_fs::PrivateDir::open(root).map_err(map_private_error)?;
    let capability = super::world_backup::capability(&directory)?;
    let generation = match capability.open_dir_nofollow(format!("natives-{}", installed.operation))
    {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(_) => return Ok(installed.native_files.len() as u32),
    };
    let mut retained = 0;
    for native in &installed.native_files {
        let mut parent = generation.try_clone().map_err(io)?;
        let parts: Vec<_> = native.path.split('/').collect();
        let mut unavailable = false;
        for part in &parts[..parts.len() - 1] {
            match parent.open_dir_nofollow(part) {
                Ok(child) => parent = child,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    unavailable = true;
                    break;
                }
                Err(_) => {
                    unavailable = true;
                    retained += 1;
                    break;
                }
            }
        }
        if unavailable {
            continue;
        }
        let leaf = parts.last().ok_or_else(invalid)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = match parent.open_with(leaf, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                retained += 1;
                continue;
            }
        };
        let metadata = file.metadata().map_err(io)?;
        if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() != native.size {
            retained += 1;
            continue;
        }
        let mut hash = Sha1::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut reader = Read::by_ref(&mut file).take(native.size + 1);
        loop {
            let count = reader.read(&mut buffer).map_err(io)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let current = file.metadata().map_err(io)?;
        if format!("{:x}", hash.finalize()) != native.sha1
            || current.len() != metadata.len()
            || current.modified().map_err(io)? != metadata.modified().map_err(io)?
        {
            retained += 1;
            continue;
        }
        if remove {
            let current = match parent.open_with(leaf, &options) {
                Ok(file) => file.metadata().map_err(io)?,
                Err(_) => {
                    retained += 1;
                    continue;
                }
            };
            if current.dev() != metadata.dev()
                || current.ino() != metadata.ino()
                || current.modified().map_err(io)? != metadata.modified().map_err(io)?
            {
                retained += 1;
                continue;
            }
            parent.remove_file(leaf).map_err(io)?;
        }
    }
    Ok(retained)
}

fn finish_uninstall(
    paths: &AppPaths,
    id: Uuid,
    root: &Path,
    journal: &mut Journal,
) -> Result<(), CoreError> {
    let installed = read_json::<InstalledLock>(root, "installed.json")?;
    match (&installed, journal.previous_operation) {
        (Some(installed), Some(previous)) => {
            validate_manifest(installed, id)?;
            if installed.operation != previous {
                return Err(invalid());
            }
            journal.retained_files = natives(root, installed, true)?;
            let mut tombstone = installed.clone();
            tombstone.schema_version = 3;
            write_json(root, "installed.json", &tombstone)?;
        }
        (None, None) => (),
        _ => return Err(invalid()),
    }
    save_record(
        paths,
        id,
        InstallationState::NotInstalled,
        installed
            .as_ref()
            .map(|installed| installed.plan.version.id.as_str()),
    )?;
    complete(root, journal)
}

pub fn uninstall(
    paths: &AppPaths,
    id: Uuid,
    cancel: &Cancellation,
) -> Result<MaintenanceOutcome, CoreError> {
    cancel.check()?;
    let root = install::metadata_root(paths, id)?;
    let _lease = try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    recover_for_maintenance(paths, id, &root)?;
    let previous = read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
    validate_manifest(&previous, id)?;
    let mut journal = Journal {
        schema_version: 3,
        instance_id: id,
        operation: Uuid::new_v4(),
        previous_operation: Some(previous.operation),
        kind: Kind::Uninstall,
        active: true,
        prepared: None,
        retained_files: 0,
    };
    cancel.commit(|| {
        write_json(&root, "install-journal.json", &journal)?;
        finish_uninstall(paths, id, &root, &mut journal)
    })?;
    Ok(MaintenanceOutcome {
        retained_files: journal.retained_files,
    })
}

pub async fn repair(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    cancel: &Cancellation,
    progress: Progress,
) -> Result<MaintenanceOutcome, CoreError> {
    repair_with_catalog(paths, id, platform, cancel, progress, CatalogClient::new()?).await
}

async fn repair_with_catalog(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    cancel: &Cancellation,
    progress: Progress,
    client: CatalogClient,
) -> Result<MaintenanceOutcome, CoreError> {
    cancel.check()?;
    let catalog = tokio::select! { _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)), result = client.detail(id) => result? };
    let registry = InstanceRegistry::new(paths.clone());
    registry.remember_catalog(std::slice::from_ref(&catalog))?;
    catalog.installation_allowed(
        &semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?,
    )?;
    let root = install::metadata_root(paths, id)?;
    let _lease = try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    recover_for_maintenance(paths, id, &root)?;
    let previous = read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
    validate_manifest(&previous, id)?;
    let record = registry.get(id)?.ok_or_else(invalid)?;
    if previous.schema_version != 2
        || record.state == InstallationState::NotInstalled
        || record.resolved_source
            != (CatalogSource::Vanilla {
                minecraft_version: previous.plan.version.id.clone(),
            })
    {
        return Err(invalid());
    }
    let retained_files = natives(&root, &previous, false)?;
    let mut journal = Journal {
        schema_version: 3,
        instance_id: id,
        operation: Uuid::new_v4(),
        previous_operation: Some(previous.operation),
        kind: Kind::Repair,
        active: true,
        prepared: None,
        retained_files,
    };
    write_json(&root, "install-journal.json", &journal)?;
    let result: Result<(), CoreError> = async {
        let installed = install::prepare_recorded(
            paths,
            id,
            journal.operation,
            platform,
            previous.plan,
            cancel,
            progress,
        )
        .await?;
        journal.prepared = Some(installed.clone());
        journal.validate(id)?;
        write_json(&root, "install-journal.json", &journal)?;
        cancel.commit(|| {
            write_json(&root, "installed.json", &installed)?;
            save_record(
                paths,
                id,
                InstallationState::Installed,
                Some(&installed.plan.version.id),
            )?;
            complete(&root, &mut journal)
        })
    }
    .await;
    if let Err(error) = result {
        if recover_locked(paths, id, &root).is_err() {
            return Err(CoreError::new(ErrorCode::StorageCommitUncertain));
        }
        let current = read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
        if current.operation != journal.operation {
            return Err(error);
        }
    }
    Ok(MaintenanceOutcome { retained_files })
}

#[cfg(test)]
mod tests;
