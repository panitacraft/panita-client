//! UUID-owned metadata; remote catalog changes never rename or remove game directories.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::catalog::{CatalogInstance, CatalogSnapshot, CatalogSource};
use crate::error::{CoreError, ErrorCode};
use crate::paths::AppPaths;
use crate::settings::InstanceSettings;
use crate::storage::PrivateStore;
use crate::storage::map_private_error;

const RECORD: &str = "instance.json";
const LEGACY_RECORD: &str = "installation.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationState {
    NotInstalled,
    Installing,
    Installed,
    UpdateAvailable,
    NeedsRepair,
    Running,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationRecord {
    pub schema_version: u32,
    pub id: Uuid,
    /// Stable physical directory, independent of later catalog slug changes.
    /// In a legacy UUID folder this is also the durable migration destination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory_name: Option<String>,
    pub last_catalog: CatalogInstance,
    pub resolved_source: CatalogSource,
    pub state: InstallationState,
    pub managed_files: Vec<String>,
    #[serde(default)]
    pub settings: InstanceSettings,
}

impl InstallationRecord {
    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        self.settings.validate()?;
        if !matches!(self.schema_version, 1..=3)
            || (self.schema_version == 1 && self.directory_name.is_some())
            || (self.schema_version >= 2 && self.directory_name.is_none())
            || self
                .directory_name
                .as_deref()
                .is_some_and(|name| !valid_directory_name(name))
            || self.id != self.last_catalog.id
            || self.managed_files.len() > 10_000
        {
            return Err(invalid());
        }
        self.last_catalog.validate()?;
        let mut resolved = self.last_catalog.clone();
        resolved.source = self.resolved_source.clone();
        if let CatalogSource::Vanilla { minecraft_version } = &resolved.source {
            resolved.minecraft_version = Some(minecraft_version.clone());
        }
        resolved.validate()?;
        let mut paths = std::collections::HashSet::new();
        for file in &self.managed_files {
            if file.is_empty()
                || file.len() > 2048
                || file.contains(['\\', ':'])
                || file.chars().any(|ch| ch.is_control())
                || file.starts_with('/')
                || file.split('/').any(|part| {
                    part.is_empty()
                        || part == "."
                        || part == ".."
                        || part.eq_ignore_ascii_case(".panita")
                })
                || !paths.insert(file.to_ascii_lowercase())
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn decode(bytes: &[u8], id: Uuid) -> Result<Self, CoreError> {
        let record: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        record.validate()?;
        if record.id != id {
            return Err(invalid());
        }
        Ok(record)
    }
}

fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidData)
}

pub(crate) fn valid_directory_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 85
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'-')
        && !matches!(name, "con" | "prn" | "aux" | "nul")
        && !(name.len() == 4
            && (name.starts_with("com") || name.starts_with("lpt"))
            && matches!(name.as_bytes()[3], b'1'..=b'9'))
}

fn formatted_json(value: &impl Serialize) -> Result<Vec<u8>, CoreError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|_| invalid())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn io(error: std::io::Error) -> CoreError {
    CoreError::new(if error.kind() == std::io::ErrorKind::PermissionDenied {
        ErrorCode::PermissionDenied
    } else {
        ErrorCode::StorageUnavailable
    })
}

fn ensure_not_symlink(path: &Path) -> Result<(), CoreError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreError::new(ErrorCode::UnsafePermissions));
    }
    Ok(())
}

pub struct InstanceRegistry {
    paths: AppPaths,
}

