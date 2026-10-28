//! Update, repair and uninstall of installed packs. Every file change is journaled in
//! a ledger before it happens. Bytes the launcher replaces or removes are set aside
//! rather than deleted, and a player file that must make way is moved to a permanent
//! backup folder. `installed.json` is the commit point: before it is published,
//! recovery puts everything back; after it, recovery finishes forward. Worlds, player
//! files, other instances and the shared caches are never touched.
use serde::{Deserialize, Serialize};

use super::*;
use crate::{
    catalog::CatalogSource,
    minecraft::{
        install::prepare_recorded,
        native_cache::{self, CleanupReport},
        world_backup::{self, BackupReceipt},
    },
    modpack::{
        diff::{self, Conflict, Diff, Incoming, Local, Op},
        gamefs::Stash,
    },
};

/// Test-only interruption points, so recovery is exercised at each durable boundary.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    AfterApply,
    AfterPointer,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct Interrupt {
    pub(crate) stage: Stage,
    /// Skip the in-process recovery, as a crash would.
    pub(crate) crash: bool,
}

#[cfg(test)]
thread_local! {
    pub(crate) static INTERRUPT: std::cell::Cell<Option<Interrupt>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn interrupt(stage: Stage) -> Result<(), CoreError> {
    if INTERRUPT.with(|cell| cell.get().is_some_and(|set| set.stage == stage)) {
        return Err(CoreError::new(ErrorCode::StorageUnavailable));
    }
    Ok(())
}

#[cfg(test)]
fn crashed() -> bool {
    INTERRUPT.with(|cell| cell.get().is_some_and(|set| set.crash))
}

#[cfg(not(test))]
fn crashed() -> bool {
    false
}

/// Where replaced and removed pack bytes wait until the operation completes.
const TRANSIENT: &str = ".pack-transaction";
/// Permanent home of player files an operation moved out of the way.
pub const BACKUPS: &str = "pack-backups";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    #[serde(rename = "pack_update")]
    Update,
    #[serde(rename = "pack_repair")]
    Repair,
    #[serde(rename = "pack_uninstall")]
    Uninstall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Write,
    Replace,
    Remove,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One planned file change. The new bytes are known by hash before anything happens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    action: Action,
    /// The existing file was the player's and moved to the permanent backups.
    #[serde(default, skip_serializing_if = "is_false")]
    backup: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha512: Option<String>,
}

