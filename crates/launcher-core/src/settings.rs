use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorCode};
use crate::storage::PrivateStore;

pub const SETTINGS_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherPreferences {
    pub selected_instance_id: Option<uuid::Uuid>,
    pub window: Option<LauncherWindow>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherWindow {
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl LauncherWindow {
    pub fn validate(&self) -> Result<(), CoreError> {
        if !(780..=16_384).contains(&self.width) || !(520..=16_384).contains(&self.height) {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        Ok(())
    }
}

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
    #[serde(default)]
    pub launcher: LauncherPreferences,
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
            launcher: LauncherPreferences::default(),
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
        let mut settings: Self =
            serde_json::from_slice(bytes).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
        if settings.schema_version == 1 {
            settings.schema_version = SETTINGS_SCHEMA_VERSION;
        }
        settings.validate()?;
        Ok(settings)
    }

    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION
            || !(512..=262_144).contains(&self.ram_mib)
            || !(320..=16_384).contains(&self.window_size.width)
            || !(240..=16_384).contains(&self.window_size.height)
            || self.extra_jvm_args.len() > 64
            || self
                .launcher
                .selected_instance_id
                .is_some_and(|id| id.is_nil())
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
        if let Some(window) = &self.launcher.window {
            window.validate()?;
        }
        Ok(())
    }

    /// Validate editable launch options without executing Java or starting a game.
    pub fn validate_launch(&self) -> Result<(), CoreError> {
        self.validate()?;
        if self.java_path_override.is_some() {
            return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
        }
        for arg in &self.extra_jvm_args {
            validate_jvm_argument(arg)?;
        }
        Ok(())
    }

    /// Reload beneath the writer lock so window/selection updates cannot be lost.
    pub fn save_launch_settings(&self, store: &PrivateStore) -> Result<Self, CoreError> {
        self.validate_launch()?;
        Self::update(store, |current| {
            let launcher = current.launcher.clone();
            *current = self.clone();
            current.launcher = launcher;
            Ok(())
        })
    }

    pub fn select_instance(
        store: &PrivateStore,
        id: Option<uuid::Uuid>,
    ) -> Result<Self, CoreError> {
        Self::update(store, |current| {
            current.launcher.selected_instance_id = id;
            Ok(())
        })
    }

    pub fn save_launcher_window(
        store: &PrivateStore,
        window: LauncherWindow,
    ) -> Result<Self, CoreError> {
        window.validate()?;
        Self::update(store, |current| {
            current.launcher.window = Some(window);
            Ok(())
        })
    }

    fn update(
        store: &PrivateStore,
        change: impl FnOnce(&mut Self) -> Result<(), CoreError>,
    ) -> Result<Self, CoreError> {
        store.transaction("settings.json", |bytes| {
            let mut current = bytes
                .as_deref()
                .map(Self::decode)
                .transpose()?
                .unwrap_or_default();
            change(&mut current)?;
            current.validate()?;
            let bytes =
                serde_json::to_vec(&current).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
            Ok((bytes, current))
        })
    }
}

fn validate_jvm_argument(arg: &str) -> Result<(), CoreError> {
    let option = arg.split('=').next().unwrap_or_default();
    if arg.chars().any(char::is_control)
        || !(arg.starts_with("-XX:") || arg.starts_with("-D"))
        || matches!(
            option,
            "-XX:Flags"
                | "-XX:VMOptionsFile"
                | "-XX:MaxHeapSize"
                | "-XX:InitialHeapSize"
                | "-Djava.class.path"
                | "-Djava.library.path"
                | "-Dorg.lwjgl.librarypath"
                | "-Djna.tmpdir"
                | "-Dlog4j.configurationFile"
                | "-Djava.system.class.loader"
                | "-Dorg.lwjgl.system.SharedLibraryExtractPath"
                | "-Dio.netty.native.workdir"
        )
    {
        return Err(CoreError::new(ErrorCode::InvalidData));
    }
    Ok(())
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
            schema_version: 1,
            ram_mib: None,
            extra_jvm_args: None,
            java_path_override: None,
            window_size: None,
        }
    }
}

