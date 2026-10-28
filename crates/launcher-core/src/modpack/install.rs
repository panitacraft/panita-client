//! First installation of a custom pack and its recovery. The base game and loader
//! are prepared first; pack files then flow through the verified cache into the game
//! directory under an active journal. `installed.json` is the single commit point:
//! before it exists a failure removes only what this operation created, and after
//! it exists recovery rolls forward. Existing player files are never overwritten.
use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{
    Environment, Manifest, ManifestFile, Origin,
    archive::{self, Limits},
    cache::ManifestClient,
    config::{self, Kind},
    content::{self, ProgressSink, Request},
    gamefs::{Content, GameDir},
    invalid,
    manual::{self, PendingFile, PendingReason, Placement},
    state::{ManagedFile, Ownership, PackIdentity, PackState},
    transfer::{Expected, PackDownloader},
};
use crate::{
    catalog::{CatalogClient, CatalogSource},
    error::{CoreError, ErrorCode},
    minecraft::{
        Platform,
        download::{Cancellation, io, lock, try_lock},
        install::{
            InstalledLock, Progress, metadata_root, read_json, recover_locked as recover_base,
            verify_manifest, write_json,
        },
        maintenance::validate_manifest,
        transaction,
    },
    panita_api::PanitaCredentials,
    paths::AppPaths,
    registry::{InstallationRecord, InstallationState, InstanceRegistry},
};

const KIND: &str = "pack_install";

/// A destination this operation created because it was absent at preflight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Created {
    path: String,
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha512: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Journal {
    schema_version: u32,
    kind: String,
    instance_id: Uuid,
    operation: Uuid,
    active: bool,
    created: Vec<Created>,
}

impl Journal {
    fn validate(&self, id: Uuid) -> Result<(), CoreError> {
        if self.schema_version != 4
            || self.kind != KIND
            || self.instance_id != id
            || self.operation.is_nil()
            || self.created.len() > super::MAX_FILES * 2
        {
            return Err(invalid());
        }
        for file in &self.created {
            super::game_path(&file.path)?;
        }
        Ok(())
    }
}

pub(crate) fn journal_inactive(value: Value, id: Uuid) -> bool {
    serde_json::from_value::<Journal>(value)
        .is_ok_and(|journal| journal.validate(id).is_ok() && !journal.active)
}

/// Result of a completed installation pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Manual files the player must still supply; Play stays blocked until empty.
    pub awaiting: Vec<PendingFile>,
    /// Configuration files kept whole because they already existed.
    pub skipped_configuration: Vec<String>,
    pub files: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Installed(Report),
    /// Existing player files sit where required pack files belong; nothing changed.
    Conflicts(Vec<String>),
}

fn expected_for(file: &ManagedFile) -> Expected {
    Expected::file(&file.desired, file.size)
}

fn created_content(file: &ManifestFile) -> Created {
    Created {
        path: file.path.clone(),
        size: file.size,
        sha1: file.hashes.sha1.clone(),
        sha512: file.hashes.sha512.clone(),
    }
}

/// Delete only files this operation created and that still look like them.
fn rollback(game: &GameDir, created: &[Created], cancel: &Cancellation) -> Result<(), CoreError> {
    for file in created {
        let expected = Expected {
            size: file.size,
            sha1: file.sha1.clone(),
            sha256: None,
            sha512: file.sha512.clone(),
        };
        let matches = if expected.sha1.is_none() && expected.sha512.is_none() {
            // Configuration entries are known only by declared size.
            game.entry(&file.path)? == super::gamefs::Entry::File
                && game.size(&file.path)? == Some(file.size)
        } else {
            game.compare(&file.path, &expected, cancel)? == Content::Matches
        };
        if matches {
            game.remove_file(&file.path)?;
        }
    }
    Ok(())
}