impl Entry {
    fn hashes(&self) -> Hashes {
        Hashes {
            sha1: self.sha1.clone(),
            sha512: self.sha512.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    kind: Operation,
    instance_id: Uuid,
    /// Names the set-aside and backup folders; distinct from the installed operation.
    operation: Uuid,
    previous_operation: Uuid,
    active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    backup: Option<BackupReceipt>,
    ledger: Vec<Entry>,
    /// The exact pointer to publish; its presence in `installed.json` is the commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prepared: Option<InstalledLock>,
}

impl Journal {
    fn validate(&self, id: Uuid) -> Result<(), CoreError> {
        if self.schema_version != 5
            || self.instance_id != id
            || id.is_nil()
            || self.operation.is_nil()
            || self.previous_operation.is_nil()
            || self.ledger.len() > super::super::MAX_FILES * 2
            || (!self.active && self.prepared.is_some())
            || self.backup.as_ref().is_some_and(|backup| {
                backup.schema_version != 1
                    || backup.instance_id != id
                    || backup.operation != self.operation
            })
            || (self.kind == Operation::Uninstall && self.prepared.is_some())
        {
            return Err(invalid());
        }
        for entry in &self.ledger {
            super::super::game_path(&entry.path)?;
            let hashes = entry.hashes();
            let written = entry.action != Action::Remove;
            if written != entry.size.is_some()
                || (written && hashes.validate().is_err())
                || (!written && (hashes.sha1.is_some() || hashes.sha512.is_some()))
            {
                return Err(invalid());
            }
        }
        if let Some(prepared) = &self.prepared {
            validate_manifest(prepared, id)?;
            if prepared.pack.is_none() || prepared.schema_version != 2 {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

pub(super) fn journal_inactive(value: Value, id: Uuid) -> bool {
    serde_json::from_value::<Journal>(value)
        .is_ok_and(|journal| journal.validate(id).is_ok() && !journal.active)
}

fn same_document(left: &InstalledLock, right: &InstalledLock) -> bool {
    match (serde_json::to_value(left), serde_json::to_value(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// The catalog pin an installed pack stands for, derived from its recorded state.
fn source_of(pack: &PackState) -> CatalogSource {
    match &pack.source {
        PackIdentity::Custom {
            version_id,
            published_at,
            label,
        } => CatalogSource::Custom {
            minecraft_version: pack.minecraft_version.clone(),
            loader: pack.loader.clone(),
            current_version_id: Some(*version_id),
            label: label.clone(),
            published_at: Some(published_at.clone()),
        },
        PackIdentity::Modrinth {
            project_id,
            version_id,
            ..
        } => CatalogSource::Modrinth {
            project_id: project_id.clone(),
            pack_version_id: version_id.clone(),
            loader: Some(pack.loader.clone()),
        },
    }
}

/// True when the instance's installation is a pack, tombstones included.
pub fn is_pack(paths: &AppPaths, id: Uuid) -> Result<bool, CoreError> {
    let root = paths.instance(id)?;
    Ok(read_json::<InstalledLock>(&root, "installed.json")?
        .is_some_and(|installed| installed.pack.is_some()))
}

/// The pin string the update request names: the custom version UUID or Modrinth id.
pub fn pin_of(pack: &PackState) -> String {
    identity_pin(&pack.source)
}

fn identity_pin(identity: &PackIdentity) -> String {
    match identity {
        PackIdentity::Custom { version_id, .. } => version_id.to_string(),
        PackIdentity::Modrinth { version_id, .. } => version_id.clone(),
    }
}

fn record_state(
    paths: &AppPaths,
    id: Uuid,
    pack: &PackState,
    update_source: bool,
) -> Result<(), CoreError> {
    let registry = InstanceRegistry::new(paths.clone());
    if let Some(mut record) = registry.get(id)? {
        if update_source {
            record.resolved_source = source_of(pack);
        }
        record.state = pending_state(pack);
        registry.save_installation(&record)?;
    }
    Ok(())
}

fn mark_needs_repair(paths: &AppPaths, id: Uuid) -> Result<(), CoreError> {
    let registry = InstanceRegistry::new(paths.clone());
    if let Some(mut record) = registry.get(id)? {
        record.state = InstallationState::NeedsRepair;
        registry.save_installation(&record)?;
    }
    Ok(())
}

/// Record Play receipts for the new files. A kept edit shows up here as damage.
fn audit(paths: &AppPaths, id: Uuid, root: &Path) -> Result<(), CoreError> {
    match verify_play_under_lock(paths, id, root, &Cancellation::default()) {
        Err(error) if error.code() == ErrorCode::ArtifactMismatch => mark_needs_repair(paths, id),
        _ => Ok(()),
    }
}

fn complete(root: &Path, journal: &mut Journal) -> Result<(), CoreError> {
    journal.active = false;
    journal.prepared = None;
    write_json(root, "transaction.json", journal)?;
    transaction::remove(root)
}

/// Delete the transient set-aside folder. Handles must be closed first.
fn clear_transient(root: &Path, operation: Uuid) -> Result<(), CoreError> {
    Stash::discard(root, TRANSIENT, operation)
}

/// Remove the permanent backup folder only when nothing was backed up into it.
fn tidy_backups(root: &Path, operation: Uuid) -> Result<(), CoreError> {
    if let Some(stash) = Stash::existing(root, BACKUPS, operation)? {
        let empty = stash.is_empty()?;
        drop(stash);
        if empty {
            Stash::discard(root, BACKUPS, operation)?;
        }
    }
    Ok(())
}

/// Undo an operation whose pointer was never published. Everything the ledger wrote
/// is removed if it is still the launcher's, and everything set aside returns.
fn roll_back(root: &Path, journal: &Journal) -> Result<(), CoreError> {
    let game = GameDir::open(&root.join("minecraft"))?;
    let cancel = Cancellation::default();
    let transient = Stash::existing(root, TRANSIENT, journal.operation)?;
    let backups = Stash::existing(root, BACKUPS, journal.operation)?;
    for entry in journal.ledger.iter().rev() {
        if entry.action != Action::Remove
            && let Local::File(found) = game.fingerprint(&entry.path, &cancel)?
            && diff::same(&found, &entry.hashes())
        {
            game.remove_file(&entry.path)?;
        }
        if entry.action != Action::Write {
            let stash = if entry.backup { &backups } else { &transient };
            if let Some(stash) = stash {
                // An occupied destination is a conflict: keep the journal and try again.
                game.restore(&entry.path, stash)?;
            }
        }
    }
    drop(transient);
    drop(backups);
    clear_transient(root, journal.operation)?;
    tidy_backups(root, journal.operation)
}

/// Caller holds `.installation.lock`. Recovers a schema 5 journal.
pub(super) fn recover(
    paths: &AppPaths,
    id: Uuid,
    root: &Path,
    value: Value,
) -> Result<bool, CoreError> {
    let mut journal: Journal = serde_json::from_value(value).map_err(|_| invalid())?;
    journal.validate(id)?;
    if !journal.active {
        transaction::remove(root)?;
        return Ok(true);
    }
    let installed = read_json::<InstalledLock>(root, "installed.json")?.ok_or_else(invalid)?;
    validate_manifest(&installed, id)?;
    if journal.kind == Operation::Uninstall {
        let report = finish_uninstall(paths, id, root, &mut journal)?;
        if report
            .native_cleanup
            .is_some_and(|cleanup| cleanup.retained_generations > 0)
        {
            return Err(CoreError::new(ErrorCode::NativeCleanupIncomplete));
        }
        return Ok(true);
    }
    let prepared = journal.prepared.as_ref();
    if prepared.is_some_and(|prepared| same_document(prepared, &installed)) {
        // The pointer is durable: finish forward. A kept player edit or a damaged file
        // never blocks that; it only means the pack now needs a repair.
        let intact = match verify_manifest(paths, id, root, &installed, &Cancellation::default()) {
            Ok(()) => true,
            Err(error) if error.code() == ErrorCode::ArtifactMismatch => false,
            Err(error) => return Err(error),
        };
        let pack = installed.pack.as_ref().ok_or_else(invalid)?;
        record_state(paths, id, pack, true)?;
        if !intact {
            mark_needs_repair(paths, id)?;
        }
        if journal.kind == Operation::Update {
            let _ = native_cache::after_update(root, &installed);
        }
        clear_transient(root, journal.operation)?;
    } else if installed.operation == journal.previous_operation {
        roll_back(root, &journal)?;
        let state = match verify_manifest(paths, id, root, &installed, &Cancellation::default()) {
            Ok(()) => InstallationState::Installed,
            Err(error) if error.code() == ErrorCode::ArtifactMismatch => {
                InstallationState::NeedsRepair
            }
            Err(error) => return Err(error),
        };
        let registry = InstanceRegistry::new(paths.clone());
        if let Some(mut record) = registry.get(id)? {
            record.state = state;
            registry.save_installation(&record)?;
        }
    } else {
        return Err(invalid());
    }
    complete(root, &mut journal)?;
    Ok(true)
}

/// Where a new file's bytes come from.
enum Source {
    Item(Expected),
    Entry {
        layer: usize,
        entry: archive::ArchiveEntry,
    },
    /// Placed by the player; nothing is written.
    Manual,
}

/// Everything an operation needs once its inputs are verified.
struct Gathered {
    incoming: Vec<Incoming>,
    sources: Vec<Source>,
    layers: Vec<(Kind, Opened<File>)>,
    /// Manual files the player still has to supply.
    needs: Vec<PendingFile>,
}

/// Open the plan's configuration layers and describe every file the plan wants.
/// Content bytes are not downloaded here: the diff only needs their hashes.
async fn gather(
    paths: &AppPaths,
    downloader: &PackDownloader,
    plan: &Plan,
    game: &GameDir,
    cancel: &Cancellation,
    progress: ProgressSink,
) -> Result<Gathered, CoreError> {
    let mut out = Gathered {
        incoming: Vec::new(),
        sources: Vec::new(),
        layers: Vec::new(),
        needs: Vec::new(),
    };
    for item in &plan.items {
        out.incoming.push(Incoming {
            path: item.path.clone(),
            ownership: Ownership::Content,
            desired: item.hashes.clone(),
            size: item.size,
            adopt_equal: false,
            source: out.sources.len(),
        });
        out.sources
            .push(Source::Item(Expected::file(&item.hashes, item.size)));
    }
    for file in &plan.pending {
        match manual::check(game, file, cancel)? {
            Placement::Satisfied => {
                out.incoming.push(Incoming {
                    path: file.path.clone(),
                    ownership: Ownership::Content,
                    desired: Hashes {
                        sha1: Some(file.sha1.clone()),
                        sha512: None,
                    },
                    size: file.size,
                    adopt_equal: true,
                    source: out.sources.len(),
                });
                out.sources.push(Source::Manual);
            }
            _ => out.needs.push(file.clone()),
        }
    }
    let mut archives = Vec::new();
    let mut requests = Vec::new();
    for layer in &plan.layers {
        if let Layer::Archive {
            kind,
            url,
            expected,
        } = layer
        {
            requests.push(Request {
                url: url.clone(),
                origin: Origin::Upload,
                expected: expected.clone(),
                fallbacks: Vec::new(),
            });
            archives.push((*kind, expected.clone()));
        }
    }
    if !requests.is_empty() {
        content::ensure_all(paths, downloader, &requests, cancel, progress).await?;
    }
    for layer in &plan.layers {
        match layer {
            Layer::Overrides {
                expected,
                index,
                index_sha256,
            } => {
                let opened = mrpack::layer(paths, expected, index, index_sha256, cancel)?;
                out.layers.push((Kind::Override, opened));
            }
            Layer::Archive { kind, expected, .. } => {
                let file = content::open_verified(paths, expected, cancel)?;
                out.layers
                    .push((*kind, archive::preflight(file, Limits::PACK)?));
            }
        }
    }
    let named = |wanted: Kind| out.layers.iter().find(|(kind, _)| *kind == wanted);
    if let (Some((_, first)), Some((_, second))) = (named(Kind::Defaults), named(Kind::Enforced)) {
        config::ensure_disjoint(&first.entries, &second.entries)?;
    }
    for (number, (kind, opened)) in out.layers.iter_mut().enumerate() {
        let mut budget = opened.budget();
        for entry in opened.entries.clone() {
            cancel.check()?;
            let digest = opened.stream(&entry, &mut budget, cancel, None)?;
            let ownership = match kind {
                Kind::Defaults => Ownership::Defaults,
                Kind::Enforced => Ownership::Enforced,
                Kind::Override if mrpack::is_content(&entry.path) => Ownership::Content,
                Kind::Override => Ownership::Override,
            };
            out.incoming.push(Incoming {
                path: entry.path.clone(),
                ownership,
                desired: digest.hashes,
                size: digest.size,
                adopt_equal: false,
                source: out.sources.len(),
            });
            out.sources.push(Source::Entry {
                layer: number,
                entry,
            });
        }
    }
    Ok(out)
}

/// Download only the content the steps will write. Withdrawn files cannot be
/// fetched; they become manual files the player must supply, before any change.
async fn fetch_needed(
    paths: &AppPaths,
    downloader: &PackDownloader,
    plan: &Plan,
    diff: &Diff,
    cancel: &Cancellation,
    progress: ProgressSink,
) -> Result<Vec<PendingFile>, CoreError> {
    let wanted: BTreeSet<String> = diff
        .steps
        .iter()
        .filter(|step| step.op != Op::Remove)
        .map(|step| step.path.to_ascii_lowercase())
        .collect();
    let items: Vec<&Item> = plan
        .items
        .iter()
        .filter(|item| wanted.contains(&item.path.to_ascii_lowercase()))
        .collect();
    let requests: Vec<Request> = items
        .iter()
        .map(|item| Request {
            url: item.url.clone(),
            origin: item.origin.clone(),
            expected: Expected::file(&item.hashes, item.size),
            fallbacks: item.fallbacks.clone(),
        })
        .collect();
    let ensured = content::ensure_all(paths, downloader, &requests, cancel, progress).await?;
    let mut needs = Vec::new();
    for (item, result) in items.iter().zip(&ensured) {
        if matches!(result, Ensured::Withdrawn) {
            needs.push(item.withdrawn.clone().ok_or_else(invalid)?);
        }
    }
    Ok(needs)
}

fn ledger_of(diff: &Diff) -> Vec<Entry> {
    diff.steps
        .iter()
        .map(|step| {
            let (action, new) = match step.op {
                Op::Write => (Action::Write, step.incoming.as_ref()),
                Op::Replace => (Action::Replace, step.incoming.as_ref()),
                Op::Remove => (Action::Remove, None),
            };
            Entry {
                path: step.path.clone(),
                action,
                backup: step.backup,
                size: new.map(|incoming| incoming.size),
                sha1: new.and_then(|incoming| incoming.desired.sha1.clone()),
                sha512: new.and_then(|incoming| incoming.desired.sha512.clone()),
            }
        })
        .collect()
}

/// Apply the diff's steps. Set-aside happens before each write, so an interruption
/// at any point can be undone from the ledger and the stashes.
#[allow(clippy::too_many_arguments)]
fn apply(
    paths: &AppPaths,
    root: &Path,
    game: &GameDir,
    diff: &Diff,
    sources: &[Source],
    layers: &mut [(Kind, Opened<File>)],
    operation: Uuid,
    cancel: &Cancellation,
) -> Result<(), CoreError> {
    let transient = Stash::open(root, TRANSIENT, operation)?;
    let backups = Stash::open(root, BACKUPS, operation)?;
    for step in &diff.steps {
        cancel.check()?;
        if step.op != Op::Write {
            let stash = if step.backup { &backups } else { &transient };
            game.stash(&step.path, stash)?;
        }
        let Some(incoming) = &step.incoming else {
            continue;
        };
        match sources.get(incoming.source).ok_or_else(invalid)? {
            Source::Item(expected) => {
                game.write_new(&step.path, |output| {
                    content::stream_verified(paths, expected, cancel, |chunk| {
                        output.write_all(chunk).map_err(io)
                    })
                })?;
            }
            Source::Entry { layer, entry } => {
                let (_, opened) = layers.get_mut(*layer).ok_or_else(invalid)?;
                let mut budget = opened.budget();
                let mut digest = None;
                game.write_new(&step.path, |output| {
                    digest = Some(opened.stream(entry, &mut budget, cancel, Some(output))?);
                    Ok(())
                })?;
                // The bytes written must be the bytes the plan was diffed against.
                let written = digest.ok_or_else(invalid)?;
                if !diff::same(&written.hashes, &incoming.desired) {
                    return Err(CoreError::new(ErrorCode::ArtifactMismatch));
                }
            }
            Source::Manual => return Err(invalid()),
        }
    }
    Ok(())
}

/// What a completed update changed, for the player.
#[derive(Clone, Default)]
pub struct UpdateReport {
    pub added: usize,
    pub replaced: usize,
    pub removed: usize,
    /// Pack files that stop being managed; they stay as the player's files.
    pub released: Vec<String>,
    /// Configuration kept whole because the player changed it.
    pub skipped_configuration: Vec<String>,
    /// Player files moved to the permanent backups to make room for the pack's.
    pub backed_up: Vec<String>,
    pub backup: Option<BackupReceipt>,
    pub native_cleanup: Option<CleanupReport>,
}

/// Why an update changed nothing and what the player can do about it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocked {
    pub conflicts: Vec<Conflict>,
    pub files: Vec<PendingFile>,
}

#[derive(Clone)]
pub enum UpdateOutcome {
    Updated(UpdateReport),
    Blocked(Blocked),
}

/// An explicit version transition of one installed pack.
#[derive(Clone, Debug)]
pub struct UpdateRequest {
    /// The installed and the requested pin: custom version UUID or Modrinth id.
    pub from_version: String,
    pub to_version: String,
    pub backup_worlds: bool,
    /// Continue past conflicts by moving the player's files to the backups.
    pub move_aside: bool,
}

fn lock_and_recover(
    paths: &AppPaths,
    id: Uuid,
) -> Result<(std::path::PathBuf, impl Sized), CoreError> {
    let root = metadata_root(paths, id)?;
    let guard = try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    recover_base(paths, id, &root)?;
    Ok((root, guard))
}

/// The recorded installation of a pack that may be maintained.
fn installed_pack(root: &Path, id: Uuid) -> Result<InstalledLock, CoreError> {
    let installed = read_json::<InstalledLock>(root, "installed.json")?.ok_or_else(invalid)?;
    validate_manifest(&installed, id)?;
    if installed.schema_version != 2 || installed.pack.is_none() {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    Ok(installed)
}

fn maintainable(record: &InstallationRecord) -> bool {
    matches!(
        record.state,
        InstallationState::Installed
            | InstallationState::UpdateAvailable
            | InstallationState::NeedsRepair
            | InstallationState::AwaitingFiles
    )
}

#[allow(clippy::too_many_arguments)]
pub async fn update(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    request: UpdateRequest,
    cancel: &Cancellation,
    progress: Progress,
    pack_progress: ProgressSink,
    auth: Option<PanitaCredentials>,
) -> Result<UpdateOutcome, CoreError> {
    let client = CatalogClient::new()?.with_auth(auth);
    cancel.check()?;
    let catalog = tokio::select! {
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = client.detail(id) => result?,
    };
    update_with_catalog(
        paths,
        id,
        platform,
        request,
        cancel,
        progress,
        pack_progress,
        catalog,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_with_catalog(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    request: UpdateRequest,
    cancel: &Cancellation,
    progress: Progress,
    pack_progress: ProgressSink,
    catalog: CatalogInstance,
) -> Result<UpdateOutcome, CoreError> {
    if id.is_nil() || request.from_version == request.to_version {
        return Err(invalid());
    }
    let registry = InstanceRegistry::new(paths.clone());
    registry.remember_catalog(std::slice::from_ref(&catalog))?;
    catalog.installation_allowed(
        &semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?,
    )?;
    let target = match &catalog.source {
        CatalogSource::Custom {
            current_version_id: Some(version),
            ..
        } => version.to_string(),
        CatalogSource::Modrinth {
            pack_version_id, ..
        } => pack_version_id.clone(),
        _ => return Err(CoreError::new(ErrorCode::UnsupportedInstance)),
    };
    if target != request.to_version {
        return Err(CoreError::new(ErrorCode::UpdateSelectionChanged));
    }
    let downloader = PackDownloader::new()?;
    // Everything below the lock needs the plan, so resolve it first, as install does.
    let plan = match &catalog.source {
        CatalogSource::Custom { .. } => custom_plan(&catalog, id, cancel).await?,
        _ => mrpack_plan(paths, &downloader, &catalog, cancel, pack_progress.clone()).await?,
    };
    run_update(
        paths,
        id,
        platform,
        request,
        plan,
        &downloader,
        BaseSource::Network,
        cancel,
        progress,
        pack_progress,
    )
    .await
}

/// Where the base installation of an operation comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BaseSource {
    /// Game, Java and loader are prepared or verified, downloading what is missing.
    Network,
    /// Keep the recorded base as it is. Only tests that run without a network use it.
    #[allow(dead_code)]
    Recorded,
}

/// A recorded base under a fresh operation, without touching the network.
fn recorded_base(previous: &InstalledLock) -> InstalledLock {
    let operation = Uuid::new_v4();
    InstalledLock {
        operation,
        native_directory: Some(format!("natives/{operation}")),
        ..previous.clone()
    }
}

/// Update an installed pack to an already resolved plan, under the instance lease.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_update(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    request: UpdateRequest,
    plan: Plan,
    downloader: &PackDownloader,
    base_source: BaseSource,
    cancel: &Cancellation,
    progress: Progress,
    pack_progress: ProgressSink,
) -> Result<UpdateOutcome, CoreError> {
    let registry = InstanceRegistry::new(paths.clone());
    let (root, _guard) = lock_and_recover(paths, id)?;
    let previous = installed_pack(&root, id)?;
    let old = previous.pack.clone().ok_or_else(invalid)?;
    let record = registry.get(id)?.ok_or_else(invalid)?;
    if !maintainable(&record)
        || pin_of(&old) != request.from_version
        || identity_pin(&plan.identity) != request.to_version
    {
        return Err(CoreError::new(ErrorCode::UpdateSelectionChanged));
    }
    // A pack may move to new game and loader versions, but not to another loader.
    if old.loader != plan.kind.name() {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    let game = GameDir::open(&root.join("minecraft"))?;
    let gathered = gather(
        paths,
        downloader,
        &plan,
        &game,
        cancel,
        pack_progress.clone(),
    )
    .await?;
    let Gathered {
        incoming,
        sources,
        mut layers,
        mut needs,
    } = gathered;
    let result = diff::diff(&old, incoming, &mut |path| game.fingerprint(path, cancel))?;
    if !result.conflicts.is_empty() && (!request.move_aside || result.unresolvable()) {
        return Ok(UpdateOutcome::Blocked(Blocked {
            conflicts: result.conflicts,
            files: needs,
        }));
    }
    needs.extend(fetch_needed(paths, downloader, &plan, &result, cancel, pack_progress).await?);
    if !needs.is_empty() {
        return Ok(UpdateOutcome::Blocked(Blocked {
            conflicts: result.conflicts,
            files: needs,
        }));
    }

    let operation = Uuid::new_v4();
    let mut journal = Journal {
        schema_version: 5,
        kind: Operation::Update,
        instance_id: id,
        operation,
        previous_operation: previous.operation,
        active: true,
        backup: None,
        ledger: Vec::new(),
        prepared: None,
    };
    write_json(&root, "transaction.json", &journal)?;
    let published: Result<UpdateReport, CoreError> = async {
        let same_base = plan.minecraft_version == old.minecraft_version
            && plan.loader_version == old.loader_version;
        let base = if same_base {
            previous.clone()
        } else if base_source == BaseSource::Recorded {
            recorded_base(&previous)
        } else {
            crate::loader::prepare_base(
                paths,
                id,
                Uuid::new_v4(),
                platform,
                plan.kind,
                &plan.minecraft_version,
                &plan.loader_version,
                cancel,
                progress,
            )
            .await?
        };
        if request.backup_worlds {
            journal.backup =
                world_backup::create(&root, id, &previous.plan.version.id, operation, cancel)?;
        }
        let state = PackState {
            schema: 1,
            id,
            source: plan.identity.clone(),
            minecraft_version: plan.minecraft_version.clone(),
            loader: plan.loader.clone(),
            loader_version: plan.loader_version.clone(),
            files: result.files.clone(),
            consumed_defaults: result.consumed.clone(),
            skipped_configuration: result.skipped.clone(),
            pending: Vec::new(),
        };
        state.validate(id)?;
        let installed = InstalledLock {
            pack: Some(state),
            ..base
        };
        validate_manifest(&installed, id)?;
        journal.ledger = ledger_of(&result);
        journal.prepared = Some(installed.clone());
        journal.validate(id)?;
        write_json(&root, "transaction.json", &journal)?;
        apply(
            paths,
            &root,
            &game,
            &result,
            &sources,
            &mut layers,
            operation,
            cancel,
        )?;
        #[cfg(test)]
        interrupt(Stage::AfterApply)?;
        cancel.commit(|| write_json(&root, "installed.json", &installed))?;
        #[cfg(test)]
        interrupt(Stage::AfterPointer)?;
        let pack = installed.pack.as_ref().ok_or_else(invalid)?;
        record_state(paths, id, pack, true)?;
        let cleanup = native_cache::after_update(&root, &installed);
        clear_transient(&root, operation)?;
        complete(&root, &mut journal)?;
        audit(paths, id, &root)?;
        Ok(UpdateReport {
            added: result
                .steps
                .iter()
                .filter(|step| step.op == Op::Write)
                .count(),
            replaced: result
                .steps
                .iter()
                .filter(|step| step.op == Op::Replace)
                .count(),
            removed: result
                .steps
                .iter()
                .filter(|step| step.op == Op::Remove)
                .count(),
            released: result.released.clone(),
            skipped_configuration: result.skipped.iter().map(|s| s.path.clone()).collect(),
            backed_up: result
                .steps
                .iter()
                .filter(|step| step.backup)
                .map(|step| step.path.clone())
                .collect(),
            backup: journal.backup.clone(),
            native_cleanup: Some(cleanup),
        })
    }
    .await;
    match published {
        Ok(report) => Ok(UpdateOutcome::Updated(report)),
        Err(error) => {
            // Undo or finish under the same lease; never clear a journal that cannot
            // be resolved. A pointer that did publish counts as success.
            if crashed() {
                return Err(error);
            }
            drop(game);
            if recover_base(paths, id, &root).is_err() {
                return Err(CoreError::new(ErrorCode::StorageCommitUncertain));
            }
            let current = read_json::<InstalledLock>(&root, "installed.json")?;
            if current.is_some_and(|current| {
                current
                    .pack
                    .as_ref()
                    .is_some_and(|pack| pin_of(pack) == request.to_version)
            }) {
                return Ok(UpdateOutcome::Updated(UpdateReport {
                    backup: journal.backup,
                    ..UpdateReport::default()
                }));
            }
            Err(error)
        }
    }
}

/// What a completed repair restored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairReport {
    pub restored: usize,
    /// Changed pack files moved to the backups before the pack's bytes returned.
    pub backed_up: Vec<String>,
}

/// Restore the installed exact pin: missing or changed content files, the game and
/// the loader. Configuration, history and the pin itself are never touched.
pub async fn repair(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    cancel: &Cancellation,
    progress: Progress,
    pack_progress: ProgressSink,
    auth: Option<PanitaCredentials>,
) -> Result<RepairReport, CoreError> {
    let client = CatalogClient::new()?.with_auth(auth);
    cancel.check()?;
    let catalog = tokio::select! {
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = client.detail(id) => result?,
    };
    repair_with_catalog(
        paths,
        id,
        platform,
        cancel,
        progress,
        pack_progress,
        catalog,
    )
    .await
}

pub(crate) async fn repair_with_catalog(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    cancel: &Cancellation,
    progress: Progress,
    pack_progress: ProgressSink,
    catalog: CatalogInstance,
) -> Result<RepairReport, CoreError> {
    let registry = InstanceRegistry::new(paths.clone());
    registry.remember_catalog(std::slice::from_ref(&catalog))?;
    catalog.installation_allowed(
        &semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?,
    )?;
    // A first look at the recorded pin; the runner repeats the check under the lock.
    let root = metadata_root(paths, id)?;
    let old = installed_pack(&root, id)?.pack.ok_or_else(invalid)?;
    let downloader = PackDownloader::new()?;
    // The recorded pin, exactly: never the catalog's current one.
    let plan = match &old.source {
        PackIdentity::Custom { version_id, .. } => {
            custom_plan_pinned(*version_id, None, id, cancel).await?
        }
        PackIdentity::Modrinth {
            project_id,
            version_id,
            archive_sha512,
            index_sha256,
        } => {
            let plan = mrpack_plan_pinned(
                paths,
                &downloader,
                project_id,
                version_id,
                None,
                cancel,
                pack_progress.clone(),
            )
            .await?;
            if plan.identity
                != (PackIdentity::Modrinth {
                    project_id: project_id.clone(),
                    version_id: version_id.clone(),
                    archive_sha512: archive_sha512.clone(),
                    index_sha256: index_sha256.clone(),
                })
            {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
            plan
        }
    };
    run_repair(
        paths,
        id,
        platform,
        plan,
        &downloader,
        BaseSource::Network,
        cancel,
        progress,
        pack_progress,
    )
    .await
}

/// Repair an installed pack against the plan of its own recorded pin.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_repair(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    plan: Plan,
    downloader: &PackDownloader,
    base_source: BaseSource,
    cancel: &Cancellation,
    progress: Progress,
    pack_progress: ProgressSink,
) -> Result<RepairReport, CoreError> {
    let registry = InstanceRegistry::new(paths.clone());
    let (root, _guard) = lock_and_recover(paths, id)?;
    let previous = installed_pack(&root, id)?;
    let old = previous.pack.clone().ok_or_else(invalid)?;
    let record = registry.get(id)?.ok_or_else(invalid)?;
    if !maintainable(&record) {
        return Err(invalid());
    }
    // The plan must be the one for exactly what is installed.
    if plan.identity != old.source {
        return Err(CoreError::new(ErrorCode::UpdateSelectionChanged));
    }
    if plan.minecraft_version != old.minecraft_version
        || plan.loader != old.loader
        || plan.loader_version != old.loader_version
    {
        return Err(CoreError::new(ErrorCode::ArtifactMismatch));
    }
    let game = GameDir::open(&root.join("minecraft"))?;
    // Content only: a manual file the player supplied is checked by hash like the rest.
    let mut incoming = Vec::new();
    let mut sources = Vec::new();
    for item in &plan.items {
        incoming.push(Incoming {
            path: item.path.clone(),
            ownership: Ownership::Content,
            desired: item.hashes.clone(),
            size: item.size,
            adopt_equal: false,
            source: sources.len(),
        });
        sources.push(Source::Item(Expected::file(&item.hashes, item.size)));
    }
    // Bundled content of an mrpack comes from its overrides, not from the index.
    let mut layers: Vec<(Kind, Opened<File>)> = Vec::new();
    for layer in &plan.layers {
        if let Layer::Overrides {
            expected,
            index,
            index_sha256,
        } = layer
        {
            let mut opened = mrpack::layer(paths, expected, index, index_sha256, cancel)?;
            let number = layers.len();
            let mut budget = opened.budget();
            for entry in opened.entries.clone() {
                if !mrpack::is_content(&entry.path) {
                    continue;
                }
                let digest = opened.stream(&entry, &mut budget, cancel, None)?;
                incoming.push(Incoming {
                    path: entry.path.clone(),
                    ownership: Ownership::Content,
                    desired: digest.hashes,
                    size: digest.size,
                    adopt_equal: false,
                    source: sources.len(),
                });
                sources.push(Source::Entry {
                    layer: number,
                    entry,
                });
            }
            layers.push((Kind::Override, opened));
        }
    }
    // Recorded content the plan cannot refetch was supplied by hand. It is checked by
    // hash like the rest, and its absence or change needs the player, not a download.
    for file in old
        .files
        .iter()
        .filter(|file| file.ownership == Ownership::Content)
    {
        if incoming
            .iter()
            .any(|new| new.path.eq_ignore_ascii_case(&file.path))
        {
            continue;
        }
        match game.fingerprint(&file.path, cancel)? {
            Local::File(found) if diff::same(&found, &file.desired) => {}
            _ => return Err(CoreError::new(ErrorCode::ManualFilesRequired)),
        }
    }
    let result = diff::repair_diff(&old, incoming, &mut |path| game.fingerprint(path, cancel))?;
    if result.unresolvable() {
        return Err(CoreError::new(ErrorCode::PackConflict));
    }
    let withdrawn = fetch_needed(paths, downloader, &plan, &result, cancel, pack_progress).await?;
    if !withdrawn.is_empty() {
        return Err(CoreError::new(ErrorCode::ManualFilesRequired));
    }

    let operation = Uuid::new_v4();
    let mut journal = Journal {
        schema_version: 5,
        kind: Operation::Repair,
        instance_id: id,
        operation,
        previous_operation: previous.operation,
        active: true,
        backup: None,
        ledger: Vec::new(),
        prepared: None,
    };
    write_json(&root, "transaction.json", &journal)?;
    let done: Result<RepairReport, CoreError> = async {
        // Game, Java and loader are verified and repaired from the recorded plan.
        let base = if base_source == BaseSource::Recorded {
            recorded_base(&previous)
        } else {
            prepare_recorded(
                paths,
                id,
                operation,
                platform,
                previous.plan.clone(),
                cancel,
                progress,
            )
            .await?
        };
        let installed = InstalledLock {
            pack: previous.pack.clone(),
            ..base
        };
        validate_manifest(&installed, id)?;
        journal.ledger = ledger_of(&result);
        journal.prepared = Some(installed.clone());
        journal.validate(id)?;
        write_json(&root, "transaction.json", &journal)?;
        apply(
            paths,
            &root,
            &game,
            &result,
            &sources,
            &mut layers,
            operation,
            cancel,
        )?;
        #[cfg(test)]
        interrupt(Stage::AfterApply)?;
        cancel.commit(|| write_json(&root, "installed.json", &installed))?;
        #[cfg(test)]
        interrupt(Stage::AfterPointer)?;
        let pack = installed.pack.as_ref().ok_or_else(invalid)?;
        record_state(paths, id, pack, false)?;
        clear_transient(&root, operation)?;
        complete(&root, &mut journal)?;
        audit(paths, id, &root)?;
        Ok(RepairReport {
            restored: result.steps.len(),
            backed_up: result
                .steps
                .iter()
                .filter(|step| step.backup)
                .map(|step| step.path.clone())
                .collect(),
        })
    }
    .await;
    match done {
        Ok(report) => Ok(report),
        Err(error) => {
            if crashed() {
                return Err(error);
            }
            drop(game);
            if recover_base(paths, id, &root).is_err() {
                return Err(CoreError::new(ErrorCode::StorageCommitUncertain));
            }
            let current = read_json::<InstalledLock>(&root, "installed.json")?;
            if current.is_some_and(|current| current.operation == operation) {
                return Ok(RepairReport::default());
            }
            Err(error)
        }
    }
}

/// What an uninstall removed and what it left alone.
#[derive(Clone, Default)]
pub struct UninstallReport {
    pub removed: usize,
    pub kept: usize,
    pub native_cleanup: Option<CleanupReport>,
}

/// Remove unedited pack content and mark the instance uninstalled. Configuration,
/// edited or added files, worlds and screenshots stay, as does the configuration
/// history, so a reinstall never overwrites what the player chose.
pub fn uninstall(
    paths: &AppPaths,
    id: Uuid,
    cancel: &Cancellation,
) -> Result<UninstallReport, CoreError> {
    cancel.check()?;
    let root = metadata_root(paths, id)?;
    let _guard = try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    // A committed uninstall whose cleanup was incomplete is retried, and reported.
    if let Some(value) = read_json::<Value>(&root, "transaction.json")?
        && value["schema_version"] == 5
    {
        let mut journal: Journal = serde_json::from_value(value).map_err(|_| invalid())?;
        journal.validate(id)?;
        if journal.active && journal.kind == Operation::Uninstall {
            return finish_uninstall(paths, id, &root, &mut journal);
        }
    }
    recover_base(paths, id, &root)?;
    let previous = installed_pack(&root, id)?;
    let mut journal = Journal {
        schema_version: 5,
        kind: Operation::Uninstall,
        instance_id: id,
        operation: Uuid::new_v4(),
        previous_operation: previous.operation,
        active: true,
        backup: None,
        ledger: Vec::new(),
        prepared: None,
    };
    write_json(&root, "transaction.json", &journal)?;
    finish_uninstall(paths, id, &root, &mut journal)
}

/// Roll an uninstall forward. Safe to repeat: each step observes the disk again.
fn finish_uninstall(
    paths: &AppPaths,
    id: Uuid,
    root: &Path,
    journal: &mut Journal,
) -> Result<UninstallReport, CoreError> {
    let installed = read_json::<InstalledLock>(root, "installed.json")?.ok_or_else(invalid)?;
    validate_manifest(&installed, id)?;
    if installed.operation != journal.previous_operation {
        return Err(invalid());
    }
    let cancel = Cancellation::default();
    let mut report = UninstallReport::default();
    if installed.schema_version == 2 {
        let pack = installed.pack.as_ref().ok_or_else(invalid)?;
        let game = GameDir::open(&root.join("minecraft"))?;
        let stash = Stash::open(root, TRANSIENT, journal.operation)?;
        for file in pack
            .files
            .iter()
            .filter(|file| file.ownership == Ownership::Content)
        {
            let unedited = match (&file.baseline, game.fingerprint(&file.path, &cancel)?) {
                (Some(baseline), Local::File(found)) => diff::same(&found, baseline),
                _ => false,
            };
            if unedited {
                game.stash(&file.path, &stash)?;
                report.removed += 1;
            } else {
                report.kept += 1;
            }
        }
        drop(stash);
        let mut kept = pack.clone();
        kept.files
            .retain(|file| file.ownership != Ownership::Content);
        kept.skipped_configuration
            .retain(|skip| kept.files.iter().any(|file| file.path == skip.path));
        kept.pending.clear();
        kept.validate(id)?;
        let mut tombstone = installed.clone();
        tombstone.schema_version = 3;
        tombstone.pack = Some(kept);
        write_json(root, "installed.json", &tombstone)?;
    }
    let registry = InstanceRegistry::new(paths.clone());
    if let Some(mut record) = registry.get(id)? {
        record.state = InstallationState::NotInstalled;
        registry.save_installation(&record)?;
    }
    clear_transient(root, journal.operation)?;
    let cleanup = native_cache::remove_all(root)?;
    if cleanup.retained_generations == 0 {
        complete(root, journal)?;
    }
    report.native_cleanup = Some(cleanup);
    Ok(report)
}

#[cfg(test)]
mod tests;
