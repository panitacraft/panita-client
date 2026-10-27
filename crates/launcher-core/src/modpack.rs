//! Public pack contracts. Parsing never authorizes installation or game execution.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    catalog::{CatalogInstance, CatalogSource},
    error::{CoreError, ErrorCode},
    minecraft::safe_path,
};

pub mod cache;
pub mod modrinth;
pub mod state;

pub const STORAGE_HOST: &str = "jqleaywgcoedgvglrvhc.supabase.co";
pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FILES: usize = 20_000;
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_PACK_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_REDIRECTS: usize = 3;
pub const MAX_DOWNLOADS: usize = 8;

fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidData)
}
fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
}
fn pin(value: &str) -> bool {
    text(value, 64)
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-+".contains(&c))
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn date(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Modrinth,
    Curseforge,
    Github,
    Upload,
}

/// Validate before connecting. Redirect authorization is separate from initial URLs.
pub fn download_url(raw: &str, origin: &Origin) -> Result<url::Url, CoreError> {
    let url = clean_url(raw)?;
    let parts: Vec<_> = url.path().split('/').skip(1).collect();
    let allowed = match origin {
        Origin::Modrinth => {
            url.host_str() == Some("cdn.modrinth.com")
                && parts.len() == 5
                && parts[0] == "data"
                && parts[2] == "versions"
        }
        Origin::Curseforge => {
            matches!(
                url.host_str(),
                Some("edge.forgecdn.net" | "mediafilecdn.forgecdn.net")
            ) && parts.len() >= 4
                && parts[0] == "files"
        }
        Origin::Github => {
            url.host_str() == Some("github.com")
                && parts.len() == 6
                && parts[2] == "releases"
                && parts[3] == "download"
        }
        Origin::Upload => {
            url.host_str() == Some(STORAGE_HOST)
                && url
                    .path()
                    .starts_with("/storage/v1/object/public/modpacks/")
                && parts.len() > 6
        }
    };
    if !allowed {
        return Err(invalid());
    }
    Ok(url)
}

fn clean_url(raw: &str) -> Result<url::Url, CoreError> {
    let lower = raw.to_ascii_lowercase();
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    if raw.len() > 2048
        || !raw.is_ascii()
        || raw
            .bytes()
            .any(|c| c.is_ascii_whitespace() || c.is_ascii_control())
        || raw.contains('\\')
        || lower.contains("%2f")
        || lower.contains("%5c")
        || lower.contains("%2e")
        || raw.split('/').any(|part| matches!(part, "." | ".."))
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().split('/').skip(1).any(str::is_empty)
    {
        return Err(invalid());
    }
    Ok(url)
}

pub fn github_redirect(raw: &str, hop: usize) -> Result<url::Url, CoreError> {
    // Signed GitHub CDN redirects need query parameters; never store/log them.
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    if hop == 0
        || hop > MAX_REDIRECTS
        || raw.len() > 8192
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || !url
            .host_str()
            .is_some_and(|host| host.ends_with(".githubusercontent.com"))
    {
        return Err(invalid());
    }
    Ok(url)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Required,
    Optional,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environments {
    pub client: Environment,
    pub server: Environment,
}
impl Default for Environments {
    fn default() -> Self {
        Self {
            client: Environment::Required,
            server: Environment::Required,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hashes {
    pub sha1: Option<String>,
    pub sha512: Option<String>,
}
impl Hashes {
    pub fn validate(&self) -> Result<(), CoreError> {
        if (self.sha1.is_none() && self.sha512.is_none())
            || self.sha1.as_deref().is_some_and(|v| !hex(v, 40))
            || self.sha512.as_deref().is_some_and(|v| !hex(v, 128))
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    Mod,
    Resourcepack,
    Shaderpack,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurseforgeFile {
    pub project_id: u64,
    pub file_id: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    #[serde(rename = "type")]
    pub file_type: FileType,
    pub source: Origin,
    pub path: String,
    pub name: String,
    pub env: Environments,
    pub hashes: Hashes,
    pub size: u64,
    pub url: Option<String>,
    #[serde(default)]
    pub manual: bool,
    pub curseforge: Option<CurseforgeFile>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archive {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archives {
    pub defaults: Option<Archive>,
    pub enforced: Option<Archive>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub instance_id: Uuid,
    pub version_id: Uuid,
    pub number: u64,
    pub label: Option<String>,
    pub published_at: String,
    pub minecraft_version: String,
    pub loader: String,
    pub loader_version: String,
    pub files: Vec<ManifestFile>,
    pub archives: Archives,
}
#[derive(Deserialize)]
struct Response {
    data: Manifest,
}

impl Manifest {
    pub fn parse(bytes: &[u8], instance: Uuid, version: Uuid) -> Result<Self, CoreError> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid());
        }
        let response: Response = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        response.data.validate(instance, version)?;
        Ok(response.data)
    }

    pub fn validate(&self, instance: Uuid, version: Uuid) -> Result<(), CoreError> {
        if instance.is_nil()
            || version.is_nil()
            || self.instance_id != instance
            || self.version_id != version
            || self.number == 0
            || !date(&self.published_at)
            || !pin(&self.minecraft_version)
            || self.minecraft_version.len() > 32
            || !pin(&self.loader)
            || !pin(&self.loader_version)
            || self.label.as_deref().is_some_and(|label| !text(label, 32))
            || self.files.len() > MAX_FILES
        {
            return Err(invalid());
        }
        let mut paths = BTreeSet::new();
        let mut total = 0u64;
        for file in &self.files {
            safe_path(&file.path)?;
            let parts: Vec<_> = file.path.split('/').collect();
            let folder = match file.file_type {
                FileType::Mod => "mods",
                FileType::Resourcepack => "resourcepacks",
                FileType::Shaderpack => "shaderpacks",
            };
            file.hashes.validate()?;
            if parts.len() != 2
                || parts[0] != folder
                || !paths.insert(file.path.to_ascii_lowercase())
                || !text(&file.name, 256)
                || file.size > MAX_FILE_BYTES
                || (file.source != Origin::Curseforge && file.hashes.sha512.is_none())
                || (file.source == Origin::Curseforge
                    && (file.hashes.sha1.is_none()
                        || !file
                            .curseforge
                            .as_ref()
                            .is_some_and(|ids| ids.project_id > 0 && ids.file_id > 0)))
                || (file.source != Origin::Curseforge && file.curseforge.is_some())
                || (file.manual && (file.source != Origin::Curseforge || file.url.is_some()))
                || (!file.manual && file.url.is_none())
            {
                return Err(invalid());
            }
            if let Some(url) = &file.url {
                download_url(url, &file.source)?;
            }
            total = total.checked_add(file.size).ok_or_else(invalid)?;
        }
        for archive in [&self.archives.defaults, &self.archives.enforced]
            .into_iter()
            .flatten()
        {
            download_url(&archive.url, &Origin::Upload)?;
            if !hex(&archive.sha256, 64) || archive.size > MAX_ARCHIVE_BYTES {
                return Err(invalid());
            }
            total = total.checked_add(archive.size).ok_or_else(invalid)?;
        }
        if total > MAX_PACK_BYTES {
            return Err(invalid());
        }
        Ok(())
    }

    /// Use only for a newly selected current pin; historical repair pins remain independent.
    pub fn bind_current(&self, catalog: &CatalogInstance) -> Result<(), CoreError> {
        let CatalogSource::Custom {
            minecraft_version,
            loader,
            current_version_id: Some(version),
            label,
            published_at: Some(published),
        } = &catalog.source
        else {
            return Err(invalid());
        };
        self.validate(catalog.id, *version)?;
        if &self.minecraft_version != minecraft_version
            || &self.loader != loader
            || &self.label != label
            || chrono::DateTime::parse_from_rfc3339(&self.published_at).ok()
                != chrono::DateTime::parse_from_rfc3339(published).ok()
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn supported_loader(&self) -> bool {
        self.loader == "neoforge"
    }
}

/// The ZIP and its exact upstream hashes must be verified before parsing its index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackIndex {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    pub summary: Option<String>,
    pub files: Vec<MrpackFile>,
    #[serde(deserialize_with = "unique_dependencies")]
    pub dependencies: BTreeMap<String, String>,
}

fn unique_dependencies<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("unique dependency pins")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut pins = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if pins.len() >= 16 || pins.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("conflicting dependency pins"));
                }
            }
            Ok(pins)
        }
    }
    deserializer.deserialize_map(Visitor)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
    pub path: String,
    pub hashes: Hashes,
    #[serde(default)]
    pub env: Environments,
    pub downloads: Vec<String>,
    pub file_size: u64,
}

