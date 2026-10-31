//! Per-session diagnostic log files under `logs/`, written for support.
//!
//! The running session writes `latest.log`. When the next session starts, that file
//! is renamed to its own `session-{start}-{id}.log` name. Files are pruned after 48 hours. Lines are
//! redacted before they reach the disk; the file never receives credentials,
//! callbacks, serials or account files.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::operation::ProcessSessionId;

pub const RETENTION: Duration = Duration::from_secs(48 * 60 * 60);
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_MESSAGE_CHARS: usize = 1024;
const FILE_PREFIX: &str = "session-";
const FILE_SUFFIX: &str = ".log";
/// Name of the file the running session writes.
pub const LATEST: &str = "latest.log";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO ",
            Self::Warn => "WARN ",
            Self::Error => "ERROR",
        }
    }
}

pub struct SessionFile {
    file: File,
    path: PathBuf,
    written: u64,
    full: bool,
}

impl SessionFile {
    /// Archive the previous session's `latest.log`, prune expired logs, then create
    /// this process's `latest.log`. If it cannot be created (for example another
    /// launcher still holds it), this session writes a uniquely named file instead.
    pub fn create(directory: &Path, process: ProcessSessionId) -> std::io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        archive_latest(directory);
        prune(directory, SystemTime::now());
        let id = process.value().simple().to_string();
        let (stamp, _) = timestamp(SystemTime::now());
        let named = directory.join(archive_name(&stamp, &id[..8]));
        let latest = directory.join(LATEST);
        let (file, path) = match open_new(&latest) {
            Ok(file) => (file, latest),
            Err(_) => (open_new(&named)?, named),
        };
        Ok(Self {
            file,
            path,
            written: 0,
            full: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&mut self, level: Level, text: &str) {
        self.write_at(SystemTime::now(), level, text);
    }

    pub fn write_at(&mut self, time: SystemTime, level: Level, text: &str) {
        if self.full {
            return;
        }
        let (stamp, _) = timestamp(time);
        let line = format!("{stamp} [{}] {text}\n", level.label());
        if self.written + line.len() as u64 > MAX_FILE_BYTES {
            self.full = true;
            let _ = self
                .file
                .write_all(b"-- size limit reached; later entries are not recorded --\n");
            return;
        }
        // A failing diagnostic write must never disturb the launcher.
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.written += line.len() as u64;
        }
    }
}

