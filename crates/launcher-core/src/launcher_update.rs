//! Launcher self-update policy: the release channel, the rules a feed entry must
//! satisfy, and the durable pending requirement that blocks new launches. It performs
//! no network access; the desktop layer fetches, verifies and applies updates.
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorCode};
use crate::storage::{PrivateStore, StoreTransaction};

const STATE_FILE: &str = "update-state.json";
const RELEASES: &str = "https://github.com/panitacraft/panita-client/releases/";

/// The beta feed also carries every stable release, so a beta player graduates to stable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

impl UpdateChannel {
    pub fn feed_url(self) -> &'static str {
        match self {
            Self::Stable => {
                "https://github.com/panitacraft/panita-client/releases/download/updater-stable/latest.json"
            }
            Self::Beta => {
                "https://github.com/panitacraft/panita-client/releases/download/updater-beta/latest.json"
            }
        }
    }

    /// Whether a feed of this channel may legitimately offer `version`.
    pub fn accepts(self, version: &Version) -> bool {
        match self {
            Self::Stable => version.pre.is_empty(),
            Self::Beta => true,
        }
    }
}

/// The nonsecret update state. `pending_version` survives restarts and failed checks.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateState {
    pub channel: UpdateChannel,
    pub pending_version: Option<Version>,
    pub checked_at: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateFile {
    schema: u32,
    channel: UpdateChannel,
    pending_version: Option<String>,
    checked_at: Option<u64>,
}

/// The running launcher's version.
pub fn current_version() -> Result<Version, CoreError> {
    Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| CoreError::new(ErrorCode::InvalidData))
}

/// Only files published under this repository's release for exactly `version` qualify.
pub fn artifact_allowed(url: &str, version: &Version) -> bool {
    let prefix = format!("{RELEASES}download/v{version}/");
    url.strip_prefix(&prefix).is_some_and(|name| {
        !name.is_empty()
            && !name.contains(['/', '\\', '%', '?', '#', '@', ':'])
            && !name.contains("..")
    })
}

/// Accept a feed entry only if it is a newer version that this channel may offer and its
/// artifact belongs to that exact release. The feed is untrusted until this passes.
pub fn validate_offer(
    channel: UpdateChannel,
    current: &Version,
    version: &str,
    artifact_url: &str,
) -> Result<Version, CoreError> {
    let offered =
        Version::parse(version).map_err(|_| CoreError::new(ErrorCode::ArtifactMismatch))?;
    if offered <= *current
        || !channel.accepts(&offered)
        || !artifact_allowed(artifact_url, &offered)
    {
        return Err(CoreError::new(ErrorCode::ArtifactMismatch));
    }
    Ok(offered)
}

/// The human-readable release page, used when the package cannot update itself.
pub fn release_page(version: &Version) -> String {
    format!("{RELEASES}tag/v{version}")
}

/// Absent is the default state. A damaged or unsupported file is an error and is never replaced.
pub fn load(store: &PrivateStore) -> Result<UpdateState, CoreError> {
    store.with_lock(load_document)
}

pub fn set_channel(store: &PrivateStore, channel: UpdateChannel) -> Result<UpdateState, CoreError> {
    store.with_lock(|tx| {
        let mut state = load_document(tx)?;
        state.channel = channel;
        write_document(tx, &state)?;
        Ok(state)
    })
}

/// Record a successful feed check. `newest` is the version the feed offers, if newer than
/// the running launcher. A requirement is never cleared by a failed check (the caller does
/// not call this then) and is cleared only once the running version has reached it.
pub fn record_check(
    store: &PrivateStore,
    current: &Version,
    newest: Option<&Version>,
    now: u64,
) -> Result<UpdateState, CoreError> {
    store.with_lock(|tx| {
        let mut state = load_document(tx)?;
        let channel = state.channel;
        // A requirement the selected channel does not offer cannot be satisfied, so it lapses.
        let mut pending = state
            .pending_version
            .take()
            .filter(|pending| pending > current && channel.accepts(pending));
        if let Some(newest) =
            newest.filter(|newest| *newest > current && state.channel.accepts(newest))
            && pending.as_ref().is_none_or(|pending| newest > pending)
        {
            pending = Some(newest.clone());
        }
        state.pending_version = pending;
        state.checked_at = Some(now);
        write_document(tx, &state)?;
        Ok(state)
    })
}

