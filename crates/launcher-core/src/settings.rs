use serde::{Deserialize, Serialize};

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
}