impl InstanceSettings {
    pub fn validate_launch(&self, global: &GlobalSettings) -> Result<(), CoreError> {
        self.validate()?;
        self.effective(global).validate_launch()
    }

    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version != 1 {
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
            launcher: global.launcher.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use uuid::Uuid;

    fn isolated_store() -> (AppPaths, PrivateStore) {
        let paths =
            AppPaths::new(std::env::temp_dir().join(format!("panita-settings-{}", Uuid::new_v4())))
                .unwrap_or_else(|_| panic!("paths"));
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store"));
        (paths, store)
    }

    #[test]
    fn legacy_settings_migrate_without_losing_launch_values_or_writing_on_read() {
        let (paths, store) = isolated_store();
        let legacy = br#"{"schema_version":1,"ram_mib":8192,"extra_jvm_args":["-Dtest=value"],"java_path_override":null,"window_size":{"width":1024,"height":768}}"#;
        assert!(
            store
                .transaction("settings.json", |_| Ok((legacy.to_vec(), ())))
                .is_ok()
        );
        let migrated = GlobalSettings::load(&store).unwrap_or_else(|_| panic!("migration"));
        assert_eq!(migrated.schema_version, 2);
        assert_eq!(migrated.ram_mib, 8192);
        assert_eq!(migrated.extra_jvm_args, vec!["-Dtest=value"]);
        assert_eq!(migrated.window_size.width, 1024);
        assert_eq!(
            store.read("settings.json").ok().flatten(),
            Some(legacy.to_vec())
        );
        let id = Uuid::new_v4();
        assert!(GlobalSettings::select_instance(&store, Some(id)).is_ok());
        let saved = GlobalSettings::load(&store).unwrap_or_else(|_| panic!("saved"));
        assert_eq!(saved.ram_mib, migrated.ram_mib);
        assert_eq!(saved.launcher.selected_instance_id, Some(id));
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn independent_settings_writers_preserve_selection_window_and_launch_changes() {
        let (paths, store) = isolated_store();
        let stale_editor = GlobalSettings {
            ram_mib: 8192,
            ..GlobalSettings::default()
        };
        let other = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("second store"));
        let id = Uuid::new_v4();
        assert!(GlobalSettings::select_instance(&other, Some(id)).is_ok());
        let window = LauncherWindow {
            width: 1000,
            height: 600,
            maximized: true,
        };
        assert!(GlobalSettings::save_launcher_window(&other, window.clone()).is_ok());
        assert!(stale_editor.save_launch_settings(&store).is_ok());
        let saved = GlobalSettings::load(&other).unwrap_or_else(|_| panic!("reload"));
        assert_eq!(saved.ram_mib, 8192);
        assert_eq!(saved.launcher.selected_instance_id, Some(id));
        assert_eq!(saved.launcher.window, Some(window));
        assert!(GlobalSettings::select_instance(&other, None).is_ok());
        assert_eq!(
            GlobalSettings::load(&store).ok().map(|value| value.ram_mib),
            Some(8192)
        );
        drop(other);
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn invalid_launch_options_and_preferences_leave_previous_settings_unchanged() {
        let (paths, store) = isolated_store();
        let valid = GlobalSettings::default();
        assert!(valid.save(&store).is_ok());
        let before = store.read("settings.json").unwrap_or_default();
        for arg in [
            "-Xmx8G",
            "-javaagent:agent.jar",
            "@options",
            "-Djava.class.path=other",
            "-XX:Flags=other",
            "-Dtest=line\nbreak",
        ] {
            let invalid = GlobalSettings {
                extra_jvm_args: vec![arg.into()],
                ..valid.clone()
            };
            assert!(invalid.save_launch_settings(&store).is_err());
        }
        let custom_java = GlobalSettings {
            java_path_override: Some("C:/custom/java.exe".into()),
            ..valid.clone()
        };
        assert_eq!(
            custom_java
                .validate_launch()
                .err()
                .map(|error| error.code()),
            Some(ErrorCode::RuntimeUnsupported)
        );
        assert!(custom_java.save_launch_settings(&store).is_err());
        assert!(GlobalSettings::select_instance(&store, Some(Uuid::nil())).is_err());
        assert!(
            GlobalSettings::save_launcher_window(
                &store,
                LauncherWindow {
                    width: 0,
                    height: 0,
                    maximized: false
                }
            )
            .is_err()
        );
        assert_eq!(store.read("settings.json").unwrap_or_default(), before);
        let overrides = InstanceSettings {
            ram_mib: Some(1),
            ..InstanceSettings::default()
        };
        assert!(overrides.validate_launch(&valid).is_err());
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn preference_updates_preserve_corrupt_and_future_schema_documents() {
        let (paths, store) = isolated_store();
        for bytes in [b"{broken".to_vec(), br#"{"schema_version":99,"ram_mib":4096,"extra_jvm_args":[],"java_path_override":null,"window_size":{"width":1280,"height":720}}"#.to_vec()] {
            assert!(store.transaction("settings.json", |_| Ok((bytes.clone(), ()))).is_ok());
            assert!(GlobalSettings::select_instance(&store, Some(Uuid::new_v4())).is_err());
            assert!(GlobalSettings::default().save_launch_settings(&store).is_err());
            assert_eq!(store.read("settings.json").ok().flatten(), Some(bytes));
        }
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

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
