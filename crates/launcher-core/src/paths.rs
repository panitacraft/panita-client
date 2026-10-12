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

    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn catalog_cache(&self) -> PathBuf {
        self.root.join("catalog-cache.json")
    }

    pub fn instance(&self, id: Uuid) -> PathBuf {
        self.root
            .join("instances")
            .join(id.hyphenated().to_string())
    }

    pub fn instance_settings(&self, id: Uuid) -> PathBuf {
        self.instance(id).join(".panita").join("settings.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_path_uses_immutable_uuid() {
        let root = std::env::temp_dir().join("panita-test");
        let Ok(paths) = AppPaths::new(&root) else {
            panic!("valid absolute root");
        };
        let Ok(id) = Uuid::parse_str("62d7e99d-fb4e-4167-8732-ce3a334d4a58") else {
            panic!("valid catalog UUID");
        };
        assert_eq!(
            paths.instance(id),
            root.join("instances").join(id.to_string())
        );
        assert_eq!(
            paths.instance_settings(id),
            paths.instance(id).join(".panita/settings.json")
        );
    }

    #[test]
    fn rejects_relative_data_root() {
        assert!(AppPaths::new("PanitaClient").is_err());
    }
}