/// Every managed content file must exist with its exact bytes.
pub fn verify_files(
    game: &GameDir,
    pack: &PackState,
    cancel: &Cancellation,
) -> Result<(), CoreError> {
    for file in pack
        .files
        .iter()
        .filter(|file| file.ownership == Ownership::Content)
    {
        cancel.check()?;
        if game.compare(&file.path, &expected_for(file), cancel)? != Content::Matches {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
    }
    Ok(())
}

/// Caller holds `.installation.lock`. Returns true when a pack journal was handled.
pub(crate) fn recover_locked(paths: &AppPaths, id: Uuid, root: &Path) -> Result<bool, CoreError> {
    let Some(value) = read_json::<Value>(root, "transaction.json")? else {
        return Ok(false);
    };
    if value["schema_version"] != 4 {
        return Ok(false);
    }
    let mut journal: Journal = serde_json::from_value(value).map_err(|_| invalid())?;
    journal.validate(id)?;
    if !journal.active {
        transaction::remove(root)?;
        return Ok(true);
    }
    let cancel = Cancellation::default();
    let registry = InstanceRegistry::new(paths.clone());
    let committed = read_json::<InstalledLock>(root, "installed.json")?
        .filter(|installed| installed.operation == journal.operation && installed.pack.is_some());
    let state = if let Some(installed) = committed {
        // The pointer is durable: finish forward after a complete local audit.
        validate_manifest(&installed, id)?;
        verify_manifest(paths, id, root, &installed, &cancel)?;
        let pack = installed.pack.as_ref().ok_or_else(invalid)?;
        verify_files(&GameDir::open(&root.join("minecraft"))?, pack, &cancel)?;
        if pack.pending.is_empty() {
            InstallationState::Installed
        } else {
            InstallationState::AwaitingFiles
        }
    } else {
        let game = GameDir::open(&root.join("minecraft"))?;
        rollback(&game, &journal.created, &cancel)?;
        InstallationState::Failed
    };
    if let Some(mut record) = registry.get(id)? {
        record.state = state;
        registry.save_installation(&record)?;
    }
    journal.active = false;
    write_json(root, "transaction.json", &journal)?;
    transaction::remove(root)?;
    Ok(true)
}

pub struct Publish<'a> {
    pub paths: &'a AppPaths,
    pub id: Uuid,
    pub root: &'a Path,
    /// The prepared vanilla and loader installation, not yet published.
    pub base: InstalledLock,
    pub manifest: &'a Manifest,
    pub downloader: &'a PackDownloader,
    pub cancel: &'a Cancellation,
    pub progress: ProgressSink,
}

fn pending_state(pack: &PackState) -> InstallationState {
    if pack.pending.is_empty() {
        InstallationState::Installed
    } else {
        InstallationState::AwaitingFiles
    }
}

