//! First installation of a custom pack and its recovery. The base game and loader
//! are prepared first; pack files then flow through the verified cache into the game
//! directory under an active journal. `installed.json` is the single commit point:
//! before it exists a failure removes only what this operation created, and after
//! it exists recovery rolls forward. Existing player files are never overwritten.
use std::collections::BTreeSet;
use std::fs::File;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{
    Environment, Hashes, Manifest, MrpackIndex, Origin,
    archive::{self, Limits, Opened},
    cache::ManifestClient,
    config::{self, Kind},
    content::{self, Ensured, ProgressSink, Request},
    gamefs::{Content, GameDir},
    invalid,
    manual::{self, PendingFile, PendingReason, Placement},
    modrinth::{ModrinthClient, PackArtifact},
    mrpack,
    state::{ManagedFile, Ownership, PackIdentity, PackState},
    transfer::{Expected, PackDownloader},
};
use crate::{
    catalog::{CatalogClient, CatalogInstance, CatalogSource},
    error::{CoreError, ErrorCode},
    minecraft::{
        Platform,
        download::{Cancellation, io, lock, try_lock},
        install::{
            InstalledLock, Progress, metadata_root, read_json, recover_locked as recover_base,
            verify_manifest, verify_play_under_lock, write_json,
        },
        maintenance::validate_manifest,
        transaction,
    },
    panita_api::PanitaCredentials,
    paths::AppPaths,
    registry::{InstallationRecord, InstallationState, InstanceRegistry},
};

const KIND: &str = "pack_install";

mod maintain;
pub use maintain::{
    Blocked, RepairReport, UninstallReport, UpdateOutcome, UpdateReport, UpdateRequest, is_pack,
    pin_of, repair as repair_pack, uninstall as uninstall_pack, update as update_pack,
};

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
    if value["schema_version"] == 5 {
        return maintain::journal_inactive(value, id);
    }
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