impl MrpackIndex {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid());
        }
        let index: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        if index.format_version != 1
            || index.game != "minecraft"
            || !text(&index.version_id, 256)
            || !text(&index.name, 256)
            || index.summary.as_ref().is_some_and(|v| v.len() > 32768)
            || index.files.len() > MAX_FILES
            || index.dependencies.len() > 16
            || !index
                .dependencies
                .get("minecraft")
                .is_some_and(|v| pin(v) && v.len() <= 32)
            || index
                .dependencies
                .iter()
                .any(|(key, value)| !pin(key) || !pin(value))
        {
            return Err(invalid());
        }
        let known_loaders = ["neoforge", "forge", "fabric-loader", "quilt-loader"];
        if known_loaders
            .iter()
            .filter(|key| index.dependencies.contains_key(**key))
            .count()
            > 1
        {
            return Err(invalid());
        }
        let mut paths = BTreeSet::new();
        let mut total = 0u64;
        for file in &index.files {
            game_path(&file.path)?;
            file.hashes.validate()?;
            if file.hashes.sha1.is_none()
                || file.hashes.sha512.is_none()
                || file.file_size > MAX_FILE_BYTES
                || !paths.insert(file.path.to_ascii_lowercase())
                || file.downloads.is_empty()
                || file.downloads.len() > 8
            {
                return Err(invalid());
            }
            for url in &file.downloads {
                mrpack_url(url)?;
            }
            total = total.checked_add(file.file_size).ok_or_else(invalid)?;
        }
        if total > MAX_PACK_BYTES {
            return Err(invalid());
        }
        // A file and its parent cannot both be file destinations.
        for path in &paths {
            for (end, _) in path.match_indices('/') {
                if paths.contains(&path[..end]) {
                    return Err(invalid());
                }
            }
        }
        Ok(index)
    }

    pub fn supported_loader(&self) -> bool {
        self.dependencies.contains_key("neoforge") && self.dependencies.len() == 2
    }
}

/// Reject launcher metadata and player worlds even though the target root is private.
pub fn game_path(path: &str) -> Result<(), CoreError> {
    safe_path(path)?;
    let root = path
        .split('/')
        .next()
        .ok_or_else(invalid)?
        .to_ascii_lowercase();
    if root.starts_with('.')
        || matches!(
            root.as_str(),
            "saves"
                | "screenshots"
                | "backups"
                | "world-backups"
                | "logs"
                | "crash-reports"
                | "instance.json"
                | "installed.json"
                | "transaction.json"
                | "accounts.json"
        )
    {
        return Err(invalid());
    }
    Ok(())
}

fn mrpack_url(raw: &str) -> Result<url::Url, CoreError> {
    let url = clean_url(raw)?;
    match url.host_str() {
        Some("cdn.modrinth.com") => download_url(raw, &Origin::Modrinth),
        Some("github.com") => download_url(raw, &Origin::Github),
        Some("raw.githubusercontent.com") if url.path().split('/').count() >= 5 => Ok(url),
        Some("gitlab.com")
            if url.path().contains("/-/raw/") || url.path().contains("/-/releases/") =>
        {
            Ok(url)
        }
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests;
