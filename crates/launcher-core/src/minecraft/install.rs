//! Recoverable installation: verified shared artifacts and an immutable per-instance
//! generation are prepared first; the installed lock is published only after Java checks.
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use super::download::{
    Cancellation, DownloadProgress, Downloader, ensure_parent, io, lock, verified_file,
    verified_planned_file,
};
use super::natives::NativeFile;
use super::plan::{InstallPlan, PlannedArtifact};
use super::{Artifact, Platform, invalid};
use crate::{
    catalog::{CatalogClient, CatalogSource},
    error::{CoreError, ErrorCode},
    panita_api::PanitaCredentials,
    paths::AppPaths,
    registry::{InstallationRecord, InstallationState, InstanceRegistry},
    storage::map_private_error,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    operation: Uuid,
    active: bool,
}
mod format;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "format::StoredInstalled", into = "format::StoredInstalled")]
pub struct InstalledLock {
    /// Legacy model states: 2 is installed, 3 is uninstalled. Disk schema 4 stores
    /// this distinction as an explicit status, independent of its format version.
    pub schema_version: u32,
    pub instance_id: Uuid,
    pub operation: Uuid,
    pub plan: InstallPlan,
    pub native_files: Vec<NativeFile>,
    /// Missing in older manifests, which retain their original generation path.
    pub native_directory: Option<String>,
}

impl InstalledLock {
    pub fn native_directory(&self) -> Result<String, CoreError> {
        match &self.native_directory {
            None => Ok(format!("natives-{}", self.operation)),
            Some(path) if *path == format!("natives/{}", self.operation) => Ok(path.clone()),
            _ => Err(invalid()),
        }
    }
}

pub type Progress = Arc<dyn Fn(DownloadProgress) + Send + Sync>;

pub(super) fn metadata_root(paths: &AppPaths, id: Uuid) -> Result<std::path::PathBuf, CoreError> {
    if id.is_nil() {
        return Err(invalid());
    }
    InstanceRegistry::new(paths.clone()).prepare_instance(id)
}

pub(super) fn read_json<T: serde::de::DeserializeOwned>(
    root: &std::path::Path,
    name: &str,
) -> Result<Option<T>, CoreError> {
    if name == "transaction.json" {
        return super::transaction::read(root)?
            .map(|value| serde_json::from_value(value).map_err(|_| invalid()))
            .transpose();
    }
    read_json_raw(root, name)
}

pub(super) fn read_json_raw<T: serde::de::DeserializeOwned>(
    root: &std::path::Path,
    name: &str,
) -> Result<Option<T>, CoreError> {
    let directory = private_fs::PrivateDir::open(root).map_err(map_private_error)?;
    let mut file = match directory.open_file(name) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(map_private_error(error)),
    };
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(invalid());
    }
    Ok(Some(serde_json::from_slice(&bytes).map_err(|_| invalid())?))
}
pub(super) fn write_json(
    root: &std::path::Path,
    relative: &str,
    value: &impl Serialize,
) -> Result<(), CoreError> {
    if relative == "transaction.json" {
        return super::transaction::write(
            root,
            serde_json::to_value(value).map_err(|_| invalid())?,
        );
    }
    write_json_raw(root, relative, value)
}

pub(super) fn write_json_raw(
    root: &std::path::Path,
    relative: &str,
    value: &impl Serialize,
) -> Result<(), CoreError> {
    let bytes = crate::storage_json::pretty(value).map_err(|_| invalid())?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(invalid());
    }
    // Share the registry's mutation lock, including its stale-temporary cleanup.
    // Installation locks are larger than account documents, with a separate 32 MiB bound.
    let store = crate::storage::PrivateStore::open(&AppPaths::new(root)?)?;
    store.with_lock(|_| {
        let (directory, leaf) = ensure_parent(root, relative)?;
        directory
            .write_atomic_durable(leaf, &bytes)
            .map_err(map_private_error)
    })
}

