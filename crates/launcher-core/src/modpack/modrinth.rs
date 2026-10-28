//! Pinned public version resolution and verified index inspection. Overrides are
//! not extracted here; their publication and ownership belong to installation.
use super::*;
use reqwest::{Client, StatusCode, header};
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::io::Read;
use std::time::Duration;

pub const MAX_MRPACK_BYTES: u64 = 128 * 1024 * 1024;
const API: &str = "https://api.modrinth.com";

/// Public Modrinth version metadata. It sends no credentials and follows no
/// redirects, and it only ever asks for the exact version the catalog pins.
pub struct ModrinthClient {
    client: Client,
    base: String,
}

impl ModrinthClient {
    pub fn new() -> Result<Self, CoreError> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!(
                "PanitaClient/",
                env!("CARGO_PKG_VERSION"),
                " (+https://www.panitacraft.com/)"
            ))
            .build()
            .map_err(|_| invalid())?;
        Ok(Self {
            client,
            base: API.to_owned(),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_tests(base: &str) -> Self {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| panic!("client"));
        Self {
            client,
            base: base.to_owned(),
        }
    }

    /// Resolve the pinned version of a project to its pack archive. A response for a
    /// different project or version, or an ambiguous archive, is rejected.
    pub async fn pack_artifact(
        &self,
        project: &str,
        version: &str,
    ) -> Result<PackArtifact, CoreError> {
        if !pin(project) || !pin(version) {
            return Err(invalid());
        }
        let mut response = self
            .client
            .get(format!("{}/v2/version/{version}", self.base))
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Err(CoreError::new(ErrorCode::InstanceMissing));
        }
        if status == StatusCode::REQUEST_TIMEOUT
            || status == StatusCode::TOO_MANY_REQUESTS
            || status.is_server_error()
        {
            return Err(CoreError::new(ErrorCode::ServiceUnavailable));
        }
        if status != StatusCode::OK
            || response
                .content_length()
                .is_some_and(|length| length > MAX_DOCUMENT_BYTES as u64)
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
        resolve_version(&bytes, project, version)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackArtifact {
    pub url: String,
    pub hashes: Hashes,
    pub size: u64,
}

#[derive(Deserialize)]
struct Version {
    id: String,
    project_id: String,
    status: String,
    date_published: String,
    files: Vec<VersionFile>,
}
#[derive(Deserialize)]
struct VersionFile {
    filename: String,
    primary: bool,
    url: String,
    hashes: Hashes,
    size: u64,
}

pub fn resolve_version(
    bytes: &[u8],
    project: &str,
    version: &str,
) -> Result<PackArtifact, CoreError> {
    if bytes.len() > MAX_DOCUMENT_BYTES || !pin(project) || !pin(version) {
        return Err(invalid());
    }
    let metadata: Version = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if metadata.id != version
        || metadata.project_id != project
        || !matches!(metadata.status.as_str(), "listed" | "unlisted" | "archived")
        || !date(&metadata.date_published)
        || metadata.files.len() > 64
    {
        return Err(invalid());
    }
    let candidates: Vec<_> = metadata
        .files
        .iter()
        .filter(|file| file.filename.ends_with(".mrpack"))
        .collect();
    let primaries: Vec<_> = candidates.iter().filter(|file| file.primary).collect();
    let selected = match (primaries.len(), candidates.len()) {
        (1, _) => *primaries[0],
        (0, 1) => candidates[0],
        _ => return Err(invalid()),
    };
    safe_path(&selected.filename)?;
    selected.hashes.validate()?;
    let url = download_url(&selected.url, &Origin::Modrinth)?;
    let parts: Vec<_> = url.path().split('/').collect();
    if selected.filename.contains('/')
        || selected.hashes.sha1.is_none()
        || selected.hashes.sha512.is_none()
        || selected.size == 0
        || selected.size > MAX_MRPACK_BYTES
        || parts[2] != project
        || parts[4] != version
    {
        return Err(invalid());
    }
    Ok(PackArtifact {
        url: selected.url.clone(),
        hashes: selected.hashes.clone(),
        size: selected.size,
    })
}

pub fn verified_index(bytes: &[u8], artifact: &PackArtifact) -> Result<MrpackIndex, CoreError> {
    artifact.hashes.validate()?;
    download_url(&artifact.url, &Origin::Modrinth)?;
    if artifact.size > MAX_MRPACK_BYTES
        || bytes.len() as u64 != artifact.size
        || artifact.hashes.sha512.as_ref() != Some(&format!("{:x}", Sha512::digest(bytes)))
        || artifact.hashes.sha1.as_ref() != Some(&format!("{:x}", Sha1::digest(bytes)))
    {
        return Err(CoreError::new(ErrorCode::ArtifactMismatch));
    }
    let declared_entries = bounded_entry_count(bytes)?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| invalid())?;
    if archive.len() != declared_entries {
        return Err(invalid());
    }
    let mut index = None;
    for number in 0..archive.len() {
        let entry = archive.by_index_raw(number).map_err(|_| invalid())?;
        if entry.name() == "modrinth.index.json"
            && (index.replace(number).is_some() || entry.size() > MAX_DOCUMENT_BYTES as u64)
        {
            return Err(invalid());
        }
    }
    let entry = archive
        .by_index(index.ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    if entry
        .unix_mode()
        .is_some_and(|mode| mode & 0o170000 == 0o120000)
    {
        return Err(invalid());
    }
    let mut document = Vec::new();
    entry
        .take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut document)
        .map_err(|_| invalid())?;
    MrpackIndex::parse(&document)
}

