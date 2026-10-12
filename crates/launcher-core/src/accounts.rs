//! Sensitive persistence stays separate from the metadata returned to the UI.
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::{CoreError, ErrorCode};
use crate::storage::{PrivateStore, StoreTransaction};

pub const OAUTH_CLIENT_ID: &str = "cd7a6116-ad43-4eff-aa9b-a177e1101c45";
const ACCOUNT_FILE: &str = "accounts.json";
const REMOVAL_FILE: &str = "pending-account-removals.json";

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
    store: PrivateStore,
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
                    || account.metadata.oauth_client_id.as_deref() != Some(OAUTH_CLIENT_ID)
                {
                    return Err(CoreError::new(ErrorCode::AccountMismatch));
                }
                account.metadata.username = username.to_owned();
                account.refresh_token = Some(token.expose().to_owned());
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
            account.refresh_token = Some(token.expose().to_owned());
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
                // A retained marker is harmless; startup reconciliation is idempotent.
                let _ = write_json(tx, REMOVAL_FILE, &pending);
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
    let parsed = serde_json::from_slice::<AccountDocument>(&bytes);
    bytes.zeroize();
    let document = parsed.map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
    validate_document(&document)?;
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
    let pending: PendingRemovals = match tx.read(REMOVAL_FILE)? {
        Some(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| CoreError::new(ErrorCode::InvalidData))?
        }
        None => PendingRemovals {
            schema_version: 1,
            account_ids: Vec::new(),
        },
    };
    if pending.schema_version != 1
        || pending.account_ids.len() > 128
        || pending.account_ids.iter().any(Uuid::is_nil)
    {
        return Err(CoreError::new(ErrorCode::InvalidData));
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
    write_json(tx, ACCOUNT_FILE, document)
}

fn write_json(
    tx: &StoreTransaction<'_>,
    name: &'static str,
    value: &impl Serialize,
) -> Result<(), CoreError> {
    let mut bytes =
        serde_json::to_vec(value).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
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
        assert!(repository.add_offline("PlayerOne").is_err());
        assert_eq!(
            repository.store.read(ACCOUNT_FILE).ok().flatten(),
            Some(bytes.to_vec())
        );
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
}