impl InstanceRegistry {
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    fn record_name(directory: &Path) -> &'static str {
        if directory.join(".panita/installation.json").exists() {
            LEGACY_RECORD
        } else {
            RECORD
        }
    }

    fn store_at(directory: &Path) -> Result<PrivateStore, CoreError> {
        private_fs::PrivateDir::open(directory).map_err(map_private_error)?;
        let root = if Self::record_name(directory) == LEGACY_RECORD {
            directory.join(".panita")
        } else {
            directory.to_path_buf()
        };
        PrivateStore::open(&AppPaths::new(root)?)
    }

    /// Caller holds the root mutation lock. Only metadata bearing UUID ownership
    /// is discoverable; arbitrary user folders are never adopted by their slug.
    fn scan(&self) -> Result<Vec<(PathBuf, InstallationRecord)>, CoreError> {
        let root = self.paths.root().join("instances");
        match std::fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io(error)),
            Ok(_) => ensure_not_symlink(&root)?,
        }
        private_fs::PrivateDir::open(&root).map_err(map_private_error)?;
        let mut records = Vec::new();
        let mut ids = std::collections::HashSet::new();
        for entry in std::fs::read_dir(root).map_err(io)? {
            let entry = entry.map_err(io)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_directory_name(&name) {
                continue;
            }
            let directory = entry.path();
            if !directory.join(".panita").try_exists().map_err(io)?
                && !directory.join(RECORD).try_exists().map_err(io)?
            {
                continue;
            }
            private_fs::PrivateDir::open(&directory).map_err(map_private_error)?;
            if directory.join(".panita").try_exists().map_err(io)? {
                private_fs::PrivateDir::open(directory.join(".panita"))
                    .map_err(map_private_error)?;
            }
            let store = Self::store_at(&directory)?;
            if let Some(bytes) = store.read(Self::record_name(&directory))? {
                let record: InstallationRecord =
                    serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                record.validate()?;
                // A UUID folder with a different assigned name is an interrupted move.
                if record.directory_name.as_deref() != Some(name.as_str())
                    && name != record.id.to_string()
                {
                    return Err(invalid());
                }
                if !ids.insert(record.id) {
                    return Err(invalid());
                }
                records.push((directory, record));
            }
        }
        records.sort_by_key(|(_, record)| record.id);
        Ok(records)
    }

    fn available(&self, name: &str) -> Result<bool, CoreError> {
        for entry in std::fs::read_dir(self.paths.root().join("instances")).map_err(io)? {
            if entry
                .map_err(io)?
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn allocate_name(&self, record: &InstallationRecord) -> Result<String, CoreError> {
        let slug = &record.last_catalog.slug;
        if valid_directory_name(slug) && self.available(slug)? {
            return Ok(slug.clone());
        }
        let suffixed = format!("{slug}-{}", record.id);
        if !valid_directory_name(&suffixed) || !self.available(&suffixed)? {
            return Err(invalid());
        }
        Ok(suffixed)
    }

    /// Persist the destination before renaming. A crash on either side leaves
    /// UUID ownership discoverable, without copying or replacing user data.
    fn migrate(
        &self,
        directory: PathBuf,
        record: InstallationRecord,
    ) -> Result<(PathBuf, InstallationRecord), CoreError> {
        if directory.file_name().and_then(|name| name.to_str()) == record.directory_name.as_deref()
        {
            let record = crate::instance_layout::prepare(&directory, record)?;
            return Ok((directory, record));
        }
        if record.state == InstallationState::Running {
            return Err(CoreError::new(ErrorCode::OperationBusy));
        }
        let operation_root = if Self::record_name(&directory) == LEGACY_RECORD {
            directory.join(".panita")
        } else {
            directory.clone()
        };
        let operation =
            crate::minecraft::download::try_lock(&operation_root, ".installation.lock")?
                .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
        let name = match &record.directory_name {
            Some(name) => name.clone(),
            None => self.allocate_name(&record)?,
        };
        let destination = self.paths.root().join("instances").join(&name);
        if destination != directory && !self.available(&name)? {
            return Err(invalid());
        }
        let store = Self::store_at(&directory)?;
        let record_name = Self::record_name(&directory);
        let next = store.with_lock(|transaction| {
            let bytes = transaction.read(record_name)?.ok_or_else(invalid)?;
            let mut current = InstallationRecord::decode(&bytes, record.id)?;
            if current.state == InstallationState::Running
                || current.directory_name != record.directory_name
            {
                return Err(CoreError::new(ErrorCode::OperationBusy));
            }
            current.schema_version = current.schema_version.max(2);
            current.directory_name = Some(name);
            current.validate()?;
            transaction.write(record_name, &formatted_json(&current)?)?;
            Ok(current)
        })?;
        // Windows may refuse a directory move while descendants have open
        // handles. Keep the root mutation lock throughout, publish the new
        // schema first, then close all source handles before the rename.
        drop(store);
        drop(operation);
        if destination != directory {
            #[cfg(windows)]
            std::fs::rename(&directory, &destination).map_err(io)?;
            #[cfg(target_os = "linux")]
            rustix::fs::renameat_with(
                rustix::fs::CWD,
                &directory,
                rustix::fs::CWD,
                &destination,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|error| io(error.into()))?;
            #[cfg(not(any(windows, target_os = "linux")))]
            return Err(CoreError::new(ErrorCode::UnsupportedInstance));
            #[cfg(unix)]
            std::fs::File::open(self.paths.root().join("instances"))
                .and_then(|parent| parent.sync_all())
                .map_err(io)?;
        }
        let next = crate::instance_layout::prepare(&destination, next)?;
        Ok((destination, next))
    }

    pub fn instance_path(&self, id: Uuid) -> Result<PathBuf, CoreError> {
        PrivateStore::open(&self.paths)?.with_lock(|_| {
            self.scan()?
                .into_iter()
                .find(|(_, record)| record.id == id)
                .map(|(directory, _)| directory)
                .ok_or_else(|| CoreError::new(ErrorCode::InstanceMissing))
        })
    }

    /// Read one identity without attempting unrelated directory migrations.
    pub fn get(&self, id: Uuid) -> Result<Option<InstallationRecord>, CoreError> {
        PrivateStore::open(&self.paths)?.with_lock(|_| {
            Ok(self
                .scan()?
                .into_iter()
                .find(|(_, record)| record.id == id)
                .map(|(_, record)| record))
        })
    }

    /// Finish legacy migration before any installation, repair, audit or game lease.
    pub fn prepare_instance(&self, id: Uuid) -> Result<PathBuf, CoreError> {
        PrivateStore::open(&self.paths)?.with_lock(|_| {
            let (directory, record) = self
                .scan()?
                .into_iter()
                .find(|(_, record)| record.id == id)
                .ok_or_else(|| CoreError::new(ErrorCode::InstanceMissing))?;
            Ok(self.migrate(directory, record)?.0)
        })
    }

    #[cfg(test)]
    fn store(&self, id: Uuid) -> Result<PrivateStore, CoreError> {
        Self::store_at(&self.instance_path(id)?)
    }

    pub fn load(&self) -> Result<Vec<InstallationRecord>, CoreError> {
        PrivateStore::open(&self.paths)?.with_lock(|_| {
            self.scan()?
                .into_iter()
                .map(|(directory, record)| {
                    match self.migrate(directory, record.clone()) {
                        Ok((_, migrated)) => Ok(migrated),
                        // Keep existing operations discoverable while postponing their move.
                        Err(error) if error.code() == ErrorCode::OperationBusy => Ok(record),
                        Err(error) => Err(error),
                    }
                })
                .collect()
        })
    }

    /// A failed move must not hide valid installations from the catalog. Read
    /// their original ownership records and surface a warning; operations still
    /// require successful preparation before accessing game files.
    fn load_for_overview(&self) -> Result<(Vec<InstallationRecord>, Option<ErrorCode>), CoreError> {
        match self.load() {
            Ok(records) => Ok((records, None)),
            Err(error) => PrivateStore::open(&self.paths)?.with_lock(|_| {
                Ok((
                    self.scan()?.into_iter().map(|(_, record)| record).collect(),
                    Some(error.code()),
                ))
            }),
        }
    }

    fn create(&self, record: &InstallationRecord) -> Result<(), CoreError> {
        let root = private_fs::PrivateDir::ensure(self.paths.root()).map_err(map_private_error)?;
        let instances = root.ensure_dir("instances").map_err(map_private_error)?;
        let legacy = self
            .paths
            .root()
            .join("instances")
            .join(record.id.to_string());
        if legacy.try_exists().map_err(io)? {
            // Without an installation record, the UUID-shaped name is not proof
            // of ownership. Preserve it and require recovery instead of adoption.
            return Err(invalid());
        } else {
            let name = self.allocate_name(record)?;
            instances
                .create_dir(&name)
                .map_err(map_private_error)?
                .create_dir("minecraft")
                .map_err(map_private_error)?;
            let store = Self::store_at(&self.paths.root().join("instances").join(&name))?;
            let mut next = record.clone();
            next.schema_version = 3;
            next.directory_name = Some(name);
            next.validate()?;
            store.transaction(RECORD, |existing| {
                if existing.is_some() {
                    return Err(invalid());
                }
                Ok((formatted_json(&next)?, ()))
            })?;
        }
        Ok(())
    }

    /// Allocate without resetting an installation concurrently started elsewhere.
    pub(crate) fn ensure_catalog_instance(
        &self,
        catalog: &CatalogInstance,
    ) -> Result<PathBuf, CoreError> {
        catalog.validate()?;
        PrivateStore::open(&self.paths)?.with_lock(|_| {
            if let Some((directory, record)) = self
                .scan()?
                .into_iter()
                .find(|(_, record)| record.id == catalog.id)
            {
                return Ok(self.migrate(directory, record)?.0);
            }
            self.create(&InstallationRecord {
                schema_version: 1,
                id: catalog.id,
                directory_name: None,
                last_catalog: catalog.clone(),
                resolved_source: catalog.source.clone(),
                state: InstallationState::NotInstalled,
                managed_files: Vec::new(),
                settings: InstanceSettings::default(),
            })?;
            self.scan()?
                .into_iter()
                .find(|(_, record)| record.id == catalog.id)
                .map(|(directory, _)| directory)
                .ok_or_else(invalid)
        })
    }

    fn save_record(
        &self,
        record: &InstallationRecord,
        preserve_catalog: bool,
    ) -> Result<(), CoreError> {
        record.validate()?;
        PrivateStore::open(&self.paths)?.with_lock(|_| {
            let Some((directory, previous)) = self
                .scan()?
                .into_iter()
                .find(|(_, previous)| previous.id == record.id)
            else {
                return self.create(record);
            };
            let (directory, previous) = match self.migrate(directory.clone(), previous.clone()) {
                Ok(migrated) => migrated,
                Err(error) if error.code() == ErrorCode::OperationBusy => (directory, previous),
                Err(error) => return Err(error),
            };
            if record.directory_name.is_some() && record.directory_name != previous.directory_name {
                return Err(invalid());
            }
            Self::store_at(&directory)?.transaction(Self::record_name(&directory), |existing| {
                let current =
                    InstallationRecord::decode(&existing.ok_or_else(invalid)?, record.id)?;
                let mut next = record.clone();
                next.schema_version = current.schema_version;
                next.directory_name = current.directory_name;
                next.settings = current.settings;
                if preserve_catalog {
                    next.last_catalog = current.last_catalog;
                }
                next.validate()?;
                Ok((formatted_json(&next)?, ()))
            })
        })
    }

    /// Metadata only: actual game files must be verified before completion.
    pub fn save(&self, record: &InstallationRecord) -> Result<(), CoreError> {
        self.save_record(record, false)
    }

    pub(crate) fn save_installation(&self, record: &InstallationRecord) -> Result<(), CoreError> {
        self.save_record(record, true)
    }

    pub fn load_settings(&self, id: Uuid) -> Result<InstanceSettings, CoreError> {
        PrivateStore::open(&self.paths)?.with_lock(|_| {
            let Some((directory, record)) =
                self.scan()?.into_iter().find(|(_, record)| record.id == id)
            else {
                return Ok(InstanceSettings::default());
            };
            if record.schema_version == 3 {
                return Ok(record.settings);
            }
            match Self::store_at(&directory)?.read("settings.json")? {
                None => Ok(InstanceSettings::default()),
                Some(bytes) => decode_settings(&bytes),
            }
        })
    }

    pub fn save_settings(&self, id: Uuid, settings: &InstanceSettings) -> Result<(), CoreError> {
        settings.validate()?;
        PrivateStore::open(&self.paths)?.with_lock(|_| {
            let (directory, record) = self
                .scan()?
                .into_iter()
                .find(|(_, record)| record.id == id)
                .ok_or_else(|| CoreError::new(ErrorCode::InstanceMissing))?;
            let (directory, _) = self.migrate(directory, record)?;
            Self::store_at(&directory)?.transaction(RECORD, |existing| {
                let mut current = InstallationRecord::decode(&existing.ok_or_else(invalid)?, id)?;
                current.settings = settings.clone();
                current.validate()?;
                Ok((formatted_json(&current)?, ()))
            })
        })
    }

    /// Catalog changes update availability, never the assigned directory or pin.
    pub fn remember_catalog(&self, records: &[CatalogInstance]) -> Result<(), CoreError> {
        PrivateStore::open(&self.paths)?.with_lock(|_| {
            for (directory, installed) in self.scan()? {
                let Some(remote) = records.iter().find(|remote| remote.id == installed.id) else {
                    continue;
                };
                if remote == &installed.last_catalog {
                    continue;
                }
                remote.validate()?;
                Self::store_at(&directory)?.transaction(
                    Self::record_name(&directory),
                    |existing| {
                        let mut current = InstallationRecord::decode(
                            &existing.ok_or_else(invalid)?,
                            installed.id,
                        )?;
                        current.last_catalog = remote.clone();
                        current.validate()?;
                        Ok((formatted_json(&current)?, ()))
                    },
                )?;
            }
            Ok(())
        })
    }
}

fn decode_settings(bytes: &[u8]) -> Result<InstanceSettings, CoreError> {
    let settings: InstanceSettings = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    settings.validate()?;
    Ok(settings)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Enabled,
    Disabled,
    Missing,
    Unverified,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceView {
    #[serde(flatten)]
    pub catalog: CatalogInstance,
    pub availability: Availability,
    pub local_state: InstallationState,
    pub installed_version: Option<String>,
    pub has_installation_manifest: bool,
    pub compatible: bool,
    pub supported: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogOverview {
    pub instances: Vec<InstanceView>,
    pub stale: bool,
    pub cache_warning: Option<ErrorCode>,
    pub registry_warning: Option<ErrorCode>,
}

pub async fn load_catalog(paths: AppPaths) -> Result<CatalogOverview, CoreError> {
    let store = PrivateStore::open(&paths)?;
    let registry = InstanceRegistry::new(paths.clone());
    let (mut local, migration_warning) = registry.load_for_overview()?;
    if migration_warning.is_none() {
        for record in &local {
            if let Err(error) = crate::minecraft::install::recover(
                &paths,
                record.id,
                &crate::minecraft::download::Cancellation::default(),
            )
            .await
                && error.code() != ErrorCode::OperationBusy
            {
                return Err(error);
            }
        }
        local = registry.load()?;
    }
    let version = Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?;
    let remote = crate::catalog::CatalogClient::new()?.refresh(&store).await;
    let mut overview = match remote {
        Ok(snapshot) => {
            let registry_warning = if snapshot.stale {
                None
            } else {
                registry
                    .remember_catalog(&snapshot.instances)
                    .err()
                    .map(|error| error.code())
            };
            CatalogOverview {
                instances: reconcile(Some(&snapshot), &local, &version),
                stale: snapshot.stale,
                cache_warning: snapshot.cache_warning,
                registry_warning: migration_warning.or(registry_warning),
            }
        }
        Err(error)
            if !local.is_empty()
                && matches!(
                    error.code(),
                    ErrorCode::NetworkUnavailable | ErrorCode::ServiceUnavailable
                ) =>
        {
            CatalogOverview {
                instances: reconcile(None, &local, &version),
                stale: true,
                cache_warning: None,
                registry_warning: migration_warning,
            }
        }
        Err(error) => return Err(error),
    };
    if migration_warning.is_none() {
        for view in &mut overview.instances {
            if local.iter().any(|record| record.id == view.catalog.id) {
                match crate::minecraft::maintenance::has_installation_manifest(
                    &paths,
                    view.catalog.id,
                ) {
                    Ok(has_manifest) => view.has_installation_manifest = has_manifest,
                    Err(error) => {
                        overview.registry_warning.get_or_insert(error.code());
                    }
                }
            }
        }
    }
    Ok(overview)
}

pub fn reconcile(
    snapshot: Option<&CatalogSnapshot>,
    local: &[InstallationRecord],
    client: &Version,
) -> Vec<InstanceView> {
    let mut merged: HashMap<Uuid, InstanceView> = local
        .iter()
        .map(|record| {
            let availability = if !record.last_catalog.enabled {
                Availability::Disabled
            } else if snapshot.is_some_and(|snapshot| !snapshot.stale) {
                Availability::Missing
            } else {
                Availability::Unverified
            };
            (
                record.id,
                view(
                    record.last_catalog.clone(),
                    Some(record),
                    availability,
                    client,
                ),
            )
        })
        .collect();
    if let Some(snapshot) = snapshot {
        for remote in &snapshot.instances {
            let installed = local.iter().find(|record| record.id == remote.id);
            let known_disabled =
                snapshot.stale && installed.is_some_and(|record| !record.last_catalog.enabled);
            let availability = if !remote.enabled || known_disabled {
                Availability::Disabled
            } else {
                Availability::Enabled
            };
            merged.insert(
                remote.id,
                view(remote.clone(), installed, availability, client),
            );
        }
    }
    let mut views: Vec<_> = merged.into_values().collect();
    views.sort_by_key(|view| (view.catalog.sort_order, view.catalog.id));
    views
}

fn view(
    catalog: CatalogInstance,
    installed: Option<&InstallationRecord>,
    availability: Availability,
    client: &Version,
) -> InstanceView {
    let compatible = catalog.compatible_with(client);
    let supported = matches!(catalog.source, CatalogSource::Vanilla { .. }) && !catalog.restricted;
    let local_state = installed.map_or(InstallationState::NotInstalled, |record| {
        if matches!(
            record.state,
            InstallationState::Installed | InstallationState::UpdateAvailable
        ) && record.resolved_source != catalog.source
        {
            InstallationState::UpdateAvailable
        } else if record.state == InstallationState::UpdateAvailable {
            InstallationState::Installed
        } else {
            record.state
        }
    });
    let installed_version = installed.and_then(|record| match &record.resolved_source {
        CatalogSource::Vanilla { minecraft_version } => Some(minecraft_version.clone()),
        _ => None,
    });
    InstanceView {
        catalog,
        availability,
        local_state,
        installed_version,
        has_installation_manifest: false,
        compatible,
        supported,
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