/// The required version, if one newer than `current` is known.
pub fn pending_update(
    store: &PrivateStore,
    current: &Version,
) -> Result<Option<Version>, CoreError> {
    let state = load(store)?;
    Ok(state
        .pending_version
        .filter(|pending| pending > current && state.channel.accepts(pending)))
}

/// The launch gate. A known newer release blocks new launches. A damaged state file cannot
/// tell us anything, so it does not block play; it stays untouched and is reported elsewhere.
pub fn require_current(store: &PrivateStore) -> Result<(), CoreError> {
    match pending_update(store, &current_version()?) {
        Ok(Some(_)) => Err(CoreError::new(ErrorCode::LauncherUpdateRequired)),
        Ok(None) | Err(_) => Ok(()),
    }
}

fn load_document(tx: &StoreTransaction<'_>) -> Result<UpdateState, CoreError> {
    let Some(bytes) = tx.read(STATE_FILE)? else {
        return Ok(UpdateState::default());
    };
    let invalid = || CoreError::new(ErrorCode::InvalidData);
    let file = serde_json::from_slice::<StateFile>(&bytes).map_err(|_| invalid())?;
    if file.schema != 1 {
        return Err(invalid());
    }
    let pending_version = file
        .pending_version
        .map(|text| Version::parse(&text).map_err(|_| invalid()))
        .transpose()?;
    Ok(UpdateState {
        channel: file.channel,
        pending_version,
        checked_at: file.checked_at,
    })
}

