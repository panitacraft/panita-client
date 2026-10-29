//! One-time retirement of legacy directory ACLs, without reading player files.
use private_fs::PrivateDir;

use super::{PrivateStore, map_io_error, map_private_error};
use crate::error::{CoreError, ErrorCode};

const MARKER: &str = "directory-permissions.json";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    schema: u32,
}

pub(super) fn migrate(store: &PrivateStore) -> Result<(), CoreError> {
    store.with_lock(|transaction| {
        if let Some(bytes) = transaction.read(MARKER)? {
            let marker: Marker = serde_json::from_slice(&bytes)
                .map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
            if marker.schema != 1 {
                return Err(CoreError::new(ErrorCode::InvalidData));
            }
            return Ok(());
        }
        let mut remaining = 100_000;
        walk(&store.root, 0, &mut remaining)?;
        transaction.write(MARKER, b"{\n  \"schema\": 1\n}\n")
    })
}

fn walk(root: &PrivateDir, depth: usize, remaining: &mut usize) -> Result<(), CoreError> {
    if depth > 128 || *remaining == 0 {
        return Err(CoreError::new(ErrorCode::InvalidData));
    }
    *remaining -= 1;
    let directory = crate::minecraft::directory_capability(root)?;
    for entry in directory.entries().map_err(map_io_error)? {
        let entry = entry.map_err(map_io_error)?;
        let kind = entry.file_type().map_err(map_io_error)?;
        if kind.is_symlink() {
            return Err(CoreError::new(ErrorCode::UnsafePermissions));
        }
        if kind.is_dir() {
            // Open one child relative to the retained parent. The dependency
            // migrates only its exact legacy ACL and refuses redirected nodes.
            let child = root
                .open_dir(entry.file_name())
                .map_err(map_private_error)?;
            walk(&child, depth + 1, remaining)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;

    #[test]
    fn unknown_or_corrupt_migration_marker_is_preserved() {
        for bytes in [b"{\"schema\":2}".as_slice(), b"invalid".as_slice()] {
            let path = std::env::temp_dir().join(format!("panita-acl-{}", uuid::Uuid::new_v4()));
            let paths = AppPaths::new(&path).unwrap_or_else(|_| panic!("test path"));
            let root = PrivateDir::ensure(&path).unwrap_or_else(|_| panic!("test root"));
            root.write_atomic_durable(MARKER, bytes)
                .unwrap_or_else(|_| panic!("synthetic marker"));
            assert!(PrivateStore::open_application(&paths).is_err());
            assert_eq!(
                std::fs::read(path.join(MARKER)).ok().as_deref(),
                Some(bytes)
            );
            drop(root);
            std::fs::remove_dir_all(path).unwrap_or_else(|_| panic!("test cleanup"));
        }
    }

    #[test]
    fn migration_completes_once_and_preserves_player_bytes_on_restart() {
        let path = std::env::temp_dir().join(format!("panita-acl-{}", uuid::Uuid::new_v4()));
        let paths = AppPaths::new(&path).unwrap_or_else(|_| panic!("test path"));
        let root = PrivateDir::ensure(&path).unwrap_or_else(|_| panic!("test root"));
        let mods = root
            .ensure_dir("instances")
            .and_then(|dir| dir.ensure_dir("test"))
            .and_then(|dir| dir.ensure_dir("minecraft"))
            .and_then(|dir| dir.ensure_dir("mods"))
            .unwrap_or_else(|_| panic!("game folders"));
        let jar = path.join("instances/test/minecraft/mods/player.jar");
        std::fs::write(&jar, b"player-owned").unwrap_or_else(|_| panic!("player file"));
        drop(mods);
        drop(root);
        let store = PrivateStore::open_application(&paths).unwrap_or_else(|_| panic!("migration"));
        assert!(store.read(MARKER).ok().flatten().is_some());
        drop(store);
        let store = PrivateStore::open_application(&paths).unwrap_or_else(|_| panic!("restart"));
        assert_eq!(
            std::fs::read(&jar).ok().as_deref(),
            Some(b"player-owned".as_slice())
        );
        std::fs::write(&jar, b"edited").unwrap_or_else(|_| panic!("edit player file"));
        std::fs::remove_file(jar).unwrap_or_else(|_| panic!("remove player file"));
        drop(store);
        std::fs::remove_dir_all(path).unwrap_or_else(|_| panic!("test cleanup"));
    }
}
