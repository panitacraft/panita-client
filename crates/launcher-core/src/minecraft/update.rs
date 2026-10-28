//! Explicit vanilla version transitions. Publication is recovered forward; worlds
//! are never rolled back, restored, modified, or deleted by this transaction.
pub use super::world_backup::BackupReceipt;
use super::{
    Platform,
    download::{Cancellation, try_lock},
    install::{self, InstalledLock, Progress, read_json, write_json},
    invalid, world_backup,
};
use crate::{
    catalog::{CatalogClient, CatalogSource},
    error::{CoreError, ErrorCode},
    panita_api::PanitaCredentials,
    paths::AppPaths,
    registry::{InstallationState, InstanceRegistry},
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    pub from_version: String,
    pub to_version: String,
    pub backup_worlds: bool,
    /// Packs only: continue past conflicts by moving the player's files aside.
    #[serde(default)]
    pub move_aside: bool,
}

pub struct UpdateOutcome {
    pub installed: InstalledLock,
    pub backup: Option<BackupReceipt>,
    pub native_cleanup: super::native_cache::CleanupReport,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    instance_id: Uuid,
    operation: Uuid,
    previous_operation: Uuid,
    from_version: String,
    to_version: String,
    active: bool,
    backup_requested: bool,
    backup: Option<world_backup::BackupReceipt>,
    prepared: Option<InstalledLock>,
}

fn version(value: &str) -> Result<(), CoreError> {
    if value.len() > 128 || value.contains('/') {
        return Err(invalid());
    }
    super::safe_path(value)
}

