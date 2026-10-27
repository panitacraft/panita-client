//! Transient, protected operation journals with recoverable legacy-name migration.
use super::{
    download::io,
    install::{read_json_raw, write_json_raw},
    invalid,
};
use crate::{
    error::CoreError,
    paths::AppPaths,
    storage::{PrivateStore, map_private_error},
};
use serde_json::Value;
use std::path::Path;

pub(super) const FILE: &str = "transaction.json";
const LEGACY: &str = "install-journal.json";

fn canonical(mut value: Value) -> Result<Value, CoreError> {
    let object = value.as_object_mut().ok_or_else(invalid)?;
    if let Some(kind) = object.remove("type")
        && object.insert("kind".into(), kind).is_some()
    {
        return Err(invalid());
    }
    if let Some(schema) = object.remove("schema") {
        if object.insert("schema_version".into(), schema).is_some() {
            return Err(invalid());
        }
        if let Some(id) = object.remove("id")
            && object.insert("instance_id".into(), id).is_some()
        {
            return Err(invalid());
        }
        match object.get("schema_version").and_then(Value::as_u64) {
            Some(1) if object.get("kind").and_then(Value::as_str) == Some("install") => {
                object.remove("kind");
            }
            Some(2) if object.get("kind").and_then(Value::as_str) == Some("update") => {
                object.remove("kind");
            }
            Some(3) => (),
            _ => return Err(invalid()),
        }
    }
    Ok(value)
}

fn stored(mut value: Value) -> Result<Value, CoreError> {
    let object = value.as_object_mut().ok_or_else(invalid)?;
    let schema = object.remove("schema_version").ok_or_else(invalid)?;
    match schema.as_u64() {
        Some(1) => {
            object.insert("kind".into(), "install".into());
        }
        Some(2) => {
            object.insert("kind".into(), "update".into());
        }
        Some(3) => (),
        _ => return Err(invalid()),
    }
    object.insert("schema".into(), schema);
    if let Some(kind) = object.remove("kind")
        && object.insert("type".into(), kind).is_some()
    {
        return Err(invalid());
    }
    if let Some(id) = object.remove("instance_id") {
        object.insert("id".into(), id);
    }
    Ok(value)
}

/// The caller retains the installation lease. Conflicting copies remain untouched.
pub(super) fn read(root: &Path) -> Result<Option<Value>, CoreError> {
    let current = read_json_raw::<Value>(root, FILE)?
        .map(canonical)
        .transpose()?;
    let legacy = read_json_raw::<Value>(root, LEGACY)?;
    match (current, legacy) {
        (Some(current), Some(legacy)) if current != legacy => Err(invalid()),
        (Some(current), _) => Ok(Some(current)),
        (None, legacy) => Ok(legacy),
    }
}

pub(super) fn write(root: &Path, value: Value) -> Result<(), CoreError> {
    // Copy the original intent first, durably, before removing its old name. A
    // crash here leaves equivalent copies that read() accepts. Never replace a
    // conflicting transaction or remove the only durable intent before publication.
    if let Some(legacy) = read_json_raw::<Value>(root, LEGACY)? {
        if let Some(current) = read_json_raw::<Value>(root, FILE)? {
            if canonical(current)? != legacy {
                return Err(invalid());
            }
        } else {
            write_json_raw(root, FILE, &crate::storage_json::Ordered(&stored(legacy)?))?;
        }
        remove_one(root, LEGACY)?;
    }
    write_json_raw(root, FILE, &crate::storage_json::Ordered(&stored(value)?))
}

fn remove_one(root: &Path, name: &str) -> Result<(), CoreError> {
    PrivateStore::open(&AppPaths::new(root)?)?.with_lock(|_| {
        let private = private_fs::PrivateDir::open(root).map_err(map_private_error)?;
        match private.open_file(name) {
            Ok(file) => file.verify().map_err(map_private_error)?,
            Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => return Err(map_private_error(error)),
        }
        let capability = super::world_backup::capability(&private)?;
        capability.remove_file(name).map_err(io)?;
        #[cfg(unix)]
        capability.into_std_file().sync_all().map_err(io)?;
        Ok(())
    })
}

