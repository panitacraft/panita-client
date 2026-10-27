//! Public catalog validation and revalidation. Catalog data never authorizes a game launch.
use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{Client, StatusCode, header};
use semver::Version;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CoreError, ErrorCode};
use crate::storage::PrivateStore;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

pub(crate) const BASE: &str = "https://api.panitacraft.com";
const MAX_BODY: usize = 512 * 1024;
const MAX_RECORDS: usize = 1000;
const CACHE_FILE: &str = "insts-cache.json";
const PREVIOUS_CACHE_FILE: &str = "inst-cache.json";
const LEGACY_CACHE_FILE: &str = "catalog-cache.json";

fn read_cache(store: &PrivateStore) -> Result<Option<Vec<u8>>, CoreError> {
    store.with_lock(|transaction| match transaction.read(CACHE_FILE)? {
        Some(bytes) => Ok(Some(bytes)),
        None => match transaction.read(PREVIOUS_CACHE_FILE)? {
            Some(bytes) => Ok(Some(bytes)),
            None => transaction.read(LEGACY_CACHE_FILE),
        },
    })
}

/// A member's catalog can include restricted entries, so no copy of it outlives
/// the session it was fetched with. Every cache spelling is attempted.
pub(crate) fn purge_cache(store: &PrivateStore) -> Result<(), CoreError> {
    store.with_lock(|transaction| {
        let mut result = Ok(());
        for name in [CACHE_FILE, PREVIOUS_CACHE_FILE, LEGACY_CACHE_FILE] {
            let removed = transaction.remove_verified_file(name);
            result = result.and(removed);
        }
        result
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CatalogSource {
    Vanilla {
        minecraft_version: String,
    },
    Modrinth {
        project_id: String,
        pack_version_id: String,
        loader: Option<String>,
    },
    Custom {
        minecraft_version: String,
        loader: String,
        current_version_id: Option<Uuid>,
        label: Option<String>,
        published_at: Option<String>,
    },
    Unsupported {
        source_type: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogInstance {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub minecraft_version: Option<String>,
    pub enabled: bool,
    pub restricted: bool,
    pub min_launcher_version: Option<String>,
    pub icon_url: Option<String>,
    pub banner_url: Option<String>,
    pub sort_order: u32,
    pub updated_at: String,
    pub source: CatalogSource,
}

#[derive(Deserialize)]
struct RawInstance {
    id: Uuid,
    slug: String,
    name: String,
    description: String,
    minecraft_version: Option<String>,
    enabled: bool,
    #[serde(default)]
    restricted: bool,
    min_launcher_version: Option<String>,
    icon_url: Option<String>,
    banner_url: Option<String>,
    sort_order: u32,
    updated_at: String,
    source_type: String,
    modrinth_project_id: Option<String>,
    pack_version_id: Option<String>,
    loader: Option<String>,
    current_version_id: Option<Uuid>,
    current_version_label: Option<String>,
    current_version_published_at: Option<String>,
}

fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidData)
}

fn bounded_text(text: &str, max: usize) -> bool {
    !text.is_empty() && text.chars().count() <= max && !text.chars().any(|ch| ch.is_control())
}

fn pin(text: &str) -> bool {
    bounded_text(text, 64)
        && text
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"._-+".contains(&ch))
}

fn https_url(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|value| {
        value.len() <= 2048
            && url::Url::parse(value).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
    })
}

impl TryFrom<RawInstance> for CatalogInstance {
    type Error = CoreError;
    fn try_from(raw: RawInstance) -> Result<Self, CoreError> {
        if raw.id.is_nil()
            || !bounded_text(&raw.name, 100)
            || raw.description.len() > 32 * 1024
            || raw.description.contains('\0')
            || raw.slug.is_empty()
            || raw.slug.len() > 48
            || raw.slug.starts_with('-')
            || raw.slug.ends_with('-')
            || raw.slug.contains("--")
            || !raw
                .slug
                .bytes()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'-')
            || !https_url(&raw.icon_url)
            || !https_url(&raw.banner_url)
            || chrono::DateTime::parse_from_rfc3339(&raw.updated_at).is_err()
            || raw
                .minecraft_version
                .as_deref()
                .is_some_and(|version| !pin(version) || version.len() > 32)
        {
            return Err(invalid());
        }
        if let Some(minimum) = &raw.min_launcher_version {
            let version = Version::parse(minimum).map_err(|_| invalid())?;
            if !version.pre.is_empty() || !version.build.is_empty() {
                return Err(invalid());
            }
        }
        if raw.source_type != "custom"
            && (raw.current_version_id.is_some()
                || raw.current_version_label.is_some()
                || raw.current_version_published_at.is_some())
        {
            return Err(invalid());
        }
        let source = match raw.source_type.as_str() {
            "vanilla" => {
                if raw.modrinth_project_id.is_some()
                    || raw.pack_version_id.is_some()
                    || raw.loader.is_some()
                {
                    return Err(invalid());
                }
                CatalogSource::Vanilla {
                    minecraft_version: raw.minecraft_version.clone().ok_or_else(invalid)?,
                }
            }
            "modrinth" => {
                let project_id = raw.modrinth_project_id.ok_or_else(invalid)?;
                let pack_version_id = raw.pack_version_id.ok_or_else(invalid)?;
                if !pin(&project_id)
                    || !pin(&pack_version_id)
                    || raw.loader.as_deref().is_some_and(|loader| !pin(loader))
                {
                    return Err(invalid());
                }
                CatalogSource::Modrinth {
                    project_id,
                    pack_version_id,
                    loader: raw.loader,
                }
            }
            "custom" => {
                let loader = raw.loader.ok_or_else(invalid)?;
                if raw.modrinth_project_id.is_some()
                    || raw.pack_version_id.is_some()
                    || !pin(&loader)
                    || raw.current_version_id.is_some_and(|id| id.is_nil())
                    || raw
                        .current_version_label
                        .as_deref()
                        .is_some_and(|label| !bounded_text(label, 32))
                    || raw
                        .current_version_published_at
                        .as_deref()
                        .is_some_and(|date| chrono::DateTime::parse_from_rfc3339(date).is_err())
                    || (raw.enabled && raw.current_version_id.is_none())
                    || (raw.current_version_id.is_some()
                        != raw.current_version_published_at.is_some())
                    || (raw.current_version_id.is_none() && raw.current_version_label.is_some())
                {
                    return Err(invalid());
                }
                CatalogSource::Custom {
                    minecraft_version: raw.minecraft_version.clone().ok_or_else(invalid)?,
                    loader,
                    current_version_id: raw.current_version_id,
                    label: raw.current_version_label,
                    published_at: raw.current_version_published_at,
                }
            }
            _ if bounded_text(&raw.source_type, 64) => CatalogSource::Unsupported {
                source_type: raw.source_type,
            },
            _ => return Err(invalid()),
        };
        Ok(Self {
            id: raw.id,
            slug: raw.slug,
            name: raw.name,
            description: raw.description,
            minecraft_version: raw.minecraft_version,
            enabled: raw.enabled,
            restricted: raw.restricted,
            min_launcher_version: raw.min_launcher_version,
            icon_url: raw.icon_url,
            banner_url: raw.banner_url,
            sort_order: raw.sort_order,
            updated_at: raw.updated_at,
            source,
        })
    }
}

