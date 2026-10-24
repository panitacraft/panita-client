//! Compact disk manifest with explicit state, retaining legacy readers for migration.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum StoredInstalled {
    Current(Current),
    Legacy(Legacy),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Current {
    schema: u32,
    id: Uuid,
    operation: Uuid,
    status: Status,
    plan: InstallPlan,
    natives: Vec<NativeFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_directory: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Installed,
    Uninstalled,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Legacy {
    schema_version: u32,
    instance_id: Uuid,
    operation: Uuid,
    plan: InstallPlan,
    native_files: Vec<NativeFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_directory: Option<String>,
}

impl TryFrom<StoredInstalled> for InstalledLock {
    type Error = CoreError;

    fn try_from(value: StoredInstalled) -> Result<Self, Self::Error> {
        match value {
            StoredInstalled::Current(value) => {
                if value.schema != 4 {
                    return Err(invalid());
                }
                Ok(Self {
                    schema_version: match value.status {
                        Status::Installed => 2,
                        Status::Uninstalled => 3,
                    },
                    instance_id: value.id,
                    operation: value.operation,
                    plan: value.plan,
                    native_files: value.natives,
                    native_directory: value.native_directory,
                })
            }
            StoredInstalled::Legacy(value) => {
                if !matches!(value.schema_version, 1..=3) {
                    return Err(invalid());
                }
                Ok(Self {
                    schema_version: value.schema_version,
                    instance_id: value.instance_id,
                    operation: value.operation,
                    plan: value.plan,
                    native_files: value.native_files,
                    native_directory: value.native_directory,
                })
            }
        }
    }
}

impl From<InstalledLock> for StoredInstalled {
    fn from(value: InstalledLock) -> Self {
        if matches!(value.schema_version, 2 | 3) {
            Self::Current(Current {
                schema: 4,
                id: value.instance_id,
                operation: value.operation,
                status: if value.schema_version == 2 {
                    Status::Installed
                } else {
                    Status::Uninstalled
                },
                plan: value.plan,
                natives: value.native_files,
                native_directory: value.native_directory,
            })
        } else {
            Self::Legacy(Legacy {
                schema_version: value.schema_version,
                instance_id: value.instance_id,
                operation: value.operation,
                plan: value.plan,
                native_files: value.native_files,
                native_directory: value.native_directory,
            })
        }
    }
}
