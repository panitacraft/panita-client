//! Pinned public version resolution and verified index inspection. Overrides are
//! not extracted here; their publication and ownership belong to installation.
use super::*;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::io::Read;

pub const MAX_MRPACK_BYTES: u64 = 128 * 1024 * 1024;

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
}