/// Only remove a durably inactive, recognized journal. If deletion is interrupted,
/// its possible reappearance is harmless and the next recovery repeats cleanup.
pub(super) fn remove(root: &Path) -> Result<(), CoreError> {
    if let Some(value) = read(root)? {
        if value.get("active") != Some(&Value::Bool(false)) {
            return Err(invalid());
        }
        // Recover callers have validated their complete typed journal before this call.
        remove_one(root, LEGACY)?;
        remove_one(root, FILE)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::minecraft::{download::Cancellation, install};
    use uuid::Uuid;

    #[test]
    fn transaction_headers_are_consistent_and_both_legacy_names_remain_readable() {
        for (schema, kind) in [
            (1, "install"),
            (2, "update"),
            (3, "repair"),
            (3, "uninstall"),
        ] {
            let mut original = serde_json::json!({"schema_version":schema,"instance_id":Uuid::new_v4(),
                "operation":Uuid::new_v4(),"active":true});
            if schema == 3 {
                original["kind"] = kind.into();
            }
            let stored = stored(original.clone()).unwrap_or_else(|_| panic!("store"));
            assert_eq!(stored["type"], kind);
            assert!(stored.get("kind").is_none());
            let bytes = crate::storage_json::pretty(&crate::storage_json::Ordered(&stored))
                .unwrap_or_default();
            assert!(
                String::from_utf8_lossy(&bytes)
                    .starts_with(&format!("{{\n  \"schema\": {schema},\n  \"type\""))
            );
            assert_eq!(canonical(stored.clone()).ok(), Some(original.clone()));
            let mut legacy = stored.clone();
            legacy
                .as_object_mut()
                .unwrap_or_else(|| panic!("object"))
                .remove("type");
            legacy["kind"] = kind.into();
            assert_eq!(canonical(legacy.clone()).ok(), Some(original));
            legacy["type"] = kind.into();
            assert!(canonical(legacy).is_err());
        }
    }

    #[tokio::test]
    async fn legacy_active_intent_recovers_and_completed_transaction_disappears() {
        let (paths, accounts, id) = crate::minecraft::play::tests::fixture();
        drop(accounts);
        let root = install::metadata_root(&paths, id).unwrap_or_else(|_| panic!("root"));
        let intent =
            serde_json::json!({"schema_version":1,"operation":Uuid::new_v4(),"active":true});
        write_json_raw(&root, LEGACY, &intent).unwrap_or_else(|_| panic!("old intent"));
        install::recover(&paths, id, &Cancellation::default())
            .await
            .unwrap_or_else(|error| panic!("recover: {error}"));
        assert!(!root.join(FILE).exists());
        assert!(!root.join(LEGACY).exists());
        assert_eq!(
            crate::registry::InstanceRegistry::new(paths.clone())
                .get(id)
                .unwrap_or_default()
                .map(|record| record.state),
            Some(crate::registry::InstallationState::NeedsRepair)
        );
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn migration_duplicate_is_recoverable_but_conflicting_copies_are_preserved() {
        let (paths, accounts, id) = crate::minecraft::play::tests::fixture();
        drop(accounts);
        let root = install::metadata_root(&paths, id).unwrap_or_else(|_| panic!("root"));
        let intent =
            serde_json::json!({"schema_version":1,"operation":Uuid::new_v4(),"active":true});
        write_json_raw(&root, LEGACY, &intent).unwrap_or_else(|_| panic!("old intent"));
        write_json_raw(&root, FILE, &stored(intent.clone()).unwrap_or_default())
            .unwrap_or_else(|_| panic!("copied intent"));
        assert_eq!(read(&root).unwrap_or_default(), Some(intent.clone()));
        let mut next = intent.clone();
        next["active"] = false.into();
        write(&root, next.clone()).unwrap_or_else(|error| panic!("finish migration: {error}"));
        assert!(!root.join(LEGACY).exists());
        assert_eq!(read(&root).unwrap_or_default(), Some(next));
        write_json_raw(&root, LEGACY, &intent).unwrap_or_else(|_| panic!("conflict"));
        let before = std::fs::read(root.join(FILE)).unwrap_or_default();
        assert!(read(&root).is_err());
        assert!(write(&root, intent).is_err());
        assert_eq!(std::fs::read(root.join(FILE)).unwrap_or_default(), before);
        assert!(root.join(LEGACY).exists());
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }
}