impl Journal {
    fn validate(&self, id: Uuid) -> Result<(), CoreError> {
        version(&self.from_version)?;
        version(&self.to_version)?;
        if self.schema_version != 2
            || self.instance_id != id
            || id.is_nil()
            || self.operation.is_nil()
            || self.previous_operation.is_nil()
            || self.operation == self.previous_operation
            || self.from_version == self.to_version
            || self.backup.as_ref().is_some_and(|backup| {
                backup.schema_version != 1
                    || backup.instance_id != id
                    || backup.installed_version != self.from_version
                    || !self.backup_requested
                    || backup.operation != self.operation
            })
            || (!self.active && self.prepared.is_some())
        {
            return Err(invalid());
        }
        if let Some(prepared) = &self.prepared {
            prepared.plan.validate()?;
            if prepared.schema_version != 2
                || prepared.instance_id != id
                || prepared.operation != self.operation
                || prepared.plan.version.id != self.to_version
            {
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

/// Return false for an older install journal. Called under the operation lock.
/// An old pointer abandons uncommitted staging; a new pointer completes metadata.
pub(super) fn recover_locked(paths: &AppPaths, id: Uuid, root: &Path) -> Result<bool, CoreError> {
    let Some(value) = read_json::<serde_json::Value>(root, "transaction.json")? else {
        return Ok(false);
    };
    if value["schema_version"] != 2 {
        return Ok(false);
    }
    let mut journal: Journal = serde_json::from_value(value).map_err(|_| invalid())?;
    journal.validate(id)?;
    if !journal.active {
        super::transaction::remove(root)?;
        return Ok(true);
    }
    let installed = read_json::<InstalledLock>(root, "installed.json")?.ok_or_else(invalid)?;
    if !matches!(installed.schema_version, 2 | 3) || installed.instance_id != id {
        return Err(invalid());
    }
    installed.plan.validate()?;
    if installed.operation == journal.previous_operation
        && installed.plan.version.id == journal.from_version
    {
        let record = InstanceRegistry::new(paths.clone())
            .get(id)?
            .ok_or_else(invalid)?;
        if record.resolved_source
            != (CatalogSource::Vanilla {
                minecraft_version: journal.from_version.clone(),
            })
        {
            return Err(invalid());
        }
        // Nothing changed in the game directory or old installation descriptor.
    } else if installed.operation == journal.operation {
        if installed.schema_version != 2 {
            return Err(invalid());
        }
        let prepared = journal.prepared.as_ref().ok_or_else(invalid)?;
        if serde_json::to_value(prepared).map_err(|_| invalid())?
            != serde_json::to_value(&installed).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        install::verify_manifest(paths, id, root, &installed, &Cancellation::default())?;
        publish_record(paths, id, &journal.from_version, &journal.to_version)?;
        let _ = super::native_cache::after_update(root, &installed);
    } else {
        return Err(invalid());
    }
    journal.active = false;
    journal.prepared = None;
    write_json(root, "transaction.json", &journal)?;
    super::transaction::remove(root)?;
    Ok(true)
}

/// A broken published update may be explicitly uninstalled, but only when its
/// private intent and current pointer prove ownership of the same generation.
pub(super) fn validate_for_uninstall(root: &Path, id: Uuid) -> Result<(), CoreError> {
    let journal = read_json::<Journal>(root, "transaction.json")?.ok_or_else(invalid)?;
    journal.validate(id)?;
    let installed = read_json::<InstalledLock>(root, "installed.json")?.ok_or_else(invalid)?;
    super::maintenance::validate_manifest(&installed, id)?;
    let previous = installed.operation == journal.previous_operation
        && installed.plan.version.id == journal.from_version;
    let published = journal.prepared.as_ref().is_some_and(|prepared| {
        serde_json::to_value(prepared).ok() == serde_json::to_value(&installed).ok()
    });
    if previous || published {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn publish_record(
    paths: &AppPaths,
    id: Uuid,
    previous: &str,
    version: &str,
) -> Result<(), CoreError> {
    let registry = InstanceRegistry::new(paths.clone());
    let mut record = registry.get(id)?.ok_or_else(invalid)?;
    if ![previous, version].iter().any(|version| {
        record.resolved_source
            == (CatalogSource::Vanilla {
                minecraft_version: (*version).to_owned(),
            })
    }) {
        return Err(invalid());
    }
    record.resolved_source = CatalogSource::Vanilla {
        minecraft_version: version.to_owned(),
    };
    record.state = InstallationState::Installed;
    registry.save_installation(&record)
}

fn publish(
    paths: &AppPaths,
    id: Uuid,
    root: &Path,
    journal: &mut Journal,
) -> Result<super::native_cache::CleanupReport, CoreError> {
    publish_steps(paths, id, root, journal, |_| Ok(()))
}

#[derive(Clone, Copy, PartialEq)]
enum CommitStage {
    BeforePointer,
    PointerPublished,
    RecordPublished,
    JournalCompleted,
}

fn publish_steps(
    paths: &AppPaths,
    id: Uuid,
    root: &Path,
    journal: &mut Journal,
    mut checkpoint: impl FnMut(CommitStage) -> Result<(), CoreError>,
) -> Result<super::native_cache::CleanupReport, CoreError> {
    checkpoint(CommitStage::BeforePointer)?;
    let prepared = journal.prepared.as_ref().ok_or_else(invalid)?;
    write_json(root, "installed.json", prepared)?;
    checkpoint(CommitStage::PointerPublished)?;
    publish_record(paths, id, &journal.from_version, &journal.to_version)?;
    checkpoint(CommitStage::RecordPublished)?;
    let native_cleanup = super::native_cache::after_update(root, prepared);
    journal.active = false;
    journal.prepared = None;
    write_json(root, "transaction.json", journal)?;
    super::transaction::remove(root)?;
    checkpoint(CommitStage::JournalCompleted)?;
    Ok(native_cleanup)
}

pub async fn update(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    request: UpdateRequest,
    cancel: &Cancellation,
    progress: Progress,
    auth: Option<PanitaCredentials>,
) -> Result<UpdateOutcome, CoreError> {
    let catalog = CatalogClient::new()?.with_auth(auth);
    update_with_catalog(paths, id, platform, request, cancel, progress, catalog).await
}

async fn update_with_catalog(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    request: UpdateRequest,
    cancel: &Cancellation,
    progress: Progress,
    client: CatalogClient,
) -> Result<UpdateOutcome, CoreError> {
    version(&request.from_version)?;
    version(&request.to_version)?;
    if id.is_nil() || request.from_version == request.to_version {
        return Err(invalid());
    }
    cancel.check()?;
    let catalog = tokio::select! {
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = client.detail(id) => result?,
    };
    let registry = InstanceRegistry::new(paths.clone());
    registry.remember_catalog(std::slice::from_ref(&catalog))?;
    catalog.installation_allowed(
        &semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?,
    )?;
    if catalog.source
        != (CatalogSource::Vanilla {
            minecraft_version: request.to_version.clone(),
        })
    {
        return Err(CoreError::new(ErrorCode::UpdateSelectionChanged));
    }
    let root = install::metadata_root(paths, id)?;
    let _lease = try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    install::recover_locked(paths, id, &root)?;
    let record = registry.get(id)?.ok_or_else(invalid)?;
    let stored = read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
    let uninstalled = stored.schema_version == 3
        && !matches!(
            record.state,
            InstallationState::Installed
                | InstallationState::UpdateAvailable
                | InstallationState::Running
        );
    let previous = if uninstalled {
        super::maintenance::validate_manifest(&stored, id)?;
        stored
    } else {
        install::verify_under_lock(paths, id, &root, cancel)?
    };
    if previous.plan.version.id != request.from_version
        || record.resolved_source
            != (CatalogSource::Vanilla {
                minecraft_version: request.from_version.clone(),
            })
    {
        return Err(CoreError::new(ErrorCode::UpdateSelectionChanged));
    }
    if !uninstalled
        && !matches!(
            record.state,
            InstallationState::Installed
                | InstallationState::UpdateAvailable
                | InstallationState::NotInstalled
        )
    {
        return Err(invalid());
    }
    let operation = Uuid::new_v4();
    let mut journal = Journal {
        schema_version: 2,
        instance_id: id,
        operation,
        previous_operation: previous.operation,
        from_version: request.from_version,
        to_version: request.to_version,
        active: true,
        backup_requested: request.backup_worlds,
        backup: None,
        prepared: None,
    };
    write_json(&root, "transaction.json", &journal)?;
    let result: Result<UpdateOutcome, CoreError> = async {
        let installed = install::prepare(
            paths,
            id,
            operation,
            platform,
            &journal.to_version,
            cancel,
            progress,
        )
        .await?;
        if journal.backup_requested {
            journal.backup =
                world_backup::create(&root, id, &journal.from_version, operation, cancel)?;
        }
        journal.prepared = Some(installed.clone());
        journal.validate(id)?;
        write_json(&root, "transaction.json", &journal)?;
        let native_cleanup = cancel.commit(|| publish(paths, id, &root, &mut journal))?;
        Ok(UpdateOutcome {
            installed,
            backup: journal.backup.clone(),
            native_cleanup,
        })
    }
    .await;
    match result {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            // Recover publication failures under the same lease. Never clear a
            // journal if recovery cannot establish which pointer is authoritative.
            if recover_locked(paths, id, &root).is_err() {
                return Err(CoreError::new(ErrorCode::StorageCommitUncertain));
            }
            let installed =
                read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
            if installed.operation == operation {
                let native_cleanup = super::native_cache::after_update(&root, &installed);
                Ok(UpdateOutcome {
                    installed,
                    backup: journal.backup,
                    native_cleanup,
                })
            } else {
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests;