fn open_new(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// `session-20261031-120000-1a2b3c4d.log` from an ISO timestamp and a short id.
fn archive_name(stamp: &str, id: &str) -> String {
    let compact: String = stamp
        .chars()
        .take(19)
        .filter(|ch| ch.is_ascii_digit() || *ch == 'T')
        .collect::<String>()
        .replace('T', "-");
    format!("{FILE_PREFIX}{compact}-{id}{FILE_SUFFIX}")
}

/// Rename the previous session's `latest.log` to the name its own header gives
/// (start time and process id). A missing or unreadable header falls back to the
/// file's modification time and an `unknown` id. Failure leaves the file in place.
fn archive_latest(directory: &Path) {
    let latest = directory.join(LATEST);
    // Never follow links or touch anything that is not a regular file.
    if !std::fs::symlink_metadata(&latest).is_ok_and(|metadata| metadata.is_file()) {
        return;
    }
    let mut head = [0u8; 256];
    let read = File::open(&latest)
        .and_then(|mut file| std::io::Read::read(&mut file, &mut head))
        .unwrap_or(0);
    let first = String::from_utf8_lossy(&head[..read]);
    let first = first.lines().next().unwrap_or_default();
    let stamp = first
        .split_whitespace()
        .next()
        .filter(|word| word.len() >= 19 && word.as_bytes()[4] == b'-')
        .map(str::to_owned)
        .or_else(|| {
            std::fs::metadata(&latest)
                .and_then(|metadata| metadata.modified())
                .ok()
                .map(|modified| timestamp(modified).0)
        })
        .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".to_owned());
    let id = first
        .split("process=")
        .nth(1)
        .map(|rest| {
            rest.chars()
                .filter(char::is_ascii_hexdigit)
                .take(8)
                .collect::<String>()
        })
        .filter(|id| id.len() == 8)
        .unwrap_or_else(|| "unknown".to_owned());
    let mut target = directory.join(archive_name(&stamp, &id));
    if target.exists() {
        target = directory.join(archive_name(&stamp, &format!("{id}-dup")));
    }
    let _ = std::fs::rename(&latest, target);
}

fn timestamp(time: SystemTime) -> (String, u64) {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let formatted = chrono::DateTime::from_timestamp(
        i64::try_from(since.as_secs()).unwrap_or(0),
        since.subsec_nanos(),
    )
    .map(|date| date.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
    .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".to_owned());
    (formatted, since.as_secs())
}

/// Delete only application-named session logs older than the retention window.
pub fn prune(directory: &Path, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let owned =
            name == LATEST || (name.starts_with(FILE_PREFIX) && name.ends_with(FILE_SUFFIX));
        if !owned {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        // Never follow or delete links, directories or other non-regular entries.
        if !metadata.is_file() {
            continue;
        }
        let expired = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > RETENTION);
        if expired && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Make free text safe for a support log: one line, bounded, without home paths,
/// URL queries, bearer tokens, JWTs, Panita serials or long opaque secrets.
pub fn redact(text: &str) -> String {
    let homes = ["USERPROFILE", "HOME"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .collect::<Vec<_>>();
    redact_with_homes(text, &homes)
}

/// Replace each home directory, in either slash style and ignoring ASCII case
/// (Windows paths differ in case), with `~`.
fn mask_home(text: &str, home: &str) -> String {
    let mut result = text.to_owned();
    for needle in [home.to_owned(), home.replace('\\', "/")] {
        let lower_needle = needle.to_ascii_lowercase();
        let mut masked = String::with_capacity(result.len());
        let mut rest = result.as_str();
        // ASCII lowercasing keeps byte offsets identical to the original text.
        while let Some(position) = rest.to_ascii_lowercase().find(&lower_needle) {
            masked.push_str(&rest[..position]);
            masked.push('~');
            rest = &rest[position + needle.len()..];
        }
        masked.push_str(rest);
        result = masked;
    }
    result
}

fn redact_with_homes(text: &str, homes: &[String]) -> String {
    let mut cleaned = text.to_owned();
    for home in homes.iter().filter(|home| home.len() > 3) {
        cleaned = mask_home(&cleaned, home);
    }
    let mut words = Vec::new();
    let mut after_bearer = false;
    for word in cleaned.split_whitespace() {
        let lower = word.to_ascii_lowercase();
        // Instance and operation UUIDs identify support cases and are not secrets.
        let identifier = uuid::Uuid::parse_str(word).is_ok();
        let opaque = looks_like_serial(word)
            || word.matches('.').count() == 2 && word.len() > 40 && lower.starts_with("ey")
            || (word.len() >= 32
                && word.chars().all(|ch| {
                    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '+' | '/' | '=')
                }));
        let secret = after_bearer || (!identifier && opaque);
        after_bearer = matches!(lower.as_str(), "bearer" | "basic" | "authorization:");
        if secret {
            words.push("[redacted]".to_owned());
        } else if let Some(position) = word.find(['?', '#']).filter(|_| word.contains("://")) {
            words.push(format!("{}[query-removed]", &word[..position]));
        } else {
            words.push(word.to_owned());
        }
    }
    let joined = words.join(" ");
    let mut result: String = joined
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(MAX_MESSAGE_CHARS)
        .collect();
    if joined.chars().count() > MAX_MESSAGE_CHARS {
        result.push_str("…[truncated]");
    }
    result
}

/// Describe which top-level fields differ, as `key: old -> new`. Fields in `skip`
/// are ignored, for example window sizes that change on every resize.
pub fn changes(
    before: &impl serde::Serialize,
    after: &impl serde::Serialize,
    skip: &[&str],
) -> Vec<String> {
    let (Ok(serde_json::Value::Object(before)), Ok(serde_json::Value::Object(after))) =
        (serde_json::to_value(before), serde_json::to_value(after))
    else {
        return Vec::new();
    };
    after
        .iter()
        .filter(|(key, _)| !skip.contains(&key.as_str()))
        .filter(|(key, value)| before.get(*key) != Some(*value))
        .map(|(key, value)| {
            let old = before
                .get(key)
                .map_or_else(|| "unset".to_owned(), ToString::to_string);
            format!("{key}: {old} -> {value}")
        })
        .collect()
}

fn looks_like_serial(word: &str) -> bool {
    let trimmed = word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    let parts: Vec<_> = trimmed.split('-').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| part.len() == 4 && part.chars().all(|ch| ch.is_ascii_alphanumeric()))
        && trimmed
            .chars()
            .any(|ch| ch.is_ascii_digit() || ch.is_ascii_uppercase())
        && trimmed == trimmed.to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("panita-session-log-{}", uuid::Uuid::new_v4()));
        assert!(std::fs::create_dir_all(&path).is_ok());
        path
    }

    #[test]
    fn redaction_removes_tokens_serials_queries_and_control_characters() {
        let text = "Authorization: Bearer abc123 serial AB12-CD34-EF56 \
                    https://x.example/cb?code=secret#frag eyJhbGciOiJIUzI1NiIsInR5cCI6.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signaturepartthatislong\nnext\u{7}line";
        let out = redact(text);
        assert!(!out.contains("abc123"));
        assert!(!out.contains("AB12-CD34-EF56"));
        assert!(!out.contains("secret"));
        assert!(!out.contains("eyJ"));
        assert!(!out.contains('\n') && !out.contains('\u{7}'));
        assert!(out.contains("https://x.example/cb[query-removed]"));
    }

    #[test]
    fn changes_lists_only_differing_fields_and_honors_the_skip_list() {
        let before = serde_json::json!({ "ram_mib": 4096, "console": false, "window": 1 });
        let after = serde_json::json!({ "ram_mib": 6144, "console": false, "window": 2 });
        assert_eq!(
            changes(&before, &after, &["window"]),
            ["ram_mib: 4096 -> 6144"]
        );
        assert!(changes(&before, &before, &[]).is_empty());
    }

    #[test]
    fn home_directories_are_masked_in_both_slash_styles_and_any_case() {
        let windows = vec!["C:\\Users\\Flore".to_owned()];
        let out = redact_with_homes("path c:\\users\\flore\\x and C:/Users/Flore/y", &windows);
        assert_eq!(out, "path ~\\x and ~/y");
        let unix = vec!["/home/ana".to_owned()];
        assert_eq!(
            redact_with_homes("/home/ana/.local/share/a", &unix),
            "~/.local/share/a"
        );
        assert_eq!(
            redact_with_homes("/home/anabel", &["/ho".to_owned()]),
            "/home/anabel",
            "short homes are ignored"
        );
    }

    #[test]
    fn redaction_bounds_message_length() {
        let out = redact(&"word ".repeat(2000));
        assert!(out.chars().count() <= MAX_MESSAGE_CHARS + 16);
        assert!(out.ends_with("[truncated]"));
    }

    #[test]
    fn prune_removes_only_expired_session_logs() {
        let dir = scratch();
        let old = dir.join("session-old.log");
        let fresh = dir.join("session-new.log");
        let other = dir.join("notes.txt");
        for path in [&old, &fresh, &other] {
            assert!(std::fs::write(path, "x").is_ok());
        }
        let later = SystemTime::now() + RETENTION + Duration::from_secs(60);
        let removed = prune(&dir, later);
        assert_eq!(removed, 2, "both session logs expire at the later clock");
        assert!(other.exists(), "unrelated files are never deleted");
        assert!(prune(&dir, SystemTime::now()) == 0);
        assert!(std::fs::remove_dir_all(dir).is_ok());
    }

    #[test]
    fn the_next_session_renames_latest_to_its_own_name() {
        let dir = scratch();
        let first_process = ProcessSessionId::new();
        let Ok(mut first) = SessionFile::create(&dir, first_process) else {
            panic!("first");
        };
        first.write(
            Level::Info,
            &format!("session started process={}", first_process.value()),
        );
        assert_eq!(
            first.path().file_name().and_then(|n| n.to_str()),
            Some(LATEST)
        );
        drop(first);
        let Ok(second) = SessionFile::create(&dir, ProcessSessionId::new()) else {
            panic!("second");
        };
        assert_eq!(
            second.path().file_name().and_then(|n| n.to_str()),
            Some(LATEST)
        );
        let id = first_process.value().simple().to_string();
        let names: Vec<String> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert!(names.contains(&LATEST.to_owned()));
        assert!(
            names.iter().any(|name| name.starts_with(FILE_PREFIX)
                && name.ends_with(&format!("{}{FILE_SUFFIX}", &id[..8]))),
            "{names:?}"
        );
        drop(second);
        assert!(std::fs::remove_dir_all(dir).is_ok());
    }

    #[test]
    fn archive_names_are_compact_and_filesystem_safe() {
        assert_eq!(
            archive_name("2026-10-31T12:00:05.123Z", "1a2b3c4d"),
            "session-20261031-120005-1a2b3c4d.log"
        );
    }

    #[test]
    fn session_file_is_created_once_and_appends_timestamped_lines() {
        let dir = scratch();
        let Ok(mut file) = SessionFile::create(&dir, ProcessSessionId::new()) else {
            panic!("create");
        };
        file.write(Level::Error, "storage failure code=storage_unavailable");
        let Ok(text) = std::fs::read_to_string(file.path()) else {
            panic!("read");
        };
        assert!(text.contains("[ERROR] storage failure code=storage_unavailable"));
        assert!(text.starts_with("20"));
        assert!(std::fs::remove_dir_all(dir).is_ok());
    }
}