impl CatalogInstance {
    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        let (source_type, project, pack, loader) = match &self.source {
            CatalogSource::Vanilla { minecraft_version }
                if Some(minecraft_version) == self.minecraft_version.as_ref() =>
            {
                ("vanilla".to_owned(), None, None, None)
            }
            CatalogSource::Vanilla { .. } => return Err(invalid()),
            CatalogSource::Modrinth {
                project_id,
                pack_version_id,
                loader,
            } => (
                "modrinth".to_owned(),
                Some(project_id.clone()),
                Some(pack_version_id.clone()),
                loader.clone(),
            ),
            CatalogSource::Custom {
                minecraft_version,
                loader,
                ..
            } if Some(minecraft_version) == self.minecraft_version.as_ref() => {
                ("custom".to_owned(), None, None, Some(loader.clone()))
            }
            CatalogSource::Unsupported { source_type }
                if !matches!(source_type.as_str(), "vanilla" | "modrinth" | "custom") =>
            {
                (source_type.clone(), None, None, None)
            }
            _ => return Err(invalid()),
        };
        let (current_version_id, current_version_label, current_version_published_at) =
            match &self.source {
                CatalogSource::Custom {
                    current_version_id,
                    label,
                    published_at,
                    ..
                } => (*current_version_id, label.clone(), published_at.clone()),
                _ => (None, None, None),
            };
        Self::try_from(RawInstance {
            id: self.id,
            slug: self.slug.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            minecraft_version: self.minecraft_version.clone(),
            enabled: self.enabled,
            restricted: self.restricted,
            min_launcher_version: self.min_launcher_version.clone(),
            icon_url: self.icon_url.clone(),
            banner_url: self.banner_url.clone(),
            sort_order: self.sort_order,
            updated_at: self.updated_at.clone(),
            source_type,
            modrinth_project_id: project,
            pack_version_id: pack,
            loader,
            current_version_id,
            current_version_label,
            current_version_published_at,
        })?;
        Ok(())
    }

    pub fn compatible_with(&self, client: &Version) -> bool {
        self.min_launcher_version.as_ref().is_none_or(|minimum| {
            Version::parse(minimum).is_ok_and(|minimum| {
                // Match the API contract: compare the three numeric components, ignoring caller prerelease.
                (client.major, client.minor, client.patch)
                    >= (minimum.major, minimum.minor, minimum.patch)
            })
        })
    }

    pub fn installation_allowed(&self, client: &Version) -> Result<(), CoreError> {
        if !self.enabled {
            return Err(CoreError::new(ErrorCode::InstanceDisabled));
        }
        if !self.compatible_with(client)
            || !matches!(self.source, CatalogSource::Vanilla { .. })
            || self.restricted
        {
            return Err(CoreError::new(ErrorCode::UnsupportedInstance));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct List {
    data: Vec<RawInstance>,
    meta: Count,
}
#[derive(Deserialize)]
struct Count {
    count: usize,
}
#[derive(Deserialize)]
struct Detail {
    data: RawInstance,
}

pub fn parse_list(bytes: &[u8]) -> Result<Vec<CatalogInstance>, CoreError> {
    if bytes.len() > MAX_BODY {
        return Err(invalid());
    }
    let list: List = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if list.data.len() > MAX_RECORDS || list.meta.count != list.data.len() {
        return Err(invalid());
    }
    let mut ids = HashSet::new();
    let mut records = Vec::with_capacity(list.data.len());
    for raw in list.data {
        let record = CatalogInstance::try_from(raw)?;
        if !ids.insert(record.id) {
            return Err(invalid());
        }
        records.push(record);
    }
    records.sort_by_key(|record| (record.sort_order, record.id));
    Ok(records)
}

#[derive(Serialize, Deserialize)]
struct CachedCatalog {
    #[serde(rename = "schema", alias = "schema_version")]
    schema_version: u32,
    #[serde(rename = "source_fingerprint", alias = "representation")]
    representation: String,
    etag: Option<String>,
    fetched_at: u64,
    // Store the API document as structured JSON; accept the legacy string on read.
    #[serde(
        serialize_with = "serialize_cache_body",
        deserialize_with = "deserialize_cache_body"
    )]
    body: String,
}

