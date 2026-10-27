//! The Panita activation session. It is independent of Minecraft accounts and
//! lives in its own private file so account recovery never touches it.
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::accounts::Secret;
use crate::error::{CoreError, ErrorCode};
use crate::storage::{PrivateStore, StoreTransaction};

const SESSION_FILE: &str = "panita-session.json";
const TOKEN_PREFIX: &str = "ps_";
const MAX_TOKEN_LENGTH: usize = 256;

/// How the member plays, as chosen in Discord. Only a hint for the launcher.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MinecraftStatus {
    Premium,
    Cracked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanitaCapabilities {
    pub authenticated: bool,
    pub admin_or_mod: bool,
    pub trusted_author: bool,
    pub gallery_editor: bool,
    pub upload_authorized: bool,
    pub wiki_editor: bool,
}

/// The nonsecret snapshot of `GET /v1/auth/me` that feeds the account panel.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanitaUser {
    pub id: u64,
    pub ign: Option<String>,
    pub discord_name: String,
    pub avatar_url: Option<String>,
    pub minecraft_status: Option<MinecraftStatus>,
    pub capabilities: PanitaCapabilities,
}

impl PanitaUser {
    fn validate(&self) -> Result<(), CoreError> {
        let text = |value: &str, limit: usize| {
            !value.is_empty()
                && value.chars().count() <= limit
                && !value.chars().any(char::is_control)
        };
        let valid = self.id > 0
            && text(&self.discord_name, 128)
            && self.ign.as_deref().is_none_or(|ign| text(ign, 64))
            && self
                .avatar_url
                .as_deref()
                .is_none_or(|url| url.len() <= 512 && url.starts_with("https://"));
        if valid {
            Ok(())
        } else {
            Err(CoreError::new(ErrorCode::InvalidData))
        }
    }
}

/// An open session. The token is shared, never copied, and zeroized with its last owner.
#[derive(Clone)]
pub struct PanitaSession {
    token: Arc<Secret>,
    pub user: PanitaUser,
    /// Unix seconds of the last successful `GET /v1/auth/me` or login.
    pub verified_at: u64,
}