/// A pending journal is conservative: files are retained for verified reuse, but
/// completion must be retried explicitly. No network access or Java execution occurs.
pub(super) fn recover_locked(
    paths: &AppPaths,
    id: Uuid,
    root: &std::path::Path,
) -> Result<(), CoreError> {
    if super::maintenance::recover_locked(paths, id, root)? {
        return Ok(());
    }
    if super::update::recover_locked(paths, id, root)? {
        return Ok(());
    }
    let Some(mut journal) = read_json::<Journal>(root, "transaction.json")? else {
        return Ok(());
    };
    if journal.schema_version != 1 || journal.operation.is_nil() {
        return Err(invalid());
    }
    if !journal.active {
        return super::transaction::remove(root);
    }
    let registry = InstanceRegistry::new(paths.clone());
    let mut recovered_state = InstallationState::NeedsRepair;
    if let Some(installed) = read_json::<InstalledLock>(root, "installed.json")? {
        super::maintenance::validate_manifest(&installed, id)?;
        if installed.operation == journal.operation && installed.schema_version == 2 {
            // A durable pointer is recoverable only after a complete local audit,
            // including the exact loader generation. Failure retains the intent.
            verify_manifest(paths, id, root, &installed, &Cancellation::default())?;
            recovered_state = InstallationState::Installed;
        }
    }
    if let Some(mut record) = registry.get(id)? {
        record.state = recovered_state;
        registry.save_installation(&record)?;
    }
    journal.active = false;
    write_json(root, "transaction.json", &journal)?;
    super::transaction::remove(root)
}

pub async fn recover(paths: &AppPaths, id: Uuid, cancel: &Cancellation) -> Result<(), CoreError> {
    cancel.check()?;
    let root = metadata_root(paths, id)?;
    let _guard = super::download::try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    recover_locked(paths, id, &root)
}

fn executable(paths: &AppPaths, file: &PlannedArtifact) -> Result<(), CoreError> {
    if !file.executable {
        return Ok(());
    }
    let (directory, leaf) = ensure_parent(paths.root(), &file.destination)?;
    #[cfg(target_os = "linux")]
    return super::executable::require(&directory, &leaf);
    #[cfg(not(target_os = "linux"))]
    {
        let file = directory.open_file(leaf).map_err(map_private_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            super::download::native_file(&file)?
                .set_permissions(std::fs::Permissions::from_mode(0o700))
                .map_err(io)?;
        }
        file.verify().map_err(map_private_error)
    }
}

