//! UUID-owned metadata; remote catalog changes never rename or remove game directories.
use std::collections::HashMap;
use std::path::Path;

use semver::Version;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::catalog::{CatalogInstance, CatalogSnapshot, CatalogSource};
use crate::error::{CoreError, ErrorCode};
use crate::paths::AppPaths;
use crate::settings::InstanceSettings;
use crate::storage::PrivateStore;

const RECORD: &str = "installation.json";

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
    pub last_catalog: CatalogInstance,
    pub resolved_source: CatalogSource,
    pub state: InstallationState,
    pub managed_files: Vec<String>,
}

impl InstallationRecord {
    fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version != 1
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

    fn store(&self, id: Uuid) -> Result<PrivateStore, CoreError> {
        if id.is_nil() {
            return Err(invalid());
        }
        // Create every level privately; normal recursive mkdir could expose files or follow redirects.
        use crate::storage::map_private_error;
        let root = private_fs::PrivateDir::ensure(self.paths.root()).map_err(map_private_error)?;
        let instances = root.ensure_dir("instances").map_err(map_private_error)?;
        let instance = instances
            .ensure_dir(id.to_string())
            .map_err(map_private_error)?;
        instance.ensure_dir(".panita").map_err(map_private_error)?;
        // The protected directory resolver rejects redirected components, including Windows junctions.
        PrivateStore::open(&AppPaths::new(self.paths.instance(id).join(".panita"))?)
    }

    pub fn load(&self) -> Result<Vec<InstallationRecord>, CoreError> {
        let root = self.paths.root().join("instances");
        match std::fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io(error)),
            Ok(_) => ensure_not_symlink(&root)?,
        }
        let mut records = Vec::new();
        for entry in std::fs::read_dir(root).map_err(io)? {
            let entry = entry.map_err(io)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(id) = Uuid::parse_str(&name) else {
                continue;
            };
            if name != id.to_string() {
                continue;
            }
            ensure_not_symlink(&entry.path())?;
            let metadata = entry.path().join(".panita");
            match std::fs::symlink_metadata(&metadata) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io(error)),
                Ok(_) => ensure_not_symlink(&metadata)?,
            }
            let store = self.store(id)?;
            if let Some(bytes) = store.read(RECORD)? {
                records.push(InstallationRecord::decode(&bytes, id)?);
            }
        }
        records.sort_by_key(|record| record.id);
        Ok(records)
    }

    /// Metadata only: Phase 4 must verify actual installation files before marking completion.
    pub fn save(&self, record: &InstallationRecord) -> Result<(), CoreError> {
        record.validate()?;
        let bytes = serde_json::to_vec(record).map_err(|_| invalid())?;
        self.store(record.id)?.transaction(RECORD, |existing| {
            if let Some(existing) = existing {
                InstallationRecord::decode(&existing, record.id)?;
            }
            Ok((bytes, ()))
        })
    }

    pub fn load_settings(&self, id: Uuid) -> Result<InstanceSettings, CoreError> {
        match self.store(id)?.read("settings.json")? {
            None => Ok(InstanceSettings::default()),
            Some(bytes) => decode_settings(&bytes),
        }
    }

    pub fn save_settings(&self, id: Uuid, settings: &InstanceSettings) -> Result<(), CoreError> {
        settings.validate()?;
        let bytes = serde_json::to_vec(settings).map_err(|_| invalid())?;
        self.store(id)?.transaction("settings.json", |existing| {
            if let Some(existing) = existing {
                decode_settings(&existing)?;
            }
            Ok((bytes, ()))
        })
    }

    /// Update known remote availability without overwriting installation pins or concurrent state.
    pub fn remember_catalog(&self, records: &[CatalogInstance]) -> Result<(), CoreError> {
        for installed in self.load()? {
            let Some(remote) = records.iter().find(|remote| remote.id == installed.id) else {
                continue;
            };
            if remote == &installed.last_catalog {
                continue;
            }
            remote.validate()?;
            self.store(installed.id)?.transaction(RECORD, |existing| {
                let mut current =
                    InstallationRecord::decode(&existing.ok_or_else(invalid)?, installed.id)?;
                current.last_catalog = remote.clone();
                current.validate()?;
                Ok((serde_json::to_vec(&current).map_err(|_| invalid())?, ()))
            })?;
        }
        Ok(())
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
    pub compatible: bool,
    pub supported: bool,
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
        compatible,
        supported,
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