fn created_content(item: &Item) -> Created {
    Created {
        path: item.path.clone(),
        size: item.size,
        sha1: item.hashes.sha1.clone(),
        sha512: item.hashes.sha512.clone(),
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
    if value["schema_version"] == 5 {
        return maintain::recover(paths, id, root, value);
    }
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
        // The full audit includes every managed pack file.
        verify_manifest(paths, id, root, &installed, &cancel)?;
        let pack = installed.pack.as_ref().ok_or_else(invalid)?;
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

/// One downloadable content file of a pack, whatever format described it.
#[derive(Clone, Debug)]
pub(crate) struct Item {
    path: String,
    hashes: Hashes,
    size: u64,
    url: String,
    origin: Origin,
    fallbacks: Vec<(String, Origin)>,
    /// What the player fetches by hand if the CDN reports the file withdrawn.
    withdrawn: Option<PendingFile>,
}

/// A source of configuration files applied once the content is in place.
#[derive(Clone, Debug)]
pub(crate) enum Layer {
    /// An archive the manifest lists; it is downloaded with the content.
    Archive {
        kind: Kind,
        url: String,
        expected: Expected,
    },
    /// The overrides of an mrpack that was already downloaded and verified.
    Overrides {
        expected: Expected,
        index: MrpackIndex,
        index_sha256: String,
    },
}

/// A pack reduced to what installation needs, independent of its format.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    identity: PackIdentity,
    minecraft_version: String,
    kind: crate::loader::Kind,
    loader: String,
    loader_version: String,
    items: Vec<Item>,
    pending: Vec<PendingFile>,
    layers: Vec<Layer>,
}

impl Plan {
    fn custom(manifest: &Manifest) -> Result<Self, CoreError> {
        let mut items = Vec::new();
        let mut pending = Vec::new();
        for file in &manifest.files {
            if file.env.client == Environment::Unsupported {
                continue;
            }
            if file.manual {
                pending.push(PendingFile::from_manifest(file, PendingReason::Manual)?);
                continue;
            }
            items.push(Item {
                path: file.path.clone(),
                hashes: file.hashes.clone(),
                size: file.size,
                url: file.url.clone().ok_or_else(invalid)?,
                origin: file.source.clone(),
                fallbacks: Vec::new(),
                withdrawn: (file.source == Origin::Curseforge)
                    .then(|| PendingFile::from_manifest(file, PendingReason::Withdrawn))
                    .transpose()?,
            });
        }
        let mut layers = Vec::new();
        for (kind, archive) in [
            (Kind::Defaults, &manifest.archives.defaults),
            (Kind::Enforced, &manifest.archives.enforced),
        ] {
            if let Some(archive) = archive {
                layers.push(Layer::Archive {
                    kind,
                    url: archive.url.clone(),
                    expected: Expected::archive(&archive.sha256, archive.size),
                });
            }
        }
        Ok(Self {
            identity: PackIdentity::Custom {
                version_id: manifest.version_id,
                published_at: manifest.published_at.clone(),
                label: manifest.label.clone(),
            },
            minecraft_version: manifest.minecraft_version.clone(),
            kind: crate::loader::Kind::NeoForge,
            loader: manifest.loader.clone(),
            loader_version: manifest.loader_version.clone(),
            items,
            pending,
            layers,
        })
    }

    /// The index is authoritative for versions: the catalog's informational loader
    /// and Minecraft version may only agree with it, never override it.
    /// `catalog` is absent when an installed pin is rebuilt for repair: that pin is
    /// historical, so only the index itself is authoritative.
    fn mrpack(
        catalog: Option<&CatalogInstance>,
        project: &str,
        version: &str,
        artifact: &PackArtifact,
        expected: Expected,
        inspected: mrpack::Inspected,
    ) -> Result<Self, CoreError> {
        let mrpack::Inspected {
            index,
            index_sha256,
            ..
        } = inspected;
        let (Some(minecraft), Some((kind, loader_version))) =
            (index.dependencies.get("minecraft"), index.loader())
        else {
            return Err(CoreError::new(ErrorCode::UnsupportedInstance));
        };
        if let Some(catalog) = catalog {
            if catalog
                .minecraft_version
                .as_ref()
                .is_some_and(|catalog| catalog != minecraft)
            {
                return Err(invalid());
            }
            // The catalog's loader is informational: it may agree with the index only.
            if let CatalogSource::Modrinth {
                loader: Some(named),
                ..
            } = &catalog.source
                && named != kind.name()
            {
                return Err(invalid());
            }
        }
        let mut items = Vec::new();
        for file in &index.files {
            if file.env.client == Environment::Unsupported {
                continue;
            }
            let mut sources = file.sources()?.into_iter();
            let (url, origin) = sources.next().ok_or_else(invalid)?;
            items.push(Item {
                path: file.path.clone(),
                hashes: file.hashes.clone(),
                size: file.file_size,
                url,
                origin,
                fallbacks: sources.collect(),
                withdrawn: None,
            });
        }
        Ok(Self {
            identity: PackIdentity::Modrinth {
                project_id: project.to_owned(),
                version_id: version.to_owned(),
                archive_sha512: artifact.hashes.sha512.clone().ok_or_else(invalid)?,
                index_sha256: index_sha256.clone(),
            },
            minecraft_version: minecraft.clone(),
            kind,
            loader: kind.name().to_owned(),
            loader_version: loader_version.to_owned(),
            items,
            pending: Vec::new(),
            layers: vec![Layer::Overrides {
                expected,
                index,
                index_sha256,
            }],
        })
    }
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

pub(crate) struct PlanPublish<'a> {
    pub paths: &'a AppPaths,
    pub id: Uuid,
    pub root: &'a Path,
    pub base: InstalledLock,
    pub plan: Plan,
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

/// Install a custom pack's files into the game directory and publish `installed.json`.
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
    publish_plan(PlanPublish {
        paths,
        id,
        root,
        base,
        plan: Plan::custom(manifest)?,
        downloader,
        cancel,
        progress,
    })
    .await
}

/// Install a pack's files into the game directory and publish `installed.json`.
/// The caller holds `.installation.lock` and has prepared `base`.
pub(crate) async fn publish_plan(request: PlanPublish<'_>) -> Result<Outcome, CoreError> {
    let PlanPublish {
        paths,
        id,
        root,
        base,
        plan,
        downloader,
        cancel,
        progress,
    } = request;
    let game = GameDir::open(&root.join("minecraft"))?;
    // An earlier installation that was uninstalled leaves its configuration history.
    let history: BTreeSet<String> = read_json::<InstalledLock>(root, "installed.json")?
        .and_then(|installed| installed.pack)
        .map(|pack| pack.consumed_defaults)
        .unwrap_or_default();
    let mut pending: Vec<PendingFile> = plan.pending.clone();
    // Existing player content is reported, never adopted or overwritten.
    let mut conflicts = BTreeSet::new();
    for item in &plan.items {
        cancel.check()?;
        if game.entry(&item.path)? != super::gamefs::Entry::Absent {
            conflicts.insert(item.path.clone());
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
    // Layers already in the cache open now, so their collisions are reported
    // before anything is downloaded.
    let mut layers: Vec<(Kind, Opened<File>)> = Vec::new();
    for layer in &plan.layers {
        if let Layer::Overrides {
            expected,
            index,
            index_sha256,
        } = layer
        {
            let opened = mrpack::layer(paths, expected, index, index_sha256, cancel)?;
            for entry in &opened.entries {
                if mrpack::is_content(&entry.path)
                    && game.entry(&entry.path)? != super::gamefs::Entry::Absent
                {
                    conflicts.insert(entry.path.clone());
                }
            }
            layers.push((Kind::Override, opened));
        }
    }
    if !conflicts.is_empty() {
        return Ok(Outcome::Conflicts(conflicts.into_iter().collect()));
    }

    let mut requests = Vec::new();
    for item in &plan.items {
        requests.push(Request {
            url: item.url.clone(),
            origin: item.origin.clone(),
            expected: Expected::file(&item.hashes, item.size),
            fallbacks: item.fallbacks.clone(),
        });
    }
    let mut archives: Vec<(Kind, Expected)> = Vec::new();
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
    let ensured = content::ensure_all(paths, downloader, &requests, cancel, progress).await?;

    let mut content_files: Vec<&Item> = Vec::new();
    for (item, result) in plan.items.iter().zip(&ensured) {
        match result {
            Ensured::Ready { .. } => content_files.push(item),
            Ensured::Withdrawn => {
                pending.push(item.withdrawn.clone().ok_or_else(invalid)?);
            }
        }
    }

    for (kind, expected) in &archives {
        let file = content::open_verified(paths, expected, cancel)?;
        layers.push((*kind, archive::preflight(file, Limits::PACK)?));
    }
    let named = |wanted: Kind| layers.iter().find(|(kind, _)| *kind == wanted);
    if let (Some((_, first)), Some((_, second))) = (named(Kind::Defaults), named(Kind::Enforced)) {
        config::ensure_disjoint(&first.entries, &second.entries)?;
    }

    let mut created: Vec<Created> = content_files.iter().map(|f| created_content(f)).collect();
    for (_, opened) in &layers {
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
        let mut consumed = history.clone();
        let mut skipped = Vec::new();
        for (kind, opened) in &mut layers {
            let mut budget = opened.budget();
            let outcome = config::apply(opened, *kind, &game, &history, &mut budget, cancel)?;
            files.extend(outcome.files);
            consumed.extend(outcome.consumed_defaults);
            skipped.extend(outcome.skipped);
        }
        let state = PackState {
            schema: 1,
            id,
            source: plan.identity.clone(),
            minecraft_version: plan.minecraft_version.clone(),
            loader: plan.loader.clone(),
            loader_version: plan.loader_version.clone(),
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

/// What a completed first installation produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    Vanilla,
    Pack(Outcome),
}

/// Install a catalog instance of any installable source: vanilla, or a pack.
pub async fn install_any(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    cancel: &Cancellation,
    progress: Progress,
    pack_progress: ProgressSink,
    auth: Option<PanitaCredentials>,
) -> Result<Installed, CoreError> {
    let client = CatalogClient::new()?.with_auth(auth.clone());
    let catalog = tokio::select! {
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = client.detail(id) => result?,
    };
    if matches!(
        catalog.source,
        CatalogSource::Custom { .. } | CatalogSource::Modrinth { .. }
    ) {
        return install(paths, id, platform, cancel, progress, pack_progress, auth)
            .await
            .map(Installed::Pack);
    }
    crate::minecraft::install::install(paths, id, platform, cancel, progress, auth).await?;
    Ok(Installed::Vanilla)
}

/// The custom pack's exact manifest for the current catalog pin.
async fn custom_plan(
    catalog: &CatalogInstance,
    id: Uuid,
    cancel: &Cancellation,
) -> Result<Plan, CoreError> {
    let CatalogSource::Custom {
        current_version_id: Some(version),
        ..
    } = &catalog.source
    else {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    };
    custom_plan_pinned(*version, Some(catalog), id, cancel).await
}

/// The manifest of one exact published version. Only the current pin is bound to
/// the catalog; an installed pin, used for repair, is historical.
async fn custom_plan_pinned(
    version: Uuid,
    bind: Option<&CatalogInstance>,
    id: Uuid,
    cancel: &Cancellation,
) -> Result<Plan, CoreError> {
    let manifests = ManifestClient::new()?;
    let cached = tokio::select! {
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = manifests.fetch(id, version, None) => result?,
    };
    let manifest = cached.manifest();
    if let Some(catalog) = bind {
        manifest.bind_current(catalog)?;
    }
    if !manifest.supported_loader() {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    Plan::custom(manifest)
}

/// The pinned Modrinth pack: resolve its exact version, verify the whole archive in
/// the shared cache, and only then read its index and overrides.
async fn mrpack_plan(
    paths: &AppPaths,
    downloader: &PackDownloader,
    catalog: &CatalogInstance,
    cancel: &Cancellation,
    progress: ProgressSink,
) -> Result<Plan, CoreError> {
    let CatalogSource::Modrinth {
        project_id,
        pack_version_id,
        loader,
    } = &catalog.source
    else {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    };
    if loader
        .as_deref()
        .is_some_and(|loader| !crate::loader::supported(loader))
    {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    mrpack_plan_pinned(
        paths,
        downloader,
        project_id,
        pack_version_id,
        Some(catalog),
        cancel,
        progress,
    )
    .await
}

/// One exact Modrinth version of a project, verified before its index is read.
async fn mrpack_plan_pinned(
    paths: &AppPaths,
    downloader: &PackDownloader,
    project_id: &str,
    pack_version_id: &str,
    catalog: Option<&CatalogInstance>,
    cancel: &Cancellation,
    progress: ProgressSink,
) -> Result<Plan, CoreError> {
    let modrinth = ModrinthClient::new()?;
    let artifact = tokio::select! {
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = modrinth.pack_artifact(project_id, pack_version_id) => result?,
    };
    let expected = Expected::file(&artifact.hashes, artifact.size);
    let request = Request {
        url: artifact.url.clone(),
        origin: Origin::Modrinth,
        expected: expected.clone(),
        fallbacks: Vec::new(),
    };
    let ensured = content::ensure_all(paths, downloader, &[request], cancel, progress).await?;
    if !matches!(ensured.as_slice(), [Ensured::Ready { .. }]) {
        return Err(invalid());
    }
    let inspected = mrpack::inspect(paths, &expected, cancel)?;
    Plan::mrpack(
        catalog,
        project_id,
        pack_version_id,
        &artifact,
        expected,
        inspected,
    )
}

/// Install the current exact catalog pin of a pack.
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
    install_catalog(paths, catalog, platform, cancel, progress, pack_progress).await
}

/// Install the pack a catalog entry that was already fetched pins. The entry must be
/// the current by-ID response; development harnesses may supply their own.
pub async fn install_catalog(
    paths: &AppPaths,
    catalog: CatalogInstance,
    platform: &Platform,
    cancel: &Cancellation,
    progress: Progress,
    pack_progress: ProgressSink,
) -> Result<Outcome, CoreError> {
    let id = catalog.id;
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
    let downloader = PackDownloader::new()?;
    let plan = match &catalog.source {
        CatalogSource::Custom { .. } => custom_plan(&catalog, id, cancel).await?,
        CatalogSource::Modrinth { .. } => {
            mrpack_plan(paths, &downloader, &catalog, cancel, pack_progress.clone()).await?
        }
        _ => return Err(CoreError::new(ErrorCode::UnsupportedInstance)),
    };
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
        let base = crate::loader::prepare_base(
            paths,
            id,
            operation,
            platform,
            plan.kind,
            &plan.minecraft_version,
            &plan.loader_version,
            cancel,
            progress,
        )
        .await?;
        publish_plan(PlanPublish {
            paths,
            id,
            root: &root,
            base,
            plan,
            downloader: &downloader,
            cancel,
            progress: pack_progress,
        })
        .await
    }
    .await;
    match &result {
        // `publish` stored the final state, or reported conflicts without installing.
        Ok(Outcome::Installed(_)) => {
            // Record Play receipts now, while the installation lock is held, so the first
            // launch does not hash every file again. Best effort: a failure only means
            // the first Play does that work, and any real damage surfaces there.
            let _ = verify_play_under_lock(paths, id, &root, &Cancellation::default());
        }
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