async fn verify_java(
    paths: &AppPaths,
    plan: &InstallPlan,
    platform: &Platform,
    cancel: &Cancellation,
) -> Result<(), CoreError> {
    let mut command = tokio::process::Command::new(paths.root().join(&plan.java_executable));
    command
        .args(["-XshowSettings:properties", "-version"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Do not inherit JVM injection options from the development environment.
    command
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("_JAVA_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS");
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|_| CoreError::new(ErrorCode::RuntimeUnsupported))?;
    let stderr = child.stderr.take().ok_or_else(invalid)?;
    let probe = async {
        let mut output = Vec::new();
        stderr
            .take(16 * 1024 + 1)
            .read_to_end(&mut output)
            .await
            .map_err(io)?;
        if output.len() > 16 * 1024 {
            return Err(invalid());
        }
        let status = child.wait().await.map_err(io)?;
        if !status.success() {
            return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
        }
        super::runtime::verify_properties(
            std::str::from_utf8(&output).map_err(|_| invalid())?,
            &plan.version.java,
            platform,
        )
    };
    tokio::select! {
        _ = cancel.cancelled() => Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = tokio::time::timeout(Duration::from_secs(20), probe) => result.map_err(|_| CoreError::new(ErrorCode::RuntimeUnsupported))?,
    }
}

async fn download_all(
    paths: &AppPaths,
    downloader: &Downloader,
    plan: &InstallPlan,
    cancel: &Cancellation,
    progress: Progress,
) -> Result<(), CoreError> {
    let mut missing = 0u64;
    for file in &plan.artifacts {
        cancel.check()?;
        let (directory, leaf) = ensure_parent(paths.root(), &file.destination)?;
        if !verified_planned_file(&directory, &leaf, &file.artifact, file.executable)? {
            missing = missing
                .checked_add(file.artifact.size + 4096)
                .ok_or_else(invalid)?;
        }
    }
    check_capacity(missing, fs2::available_space(paths.root()).map_err(io)?)?;
    let mut workers = tokio::task::JoinSet::new();
    let mut queue = plan.artifacts.iter();
    loop {
        while workers.len() < 8 {
            let Some(file) = queue.next().cloned() else {
                break;
            };
            let paths = paths.clone();
            let downloader = downloader.clone();
            let cancel = cancel.clone();
            let progress = progress.clone();
            workers.spawn(async move {
                downloader
                    .cache_planned(&paths, &file, &cancel, progress.as_ref())
                    .await?;
                executable(&paths, &file)
            });
        }
        let Some(result) = workers.join_next().await else {
            return Ok(());
        };
        let result = result
            .map_err(|_| CoreError::new(ErrorCode::StorageUnavailable))
            .and_then(|result| result);
        if let Err(error) = result {
            workers.abort_all();
            while workers.join_next().await.is_some() {}
            return Err(error);
        }
    }
}

fn check_capacity(missing: u64, available: u64) -> Result<(), CoreError> {
    if missing
        .checked_add(64 * 1024 * 1024)
        .is_none_or(|required| required > available)
    {
        Err(CoreError::new(ErrorCode::InsufficientSpace))
    } else {
        Ok(())
    }
}

/// Install the current exact catalog pin. Changing an already installed pin is a
/// separate, explicit maintenance action and is deliberately rejected here.
pub async fn install(
    paths: &AppPaths,
    id: Uuid,
    platform: &Platform,
    cancel: &Cancellation,
    progress: Progress,
    auth: Option<PanitaCredentials>,
) -> Result<InstalledLock, CoreError> {
    let catalog_client = CatalogClient::new()?.with_auth(auth);
    let catalog = tokio::select! {
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = catalog_client.detail(id) => result?,
    };
    let registry = InstanceRegistry::new(paths.clone());
    // Retain explicit availability learned here, including a disabled result.
    registry.remember_catalog(std::slice::from_ref(&catalog))?;
    if !catalog.enabled {
        return Err(CoreError::new(ErrorCode::InstanceDisabled));
    }
    if !catalog
        .compatible_with(&semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?)
    {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    let CatalogSource::Vanilla { minecraft_version } = &catalog.source else {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    };
    cancel.check()?;
    let root = registry.ensure_catalog_instance(&catalog)?;
    let _guard = lock(&root, ".installation.lock", cancel).await?;
    recover_locked(paths, id, &root)?;
    let previous = registry.get(id)?;
    if previous
        .as_ref()
        .is_some_and(|record| record.resolved_source != catalog.source)
    {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    if let Some(previous) = read_json::<InstalledLock>(&root, "installed.json")? {
        super::maintenance::validate_manifest(&previous, id)?;
        previous.plan.validate()?;
        if previous.schema_version == 3 && !previous_record_requires_installation(paths, id)? {
            return Err(invalid());
        }
        if previous.instance_id != id || previous.plan.version.id != *minecraft_version {
            return Err(invalid());
        }
    }
    let operation = Uuid::new_v4();
    let mut journal = Journal {
        schema_version: 1,
        operation,
        active: true,
    };
    write_json(&root, "transaction.json", &journal)?;
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
    let result: Result<InstalledLock, CoreError> = async {
        let installed = prepare(
            paths,
            id,
            operation,
            platform,
            minecraft_version,
            cancel,
            progress,
        )
        .await?;
        // Publishing this pointer is the commit point; failed registry/journal writes
        // leave recovery pending, never a falsely complete installation.
        write_json(&root, "installed.json", &installed)?;
        record.state = InstallationState::Installed;
        registry.save_installation(&record)?;
        journal.active = false;
        write_json(&root, "transaction.json", &journal)?;
        super::transaction::remove(&root)?;
        Ok(installed)
    }
    .await;
    if let Err(error) = &result {
        record.state = if error.code() == ErrorCode::OperationCancelled {
            InstallationState::Cancelled
        } else {
            InstallationState::Failed
        };
        // Leave the journal active if cleanup cannot be recorded. Recovery will mark
        // NeedsRepair, preserving the original error and every previous artifact.
        if registry.save_installation(&record).is_ok() {
            journal.active = false;
            if write_json(&root, "transaction.json", &journal).is_ok() {
                let _ = super::transaction::remove(&root);
            }
        }
    }
    result
}

/// Prepare a fresh immutable generation without publishing an installed pointer.
/// The caller retains the instance operation lock through preparation and commit.
pub(super) async fn prepare(
    paths: &AppPaths,
    id: Uuid,
    operation: Uuid,
    platform: &Platform,
    version: &str,
    cancel: &Cancellation,
    progress: Progress,
) -> Result<InstalledLock, CoreError> {
    let downloader = Downloader::new()?;
    let mut plan = super::plan::resolve(&downloader, version, platform, cancel).await?;
    plan.allocate_runtime(paths)?;
    prepare_recorded(paths, id, operation, platform, plan, cancel, progress).await
}

fn previous_record_requires_installation(paths: &AppPaths, id: Uuid) -> Result<bool, CoreError> {
    Ok(InstanceRegistry::new(paths.clone())
        .get(id)?
        .is_some_and(|record| {
            !matches!(
                record.state,
                InstallationState::Installed
                    | InstallationState::UpdateAvailable
                    | InstallationState::Running
            )
        }))
}

pub(super) async fn prepare_recorded(
    paths: &AppPaths,
    id: Uuid,
    operation: Uuid,
    platform: &Platform,
    mut plan: InstallPlan,
    cancel: &Cancellation,
    progress: Progress,
) -> Result<InstalledLock, CoreError> {
    let root = metadata_root(paths, id)?;
    plan.validate()?;
    if plan.platform != platform.os || plan.architecture != platform.arch {
        return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
    }
    let downloader = Downloader::new()?;
    download_all(paths, &downloader, &plan, cancel, progress).await?;
    plan.verify_dependencies(paths)?;
    verify_java(paths, &plan, platform, cancel).await?;
    if let Some(loader) = &plan.loader {
        match loader.verify(paths, cancel) {
            Ok(()) => {}
            Err(error) if error.code() == ErrorCode::ArtifactMismatch => {
                plan.loader = Some(loader.repair(paths, &plan, cancel).await?);
            }
            Err(error) => return Err(error),
        }
    }
    let native_directory = format!("natives/{operation}");
    let native_root = root.join(&native_directory);
    if !plan.version.natives.is_empty() {
        let parent = private_fs::PrivateDir::open(&root).map_err(map_private_error)?;
        parent
            .ensure_dir("natives")
            .map_err(map_private_error)?
            .ensure_dir(operation.to_string())
            .map_err(map_private_error)?;
    }
    let mut native_files = Vec::new();
    for native in &plan.version.natives {
        cancel.check()?;
        let relative = plan.library_path(&native.artifact)?;
        let (directory, leaf) = ensure_parent(paths.root(), &relative)?;
        if !verified_file(&directory, &leaf, &native.artifact)? {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        native_files.extend(super::natives::extract(
            directory.open_file(leaf).map_err(map_private_error)?,
            &native_root,
            &native.excludes,
            cancel,
        )?);
    }
    cancel.check()?;
    let installed = InstalledLock {
        schema_version: 2,
        instance_id: id,
        operation,
        plan,
        native_files,
        native_directory: Some(native_directory),
    };
    Ok(installed)
}

/// Explicit full local audit. Never consults Play receipts and never repairs.
pub fn verify_installed(paths: &AppPaths, id: Uuid) -> Result<InstalledLock, CoreError> {
    let root = metadata_root(paths, id)?;
    let _guard = super::download::try_lock(&root, ".installation.lock")?
        .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
    verify_under_lock(paths, id, &root, &Cancellation::default())
}

/// Caller must retain this instance's installation lock for the whole audit.
pub(super) fn verify_under_lock(
    paths: &AppPaths,
    id: Uuid,
    root: &std::path::Path,
    cancel: &Cancellation,
) -> Result<InstalledLock, CoreError> {
    let installed = read_installed_under_lock(id, root, cancel)?;
    verify_manifest(paths, id, root, &installed, cancel)?;
    Ok(installed)
}

/// Cached Play preflight. Installation, repair and recovery use the full audit above.
pub(super) fn verify_play_under_lock(
    paths: &AppPaths,
    id: Uuid,
    root: &std::path::Path,
    cancel: &Cancellation,
) -> Result<InstalledLock, CoreError> {
    let installed = read_installed_under_lock(id, root, cancel)?;
    let mut cache = super::verification_cache::VerificationCache::load(root, &installed)?;
    verify_manifest_inner(paths, id, root, &installed, cancel, Some(&mut cache))?;
    cancel.check()?;
    cache.save(root);
    Ok(installed)
}

fn read_installed_under_lock(
    id: Uuid,
    root: &std::path::Path,
    cancel: &Cancellation,
) -> Result<InstalledLock, CoreError> {
    cancel.check()?;
    let journal = read_json::<serde_json::Value>(root, "transaction.json")?;
    if journal.is_some_and(|journal| {
        if journal["schema_version"] == 3 {
            !super::maintenance::journal_inactive(journal, id)
        } else if journal["schema_version"] == 2 {
            !super::update::journal_inactive(journal, id)
        } else {
            serde_json::from_value::<Journal>(journal).map_or(true, |journal| {
                journal.schema_version != 1 || journal.active || journal.operation.is_nil()
            })
        }
    }) {
        return Err(invalid());
    }
    let value = read_json::<serde_json::Value>(root, "installed.json")?.ok_or_else(invalid)?;
    let installed: InstalledLock = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    super::maintenance::validate_manifest(&installed, id)?;
    if value.get("schema_version").is_some() {
        write_json(root, "installed.json", &installed)?;
    }
    Ok(installed)
}

pub(super) fn verify_manifest(
    paths: &AppPaths,
    id: Uuid,
    root: &std::path::Path,
    installed: &InstalledLock,
    cancel: &Cancellation,
) -> Result<(), CoreError> {
    verify_manifest_inner(paths, id, root, installed, cancel, None)
}

fn verify_manifest_inner(
    paths: &AppPaths,
    id: Uuid,
    root: &std::path::Path,
    installed: &InstalledLock,
    cancel: &Cancellation,
    mut cache: Option<&mut super::verification_cache::VerificationCache>,
) -> Result<(), CoreError> {
    if installed.schema_version != 2 || installed.instance_id != id || installed.operation.is_nil()
    {
        return Err(invalid());
    }
    installed.plan.validate()?;
    installed.plan.verify_dependencies(paths)?;
    // Retain verified parent handles only within this audit. Play receipts skip
    // unchanged content reads, while every file still crosses the private boundary.
    let mut parents = std::collections::HashMap::new();
    for file in &installed.plan.artifacts {
        cancel.check()?;
        let (parent, leaf) = file.destination.rsplit_once('/').ok_or_else(invalid)?;
        if !parents.contains_key(parent) {
            let (directory, _) = super::download::existing_parent(paths.root(), &file.destination)?
                .ok_or_else(|| CoreError::new(ErrorCode::ArtifactMismatch))?;
            parents.insert(parent, directory);
        }
        let directory = parents.get(parent).ok_or_else(invalid)?;
        let matches = match cache.as_deref_mut() {
            Some(cache) => cache.verify(
                directory,
                leaf,
                &format!("managed/{}", file.destination),
                &file.artifact,
                file.executable,
            )?,
            None => verified_planned_file(directory, leaf, &file.artifact, file.executable)?,
        };
        if !matches {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        executable(paths, file)?;
    }
    let native_root = root.join(installed.native_directory()?);
    for native in &installed.native_files {
        cancel.check()?;
        let artifact = Artifact {
            url: "https://libraries.minecraft.net/native".into(),
            path: None,
            sha1: native.sha1.clone(),
            size: native.size,
        };
        let (directory, leaf) = ensure_parent(&native_root, &native.path)?;
        let matches = match cache.as_deref_mut() {
            Some(cache) => cache.verify(
                &directory,
                &leaf,
                &format!("native/{}/{}", installed.native_directory()?, native.path),
                &artifact,
                false,
            )?,
            None => verified_file(&directory, &leaf, &artifact)?,
        };
        if !matches {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
    }
    if let Some(loader) = &installed.plan.loader {
        loader.verify_marker(paths)?;
        match cache {
            None => loader.verify(paths, cancel)?,
            Some(cache) => {
                let directory = private_fs::PrivateDir::open(paths.root().join(loader.directory()))
                    .map_err(map_private_error)?;
                for file in &loader.files {
                    cancel.check()?;
                    let (parent, leaf) = file
                        .path
                        .rsplit_once('/')
                        .map_or(("", file.path.as_str()), |(parent, leaf)| (parent, leaf));
                    let key = format!("loader/{}/{}", loader.directory(), file.path);
                    let matches = if parent.is_empty() {
                        cache.verify_sha256(
                            &directory,
                            leaf,
                            &key,
                            &file.artifact(),
                            &file.sha256,
                        )?
                    } else {
                        let parent = directory.open_dir(parent).map_err(map_private_error)?;
                        cache.verify_sha256(&parent, leaf, &key, &file.artifact(), &file.sha256)?
                    };
                    if !matches {
                        return Err(CoreError::new(ErrorCode::ArtifactMismatch));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "install/cache_tests.rs"]
mod cache_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::Digest;
    #[test]
    fn native_generation_paths_preserve_legacy_manifests_and_reject_arbitrary_locations() {
        let (paths, accounts, id) = super::super::play::tests::fixture();
        drop(accounts);
        super::super::play::tests::complete_installation(&paths, id);
        let root = metadata_root(&paths, id).unwrap_or_else(|_| panic!("root"));
        let mut installed = read_json::<InstalledLock>(&root, "installed.json")
            .unwrap_or_else(|_| panic!("read"))
            .unwrap_or_else(|| panic!("manifest"));
        assert_eq!(
            installed.native_directory().ok(),
            Some(format!("natives-{}", installed.operation))
        );
        installed.native_directory = Some(format!("natives/{}", installed.operation));
        let generation = private_fs::PrivateDir::open(&root)
            .unwrap_or_else(|_| panic!("instance"))
            .ensure_dir("natives")
            .unwrap_or_else(|_| panic!("natives"))
            .ensure_dir(installed.operation.to_string())
            .unwrap_or_else(|_| panic!("generation"));
        generation
            .write_atomic_durable("owned.dll", b"native")
            .unwrap_or_else(|_| panic!("native"));
        installed.native_files.push(NativeFile {
            path: "owned.dll".into(),
            size: 6,
            sha1: format!("{:x}", sha1::Sha1::digest(b"native")),
        });
        assert!(super::super::maintenance::validate_manifest(&installed, id).is_ok());
        assert!(verify_manifest(&paths, id, &root, &installed, &Cancellation::default()).is_ok());
        for path in [
            "../other".to_owned(),
            format!("natives/{}", Uuid::new_v4()),
            "minecraft".into(),
        ] {
            installed.native_directory = Some(path);
            assert!(super::super::maintenance::validate_manifest(&installed, id).is_err());
        }
        drop(generation);
        std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
    }
    #[test]
    fn uninstalled_descriptor_can_retry_after_reinstallation_failure_or_interruption() {
        let (paths, accounts, id) = super::super::play::tests::fixture();
        drop(accounts);
        super::super::play::tests::complete_installation(&paths, id);
        super::super::maintenance::uninstall(&paths, id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("uninstall"));
        let registry = InstanceRegistry::new(paths.clone());
        for state in [
            InstallationState::NotInstalled,
            InstallationState::Failed,
            InstallationState::Cancelled,
            InstallationState::NeedsRepair,
            InstallationState::Installing,
        ] {
            let mut record = registry
                .get(id)
                .unwrap_or_else(|_| panic!("registry"))
                .unwrap_or_else(|| panic!("record"));
            record.state = state;
            registry
                .save_installation(&record)
                .unwrap_or_else(|_| panic!("save"));
            assert!(previous_record_requires_installation(&paths, id).unwrap_or(false));
        }
        std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
    }
    #[test]
    fn capacity_reserves_atomic_write_space_and_disk_full_is_actionable() {
        assert!(check_capacity(1024, 64 * 1024 * 1024).is_err());
        assert!(check_capacity(1024, 65 * 1024 * 1024).is_ok());
        assert!(check_capacity(u64::MAX, u64::MAX).is_err());
        assert_eq!(
            io(std::io::Error::from(std::io::ErrorKind::StorageFull)).code(),
            ErrorCode::InsufficientSpace
        );
    }
    fn record(id: Uuid) -> InstallationRecord {
        let catalog = crate::catalog::CatalogInstance {
            id,
            slug: "test".into(),
            name: "Prueba".into(),
            description: "Prueba".into(),
            minecraft_version: Some("1.21.1".into()),
            enabled: true,
            restricted: false,
            min_launcher_version: None,
            icon_url: None,
            banner_url: None,
            sort_order: 1,
            updated_at: "2026-10-16T00:00:00Z".into(),
            source: CatalogSource::Vanilla {
                minecraft_version: "1.21.1".into(),
            },
        };
        InstallationRecord {
            schema_version: 1,
            id,
            directory_name: None,
            resolved_source: catalog.source.clone(),
            last_catalog: catalog,
            state: InstallationState::Installing,
            managed_files: Vec::new(),
            settings: crate::settings::InstanceSettings::default(),
        }
    }
    #[tokio::test]
    async fn interrupted_install_recovers_without_claiming_completion_or_modifying_worlds() {
        let paths =
            AppPaths::new(std::env::temp_dir().join(format!("panita-install-{}", Uuid::new_v4())))
                .unwrap_or_else(|_| panic!("paths"));
        let id = Uuid::new_v4();
        let registry = InstanceRegistry::new(paths.clone());
        registry
            .save(&record(id))
            .unwrap_or_else(|_| panic!("record"));
        let root = metadata_root(&paths, id).unwrap_or_else(|_| panic!("root"));
        let (directory, leaf) = ensure_parent(
            &paths.game_directory(id).unwrap_or_else(|_| panic!("path")),
            "saves/Test World/level.dat",
        )
        .unwrap_or_else(|_| panic!("world"));
        directory
            .write_atomic_durable(leaf, b"world data")
            .unwrap_or_else(|_| panic!("world data"));
        write_json(
            &root,
            "transaction.json",
            &Journal {
                schema_version: 1,
                operation: Uuid::new_v4(),
                active: true,
            },
        )
        .unwrap_or_else(|_| panic!("journal"));
        let guard = super::super::download::try_lock(&root, ".installation.lock")
            .unwrap_or_else(|_| panic!("lock"))
            .unwrap_or_else(|| panic!("available"));
        assert_eq!(
            recover(&paths, id, &Cancellation::default())
                .await
                .err()
                .map(|error| error.code()),
            Some(ErrorCode::OperationBusy)
        );
        assert!(
            read_json::<Journal>(&root, "transaction.json")
                .unwrap_or_default()
                .is_some_and(|journal| journal.active)
        );
        drop(guard);
        recover(&paths, id, &Cancellation::default())
            .await
            .unwrap_or_else(|_| panic!("recover"));
        let records = registry.load().unwrap_or_else(|_| panic!("load"));
        assert_eq!(records[0].state, InstallationState::NeedsRepair);
        assert!(verify_installed(&paths, id).is_err());
        assert_eq!(
            std::fs::read(
                paths
                    .game_directory(id)
                    .unwrap_or_else(|_| panic!("path"))
                    .join("saves/Test World/level.dat")
            )
            .ok()
            .as_deref(),
            Some(b"world data".as_slice())
        );
        recover(&paths, id, &Cancellation::default())
            .await
            .unwrap_or_else(|_| panic!("idempotent recovery"));
        assert_eq!(
            registry.load().unwrap_or_else(|_| panic!("load"))[0].state,
            InstallationState::NeedsRepair
        );
    }
    #[tokio::test]
    async fn malformed_journal_is_preserved_and_failed_writes_preserve_previous_lock() {
        let paths =
            AppPaths::new(std::env::temp_dir().join(format!("panita-install-{}", Uuid::new_v4())))
                .unwrap_or_else(|_| panic!("paths"));
        let id = Uuid::new_v4();
        InstanceRegistry::new(paths.clone())
            .save(&record(id))
            .unwrap_or_else(|_| panic!("record"));
        let root = metadata_root(&paths, id).unwrap_or_else(|_| panic!("root"));
        let directory = private_fs::PrivateDir::open(&root).unwrap_or_else(|_| panic!("directory"));
        directory
            .write_atomic_durable("transaction.json", b"{broken")
            .unwrap_or_else(|_| panic!("corrupt"));
        assert!(recover(&paths, id, &Cancellation::default()).await.is_err());
        assert_eq!(
            std::fs::read(root.join("transaction.json")).ok().as_deref(),
            Some(b"{broken".as_slice())
        );
        write_json(&root, "lock.json", &serde_json::json!({"previous":"valid"}))
            .unwrap_or_else(|_| panic!("previous"));
        let previous = std::fs::read(root.join("lock.json")).unwrap_or_else(|_| panic!("read"));
        // An existing directory at the intended file is a deterministic storage failure.
        directory
            .ensure_dir("blocked.json")
            .unwrap_or_else(|_| panic!("directory"));
        assert!(write_json(&root, "blocked.json", &serde_json::json!({"next":true})).is_err());
        assert_eq!(
            std::fs::read(root.join("lock.json")).ok().as_deref(),
            Some(previous.as_slice())
        );
    }
}