fn write_document(tx: &StoreTransaction<'_>, state: &UpdateState) -> Result<(), CoreError> {
    let file = StateFile {
        schema: 1,
        channel: state.channel,
        pending_version: state.pending_version.as_ref().map(Version::to_string),
        checked_at: state.checked_at,
    };
    let bytes =
        crate::storage_json::pretty(&file).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
    tx.write(STATE_FILE, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use uuid::Uuid;

    fn store() -> (AppPaths, PrivateStore) {
        let paths =
            AppPaths::new(std::env::temp_dir().join(format!("panita-update-{}", Uuid::new_v4())))
                .unwrap_or_else(|_| panic!("path"));
        let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store"));
        (paths, store)
    }

    fn cleanup(paths: &AppPaths, store: PrivateStore) {
        drop(store);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|_| panic!("version"))
    }

    #[test]
    fn absent_state_is_the_stable_default() {
        let (paths, store) = store();
        assert_eq!(load(&store).ok(), Some(UpdateState::default()));
        assert!(require_current(&store).is_ok());
        cleanup(&paths, store);
    }

    #[test]
    fn channels_map_to_fixed_feeds_and_stable_rejects_prereleases() {
        assert!(
            UpdateChannel::Stable
                .feed_url()
                .ends_with("/updater-stable/latest.json")
        );
        assert!(
            UpdateChannel::Beta
                .feed_url()
                .ends_with("/updater-beta/latest.json")
        );
        assert!(UpdateChannel::Stable.accepts(&version("1.2.0")));
        assert!(!UpdateChannel::Stable.accepts(&version("1.2.0-beta.1")));
        assert!(UpdateChannel::Beta.accepts(&version("1.2.0-beta.1")));
        assert!(UpdateChannel::Beta.accepts(&version("1.2.0")));
    }

    #[test]
    fn artifacts_must_sit_under_the_exact_release_of_this_repository() {
        let v = version("0.2.0-beta.4");
        let base = "https://github.com/panitacraft/panita-client/releases/download/v0.2.0-beta.4/";
        assert!(artifact_allowed(
            &format!("{base}panita-client_0.2.0-beta.4_windows_x64_unsigned.exe"),
            &v
        ));
        for bad in [
            "http://github.com/panitacraft/panita-client/releases/download/v0.2.0-beta.4/a.exe".to_owned(),
            "https://github.com/panitacraft/other/releases/download/v0.2.0-beta.4/a.exe".to_owned(),
            "https://github.com/panitacraft/panita-client/releases/download/v0.2.0-beta.3/a.exe".to_owned(),
            "https://github.com.evil.example/panitacraft/panita-client/releases/download/v0.2.0-beta.4/a.exe".to_owned(),
            base.to_string(),
            format!("{base}../a.exe"),
            format!("{base}sub/a.exe"),
            format!("{base}a.exe?x=1"),
            format!("{base}%2e%2e"),
        ] {
            assert!(!artifact_allowed(&bad, &v), "{bad}");
        }
        assert_eq!(
            release_page(&v),
            "https://github.com/panitacraft/panita-client/releases/tag/v0.2.0-beta.4"
        );
    }

    #[test]
    fn an_offer_must_be_newer_in_channel_and_hosted_by_its_own_release() {
        let current = version("1.0.0");
        let url = |v: &str| {
            format!("https://github.com/panitacraft/panita-client/releases/download/v{v}/setup.exe")
        };
        let accepted = |channel, v: &str, link: &str| validate_offer(channel, &current, v, link);
        assert_eq!(
            accepted(UpdateChannel::Stable, "1.1.0", &url("1.1.0")).ok(),
            Some(version("1.1.0"))
        );
        assert!(accepted(UpdateChannel::Beta, "1.1.0-beta.2", &url("1.1.0-beta.2")).is_ok());
        for (channel, v, link) in [
            (UpdateChannel::Stable, "1.1.0-beta.2", url("1.1.0-beta.2")),
            (UpdateChannel::Stable, "1.0.0", url("1.0.0")),
            (UpdateChannel::Stable, "0.9.0", url("0.9.0")),
            (UpdateChannel::Stable, "1.1.0", url("1.2.0")),
            (UpdateChannel::Stable, "not-a-version", url("1.1.0")),
            (
                UpdateChannel::Stable,
                "1.1.0",
                "https://example.com/setup.exe".to_owned(),
            ),
        ] {
            let error = accepted(channel, v, &link).err().map(|e| e.code());
            assert_eq!(error, Some(ErrorCode::ArtifactMismatch), "{v} {link}");
        }
    }

    #[test]
    fn a_newer_release_becomes_a_durable_requirement_that_blocks_launch() {
        let (paths, store) = store();
        let current = version("0.2.0-beta.3");
        set_channel(&store, UpdateChannel::Beta).unwrap_or_else(|_| panic!("channel"));
        let state = record_check(&store, &current, Some(&version("0.2.0-beta.4")), 10)
            .unwrap_or_else(|_| panic!("record"));
        assert_eq!(state.pending_version, Some(version("0.2.0-beta.4")));
        assert_eq!(
            pending_update(&store, &current).ok().flatten(),
            Some(version("0.2.0-beta.4"))
        );
        // The state is read back from disk, so it survives a restart.
        assert_eq!(load(&store).ok().and_then(|s| s.checked_at), Some(10));
        cleanup(&paths, store);
    }

    #[test]
    fn the_gate_blocks_only_while_a_newer_version_is_pending() {
        let (paths, store) = store();
        let running = current_version().unwrap_or_else(|_| panic!("version"));
        let newer = Version::new(running.major + 1, 0, 0);
        record_check(&store, &running, Some(&newer), 1).unwrap_or_else(|_| panic!("record"));
        let blocked = require_current(&store).err().map(|e| e.code());
        assert_eq!(blocked, Some(ErrorCode::LauncherUpdateRequired));
        // Once the running version has reached the requirement it no longer blocks.
        record_check(&store, &newer, None, 2).unwrap_or_else(|_| panic!("record"));
        assert_eq!(pending_update(&store, &newer).ok().flatten(), None);
        cleanup(&paths, store);
    }

    #[test]
    fn an_up_to_date_check_keeps_a_higher_requirement_and_never_downgrades_it() {
        let (paths, store) = store();
        let current = version("1.0.0");
        record_check(&store, &current, Some(&version("1.2.0")), 1)
            .unwrap_or_else(|_| panic!("record"));
        record_check(&store, &current, Some(&version("1.1.0")), 2)
            .unwrap_or_else(|_| panic!("record"));
        record_check(&store, &current, None, 3).unwrap_or_else(|_| panic!("record"));
        assert_eq!(
            pending_update(&store, &current).ok().flatten(),
            Some(version("1.2.0"))
        );
        cleanup(&paths, store);
    }

    #[test]
    fn a_stable_channel_ignores_a_prerelease_offer_and_the_channel_persists() {
        let (paths, store) = store();
        let current = version("1.0.0");
        let state = record_check(&store, &current, Some(&version("1.1.0-beta.1")), 1)
            .unwrap_or_else(|_| panic!("record"));
        assert_eq!(state.pending_version, None);
        set_channel(&store, UpdateChannel::Beta).unwrap_or_else(|_| panic!("channel"));
        let state = record_check(&store, &current, Some(&version("1.1.0-beta.1")), 2)
            .unwrap_or_else(|_| panic!("record"));
        assert_eq!(state.pending_version, Some(version("1.1.0-beta.1")));
        assert_eq!(
            load(&store).ok().map(|s| s.channel),
            Some(UpdateChannel::Beta)
        );
        cleanup(&paths, store);
    }

    #[test]
    fn leaving_the_beta_channel_releases_a_beta_only_requirement() {
        let (paths, store) = store();
        let current = version("1.0.0");
        set_channel(&store, UpdateChannel::Beta).unwrap_or_else(|_| panic!("channel"));
        record_check(&store, &current, Some(&version("1.1.0-beta.1")), 1)
            .unwrap_or_else(|_| panic!("record"));
        assert!(pending_update(&store, &current).ok().flatten().is_some());
        set_channel(&store, UpdateChannel::Stable).unwrap_or_else(|_| panic!("channel"));
        // The stable feed will never offer it, so it must not keep blocking play.
        assert_eq!(pending_update(&store, &current).ok().flatten(), None);
        cleanup(&paths, store);
    }

    #[test]
    fn the_file_is_schema_first_and_corrupt_or_future_files_are_preserved() {
        let (paths, store) = store();
        set_channel(&store, UpdateChannel::Beta).unwrap_or_else(|_| panic!("channel"));
        let text = String::from_utf8(store.read(STATE_FILE).ok().flatten().unwrap_or_default())
            .unwrap_or_default();
        assert!(text.starts_with("{\n  \"schema\": 1,\n"), "{text}");
        assert!(text.ends_with("}\n"));
        for content in [
            b"not json".as_slice(),
            b"{\"schema\":2,\"channel\":\"stable\",\"pending_version\":null,\"checked_at\":null}"
                .as_slice(),
            b"{\"schema\":1,\"channel\":\"stable\",\"pending_version\":\"x\",\"checked_at\":null}"
                .as_slice(),
        ] {
            store
                .transaction(STATE_FILE, |_| Ok((content.to_vec(), ())))
                .unwrap_or_else(|_| panic!("fixture"));
            assert!(load(&store).is_err());
            assert!(set_channel(&store, UpdateChannel::Stable).is_err());
            assert!(record_check(&store, &version("1.0.0"), None, 1).is_err());
            // A damaged file never blocks play and is left exactly as found.
            assert!(require_current(&store).is_ok());
            assert_eq!(
                store.read(STATE_FILE).ok().flatten(),
                Some(content.to_vec())
            );
        }
        cleanup(&paths, store);
    }
}
