//! Identity-bound public manifest revalidation. A cache hit is content reuse,
//! never proof of current enabled status, visibility, or Play availability.
use super::*;
use crate::panita_api::PanitaCredentials;
use reqwest::{Client, StatusCode, header};
use std::time::Duration;

const BASE: &str = "https://api.panitacraft.com";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedManifest {
    schema: u32,
    origin: String,
    id: Uuid,
    version_id: Uuid,
    etag: Option<String>,
    manifest: Manifest,
}

fn valid_etag(value: &str) -> bool {
    let tag = value.strip_prefix("W/").unwrap_or(value);
    tag.len() >= 2
        && tag.len() <= 1024
        && tag.starts_with('"')
        && tag.ends_with('"')
        && tag.as_bytes()[1..tag.len() - 1]
            .iter()
            .all(|c| *c >= 0x21 && *c != b'"' && *c != 0x7f)
}

impl CachedManifest {
    pub fn decode(bytes: &[u8], id: Uuid, version: Uuid) -> Result<Self, CoreError> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid());
        }
        let cached: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        cached.validate(BASE, id, version)?;
        Ok(cached)
    }
    fn validate(&self, origin: &str, id: Uuid, version: Uuid) -> Result<(), CoreError> {
        if self.schema != 1
            || self.origin != origin
            || self.id != id
            || self.version_id != version
            || self.etag.as_deref().is_some_and(|v| !valid_etag(v))
        {
            return Err(invalid());
        }
        self.manifest.validate(id, version)
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    /// The caller must use protected atomic storage under its writer lock.
    pub fn encode(&self) -> Result<Vec<u8>, CoreError> {
        self.validate(BASE, self.id, self.version_id)?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|_| invalid())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid());
        }
        Ok(bytes)
    }
}