/// Install the pack files into the game directory and publish `installed.json`.
/// The caller holds `.installation.lock` and has prepared `base`.
pub async fn publish(request: Publish<'_>) -> Result<Outcome, CoreError> {
    let Publish {
        paths,
        id,
        root,
        base,
        manifest,
        downloader,
        cancel,
        progress,
    } = request;
    let game = GameDir::open(&root.join("minecraft"))?;
    let mut installable: Vec<&ManifestFile> = Vec::new();
    let mut pending: Vec<PendingFile> = Vec::new();
    for file in &manifest.files {
        if file.env.client == Environment::Unsupported {
            continue;
        }
        if file.manual {
            pending.push(PendingFile::from_manifest(file, PendingReason::Manual)?);
        } else {
            installable.push(file);
        }
    }
    // Existing player content is reported, never adopted or overwritten.
    let mut conflicts = BTreeSet::new();
    for file in &installable {
        cancel.check()?;
        if game.entry(&file.path)? != super::gamefs::Entry::Absent {
            conflicts.insert(file.path.clone());
        }
    }
    for file in &pending {
        if !matches!(
            manual::check(&game, file, cancel)?,
            Placement::Missing | Placement::Satisfied
        ) {
            conflicts.insert(file.path.clone());
        }
    }
    if !conflicts.is_empty() {
        return Ok(Outcome::Conflicts(conflicts.into_iter().collect()));
    }

    let mut requests = Vec::new();
    for file in &installable {
        requests.push(Request {
            url: file.url.clone().ok_or_else(invalid)?,
            origin: file.source.clone(),
            expected: Expected::file(&file.hashes, file.size),
        });
    }
    let archives = [&manifest.archives.defaults, &manifest.archives.enforced];
    let mut archive_expected: [Option<Expected>; 2] = [None, None];
    for (slot, archive) in archive_expected.iter_mut().zip(archives) {
        if let Some(archive) = archive {
            let expected = Expected::archive(&archive.sha256, archive.size);
            requests.push(Request {
                url: archive.url.clone(),
                origin: Origin::Upload,
                expected: expected.clone(),
            });
            *slot = Some(expected);
        }
    }
    let ensured = content::ensure_all(paths, downloader, &requests, cancel, progress).await?;

    let mut content_files: Vec<&ManifestFile> = Vec::new();
    for (file, result) in installable.iter().zip(&ensured) {
        match result {
            content::Ensured::Ready { .. } => content_files.push(file),
            content::Ensured::Withdrawn => {
                pending.push(PendingFile::from_manifest(file, PendingReason::Withdrawn)?);
            }
        }
    }

    let mut defaults = None;
    let mut enforced = None;
    for (kind, expected) in [Kind::Defaults, Kind::Enforced]
        .into_iter()
        .zip(&archive_expected)
    {
        let Some(expected) = expected else { continue };
        let file = content::open_verified(paths, expected, cancel)?;
        let opened = archive::preflight(file, Limits::PACK)?;
        match kind {
            Kind::Defaults => defaults = Some(opened),
            Kind::Enforced => enforced = Some(opened),
        }
    }
    if let (Some(first), Some(second)) = (&defaults, &enforced) {
        config::ensure_disjoint(&first.entries, &second.entries)?;
    }

    let mut created: Vec<Created> = content_files.iter().map(|f| created_content(f)).collect();
    for opened in [&defaults, &enforced].into_iter().flatten() {
        for entry in &opened.entries {
            if game.entry(&entry.path)? == super::gamefs::Entry::Absent {
                created.push(Created {
                    path: entry.path.clone(),
                    size: entry.size,
                    sha1: None,
                    sha512: None,
                });
            }
        }
    }
    let mut journal = Journal {
        schema_version: 4,
        kind: KIND.into(),
        instance_id: id,
        operation: base.operation,
        active: true,
        created,
    };
    journal.validate(id)?;
    write_json(root, "transaction.json", &journal)?;

    let mut committed = false;
    let result: Result<PackState, CoreError> = async {
        for file in &content_files {
            cancel.check()?;
            let expected = Expected::file(&file.hashes, file.size);
            game.write_new(&file.path, |output| {
                content::stream_verified(paths, &expected, cancel, |chunk| {
                    output.write_all(chunk).map_err(io)
                })
            })?;
        }
        let mut files: Vec<ManagedFile> = content_files
            .iter()
            .map(|file| ManagedFile {
                path: file.path.clone(),
                ownership: Ownership::Content,
                desired: file.hashes.clone(),
                size: file.size,
                baseline: Some(file.hashes.clone()),
            })
            .collect();
        let mut consumed = BTreeSet::new();
        let mut skipped = Vec::new();
        for (kind, opened) in [
            (Kind::Defaults, &mut defaults),
            (Kind::Enforced, &mut enforced),
        ] {
            let Some(opened) = opened else { continue };
            let mut budget = opened.budget();
            let outcome =
                config::apply(opened, kind, &game, &BTreeSet::new(), &mut budget, cancel)?;
            files.extend(outcome.files);
            consumed.extend(outcome.consumed_defaults);
            skipped.extend(outcome.skipped);
        }
        let state = PackState {
            schema: 1,
            id,
            source: PackIdentity::Custom {
                version_id: manifest.version_id,
                published_at: manifest.published_at.clone(),
                label: manifest.label.clone(),
            },
            minecraft_version: manifest.minecraft_version.clone(),
            loader: manifest.loader.clone(),
            loader_version: manifest.loader_version.clone(),
            files,
            consumed_defaults: consumed,
            skipped_configuration: skipped,
            pending: pending.clone(),
        };
        state.validate(id)?;
        let installed = InstalledLock {
            pack: Some(state.clone()),
            ..base.clone()
        };
        validate_manifest(&installed, id)?;
        cancel.check()?;
        write_json(root, "installed.json", &installed)?;
        committed = true;
        Ok(state)
    }
    .await;

    match result {
        Ok(state) => {
            let registry = InstanceRegistry::new(paths.clone());
            if let Some(mut record) = registry.get(id)? {
                record.state = pending_state(&state);
                registry.save_installation(&record)?;
            }
            journal.active = false;
            write_json(root, "transaction.json", &journal)?;
            transaction::remove(root)?;
            Ok(Outcome::Installed(Report {
                awaiting: state.pending.clone(),
                skipped_configuration: state
                    .skipped_configuration
                    .iter()
                    .map(|skip| skip.path.clone())
                    .collect(),
                files: state.files.len(),
            }))
        }
        Err(error) => {
            if committed {
                // The pointer is durable; recovery finishes forward on next use.
                return Err(error);
            }
            // Retain the active journal if cleanup cannot finish, so recovery retries.
            rollback(&game, &journal.created, &Cancellation::default())?;
            journal.active = false;
            write_json(root, "transaction.json", &journal)?;
            transaction::remove(root)?;
            Err(error)
        }
    }
}

