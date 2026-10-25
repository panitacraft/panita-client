//! Sensitive persistence stays separate from the metadata returned to the UI.
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::{CoreError, ErrorCode};
use crate::storage::{PrivateStore, StoreTransaction};

pub const OAUTH_CLIENT_ID: &str = "cd7a6116-ad43-4eff-aa9b-a177e1101c45";
const ACCOUNT_FILE: &str = "accounts.json";
const REMOVAL_FILE: &str = "account-removals.json";
const LEGACY_REMOVAL_FILE: &str = "pending-account-removals.json";

/// Intentionally has no Debug or Serialize implementation.
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Result<Self, CoreError> {
        if value.is_empty() || value.len() > 32_768 || value.contains('\0') {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        Ok(Self(value))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Microsoft,
    Offline,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountInfo {
    pub account_id: Uuid,
    pub kind: AccountKind,
    pub username: String,
    pub profile_id: Uuid,
    pub oauth_client_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAccount {
    metadata: AccountInfo,
    refresh_token: Option<String>,
}

impl StoredAccount {
    fn replace_token(&mut self, token: &Secret) {
        if let Some(mut previous) = self.refresh_token.take() {
            previous.zeroize();
        }
        self.refresh_token = Some(token.expose().to_owned());
    }
}

impl Drop for StoredAccount {
    fn drop(&mut self) {
        if let Some(token) = &mut self.refresh_token {
            token.zeroize();
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountDocument {
    schema_version: u32,
    selected_account_id: Option<Uuid>,
    accounts: Vec<StoredAccount>,
}

// Keep the disk schema independent from UI metadata and in-memory selection.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountFile {
    #[serde(rename = "schema", alias = "schema_version")]
    schema_version: u32,
    accounts: Vec<AccountEntry>,
}

#[derive(Serialize, Deserialize)]
enum AccountType {
    #[serde(rename = "MSA")]
    Microsoft,
    #[serde(rename = "offline")]
    Offline,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountEntry {
    account_id: Uuid,
    #[serde(rename = "type")]
    account_type: AccountType,
    selected: bool,
    username: String,
    profile_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    oauth_client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

impl Drop for AccountEntry {
    fn drop(&mut self) {
        if let Some(token) = &mut self.refresh_token {
            token.zeroize();
        }
    }
}

impl AccountFile {
    fn into_document(self) -> Result<AccountDocument, CoreError> {
        if self.schema_version != 2 || self.accounts.iter().filter(|a| a.selected).count() > 1 {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let mut document = AccountDocument::default();
        for mut entry in self.accounts {
            if entry.selected {
                document.selected_account_id = Some(entry.account_id);
            }
            document.accounts.push(StoredAccount {
                metadata: AccountInfo {
                    account_id: entry.account_id,
                    kind: match entry.account_type {
                        AccountType::Microsoft => AccountKind::Microsoft,
                        AccountType::Offline => AccountKind::Offline,
                    },
                    username: std::mem::take(&mut entry.username),
                    profile_id: entry.profile_id,
                    oauth_client_id: entry.oauth_client_id.take(),
                },
                refresh_token: entry.refresh_token.take(),
            });
        }
        Ok(document)
    }
}

impl Default for AccountDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            selected_account_id: None,
            accounts: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PendingRemovals {
    #[serde(rename = "schema", alias = "schema_version")]
    schema_version: u32,
    account_ids: Vec<Uuid>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccountSnapshot {
    pub accounts: Vec<AccountInfo>,
    pub selected_account_id: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalOutcome {
    Removed,
    Pending,
    NotDurable,
}

pub struct AccountRepository {
    pub(crate) store: PrivateStore,
}

impl AccountRepository {
    pub fn new(store: PrivateStore) -> Self {
        Self { store }
    }

    /// Reconcile durable tombstones before exposing any saved session.
    pub fn snapshot(&self) -> Result<AccountSnapshot, CoreError> {
        self.store.with_lock(|tx| {
            let document = load_reconciled(tx)?;
            Ok(AccountSnapshot {
                accounts: document
                    .accounts
                    .iter()
                    .map(|a| a.metadata.clone())
                    .collect(),
                selected_account_id: document.selected_account_id,
            })
        })
    }

    pub fn add_offline(&self, username: &str) -> Result<AccountInfo, CoreError> {
        validate_name(username)?;
        let metadata = AccountInfo {
            account_id: Uuid::new_v4(),
            kind: AccountKind::Offline,
            username: username.to_owned(),
            profile_id: offline_profile_id(username),
            oauth_client_id: None,
        };
        self.upsert(metadata, None)
    }

    pub fn save_microsoft(
        &self,
        profile_id: Uuid,
        username: &str,
        token: &Secret,
        intended_account: Option<Uuid>,
    ) -> Result<AccountInfo, CoreError> {
        validate_name(username)?;
        self.store.with_lock(|tx| {
            let mut document = load_reconciled(tx)?;
            let existing = document.accounts.iter_mut().find(|a| {
                intended_account.map_or(
                    a.metadata.kind == AccountKind::Microsoft
                        && a.metadata.profile_id == profile_id,
                    |id| a.metadata.account_id == id,
                )
            });
            let metadata = if let Some(account) = existing {
                if account.metadata.kind != AccountKind::Microsoft
                    || account.metadata.profile_id != profile_id
                {
                    return Err(CoreError::new(ErrorCode::AccountMismatch));
                }
                account.metadata.username = username.to_owned();
                account.metadata.oauth_client_id = Some(OAUTH_CLIENT_ID.to_owned());
                account.replace_token(token);
                account.metadata.clone()
            } else {
                if intended_account.is_some() {
                    return Err(CoreError::new(ErrorCode::OperationCancelled));
                }
                let metadata = AccountInfo {
                    account_id: Uuid::new_v4(),
                    kind: AccountKind::Microsoft,
                    username: username.to_owned(),
                    profile_id,
                    oauth_client_id: Some(OAUTH_CLIENT_ID.to_owned()),
                };
                document.accounts.push(StoredAccount {
                    metadata: metadata.clone(),
                    refresh_token: Some(token.expose().to_owned()),
                });
                metadata
            };
            document.selected_account_id = Some(metadata.account_id);
            save_document(tx, &document)?;
            Ok(metadata)
        })
    }

    fn upsert(
        &self,
        metadata: AccountInfo,
        refresh_token: Option<String>,
    ) -> Result<AccountInfo, CoreError> {
        self.store.with_lock(|tx| {
            let mut document = load_reconciled(tx)?;
            if let Some(existing) = document.accounts.iter().find(|a| {
                a.metadata.kind == metadata.kind && a.metadata.profile_id == metadata.profile_id
            }) {
                return Ok(existing.metadata.clone());
            }
            document.selected_account_id = Some(metadata.account_id);
            document.accounts.push(StoredAccount {
                metadata: metadata.clone(),
                refresh_token,
            });
            save_document(tx, &document)?;
            Ok(metadata)
        })
    }

    pub fn select(&self, id: Uuid) -> Result<(), CoreError> {
        self.store.with_lock(|tx| {
            let mut document = load_reconciled(tx)?;
            if !document
                .accounts
                .iter()
                .any(|a| a.metadata.account_id == id)
            {
                return Err(CoreError::new(ErrorCode::AccountNotFound));
            }
            document.selected_account_id = Some(id);
            save_document(tx, &document)
        })
    }

    /// Retry a first-login save while preserving its already assigned memory ID.
    pub(crate) fn persist_unsaved(
        &self,
        metadata: &AccountInfo,
        token: &Secret,
    ) -> Result<AccountInfo, CoreError> {
        self.store.with_lock(|tx| {
            if load_pending(tx)?.account_ids.contains(&metadata.account_id) {
                return Err(CoreError::new(ErrorCode::OperationCancelled));
            }
            let mut document = load_reconciled(tx)?;
            if document.accounts.iter().any(|a| {
                a.metadata.account_id == metadata.account_id
                    || (a.metadata.kind == metadata.kind
                        && a.metadata.profile_id == metadata.profile_id)
            }) {
                return Err(CoreError::new(ErrorCode::AccountMismatch));
            }
            document.accounts.push(StoredAccount {
                metadata: metadata.clone(),
                refresh_token: Some(token.expose().to_owned()),
            });
            document.selected_account_id = Some(metadata.account_id);
            save_document(tx, &document)?;
            Ok(metadata.clone())
        })
    }

    pub fn refresh_token(&self, id: Uuid) -> Result<Secret, CoreError> {
        self.store.with_lock(|tx| {
            let document = load_reconciled(tx)?;
            let account = document
                .accounts
                .iter()
                .find(|a| a.metadata.account_id == id)
                .ok_or_else(|| CoreError::new(ErrorCode::AccountNotFound))?;
            if account.metadata.oauth_client_id.as_deref() != Some(OAUTH_CLIENT_ID) {
                return Err(CoreError::new(ErrorCode::AuthenticationRequired));
            }
            Secret::new(
                account
                    .refresh_token
                    .as_ref()
                    .ok_or_else(|| CoreError::new(ErrorCode::AuthenticationRequired))?
                    .clone(),
            )
        })
    }

    /// Persist rotation immediately after Microsoft responds, before Xbox calls.
    pub fn replace_refresh_token(&self, id: Uuid, token: &Secret) -> Result<(), CoreError> {
        self.store.with_lock(|tx| {
            let mut document = load_reconciled(tx)?;
            let account = document
                .accounts
                .iter_mut()
                .find(|a| a.metadata.account_id == id)
                .ok_or_else(|| CoreError::new(ErrorCode::OperationCancelled))?;
            if account.metadata.kind != AccountKind::Microsoft
                || account.metadata.oauth_client_id.as_deref() != Some(OAUTH_CLIENT_ID)
            {
                return Err(CoreError::new(ErrorCode::AuthenticationRequired));
            }
            account.replace_token(token);
            save_document(tx, &document)
        })
    }

    /// Keep the saved name in step with the Minecraft profile; identity never changes.
    pub fn update_username(&self, id: Uuid, name: &str) -> Result<(), CoreError> {
        validate_name(name)?;
        self.store.with_lock(|tx| {
            let mut document = load_reconciled(tx)?;
            let account = document
                .accounts
                .iter_mut()
                .find(|a| a.metadata.account_id == id)
                .ok_or_else(|| CoreError::new(ErrorCode::AccountNotFound))?;
            if account.metadata.kind != AccountKind::Microsoft || account.metadata.username == name
            {
                return Ok(());
            }
            account.metadata.username = name.to_owned();
            save_document(tx, &document)
        })
    }

    /// The caller must invalidate in-memory operations before this method.
    pub fn remove(&self, id: Uuid) -> RemovalOutcome {
        self.store
            .with_lock(|tx| {
                let mut pending = load_pending(tx)?;
                if !pending.account_ids.contains(&id) {
                    pending.account_ids.push(id);
                }
                let marker_saved = write_json(tx, REMOVAL_FILE, &pending).is_ok();
                let mut document = match load_document(tx) {
                    Ok(document) => document,
                    Err(_) => {
                        return Ok(if marker_saved {
                            RemovalOutcome::Pending
                        } else {
                            RemovalOutcome::NotDurable
                        });
                    }
                };
                remove_ids(&mut document, &pending.account_ids);
                if save_document(tx, &document).is_err() {
                    return Ok(if marker_saved {
                        RemovalOutcome::Pending
                    } else {
                        RemovalOutcome::NotDurable
                    });
                }
                pending.account_ids.clear();
                // Publish an inactive marker before deletion, so an interrupted
                // or non-durable unlink can only leave harmless completed work.
                if write_json(tx, REMOVAL_FILE, &pending).is_ok() {
                    let _ = retire_empty_marker(tx, REMOVAL_FILE);
                }
                Ok(RemovalOutcome::Removed)
            })
            .unwrap_or(RemovalOutcome::NotDurable)
    }
}

pub fn offline_profile_id(username: &str) -> Uuid {
    let mut bytes = md5::compute(format!("OfflinePlayer:{username}").as_bytes()).0;
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn validate_name(name: &str) -> Result<(), CoreError> {
    if !(3..=16).contains(&name.len())
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(CoreError::new(ErrorCode::InvalidUsername));
    }
    Ok(())
}

fn load_document(tx: &StoreTransaction<'_>) -> Result<AccountDocument, CoreError> {
    let Some(mut bytes) = tx.read(ACCOUNT_FILE)? else {
        return Ok(AccountDocument::default());
    };
    #[derive(Deserialize)]
    struct Version {
        schema: Option<u32>,
        schema_version: Option<u32>,
    }
    let invalid = || CoreError::new(ErrorCode::AccountsFileInvalid);
    let version = serde_json::from_slice::<Version>(&bytes)
        .map_err(|_| invalid())
        .and_then(|header| match (header.schema, header.schema_version) {
            (Some(version), None) => Ok((version, false)),
            (None, Some(version)) => Ok((version, true)),
            _ => Err(invalid()),
        });
    let migrate = matches!(version, Ok((1, _)) | Ok((2, true)));
    let parsed = match version {
        Ok((1, _)) => serde_json::from_slice::<AccountDocument>(&bytes).map_err(|_| invalid()),
        Ok((2, _)) => serde_json::from_slice::<AccountFile>(&bytes)
            .map_err(|_| invalid())
            .and_then(|file| file.into_document().map_err(|_| invalid())),
        _ => Err(invalid()),
    };
    bytes.zeroize();
    let document = parsed?;
    validate_document(&document).map_err(|_| invalid())?;
    if migrate {
        save_document(tx, &document)?;
    }
    Ok(document)
}

fn validate_document(document: &AccountDocument) -> Result<(), CoreError> {
    let mut ids = HashSet::new();
    let mut profiles = HashSet::new();
    let invalid = document.schema_version != 1
        || document.accounts.len() > 128
        || document.accounts.iter().any(|account| {
            let m = &account.metadata;
            m.account_id.is_nil()
                || m.profile_id.is_nil()
                || validate_name(&m.username).is_err()
                || !ids.insert(m.account_id)
                || !profiles.insert((m.kind as u8, m.profile_id))
                || match m.kind {
                    AccountKind::Offline => {
                        m.oauth_client_id.is_some()
                            || account.refresh_token.is_some()
                            || m.profile_id != offline_profile_id(&m.username)
                    }
                    AccountKind::Microsoft => {
                        m.oauth_client_id
                            .as_ref()
                            .is_none_or(|id| Uuid::parse_str(id).is_err())
                            || account.refresh_token.as_ref().is_none_or(|token| {
                                token.is_empty() || token.len() > 32_768 || token.contains('\0')
                            })
                    }
                }
        })
        || document
            .selected_account_id
            .is_some_and(|id| !ids.contains(&id));
    if invalid {
        return Err(CoreError::new(ErrorCode::InvalidData));
    }
    Ok(())
}

fn load_pending(tx: &StoreTransaction<'_>) -> Result<PendingRemovals, CoreError> {
    let mut pending = read_pending(tx, REMOVAL_FILE)?;
    if tx.read(LEGACY_REMOVAL_FILE)?.is_some() {
        let legacy = read_pending(tx, LEGACY_REMOVAL_FILE)?;
        for id in legacy.account_ids.iter().copied() {
            if !pending.account_ids.contains(&id) {
                pending.account_ids.push(id);
            }
        }
        if pending.account_ids.len() > 128 {
            return Err(CoreError::new(ErrorCode::AccountsFileInvalid));
        }
        if !legacy.account_ids.is_empty() {
            // Persist every tombstone before retiring the legacy marker. Both writes
            // share the mutation lock, and interruption leaves a safe duplicate.
            write_json(tx, REMOVAL_FILE, &pending)?;
            write_json(
                tx,
                LEGACY_REMOVAL_FILE,
                &PendingRemovals {
                    schema_version: 1,
                    account_ids: Vec::new(),
                },
            )?;
        }
        // The legacy intent is now empty or durably represented in the new file.
        // A failed unlink leaves an inactive marker for the next reconciliation.
        let _ = retire_empty_marker(tx, LEGACY_REMOVAL_FILE);
    }
    if pending.account_ids.is_empty() {
        let _ = retire_empty_marker(tx, REMOVAL_FILE);
    }
    Ok(pending)
}

fn retire_empty_marker(tx: &StoreTransaction<'_>, name: &'static str) -> Result<(), CoreError> {
    let pending = read_pending(tx, name)?;
    if !pending.account_ids.is_empty() {
        return Err(CoreError::new(ErrorCode::AccountsFileInvalid));
    }
    tx.remove_verified_file(name)
}

fn read_pending(
    tx: &StoreTransaction<'_>,
    name: &'static str,
) -> Result<PendingRemovals, CoreError> {
    let pending: PendingRemovals = match tx.read(name)? {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| CoreError::new(ErrorCode::AccountsFileInvalid))?,
        None => PendingRemovals {
            schema_version: 1,
            account_ids: Vec::new(),
        },
    };
    if pending.schema_version != 1
        || pending.account_ids.len() > 128
        || pending.account_ids.iter().any(Uuid::is_nil)
    {
        return Err(CoreError::new(ErrorCode::AccountsFileInvalid));
    }
    Ok(pending)
}

fn load_reconciled(tx: &StoreTransaction<'_>) -> Result<AccountDocument, CoreError> {
    let pending = load_pending(tx)?;
    let mut document = load_document(tx)?;
    if !pending.account_ids.is_empty() {
        remove_ids(&mut document, &pending.account_ids);
        save_document(tx, &document)?;
        write_json(
            tx,
            REMOVAL_FILE,
            &PendingRemovals {
                schema_version: 1,
                account_ids: Vec::new(),
            },
        )?;
        let _ = retire_empty_marker(tx, REMOVAL_FILE);
    }
    Ok(document)
}

fn remove_ids(document: &mut AccountDocument, ids: &[Uuid]) {
    document
        .accounts
        .retain(|account| !ids.contains(&account.metadata.account_id));
    if document
        .selected_account_id
        .is_some_and(|id| ids.contains(&id))
    {
        document.selected_account_id = None;
    }
}

fn save_document(tx: &StoreTransaction<'_>, document: &AccountDocument) -> Result<(), CoreError> {
    validate_document(document)?;
    let file = AccountFile {
        schema_version: 2,
        accounts: document
            .accounts
            .iter()
            .map(|account| AccountEntry {
                account_id: account.metadata.account_id,
                account_type: match account.metadata.kind {
                    AccountKind::Microsoft => AccountType::Microsoft,
                    AccountKind::Offline => AccountType::Offline,
                },
                selected: document.selected_account_id == Some(account.metadata.account_id),
                username: account.metadata.username.clone(),
                profile_id: account.metadata.profile_id,
                oauth_client_id: account.metadata.oauth_client_id.clone(),
                refresh_token: account.refresh_token.clone(),
            })
            .collect(),
    };
    write_json(tx, ACCOUNT_FILE, &file)
}

fn write_json(
    tx: &StoreTransaction<'_>,
    name: &'static str,
    value: &impl Serialize,
) -> Result<(), CoreError> {
    let mut bytes =
        crate::storage_json::pretty(value).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
    let result = tx.write(name, &bytes);
    bytes.zeroize();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;

    fn repository() -> (AppPaths, AccountRepository) {
        let paths =
            AppPaths::new(std::env::temp_dir().join(format!("panita-accounts-{}", Uuid::new_v4())))
                .unwrap_or_else(|_| panic!("path"));
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store"));
        (paths, AccountRepository::new(store))
    }

    #[test]
    fn java_offline_uuid_vectors() {
        assert_eq!(
            offline_profile_id("Notch").to_string(),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
        assert_eq!(
            offline_profile_id("Steve").to_string(),
            "5627dd98-e6be-3c21-b8a8-e92344183641"
        );
        assert_ne!(offline_profile_id("Steve"), offline_profile_id("steve"));
    }

    #[test]
    fn legacy_v2_header_migrates_without_losing_accounts_and_mixed_headers_are_preserved() {
        let (paths, repository) = repository();
        let account = repository
            .add_offline("Steve")
            .unwrap_or_else(|_| panic!("fixture"));
        let bytes = repository
            .store
            .read(ACCOUNT_FILE)
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        legacy
            .as_object_mut()
            .unwrap_or_else(|| panic!("object"))
            .remove("schema");
        legacy["schema_version"] = 2.into();
        let bytes = serde_json::to_vec(&legacy).unwrap_or_default();
        repository
            .store
            .transaction(ACCOUNT_FILE, |_| Ok((bytes, ())))
            .unwrap_or_else(|_| panic!("legacy fixture"));
        let snapshot = repository.snapshot().unwrap_or_else(|_| panic!("migrate"));
        assert_eq!(snapshot.accounts, vec![account]);
        let migrated = repository
            .store
            .read(ACCOUNT_FILE)
            .ok()
            .flatten()
            .unwrap_or_default();
        assert!(migrated.starts_with(b"{\n  \"schema\": 2,"));
        let mut mixed: serde_json::Value = serde_json::from_slice(&migrated).unwrap_or_default();
        mixed["schema_version"] = 2.into();
        let mixed = serde_json::to_vec(&mixed).unwrap_or_default();
        repository
            .store
            .transaction(ACCOUNT_FILE, |_| Ok((mixed.clone(), ())))
            .unwrap_or_else(|_| panic!("mixed fixture"));
        assert!(repository.snapshot().is_err());
        assert_eq!(
            repository.store.read(ACCOUNT_FILE).ok().flatten(),
            Some(mixed)
        );
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn migrate_legacy_accounts_without_losing_identity_credentials_or_selection() {
        let (paths, repository) = repository();
        let token = Secret::new("synthetic-migration-token".to_owned())
            .unwrap_or_else(|_| panic!("fixture operation failed"));
        let microsoft = repository
            .save_microsoft(Uuid::new_v4(), "PlayerOne", &token, None)
            .unwrap_or_else(|_| panic!("fixture operation failed"));
        let offline = repository
            .add_offline("Steve")
            .unwrap_or_else(|_| panic!("fixture operation failed"));
        repository
            .select(microsoft.account_id)
            .unwrap_or_else(|_| panic!("fixture operation failed"));
        repository
            .store
            .with_lock(|tx| {
                let legacy = load_document(tx)?;
                write_json(tx, ACCOUNT_FILE, &legacy)
            })
            .unwrap_or_else(|_| panic!("fixture operation failed"));
        let snapshot = repository
            .snapshot()
            .unwrap_or_else(|_| panic!("fixture operation failed"));
        assert_eq!(snapshot.selected_account_id, Some(microsoft.account_id));
        assert_eq!(snapshot.accounts.len(), 2);
        assert_eq!(
            repository
                .refresh_token(microsoft.account_id)
                .unwrap_or_else(|_| panic!("fixture operation failed"))
                .expose(),
            token.expose()
        );
        let bytes = repository
            .store
            .read(ACCOUNT_FILE)
            .unwrap_or_else(|_| panic!("fixture operation failed"))
            .unwrap_or_else(|| panic!("missing fixture file"));
        let file: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("fixture operation failed"));
        assert_eq!(file["schema"], 2);
        assert!(file.get("schema_version").is_none());
        assert!(bytes.starts_with(b"{\n  \"schema\": 2,"));
        assert!(file.get("selected_account_id").is_none());
        assert_eq!(file["accounts"][0]["type"], "MSA");
        assert_eq!(file["accounts"][0]["selected"], true);
        assert!(file["accounts"][0].get("metadata").is_none());
        assert!(file["accounts"][0].get("kind").is_none());
        assert_eq!(file["accounts"][1]["type"], "offline");
        assert_eq!(file["accounts"][1]["selected"], false);
        assert!(file["accounts"][1].get("refresh_token").is_none());
        assert!(file["accounts"][1].get("oauth_client_id").is_none());
        assert!(bytes.ends_with(b"\n"));
        repository
            .select(offline.account_id)
            .unwrap_or_else(|_| panic!("fixture operation failed"));
        assert_eq!(
            repository
                .snapshot()
                .unwrap_or_else(|_| panic!("fixture operation failed"))
                .selected_account_id,
            Some(offline.account_id)
        );
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn reject_multiple_selected_accounts_and_unknown_fields_without_overwriting() {
        let (paths, repository) = repository();
        repository
            .add_offline("Steve")
            .unwrap_or_else(|_| panic!("fixture operation failed"));
        repository
            .add_offline("Notch")
            .unwrap_or_else(|_| panic!("fixture operation failed"));
        let original = repository
            .store
            .read(ACCOUNT_FILE)
            .unwrap_or_else(|_| panic!("fixture operation failed"))
            .unwrap_or_else(|| panic!("missing fixture file"));
        for invalid in 0..3 {
            let mut file: serde_json::Value = serde_json::from_slice(&original)
                .unwrap_or_else(|_| panic!("fixture operation failed"));
            match invalid {
                0 => file["accounts"][0]["selected"] = true.into(),
                1 => file["accounts"][0]["access_token"] = "forbidden".into(),
                _ => file["schema"] = 3.into(),
            }
            let bytes =
                serde_json::to_vec(&file).unwrap_or_else(|_| panic!("fixture operation failed"));
            repository
                .store
                .transaction(ACCOUNT_FILE, |_| Ok((bytes.clone(), ())))
                .unwrap_or_else(|_| panic!("fixture operation failed"));
            assert!(matches!(
                repository.snapshot(),
                Err(error) if error.code() == ErrorCode::AccountsFileInvalid
            ));
            assert_eq!(
                repository
                    .store
                    .read(ACCOUNT_FILE)
                    .unwrap_or_else(|_| panic!("fixture operation failed"))
                    .unwrap_or_else(|| panic!("missing fixture file")),
                bytes
            );
        }
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn restart_preserves_identity_and_metadata_never_contains_tokens() {
        let (paths, repository) = repository();
        let token = Secret::new("synthetic-refresh-fixture".to_owned())
            .unwrap_or_else(|_| panic!("fixture"));
        let id = Uuid::new_v4();
        let first = repository
            .save_microsoft(id, "PlayerOne", &token, None)
            .unwrap_or_else(|_| panic!("save"));
        let renamed = repository
            .save_microsoft(id, "PlayerTwo", &token, Some(first.account_id))
            .unwrap_or_else(|_| panic!("rename"));
        assert_eq!(first.account_id, renamed.account_id);
        let mismatch =
            repository.save_microsoft(Uuid::new_v4(), "OtherUser", &token, Some(first.account_id));
        assert!(matches!(mismatch, Err(error) if error.code() == ErrorCode::AccountMismatch));
        let snapshot = repository.snapshot().unwrap_or_else(|_| panic!("snapshot"));
        let public = serde_json::to_string(&snapshot).unwrap_or_else(|_| panic!("metadata"));
        assert!(!public.contains("synthetic-refresh-fixture"));
        assert!(!public.contains("refresh_token"));
        drop(repository);
        let restarted =
            AccountRepository::new(PrivateStore::open(&paths).unwrap_or_else(|_| panic!("reopen")));
        assert_eq!(
            restarted
                .snapshot()
                .unwrap_or_else(|_| panic!("snapshot"))
                .selected_account_id,
            Some(first.account_id)
        );
        assert_eq!(
            restarted
                .refresh_token(first.account_id)
                .unwrap_or_else(|_| panic!("token"))
                .expose(),
            token.expose()
        );
        assert!(matches!(
            restarted.remove(first.account_id),
            RemovalOutcome::Removed
        ));
        assert!(restarted.refresh_token(first.account_id).is_err());
        drop(restarted);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn corruption_and_unknown_token_fields_are_preserved() {
        let (paths, repository) = repository();
        let bytes = br#"{"schema_version":1,"selected_account_id":null,"accounts":[],"access_token":"forbidden"}"#;
        assert!(
            repository
                .store
                .transaction(ACCOUNT_FILE, |_| Ok((bytes.to_vec(), ())))
                .is_ok()
        );
        assert!(matches!(
            repository.add_offline("PlayerOne"),
            Err(error) if error.code() == ErrorCode::AccountsFileInvalid
        ));
        let malformed = b"{ not json";
        assert!(
            repository
                .store
                .transaction(ACCOUNT_FILE, |_| Ok((malformed.to_vec(), ())))
                .is_ok()
        );
        assert!(matches!(
            repository.snapshot(),
            Err(error) if error.code() == ErrorCode::AccountsFileInvalid
        ));
        assert!(
            repository
                .store
                .transaction(ACCOUNT_FILE, |_| Ok((bytes.to_vec(), ())))
                .is_ok()
        );
        assert_eq!(
            repository.store.read(ACCOUNT_FILE).ok().flatten(),
            Some(bytes.to_vec())
        );
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn completed_removal_deletes_markers_and_preserves_other_accounts_after_restart() {
        let (paths, repository) = repository();
        let token =
            Secret::new("synthetic-removal-token".into()).unwrap_or_else(|_| panic!("token"));
        let removed = repository
            .save_microsoft(Uuid::new_v4(), "PlayerOne", &token, None)
            .unwrap_or_else(|_| panic!("Microsoft fixture"));
        let retained = repository
            .add_offline("PlayerTwo")
            .unwrap_or_else(|_| panic!("offline fixture"));
        assert!(matches!(
            repository.remove(removed.account_id),
            RemovalOutcome::Removed
        ));
        assert!(!paths.root().join(REMOVAL_FILE).exists());
        assert!(!paths.root().join(LEGACY_REMOVAL_FILE).exists());
        assert!(repository.refresh_token(removed.account_id).is_err());
        drop(repository);
        let restarted = AccountRepository::new(
            PrivateStore::open(&paths).unwrap_or_else(|_| panic!("restart")),
        );
        assert_eq!(
            restarted.snapshot().ok().map(|snapshot| snapshot.accounts),
            Some(vec![retained])
        );
        assert!(!paths.root().join(REMOVAL_FILE).exists());
        drop(restarted);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn empty_current_and_legacy_markers_are_retired_without_changing_accounts() {
        let (paths, repository) = repository();
        let retained = repository
            .add_offline("Steve")
            .unwrap_or_else(|_| panic!("fixture"));
        repository
            .store
            .with_lock(|tx| {
                write_json(
                    tx,
                    REMOVAL_FILE,
                    &PendingRemovals {
                        schema_version: 1,
                        account_ids: vec![],
                    },
                )?;
                tx.write(
                    LEGACY_REMOVAL_FILE,
                    br#"{"schema_version":1,"account_ids":[]}"#,
                )
            })
            .unwrap_or_else(|_| panic!("markers"));
        assert_eq!(
            repository.snapshot().ok().map(|snapshot| snapshot.accounts),
            Some(vec![retained])
        );
        assert!(!paths.root().join(REMOVAL_FILE).exists());
        assert!(!paths.root().join(LEGACY_REMOVAL_FILE).exists());
        assert!(repository.snapshot().is_ok());
        assert!(!paths.root().join(REMOVAL_FILE).exists());
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn active_removal_survives_failed_reconciliation_and_is_retired_after_recovery() {
        let (paths, repository) = repository();
        let account = repository
            .add_offline("Steve")
            .unwrap_or_else(|_| panic!("fixture"));
        let original = repository
            .store
            .read(ACCOUNT_FILE)
            .ok()
            .flatten()
            .unwrap_or_default();
        repository
            .store
            .with_lock(|tx| {
                write_json(
                    tx,
                    REMOVAL_FILE,
                    &PendingRemovals {
                        schema_version: 1,
                        account_ids: vec![account.account_id],
                    },
                )?;
                tx.write(ACCOUNT_FILE, b"corrupt synthetic account fixture")
            })
            .unwrap_or_else(|_| panic!("pending fixture"));
        assert!(repository.snapshot().is_err());
        assert!(
            repository
                .store
                .with_lock(|tx| retire_empty_marker(tx, REMOVAL_FILE))
                .is_err()
        );
        assert_eq!(
            repository
                .store
                .with_lock(load_pending)
                .ok()
                .map(|pending| pending.account_ids),
            Some(vec![account.account_id])
        );
        repository
            .store
            .with_lock(|tx| tx.write(ACCOUNT_FILE, &original))
            .unwrap_or_else(|_| panic!("restore fixture"));
        assert!(
            repository
                .snapshot()
                .is_ok_and(|snapshot| snapshot.accounts.is_empty())
        );
        assert!(!paths.root().join(REMOVAL_FILE).exists());
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn locked_empty_marker_is_retried_without_blocking_accounts() {
        use std::os::windows::fs::OpenOptionsExt;
        let (paths, repository) = repository();
        let account = repository
            .add_offline("Steve")
            .unwrap_or_else(|_| panic!("fixture"));
        repository
            .store
            .with_lock(|tx| {
                write_json(
                    tx,
                    REMOVAL_FILE,
                    &PendingRemovals {
                        schema_version: 1,
                        account_ids: vec![],
                    },
                )
            })
            .unwrap_or_else(|_| panic!("marker"));
        // Permit reads/writes but deny deletion until this external handle closes.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(paths.root().join(REMOVAL_FILE))
            .unwrap_or_else(|_| panic!("hold"));
        assert!(
            repository
                .store
                .with_lock(|tx| retire_empty_marker(tx, REMOVAL_FILE))
                .is_err()
        );
        assert_eq!(
            repository.snapshot().ok().map(|snapshot| snapshot.accounts),
            Some(vec![account])
        );
        assert!(paths.root().join(REMOVAL_FILE).exists());
        drop(held);
        assert!(repository.snapshot().is_ok());
        assert!(!paths.root().join(REMOVAL_FILE).exists());
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn unknown_or_linked_markers_are_never_deleted_as_completed_work() {
        let (paths, repository) = repository();
        assert!(repository.add_offline("Steve").is_ok());
        for bytes in [
            b"corrupt".as_slice(),
            br#"{"schema":99,"account_ids":[]}"#,
            br#"{"schema":1,"account_ids":[],"unknown":true}"#,
        ] {
            repository
                .store
                .with_lock(|tx| tx.write(REMOVAL_FILE, bytes))
                .unwrap_or_else(|_| panic!("invalid fixture"));
            assert!(repository.snapshot().is_err());
            assert!(
                repository
                    .store
                    .with_lock(|tx| retire_empty_marker(tx, REMOVAL_FILE))
                    .is_err()
            );
            assert_eq!(
                repository.store.read(REMOVAL_FILE).ok().flatten(),
                Some(bytes.to_vec())
            );
        }
        repository
            .store
            .with_lock(|tx| {
                write_json(
                    tx,
                    REMOVAL_FILE,
                    &PendingRemovals {
                        schema_version: 1,
                        account_ids: vec![],
                    },
                )
            })
            .unwrap_or_else(|_| panic!("linked fixture"));
        let linked = paths.root().join("unrelated.json");
        assert!(std::fs::hard_link(paths.root().join(REMOVAL_FILE), &linked).is_ok());
        assert!(repository.snapshot().is_err());
        assert!(
            repository
                .store
                .with_lock(|tx| retire_empty_marker(tx, REMOVAL_FILE))
                .is_err()
        );
        assert!(linked.exists() && paths.root().join(REMOVAL_FILE).exists());
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn pending_removal_blocks_restore_and_late_rotation() {
        let (paths, repository) = repository();
        let token = Secret::new("synthetic-token".to_owned()).unwrap_or_else(|_| panic!("fixture"));
        let account = repository
            .save_microsoft(Uuid::new_v4(), "PlayerOne", &token, None)
            .unwrap_or_else(|_| panic!("save"));
        assert!(
            repository
                .store
                .with_lock(|tx| write_json(
                    tx,
                    REMOVAL_FILE,
                    &PendingRemovals {
                        schema_version: 1,
                        account_ids: vec![account.account_id]
                    }
                ))
                .is_ok()
        );
        assert!(
            repository
                .replace_refresh_token(account.account_id, &token)
                .is_err()
        );
        assert!(
            repository
                .snapshot()
                .unwrap_or_else(|_| panic!("snapshot"))
                .accounts
                .is_empty()
        );
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn legacy_pending_removals_merge_before_reconciliation() {
        let (paths, repository) = repository();
        let first = repository
            .add_offline("PlayerOne")
            .unwrap_or_else(|_| panic!("first"));
        let second = repository
            .add_offline("PlayerTwo")
            .unwrap_or_else(|_| panic!("second"));
        assert!(
            repository
                .store
                .with_lock(|tx| {
                    write_json(
                        tx,
                        REMOVAL_FILE,
                        &PendingRemovals {
                            schema_version: 1,
                            account_ids: vec![first.account_id],
                        },
                    )?;
                    write_json(
                        tx,
                        LEGACY_REMOVAL_FILE,
                        &PendingRemovals {
                            schema_version: 1,
                            account_ids: vec![second.account_id],
                        },
                    )
                })
                .is_ok()
        );
        assert!(
            repository
                .snapshot()
                .unwrap_or_else(|_| panic!("snapshot"))
                .accounts
                .is_empty()
        );
        assert!(
            repository
                .store
                .with_lock(|tx| {
                    assert!(
                        read_pending(tx, LEGACY_REMOVAL_FILE)?
                            .account_ids
                            .is_empty()
                    );
                    assert!(read_pending(tx, REMOVAL_FILE)?.account_ids.is_empty());
                    assert!(tx.read(REMOVAL_FILE)?.is_none());
                    assert!(tx.read(LEGACY_REMOVAL_FILE)?.is_none());
                    Ok(())
                })
                .is_ok()
        );
        assert!(
            repository
                .snapshot()
                .unwrap_or_else(|_| panic!("restart"))
                .accounts
                .is_empty()
        );
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn failed_marker_migration_preserves_legacy_removals() {
        let (paths, repository) = repository();
        let id = Uuid::new_v4();
        assert!(
            repository
                .store
                .with_lock(|tx| write_json(
                    tx,
                    LEGACY_REMOVAL_FILE,
                    &PendingRemovals {
                        schema_version: 1,
                        account_ids: vec![id],
                    }
                ))
                .is_ok()
        );
        assert!(std::fs::create_dir(paths.root().join(REMOVAL_FILE)).is_ok());
        assert!(repository.snapshot().is_err());
        let legacy = repository
            .store
            .with_lock(|tx| read_pending(tx, LEGACY_REMOVAL_FILE))
            .unwrap_or_else(|_| panic!("legacy marker"));
        assert_eq!(legacy.account_ids, vec![id]);
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn malformed_legacy_removal_marker_blocks_restore_and_is_preserved() {
        let (paths, repository) = repository();
        assert!(repository.add_offline("PlayerOne").is_ok());
        assert!(
            repository
                .store
                .transaction(LEGACY_REMOVAL_FILE, |_| Ok((b"corrupt".to_vec(), ())))
                .is_ok()
        );
        assert!(repository.snapshot().is_err());
        assert_eq!(
            repository
                .store
                .read(LEGACY_REMOVAL_FILE)
                .unwrap_or_default(),
            Some(b"corrupt".to_vec())
        );
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn failed_account_cleanup_keeps_a_durable_removal_marker() {
        let (paths, repository) = repository();
        assert!(std::fs::create_dir(paths.accounts()).is_ok());
        let id = Uuid::new_v4();
        assert!(matches!(repository.remove(id), RemovalOutcome::Pending));
        let pending = repository
            .store
            .with_lock(load_pending)
            .unwrap_or_else(|_| panic!("marker"));
        assert_eq!(pending.account_ids, vec![id]);
        assert!(repository.snapshot().is_err());
        assert!(paths.accounts().is_dir());
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn changed_oauth_client_requires_interactive_identity_matched_reauthentication() {
        let (paths, repository) = repository();
        let token = Secret::new("synthetic-token".to_owned()).unwrap_or_else(|_| panic!("fixture"));
        let account = repository
            .save_microsoft(Uuid::new_v4(), "PlayerOne", &token, None)
            .unwrap_or_else(|_| panic!("save"));
        assert!(
            repository
                .store
                .with_lock(|tx| {
                    let mut document = load_document(tx)?;
                    document.accounts[0].metadata.oauth_client_id =
                        Some(Uuid::new_v4().to_string());
                    save_document(tx, &document)
                })
                .is_ok()
        );
        assert!(
            matches!(repository.refresh_token(account.account_id), Err(error) if error.code() == ErrorCode::AuthenticationRequired)
        );
        let confirmed = repository
            .save_microsoft(
                account.profile_id,
                "PlayerOne",
                &token,
                Some(account.account_id),
            )
            .unwrap_or_else(|_| panic!("reauth"));
        assert_eq!(confirmed.account_id, account.account_id);
        assert_eq!(confirmed.oauth_client_id.as_deref(), Some(OAUTH_CLIENT_ID));
        drop(repository);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }
}
