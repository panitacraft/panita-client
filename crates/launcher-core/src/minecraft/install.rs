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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledLock {
    pub schema_version: u32,
    pub instance_id: Uuid,
    pub operation: Uuid,
    pub plan: InstallPlan,
    pub native_files: Vec<NativeFile>,
}

pub type Progress = Arc<dyn Fn(DownloadProgress) + Send + Sync>;

fn metadata_root(paths: &AppPaths, id: Uuid) -> Result<std::path::PathBuf, CoreError> {
    if id.is_nil() {
        return Err(invalid());
    }
    InstanceRegistry::new(paths.clone()).prepare_instance(id)
}

fn read_json<T: serde::de::DeserializeOwned>(
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
fn write_json(
    root: &std::path::Path,
    relative: &str,
    value: &impl Serialize,
) -> Result<(), CoreError> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
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
    let Some(mut journal) = read_json::<Journal>(root, "install-journal.json")? else {
        return Ok(());
    };
    if journal.schema_version != 1 || journal.operation.is_nil() {
        return Err(invalid());
    }
    if !journal.active {
        return Ok(());
    }
    let registry = InstanceRegistry::new(paths.clone());
    if let Some(mut record) = registry.get(id)? {
        record.state = InstallationState::NeedsRepair;
        registry.save_installation(&record)?;
    }
    journal.active = false;
    write_json(root, "install-journal.json", &journal)
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
        while workers.len() < 4 {
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
) -> Result<InstalledLock, CoreError> {
    let catalog_client = CatalogClient::new()?;
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
    if catalog.restricted
        || !catalog.compatible_with(
            &semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?,
        )
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
        previous.plan.validate()?;
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
    write_json(&root, "install-journal.json", &journal)?;
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
        let downloader = Downloader::new()?;
        let mut plan =
            super::plan::resolve(&downloader, minecraft_version, platform, cancel).await?;
        plan.allocate_runtime(paths)?;
        download_all(paths, &downloader, &plan, cancel, progress).await?;
        plan.verify_dependencies(paths)?;
        verify_java(paths, &plan, platform, cancel).await?;
        let native_root = root.join(format!("natives-{operation}"));
        if !plan.version.natives.is_empty() {
            private_fs::PrivateDir::ensure(&native_root).map_err(map_private_error)?;
        }
        let mut native_files = Vec::new();
        for native in &plan.version.natives {
            let relative = plan.library_path(&native.artifact)?;
            let (directory, leaf) = ensure_parent(paths.root(), &relative)?;
            if !verified_file(&directory, &leaf, &native.artifact)? {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
            let file = directory.open_file(leaf).map_err(map_private_error)?;
            native_files.extend(super::natives::extract(
                file,
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
        };
        // Publishing this pointer is the commit point; failed registry/journal writes
        // leave recovery pending, never a falsely complete installation.
        write_json(&root, "installed.json", &installed)?;
        record.state = InstallationState::Installed;
        registry.save_installation(&record)?;
        journal.active = false;
        write_json(&root, "install-journal.json", &journal)?;
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
            let _ = write_json(&root, "install-journal.json", &journal);
        }
    }
    result
}

/// Local preflight used by tests and the later Play integration; it never repairs.
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
    cancel.check()?;
    let journal = read_json::<Journal>(root, "install-journal.json")?;
    if journal.is_some_and(|journal| journal.schema_version != 1 || journal.active) {
        return Err(invalid());
    }
    let installed = read_json::<InstalledLock>(root, "installed.json")?.ok_or_else(invalid)?;
    if installed.schema_version != 2 || installed.instance_id != id || installed.operation.is_nil()
    {
        return Err(invalid());
    }
    installed.plan.validate()?;
    installed.plan.verify_dependencies(paths)?;
    for file in &installed.plan.artifacts {
        cancel.check()?;
        let (directory, leaf) = ensure_parent(paths.root(), &file.destination)?;
        if !verified_planned_file(&directory, &leaf, &file.artifact, file.executable)? {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        executable(paths, file)?;
    }
    let native_root = root.join(format!("natives-{}", installed.operation));
    for native in &installed.native_files {
        cancel.check()?;
        let artifact = Artifact {
            url: "https://libraries.minecraft.net/native".into(),
            path: None,
            sha1: native.sha1.clone(),
            size: native.size,
        };
        let (directory, leaf) = ensure_parent(&native_root, &native.path)?;
        if !verified_file(&directory, &leaf, &artifact)? {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            "install-journal.json",
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
            read_json::<Journal>(&root, "install-journal.json")
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
            .write_atomic_durable("install-journal.json", b"{broken")
            .unwrap_or_else(|_| panic!("corrupt"));
        assert!(recover(&paths, id, &Cancellation::default()).await.is_err());
        assert_eq!(
            std::fs::read(root.join("install-journal.json"))
                .ok()
                .as_deref(),
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