/// Bound allocation before the ZIP reader builds its directory map. Comparing
/// the original count afterwards rejects duplicate names hidden by that map.
fn bounded_entry_count(bytes: &[u8]) -> Result<usize, CoreError> {
    let start = bytes.len().saturating_sub(22 + u16::MAX as usize);
    directory_entry_count(&bytes[start..], start as u64)
}

/// `tail` is the end of the archive starting at absolute offset `base`; it must
/// hold the whole end-of-central-directory record and its comment.
pub(super) fn directory_entry_count(tail: &[u8], base: u64) -> Result<usize, CoreError> {
    if tail.len() < 22 {
        return Err(invalid());
    }
    for position in (0..=tail.len() - 22).rev() {
        let footer = &tail[position..];
        if footer[..4] != *b"PK\x05\x06" {
            continue;
        }
        let field = |at: usize| u16::from_le_bytes([footer[at], footer[at + 1]]);
        if footer.len() != 22 + field(20) as usize {
            continue;
        }
        let count = field(10) as usize;
        if field(4) != 0 || field(6) != 0 || field(8) as usize != count || count > MAX_FILES {
            return Err(invalid());
        }
        let size = u32::from_le_bytes(footer[12..16].try_into().map_err(|_| invalid())?);
        let offset = u32::from_le_bytes(footer[16..20].try_into().map_err(|_| invalid())?);
        if size == u32::MAX
            || offset == u32::MAX
            || offset as u64 + size as u64 != base + position as u64
        {
            return Err(invalid());
        }
        return Ok(count);
    }
    Err(invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_http::{Reply, Server};
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn duplicate_zip_names_and_unbounded_directory_counts_are_rejected() {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in ["modrinth.index.json", "modrinth.indeX.json"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap_or_else(|_| panic!("start"));
            zip.write_all(b"{}").unwrap_or_else(|_| panic!("write"));
        }
        let mut bytes = zip
            .finish()
            .unwrap_or_else(|_| panic!("finish"))
            .into_inner();
        let name = b"modrinth.indeX.json";
        for at in 0..=bytes.len() - name.len() {
            if &bytes[at..at + name.len()] == name {
                bytes[at + 13] = b'x';
            }
        }
        let artifact = PackArtifact {
            url: "https://cdn.modrinth.com/data/p/versions/v/pack.mrpack".into(),
            hashes: Hashes {
                sha1: Some(format!("{:x}", Sha1::digest(&bytes))),
                sha512: Some(format!("{:x}", Sha512::digest(&bytes))),
            },
            size: bytes.len() as u64,
        };
        assert!(verified_index(&bytes, &artifact).is_err());
        let footer = bytes.len() - 22;
        bytes[footer + 8..footer + 12].fill(255);
        assert!(bounded_entry_count(&bytes).is_err());
    }

    #[test]
    fn version_identity_and_archive_hashes_gate_index_reading() {
        let index = json!({"formatVersion":1,"game":"minecraft","versionId":"internal-label", "name":"Fixture", "files":[], "dependencies":{"minecraft":"1.21.1","neoforge":"21.1.248"}});
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            "modrinth.index.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap_or_else(|_| panic!("start"));
        zip.write_all(index.to_string().as_bytes())
            .unwrap_or_else(|_| panic!("write"));
        let bytes = zip
            .finish()
            .unwrap_or_else(|_| panic!("finish"))
            .into_inner();
        let metadata = json!({"id":"version", "project_id":"project", "status":"listed", "date_published":"2026-10-25T20:54:25Z", "files":[{"filename":"pack.mrpack", "primary":true,"url":"https://cdn.modrinth.com/data/project/versions/version/pack.mrpack", "size":bytes.len(), "hashes":{"sha1":format!("{:x}",Sha1::digest(&bytes)), "sha512":format!("{:x}",Sha512::digest(&bytes))}}]});
        let artifact = resolve_version(metadata.to_string().as_bytes(), "project", "version")
            .unwrap_or_else(|_| panic!("version"));
        let parsed = verified_index(&bytes, &artifact).unwrap_or_else(|_| panic!("index"));
        assert_eq!(parsed.version_id, "internal-label");
        assert_eq!(
            parsed.dependencies.get("neoforge").map(String::as_str),
            Some("21.1.248")
        );
        assert!(resolve_version(metadata.to_string().as_bytes(), "wrong", "version").is_err());
        let mut corrupt = bytes.clone();
        corrupt[0] ^= 1;
        assert_eq!(
            verified_index(&corrupt, &artifact).err().map(|e| e.code()),
            Some(ErrorCode::ArtifactMismatch)
        );
    }

    fn pinned(project: &str, version: &str) -> String {
        json!({"id": version, "project_id": project, "status": "listed",
            "date_published": "2026-10-25T20:54:25Z",
            "files": [{"filename": "pack.mrpack", "primary": true,
                "url": format!("https://cdn.modrinth.com/data/{project}/versions/{version}/pack.mrpack"),
                "size": 100, "hashes": {"sha1": "a".repeat(40), "sha512": "b".repeat(128)}}]})
        .to_string()
    }

    fn code<T>(result: Result<T, CoreError>) -> Option<ErrorCode> {
        result.err().map(|error| error.code())
    }

    #[tokio::test]
    async fn the_pinned_version_resolves_without_credentials_redirects_or_substitution() {
        let found = pinned("project", "version");
        let server = Server::start(move |request| match request.path.as_str() {
            "/v2/version/version" => Reply::ok(found.clone()),
            // The document names another version than the one that was requested.
            "/v2/version/other" => Reply::ok(pinned("project", "version")),
            "/v2/version/moved" => {
                Reply::status(302).header("Location", "https://evil.example/v2/version/moved")
            }
            "/v2/version/gone" => Reply::status(404),
            "/v2/version/busy" => Reply::status(503),
            "/v2/version/limited" => Reply::status(429),
            "/v2/version/huge" => Reply::ok(vec![b' '; MAX_DOCUMENT_BYTES + 1]),
            _ => Reply::status(500),
        });
        let client = ModrinthClient::for_tests(server.base());
        let artifact = client
            .pack_artifact("project", "version")
            .await
            .unwrap_or_else(|_| panic!("artifact"));
        assert_eq!(artifact.size, 100);
        assert_eq!(
            code(client.pack_artifact("someone-else", "version").await),
            Some(ErrorCode::InvalidData)
        );
        for (pin, expected) in [
            ("other", ErrorCode::InvalidData),
            ("moved", ErrorCode::InvalidData),
            ("gone", ErrorCode::InstanceMissing),
            ("busy", ErrorCode::ServiceUnavailable),
            ("limited", ErrorCode::ServiceUnavailable),
            ("huge", ErrorCode::InvalidData),
        ] {
            assert_eq!(
                code(client.pack_artifact("project", pin).await),
                Some(expected),
                "{pin}"
            );
        }
        // A malformed pin never reaches the network.
        let before = server.requests().len();
        assert!(client.pack_artifact("project", "../version").await.is_err());
        assert!(client.pack_artifact("", "version").await.is_err());
        assert_eq!(server.requests().len(), before);
        for request in server.requests() {
            for forbidden in ["authorization", "x-api-key", "cookie"] {
                assert!(!request.headers.contains(forbidden), "{forbidden}");
            }
        }
    }
}