/// Install the current exact catalog pin of a custom pack.
pub async fn install(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    cancel: &Cancellation,
    progress: Progress,
    pack_progress: ProgressSink,
    auth: Option<PanitaCredentials>,
) -> Result<Outcome, CoreError> {
    let catalog_client = CatalogClient::new()?.with_auth(auth);
    let catalog = tokio::select! {
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = catalog_client.detail(id) => result?,
    };
    let registry = InstanceRegistry::new(paths.clone());
    registry.remember_catalog(std::slice::from_ref(&catalog))?;
    if !catalog.enabled {
        return Err(CoreError::new(ErrorCode::InstanceDisabled));
    }
    if !catalog
        .compatible_with(&semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?)
    {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    let CatalogSource::Custom {
        current_version_id: Some(version),
        ..
    } = &catalog.source
    else {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    };
    let manifests = ManifestClient::new()?;
    let cached = tokio::select! {
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = manifests.fetch(id, *version, None) => result?,
    };
    let manifest = cached.manifest();
    manifest.bind_current(&catalog)?;
    if !manifest.supported_loader() {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    cancel.check()?;
    let root = registry.ensure_catalog_instance(&catalog)?;
    let _guard = lock(&root, ".installation.lock", cancel).await?;
    recover_base(paths, id, &root)?;
    if read_json::<InstalledLock>(&root, "installed.json")?
        .is_some_and(|installed| installed.schema_version == 2)
    {
        return Err(CoreError::new(ErrorCode::OperationBusy));
    }
    let mut record = InstallationRecord {
        schema_version: 1,
        id,
        directory_name: None,
        last_catalog: catalog.clone(),
        resolved_source: catalog.source.clone(),
        state: InstallationState::Installing,
        managed_files: Vec::new(),
        settings: crate::settings::InstanceSettings::default(),
    };
    registry.save_installation(&record)?;
    let result = async {
        let operation = Uuid::new_v4();
        let base = crate::neoforge::loader::prepare_base(
            paths,
            id,
            operation,
            platform,
            &manifest.minecraft_version,
            &manifest.loader_version,
            cancel,
            progress,
        )
        .await?;
        let downloader = PackDownloader::new()?;
        publish(Publish {
            paths,
            id,
            root: &root,
            base,
            manifest,
            downloader: &downloader,
            cancel,
            progress: pack_progress,
        })
        .await
    }
    .await;
    match &result {
        // `publish` stored the final state, or reported conflicts without installing.
        Ok(Outcome::Installed(_)) => {}
        Ok(Outcome::Conflicts(_)) => {
            record.state = InstallationState::Failed;
            registry.save_installation(&record)?;
        }
        Err(error) => {
            // A committed pointer is finished by recovery; do not mark it failed.
            let committed = read_json::<InstalledLock>(&root, "installed.json")?
                .is_some_and(|installed| installed.pack.is_some());
            if !committed {
                record.state = if error.code() == ErrorCode::OperationCancelled {
                    InstallationState::Cancelled
                } else {
                    InstallationState::Failed
                };
                registry.save_installation(&record)?;
            }
        }
    }
    result
}

/// Re-check the pending list against the player's folder and publish the result.
/// Local only: no session and no network. Returns what is still missing.
pub fn refresh_pending(
    paths: &AppPaths,
    id: Uuid,
    cancel: &Cancellation,
) -> Result<Vec<PendingFile>, CoreError> {
    let root = metadata_root(paths, id)?;
    let _guard = try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    recover_base(paths, id, &root)?;
    let mut installed = read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
    validate_manifest(&installed, id)?;
    let mut pack = installed.pack.take().ok_or_else(invalid)?;
    let game = GameDir::open(&root.join("minecraft"))?;
    let mut remaining = Vec::new();
    let mut changed = false;
    for file in std::mem::take(&mut pack.pending) {
        cancel.check()?;
        if manual::check(&game, &file, cancel)? == Placement::Satisfied {
            let desired = super::Hashes {
                sha1: Some(file.sha1.clone()),
                sha512: None,
            };
            pack.files.push(ManagedFile {
                path: file.path.clone(),
                ownership: Ownership::Content,
                desired: desired.clone(),
                size: file.size,
                baseline: Some(desired),
            });
            changed = true;
        } else {
            remaining.push(file);
        }
    }
    pack.pending = remaining.clone();
    pack.validate(id)?;
    installed.pack = Some(pack);
    if changed {
        write_json(&root, "installed.json", &installed)?;
    }
    let registry = InstanceRegistry::new(paths.clone());
    if let Some(mut record) = registry.get(id)?
        && matches!(
            record.state,
            InstallationState::AwaitingFiles | InstallationState::Installed
        )
    {
        record.state = if remaining.is_empty() {
            InstallationState::Installed
        } else {
            InstallationState::AwaitingFiles
        };
        registry.save_installation(&record)?;
    }
    Ok(remaining)
}

/// Import a file the player picked for one pending entry, then refresh the list.
pub async fn import_pending(
    paths: &AppPaths,
    id: Uuid,
    path: &str,
    selected: &Path,
    cancel: &Cancellation,
) -> Result<Vec<PendingFile>, CoreError> {
    let root = metadata_root(paths, id)?;
    {
        let _guard = try_lock(&root, ".installation.lock")?
            .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
        let installed = read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
        let pack = installed.pack.ok_or_else(invalid)?;
        let file = pack
            .pending
            .iter()
            .find(|file| file.path == path)
            .ok_or_else(invalid)?;
        manual::import(
            paths,
            &GameDir::open(&root.join("minecraft"))?,
            file,
            selected,
            cancel,
        )
        .await?;
    }
    refresh_pending(paths, id, cancel)
}

/// Snapshot the pending list without changing anything.
pub fn pending_files(paths: &AppPaths, id: Uuid) -> Result<Vec<PendingFile>, CoreError> {
    let root = metadata_root(paths, id)?;
    let installed = read_json::<InstalledLock>(&root, "installed.json")?.ok_or_else(invalid)?;
    Ok(installed.pack.map(|pack| pack.pending).unwrap_or_default())
}

#[cfg(test)]
mod tests;
