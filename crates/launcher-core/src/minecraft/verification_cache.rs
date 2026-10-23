//! Disposable Play-time verification receipts. Never authoritative installation metadata.
//! Metadata equality avoids repeated content reads; it is not proof against tampering
//! that preserves metadata. Full installation/repair audits never consult this cache.
use std::collections::BTreeMap;
use std::path::Path;

use private_fs::PrivateDir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::download::{io, native_file, verified_open_file};
use super::install::{InstalledLock, read_json, write_json};
use super::{Artifact, invalid};
use crate::error::{CoreError, ErrorCode};
use crate::storage::map_private_error;

const FILE: &str = "verification-cache.json";

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VerificationCache {
    schema_version: u32,
    manifest_hash: String,
    entries: BTreeMap<String, Receipt>,
    #[serde(skip)]
    dirty: bool,
    #[cfg(test)]
    #[serde(skip)]
    pub hashed: usize,
    #[cfg(test)]
    #[serde(skip)]
    pub reused: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    sha1: String,
    size: u64,
    stamp: Vec<u64>,
}

impl VerificationCache {
    /// The caller holds the instance installation lock through load, audit and save.
    pub fn load(root: &Path, installed: &InstalledLock) -> Result<Self, CoreError> {
        let manifest_hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(installed).map_err(|_| invalid())?)
        );
        let cached = match read_json::<Self>(root, FILE) {
            Ok(cached) => cached,
            // Missing, malformed or unsupported receipts require fresh content checks.
            Err(error) if error.code() == ErrorCode::InvalidData => None,
            // Redirected paths and permission failures still fail closed.
            Err(error) => return Err(error),
        };
        Ok(cached
            .filter(|cache| cache.schema_version == 1 && cache.manifest_hash == manifest_hash)
            .unwrap_or_else(|| Self {
                schema_version: 1,
                manifest_hash,
                dirty: true,
                ..Self::default()
            }))
    }

    pub fn verify(
        &mut self,
        directory: &PrivateDir,
        leaf: &str,
        key: &str,
        artifact: &Artifact,
        executable: bool,
    ) -> Result<bool, CoreError> {
        // Linux executable permissions use a dedicated no-follow reader. Keep its
        // full check rather than passing 0700 files through the 0600 file boundary.
        #[cfg(target_os = "linux")]
        if executable {
            return super::executable::matches(directory, leaf, artifact);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = executable;
        let file = match directory.open_file(leaf) {
            Ok(file) => file,
            Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => return Err(map_private_error(error)),
        };
        let input = native_file(&file)?;
        let metadata = input.metadata().map_err(io)?;
        if metadata.len() != artifact.size {
            return Ok(false);
        }
        let before = stamp(&metadata);
        if self.entries.get(key).is_some_and(|receipt| {
            receipt.sha1 == artifact.sha1
                && receipt.size == artifact.size
                && receipt.stamp == before
        }) {
            // Recheck the retained protected handle even when content reads are skipped.
            file.verify().map_err(map_private_error)?;
            if stamp(&input.metadata().map_err(io)?) != before {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
            #[cfg(test)]
            {
                self.reused += 1;
            }
            return Ok(true);
        }
        #[cfg(test)]
        {
            self.hashed += 1;
        }
        if !verified_open_file(&file, artifact)? {
            return Ok(false);
        }
        // Never attach an old hash result to metadata changed during the read.
        if stamp(&input.metadata().map_err(io)?) != before {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        self.entries.insert(
            key.into(),
            Receipt {
                sha1: artifact.sha1.clone(),
                size: artifact.size,
                stamp: before,
            },
        );
        self.dirty = true;
        Ok(true)
    }

    pub fn save(&self, root: &Path) {
        if self.dirty {
            // A failed optimization-cache write does not invalidate the completed
            // audit. Atomic replacement preserves the old cache; a later Play retries.
            let _ = write_json(root, FILE, self);
        }
    }
}

#[cfg(windows)]
fn stamp(metadata: &std::fs::Metadata) -> Vec<u64> {
    use std::os::windows::fs::MetadataExt;
    vec![
        metadata.file_size(),
        metadata.creation_time(),
        metadata.last_write_time(),
        u64::from(metadata.file_attributes()),
    ]
}

#[cfg(unix)]
fn stamp(metadata: &std::fs::Metadata) -> Vec<u64> {
    use std::os::unix::fs::MetadataExt;
    vec![
        metadata.len(),
        metadata.dev(),
        metadata.ino(),
        metadata.mtime() as u64,
        metadata.mtime_nsec() as u64,
        metadata.ctime() as u64,
        metadata.ctime_nsec() as u64,
        u64::from(metadata.mode()),
    ]
}
