use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorCode};
use crate::storage::PrivateStore;

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GlobalSettings {
    pub schema_version: u32,
    pub ram_mib: u32,
    pub extra_jvm_args: Vec<String>,
    pub java_path_override: Option<String>,
    pub window_size: WindowSize,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            ram_mib: 4096,
            extra_jvm_args: Vec::new(),
            java_path_override: None,
            window_size: WindowSize {
                width: 1280,
                height: 720,
            },
        }
    }
}

impl GlobalSettings {
    pub fn load(store: &PrivateStore) -> Result<Self, CoreError> {
        match store.read("settings.json")? {
            Some(bytes) => Self::decode(&bytes),
            None => Ok(Self::default()),
        }
    }

    pub fn save(&self, store: &PrivateStore) -> Result<(), CoreError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
        store.transaction("settings.json", |previous| {
            if let Some(existing) = previous {
                Self::decode(&existing)?;
            }
            Ok((bytes, ()))
        })
    }

    fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        let settings: Self =
            serde_json::from_slice(bytes).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
        settings.validate()?;
        Ok(settings)
    }

    fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION
            || !(512..=262_144).contains(&self.ram_mib)
            || !(320..=16_384).contains(&self.window_size.width)
            || !(240..=16_384).contains(&self.window_size.height)
            || self.extra_jvm_args.len() > 64
            || self
                .extra_jvm_args
                .iter()
                .any(|arg| arg.len() > 4096 || arg.contains('\0'))
            || self
                .java_path_override
                .as_deref()
                .is_some_and(|path| path.is_empty() || path.contains('\0'))
        {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstanceSettings {
    pub schema_version: u32,
    pub ram_mib: Option<u32>,
    pub extra_jvm_args: Option<Vec<String>>,
    pub java_path_override: Option<String>,
    pub window_size: Option<WindowSize>,
}

impl Default for InstanceSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            ram_mib: None,
            extra_jvm_args: None,
            java_path_override: None,
            window_size: None,
        }
    }
}

impl InstanceSettings {
    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        self.effective(&GlobalSettings::default()).validate()
    }

    pub fn effective(&self, global: &GlobalSettings) -> GlobalSettings {
        GlobalSettings {
            schema_version: SETTINGS_SCHEMA_VERSION,
            ram_mib: self.ram_mib.unwrap_or(global.ram_mib),
            extra_jvm_args: self
                .extra_jvm_args
                .clone()
                .unwrap_or_else(|| global.extra_jvm_args.clone()),
            java_path_override: self
                .java_path_override
                .clone()
                .or_else(|| global.java_path_override.clone()),
            window_size: self
                .window_size
                .clone()
                .unwrap_or_else(|| global.window_size.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use uuid::Uuid;

    #[test]
    fn instance_overrides_only_selected_fields() {
        let global = GlobalSettings::default();
        let instance = InstanceSettings {
            ram_mib: Some(6144),
            ..InstanceSettings::default()
        };
        let effective = instance.effective(&global);
        assert_eq!(effective.ram_mib, 6144);
        assert_eq!(effective.window_size, global.window_size);
    }

    #[test]
    fn malformed_settings_remain_untouched() {
        let root = std::env::temp_dir().join(format!("panita-settings-{}", Uuid::new_v4()));
        let paths = AppPaths::new(root).unwrap_or_else(|_| panic!("absolute path"));
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("private root"));
        assert!(
            store
                .transaction("settings.json", |_| Ok((b"{broken".to_vec(), ())))
                .is_ok()
        );
        assert_eq!(
            GlobalSettings::load(&store).err().map(|error| error.code()),
            Some(ErrorCode::InvalidData)
        );
        assert!(GlobalSettings::default().save(&store).is_err());
        assert_eq!(
            store.read("settings.json").ok().flatten(),
            Some(b"{broken".to_vec())
        );
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn settings_round_trip_with_schema_version() {
        let root = std::env::temp_dir().join(format!("panita-settings-{}", Uuid::new_v4()));
        let paths = AppPaths::new(root).unwrap_or_else(|_| panic!("absolute path"));
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("private root"));
        let settings = GlobalSettings {
            ram_mib: 8192,
            ..GlobalSettings::default()
        };
        assert!(settings.save(&store).is_ok());
        assert_eq!(GlobalSettings::load(&store).ok(), Some(settings));
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }
}
