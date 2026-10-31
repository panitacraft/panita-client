//! Per-session diagnostic log files under `logs/`, written for support.
//!
//! One file is created per launcher process and pruned after 48 hours. Lines are
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
    /// Prune expired session logs, then create this process's file.
    pub fn create(directory: &Path, process: ProcessSessionId) -> std::io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        prune(directory, SystemTime::now());
        let (stamp, _) = timestamp(SystemTime::now());
        let id = process.value().simple().to_string();
        let name = format!(
            "{FILE_PREFIX}{}-{}{FILE_SUFFIX}",
            stamp.replace([':', '-'], "").replace('T', "-"),
            &id[..8]
        );
        let path = directory.join(name);
        let mut options = OpenOptions::new();
        options.create_new(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
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
        if self.full {
            return;
        }
        let (stamp, _) = timestamp(SystemTime::now());
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
        if !name.starts_with(FILE_PREFIX) || !name.ends_with(FILE_SUFFIX) {
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
    let home = ["USERPROFILE", "HOME"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .filter(|value| value.len() > 3)
        .collect::<Vec<_>>();
    let mut cleaned = text.to_owned();
    for value in home {
        cleaned = cleaned.replace(&value, "~");
        cleaned = cleaned.replace(&value.replace('\\', "/"), "~");
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