pub struct ManifestClient {
    client: Client,
    base: String,
    /// A restricted instance's manifest is only served to a request that carries
    /// the launcher key and the member session; public ones ignore both.
    auth: Option<PanitaCredentials>,
}
impl ManifestClient {
    pub fn new() -> Result<Self, CoreError> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent("PanitaClient/0.2 (https://github.com/panitacraft/panita-client)")
            .build()
            .map_err(|_| invalid())?;
        Ok(Self {
            client,
            base: BASE.to_owned(),
            auth: None,
        })
    }

    pub fn with_auth(mut self, auth: impl Into<Option<PanitaCredentials>>) -> Self {
        self.auth = auth.into();
        self
    }

    /// Credentials only reach the Panita API. No transport retry, or fallback on
    /// explicit missing responses.
    pub async fn fetch(
        &self,
        id: Uuid,
        version: Uuid,
        cached: Option<&CachedManifest>,
    ) -> Result<CachedManifest, CoreError> {
        if id.is_nil() || version.is_nil() {
            return Err(invalid());
        }
        if let Some(cached) = cached {
            cached.validate(&self.base, id, version)?;
        }
        let mut request = self
            .client
            .get(format!(
                "{}/v1/instances/{id}/versions/{version}/manifest",
                self.base
            ))
            .header(header::ACCEPT, "application/json")
            .header(header::CACHE_CONTROL, "no-cache");
        if let Some(auth) = &self.auth {
            request = auth.apply(request);
        }
        let validator = cached.and_then(|c| c.etag.as_deref());
        if let Some(etag) = validator {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?;
        if response.status() == StatusCode::NOT_MODIFIED {
            if validator.is_none() {
                return Err(invalid());
            }
            if response
                .headers()
                .get(header::ETAG)
                .is_some_and(|tag| tag.to_str().ok() != validator)
            {
                return Err(invalid());
            }
            return cached.cloned().ok_or_else(invalid);
        }
        if response.status() == StatusCode::NOT_FOUND {
            return Err(CoreError::new(ErrorCode::InstanceMissing));
        }
        if response.status() == StatusCode::TOO_MANY_REQUESTS || response.status().is_server_error()
        {
            return Err(CoreError::new(ErrorCode::ServiceUnavailable));
        }
        if response.status() != StatusCode::OK {
            return Err(invalid());
        }
        let etag = response
            .headers()
            .get(header::ETAG)
            .map(|v| v.to_str().map(str::to_owned))
            .transpose()
            .map_err(|_| invalid())?;
        if etag.as_deref().is_some_and(|tag| !valid_etag(tag))
            || response
                .content_length()
                .is_some_and(|n| n > MAX_DOCUMENT_BYTES as u64)
        {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?
        {
            if chunk.len() > MAX_DOCUMENT_BYTES - bytes.len() {
                return Err(invalid());
            }
            bytes.extend_from_slice(&chunk);
        }
        let manifest = Manifest::parse(&bytes, id, version)?;
        if cached.is_some_and(|c| c.manifest != manifest) {
            return Err(invalid());
        }
        Ok(CachedManifest {
            schema: 1,
            origin: self.base.clone(),
            id,
            version_id: version,
            etag,
            manifest,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::server;

    #[tokio::test]
    async fn missing_validator_and_service_errors_do_not_become_cache_success() {
        let (id, version, document) = super::super::tests::fixture();
        let (catalog, task) = server(vec![
            (304, String::new(), false),
            (503, String::new(), false),
        ]);
        let (client, base) = catalog.test_transport();
        let client = ManifestClient {
            client,
            base,
            auth: None,
        };
        assert!(client.fetch(id, version, None).await.is_err());
        assert_eq!(
            client
                .fetch(id, version, None)
                .await
                .err()
                .map(|e| e.code()),
            Some(ErrorCode::ServiceUnavailable)
        );
        task.join().unwrap_or_else(|_| panic!("server"));
        let cached = CachedManifest {
            schema: 1,
            origin: BASE.into(),
            id,
            version_id: version,
            etag: Some("W/\"fixture\"".into()),
            manifest: Manifest::parse(document.to_string().as_bytes(), id, version)
                .unwrap_or_else(|_| panic!("manifest")),
        };
        let bytes = cached.encode().unwrap_or_else(|_| panic!("cache"));
        assert!(bytes.ends_with(b"\n"));
        assert!(CachedManifest::decode(&bytes, id, version).is_ok());
        assert!(CachedManifest::decode(&bytes, id, Uuid::new_v4()).is_err());
        for (field, value) in [
            ("schema", serde_json::json!(99)),
            ("origin", serde_json::json!("https://other.test")),
            ("etag", serde_json::json!("unquoted")),
            ("schema_version", serde_json::json!(1)),
        ] {
            let mut bad: serde_json::Value =
                serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("JSON"));
            bad[field] = value;
            assert!(
                CachedManifest::decode(bad.to_string().as_bytes(), id, version).is_err(),
                "{field}"
            );
        }
    }

    #[tokio::test]
    async fn conditional_reuse_is_bound_and_never_hides_missing_or_changed_content() {
        let (id, version, document) = super::super::tests::fixture();
        let body = document.to_string();
        let mut changed = document.clone();
        changed["data"]["loader_version"] = serde_json::json!("21.1.265");
        let (catalog, task) = server(vec![
            (200, body, false),
            (304, String::new(), true),
            (200, changed.to_string(), true),
            (404, String::new(), true),
        ]);
        let (client, base) = catalog.test_transport();
        let client = ManifestClient {
            client,
            base,
            auth: None,
        };
        let cached = client
            .fetch(id, version, None)
            .await
            .unwrap_or_else(|_| panic!("fetch"));
        assert!(client.fetch(id, version, Some(&cached)).await.is_ok());
        assert!(client.fetch(id, version, Some(&cached)).await.is_err());
        assert_eq!(
            client
                .fetch(id, version, Some(&cached))
                .await
                .err()
                .map(|e| e.code()),
            Some(ErrorCode::InstanceMissing)
        );
        assert!(
            client
                .fetch(Uuid::new_v4(), version, Some(&cached))
                .await
                .is_err()
        );
        task.join().unwrap_or_else(|_| panic!("server"));
    }

    #[tokio::test]
    async fn a_restricted_manifest_is_fetched_only_with_the_key_and_the_session() {
        use crate::test_http::{Reply, Server};
        let (id, version, document) = super::super::tests::fixture();
        let body = document.to_string();
        // Like the API: a restricted manifest answers 404 to a request without both.
        let server = Server::start(move |request| {
            if request.headers.contains("x-api-key") && request.headers.contains("authorization") {
                Reply::ok(body.clone())
            } else {
                Reply::status(404)
            }
        });
        let client = || {
            let client = Client::builder()
                .no_proxy()
                .build()
                .unwrap_or_else(|_| panic!("client"));
            ManifestClient {
                client,
                base: server.base().to_owned(),
                auth: None,
            }
        };
        assert_eq!(
            client()
                .fetch(id, version, None)
                .await
                .err()
                .map(|e| e.code()),
            Some(ErrorCode::InstanceMissing)
        );
        let credentials = crate::panita_api::tests::credentials()
            .with_session(&crate::panita_api::tests::session("ps_member"));
        assert!(
            client()
                .with_auth(credentials)
                .fetch(id, version, None)
                .await
                .is_ok()
        );
    }
}