impl PanitaSession {
    pub fn new(mut token: String, user: PanitaUser, verified_at: u64) -> Result<Self, CoreError> {
        let shaped = token.starts_with(TOKEN_PREFIX)
            && token.len() <= MAX_TOKEN_LENGTH
            && token.bytes().all(|byte| byte.is_ascii_graphic());
        if !shaped || user.validate().is_err() {
            token.zeroize();
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        Ok(Self {
            token: Arc::new(Secret::new(token)?),
            user,
            verified_at,
        })
    }

    pub fn token(&self) -> &Arc<Secret> {
        &self.token
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionFile {
    schema: u32,
    token: String,
    user: PanitaUser,
    verified_at: u64,
}

impl Drop for SessionFile {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

/// Absent is `None`. A damaged or unsupported file is an error and is never replaced.
pub fn load(store: &PrivateStore) -> Result<Option<PanitaSession>, CoreError> {
    store.with_lock(load_document)
}

/// Replace the persisted session, refusing to overwrite a damaged file.
pub fn save(store: &PrivateStore, session: &PanitaSession) -> Result<(), CoreError> {
    store.with_lock(|tx| {
        load_document(tx)?;
        write_document(tx, session)
    })
}

/// Update the snapshot only while the file still holds `token`. A sign-out or a
/// newer activation from another process must never be undone by a late refresh.
pub fn refresh_snapshot(
    store: &PrivateStore,
    token: &Secret,
    user: PanitaUser,
    verified_at: u64,
) -> Result<bool, CoreError> {
    store.with_lock(|tx| {
        let Some(current) = load_document(tx)? else {
            return Ok(false);
        };
        if current.token.expose() != token.expose() {
            return Ok(false);
        }
        let refreshed = PanitaSession {
            token: Arc::clone(&current.token),
            user,
            verified_at,
        };
        refreshed.user.validate()?;
        write_document(tx, &refreshed)?;
        Ok(true)
    })
}

/// Remove the persisted session. With `expected`, remove it only if it still holds
/// that token. Returns whether a file was removed.
pub fn clear(store: &PrivateStore, expected: Option<&Secret>) -> Result<bool, CoreError> {
    store.with_lock(|tx| {
        let Some(current) = load_document(tx)? else {
            return Ok(false);
        };
        if expected.is_some_and(|token| token.expose() != current.token.expose()) {
            return Ok(false);
        }
        tx.remove_verified_file(SESSION_FILE)?;
        Ok(true)
    })
}

fn load_document(tx: &StoreTransaction<'_>) -> Result<Option<PanitaSession>, CoreError> {
    let Some(mut bytes) = tx.read(SESSION_FILE)? else {
        return Ok(None);
    };
    let invalid = || CoreError::new(ErrorCode::ActivationFileInvalid);
    let parsed = serde_json::from_slice::<SessionFile>(&bytes).map_err(|_| invalid());
    bytes.zeroize();
    let mut file = parsed?;
    if file.schema != 1 {
        return Err(invalid());
    }
    let token = std::mem::take(&mut file.token);
    PanitaSession::new(token, file.user.clone(), file.verified_at)
        .map(Some)
        .map_err(|_| invalid())
}

fn write_document(tx: &StoreTransaction<'_>, session: &PanitaSession) -> Result<(), CoreError> {
    let file = SessionFile {
        schema: 1,
        token: session.token.expose().to_owned(),
        user: session.user.clone(),
        verified_at: session.verified_at,
    };
    let mut bytes =
        crate::storage_json::pretty(&file).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
    let result = tx.write(SESSION_FILE, &bytes);
    bytes.zeroize();
    result?;
    #[cfg(windows)]
    let _ = tx.hide(SESSION_FILE);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use uuid::Uuid;

    const TOKEN: &str = "ps_fixture-token-value";

    fn store() -> (AppPaths, PrivateStore) {
        let paths =
            AppPaths::new(std::env::temp_dir().join(format!("panita-session-{}", Uuid::new_v4())))
                .unwrap_or_else(|_| panic!("path"));
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store"));
        (paths, store)
    }

    pub(crate) fn user() -> PanitaUser {
        PanitaUser {
            id: 42,
            ign: Some("Javithor360".to_owned()),
            discord_name: "javithor360".to_owned(),
            avatar_url: Some("https://cdn.discordapp.com/avatars/1/a.png".to_owned()),
            minecraft_status: Some(MinecraftStatus::Premium),
            capabilities: PanitaCapabilities {
                authenticated: true,
                admin_or_mod: false,
                trusted_author: false,
                gallery_editor: false,
                upload_authorized: false,
                wiki_editor: false,
            },
        }
    }

    fn session(token: &str) -> PanitaSession {
        PanitaSession::new(token.to_owned(), user(), 100).unwrap_or_else(|_| panic!("session"))
    }

    fn cleanup(paths: &AppPaths, store: PrivateStore) {
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn saved_session_round_trips_with_schema_first() {
        let (paths, store) = store();
        assert!(load(&store).is_ok_and(|loaded| loaded.is_none()));
        assert!(save(&store, &session(TOKEN)).is_ok());
        let loaded = load(&store)
            .unwrap_or_else(|_| panic!("load"))
            .unwrap_or_else(|| panic!("session present"));
        assert_eq!(loaded.token().expose(), TOKEN);
        assert_eq!(loaded.user, user());
        assert_eq!(loaded.verified_at, 100);
        let text = std::fs::read_to_string(paths.panita_session()).unwrap_or_default();
        assert!(text.starts_with("{\n  \"schema\": 1,\n"));
        assert!(text.ends_with("}\n"));
        cleanup(&paths, store);
    }

    #[test]
    fn invalid_tokens_and_snapshots_are_rejected() {
        for token in [
            "",
            "plain",
            "ps_has space",
            "ps_nul\0",
            &format!("ps_{}", "a".repeat(300)),
        ] {
            assert!(PanitaSession::new(token.to_owned(), user(), 1).is_err());
        }
        let mut bad = user();
        bad.avatar_url = Some("http://insecure.example/a.png".to_owned());
        assert!(PanitaSession::new(TOKEN.to_owned(), bad, 1).is_err());
        let mut nameless = user();
        nameless.discord_name.clear();
        assert!(PanitaSession::new(TOKEN.to_owned(), nameless, 1).is_err());
    }

    #[test]
    fn damaged_or_future_files_are_preserved_and_never_replaced() {
        for content in [
            b"not json".as_slice(),
            br#"{"schema":2,"token":"ps_x","user":{},"verified_at":1,"extra":true}"#.as_slice(),
            br#"{"schema":1,"token":"plain","user":{"id":1},"verified_at":1}"#.as_slice(),
        ] {
            let (paths, store) = store();
            assert!(
                store
                    .transaction("panita-session.json", |_| Ok((content.to_vec(), ())))
                    .is_ok()
            );
            for code in [
                load(&store).err().map(|error| error.code()),
                save(&store, &session(TOKEN))
                    .err()
                    .map(|error| error.code()),
                clear(&store, None).err().map(|error| error.code()),
            ] {
                assert_eq!(code, Some(ErrorCode::ActivationFileInvalid));
            }
            assert_eq!(
                store.read("panita-session.json").ok().flatten(),
                Some(content.to_vec())
            );
            cleanup(&paths, store);
        }
    }

    #[test]
    fn late_refresh_and_stale_clear_never_override_the_current_token() {
        let (paths, store) = store();
        let first = session(TOKEN);
        assert!(save(&store, &first).is_ok());
        let newer = session("ps_second-token-value");
        assert!(save(&store, &newer).is_ok());
        let mut changed = user();
        changed.ign = Some("Renamed".to_owned());
        // A refresh of the replaced token is dropped.
        assert_eq!(
            refresh_snapshot(&store, first.token(), changed.clone(), 200).ok(),
            Some(false)
        );
        // A rejection of the replaced token does not remove the new session.
        assert_eq!(clear(&store, Some(first.token())).ok(), Some(false));
        let loaded = load(&store)
            .unwrap_or_else(|_| panic!("load"))
            .unwrap_or_else(|| panic!("present"));
        assert_eq!(loaded.token().expose(), "ps_second-token-value");
        assert_eq!(loaded.user.ign.as_deref(), Some("Javithor360"));
        // The current token refreshes, then clears, and a late refresh cannot resurrect it.
        assert_eq!(
            refresh_snapshot(&store, newer.token(), changed.clone(), 200).ok(),
            Some(true)
        );
        assert_eq!(clear(&store, Some(newer.token())).ok(), Some(true));
        assert!(!paths.panita_session().exists());
        assert_eq!(
            refresh_snapshot(&store, newer.token(), changed, 300).ok(),
            Some(false)
        );
        assert!(!paths.panita_session().exists());
        cleanup(&paths, store);
    }

    #[test]
    fn saved_file_stays_private_and_never_contains_other_secrets() {
        let (paths, store) = store();
        assert!(save(&store, &session(TOKEN)).is_ok());
        let file = std::fs::File::open(paths.panita_session()).unwrap_or_else(|_| panic!("file"));
        assert!(private_fs::verify(&file).is_ok());
        drop(file);
        let text = std::fs::read_to_string(paths.panita_session()).unwrap_or_default();
        for field in ["\"token\"", "\"user\"", "\"verified_at\""] {
            assert!(text.contains(field));
        }
        assert!(!text.contains("poll_secret") && !text.contains("password"));
        cleanup(&paths, store);
    }

    #[test]
    #[cfg(windows)]
    fn hidden_attribute_survives_atomic_replacement() {
        use std::os::windows::fs::MetadataExt;
        let (paths, store) = store();
        for number in 0..3 {
            let token = format!("ps_replacement-{number}");
            assert!(save(&store, &session(&token)).is_ok());
            let attributes = std::fs::metadata(paths.panita_session())
                .unwrap_or_else(|_| panic!("metadata"))
                .file_attributes();
            assert_ne!(attributes & 2, 0, "hidden after write {number}");
            let loaded = load(&store)
                .unwrap_or_else(|_| panic!("load"))
                .unwrap_or_else(|| panic!("present"));
            assert_eq!(loaded.token().expose(), token);
        }
        assert_eq!(clear(&store, None).ok(), Some(true));
        cleanup(&paths, store);
    }
}
