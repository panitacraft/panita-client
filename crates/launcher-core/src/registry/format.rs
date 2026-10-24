//! Versioned disk representations; catalog DTOs and IPC retain their own contracts.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum StoredRecord {
    Current(Box<Current>),
    Legacy(Box<Legacy>),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Current {
    schema: u32,
    id: Uuid,
    directory: String,
    catalog: serde_json::Value,
    source: CatalogSource,
    status: InstallationState,
    settings: InstanceSettings,
    // Preserve nonempty legacy inventories until their ownership can be retired.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    legacy_files: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Legacy {
    schema_version: u32,
    id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    directory_name: Option<String>,
    last_catalog: CatalogInstance,
    resolved_source: CatalogSource,
    state: InstallationState,
    managed_files: Vec<String>,
    #[serde(default)]
    settings: InstanceSettings,
}

impl TryFrom<StoredRecord> for InstallationRecord {
    type Error = CoreError;

    fn try_from(value: StoredRecord) -> Result<Self, Self::Error> {
        let record = match value {
            StoredRecord::Current(mut stored) => {
                if stored.schema != 4 {
                    return Err(invalid());
                }
                let catalog = stored.catalog.as_object_mut().ok_or_else(invalid)?;
                if catalog
                    .insert("id".into(), stored.id.to_string().into())
                    .is_some()
                {
                    return Err(invalid());
                }
                Self {
                    schema_version: 4,
                    id: stored.id,
                    directory_name: Some(stored.directory),
                    last_catalog: serde_json::from_value(stored.catalog).map_err(|_| invalid())?,
                    resolved_source: stored.source,
                    state: stored.status,
                    managed_files: stored.legacy_files,
                    settings: stored.settings,
                }
            }
            StoredRecord::Legacy(stored) => {
                if !matches!(stored.schema_version, 1..=3) {
                    return Err(invalid());
                }
                Self {
                    schema_version: stored.schema_version,
                    id: stored.id,
                    directory_name: stored.directory_name,
                    last_catalog: stored.last_catalog,
                    resolved_source: stored.resolved_source,
                    state: stored.state,
                    managed_files: stored.managed_files,
                    settings: stored.settings,
                }
            }
        };
        record.validate()?;
        Ok(record)
    }
}

impl From<InstallationRecord> for StoredRecord {
    fn from(record: InstallationRecord) -> Self {
        if record.schema_version == 4 {
            let mut catalog = serde_json::json!(record.last_catalog);
            if let Some(catalog) = catalog.as_object_mut() {
                catalog.remove("id");
            }
            Self::Current(Box::new(Current {
                schema: 4,
                id: record.id,
                directory: record.directory_name.unwrap_or_default(),
                catalog,
                source: record.resolved_source,
                status: record.state,
                settings: record.settings,
                legacy_files: record.managed_files,
            }))
        } else {
            Self::Legacy(Box::new(Legacy {
                schema_version: record.schema_version,
                id: record.id,
                directory_name: record.directory_name,
                last_catalog: record.last_catalog,
                resolved_source: record.resolved_source,
                state: record.state,
                managed_files: record.managed_files,
                settings: record.settings,
            }))
        }
    }
}
