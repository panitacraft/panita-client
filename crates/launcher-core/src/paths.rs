use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::error::{CoreError, ErrorCode};

/// Explicit data root keeps host integration and filesystem tests independent.
#[derive(Clone, Debug)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, CoreError> {
        let root = root.into();
        if !root.is_absolute() || root.as_os_str().is_empty() {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn accounts(&self) -> PathBuf {
        self.root.join("accounts.json")
    }

    pub fn panita_session(&self) -> PathBuf {
        self.root.join("panita-session.json")
    }

    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn catalog_cache(&self) -> PathBuf {
        self.root.join("insts-cache.json")
    }

    /// Resolve persisted UUID ownership; never infer identity from a display slug.
    pub fn instance(&self, id: Uuid) -> Result<PathBuf, CoreError> {
        crate::registry::InstanceRegistry::new(self.clone()).instance_path(id)
    }

    pub fn instance_settings(&self, id: Uuid) -> Result<PathBuf, CoreError> {
        Ok(crate::registry::InstanceRegistry::new(self.clone())
            .prepare_instance(id)?
            .join("instance.json"))
    }

    pub fn game_directory(&self, id: Uuid) -> Result<PathBuf, CoreError> {
        Ok(crate::registry::InstanceRegistry::new(self.clone())
            .prepare_instance(id)?
            .join("minecraft"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_identity_does_not_invent_an_installation_path() {
        let root = std::env::temp_dir().join(format!("panita-paths-{}", Uuid::new_v4()));
        let Ok(paths) = AppPaths::new(&root) else {
            panic!("valid absolute root");
        };
        let Ok(id) = Uuid::parse_str("62d7e99d-fb4e-4167-8732-ce3a334d4a58") else {
            panic!("valid catalog UUID");
        };
        assert!(paths.instance(id).is_err());
        assert!(paths.instance_settings(id).is_err());
        assert!(!root.join("instances").join(id.to_string()).exists());
        assert!(std::fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn rejects_relative_data_root() {
        assert!(AppPaths::new("PanitaClient").is_err());
    }
}