fn serialize_cache_body<S: serde::Serializer>(
    body: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(serde::ser::Error::custom)?;
    value.serialize(serializer)
}

fn deserialize_cache_body<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(legacy) => legacy,
        document => document.to_string(),
    })
}

impl CachedCatalog {
    fn decode(bytes: &[u8], key: &str) -> Result<Self, CoreError> {
        let mut cache: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let fingerprint = source_fingerprint(key);
        let expected = match cache.schema_version {
            1 | 2 => key,
            3 => &fingerprint,
            _ => return Err(invalid()),
        };
        if cache.representation != expected
            || cache.body.len() > MAX_BODY
            || cache
                .etag
                .as_ref()
                .is_some_and(|tag| tag.len() > 1024 || header::HeaderValue::from_str(tag).is_err())
        {
            return Err(invalid());
        }
        parse_list(cache.body.as_bytes())?;
        cache.representation = fingerprint;
        Ok(cache)
    }
}

fn source_fingerprint(representation: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(representation.as_bytes()))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSnapshot {
    pub instances: Vec<CatalogInstance>,
    pub stale: bool,
    pub cache_warning: Option<ErrorCode>,
}

pub struct CatalogClient {
    client: Client,
    base: String,
}

#[cfg(test)]
impl CatalogClient {
    pub(crate) fn test_transport(self) -> (Client, String) {
        (self.client, self.base)
    }
}

impl CatalogClient {
    pub fn new() -> Result<Self, CoreError> {
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("PanitaClient/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?;
        Ok(Self {
            client,
            base: BASE.to_owned(),
        })
    }

    fn representation(&self) -> String {
        format!("{}/v1/instances|accept:application/json", self.base)
    }

    pub async fn refresh(&self, store: &PrivateStore) -> Result<CatalogSnapshot, CoreError> {
        let key = self.representation();
        let cached = read_cache(store)?
            .map(|bytes| CachedCatalog::decode(&bytes, &key))
            .transpose();
        let cache_invalid = cached.is_err();
        let cached = cached.ok().flatten();
        let mut request = self
            .client
            .get(format!("{}/v1/instances", self.base))
            .header(header::ACCEPT, "application/json");
        if let Some(etag) = cached.as_ref().and_then(|cache| cache.etag.as_ref()) {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let fetched = async {
            let response = request
                .send()
                .await
                .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?;
            if response.status() == StatusCode::NOT_MODIFIED {
                return cached.ok_or_else(invalid);
            }
            check_status(response.status())?;
            let etag = response
                .headers()
                .get(header::ETAG)
                .map(|value| value.to_str().map(str::to_owned))
                .transpose()
                .map_err(|_| invalid())?;
            if etag.as_ref().is_some_and(|value| value.len() > 1024) {
                return Err(invalid());
            }
            let body = body(response).await?;
            parse_list(&body)?;
            Ok(CachedCatalog {
                schema_version: 3,
                representation: source_fingerprint(&key),
                etag,
                fetched_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                body: String::from_utf8(body).map_err(|_| invalid())?,
            })
        }
        .await;
        match fetched {
            Ok(mut cache) => {
                cache.schema_version = 3;
                let instances = parse_list(cache.body.as_bytes())?;
                let bytes = crate::storage_json::pretty(&cache).map_err(|_| invalid())?;
                let cache_warning = store
                    .transaction(CACHE_FILE, |_| Ok((bytes, ())))
                    .err()
                    .map(|error| error.code());
                Ok(CatalogSnapshot {
                    instances,
                    stale: false,
                    cache_warning,
                })
            }
            Err(error)
                if matches!(
                    error.code(),
                    ErrorCode::NetworkUnavailable | ErrorCode::ServiceUnavailable
                ) =>
            {
                if let Some(cache) = read_cache(store)?
                    .map(|bytes| CachedCatalog::decode(&bytes, &key))
                    .transpose()?
                {
                    return Ok(CatalogSnapshot {
                        instances: parse_list(cache.body.as_bytes())?,
                        stale: true,
                        cache_warning: None,
                    });
                }
                if cache_invalid {
                    return Err(invalid());
                }
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    /// Every future Play request must call this, then enforce installation_allowed.
    /// Request revalidation is not a guarantee of origin freshness through a CDN.
    pub async fn detail(&self, id: Uuid) -> Result<CatalogInstance, CoreError> {
        let response = self
            .client
            .get(format!("{}/v1/instances/{id}", self.base))
            .header(header::ACCEPT, "application/json")
            .header(header::CACHE_CONTROL, "no-cache")
            .send()
            .await
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(CoreError::new(ErrorCode::InstanceMissing));
        }
        check_status(response.status())?;
        let detail: Detail =
            serde_json::from_slice(&body(response).await?).map_err(|_| invalid())?;
        let record = CatalogInstance::try_from(detail.data)?;
        if record.id != id {
            return Err(invalid());
        }
        Ok(record)
    }
}

fn check_status(status: StatusCode) -> Result<(), CoreError> {
    if status == StatusCode::OK {
        return Ok(());
    }
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(CoreError::new(ErrorCode::ServiceUnavailable));
    }
    Err(invalid())
}

async fn body(mut response: reqwest::Response) -> Result<Vec<u8>, CoreError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?
    {
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
pub(crate) mod tests;
