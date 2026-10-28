//! Disposable Play-time verification receipts. Never authoritative installation metadata.
//! Metadata equality avoids repeated content reads; it is not proof against tampering
//! that preserves metadata. Full installation/repair audits never consult this cache.
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use private_fs::PrivateDir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::download::{Cancellation, io, native_file, verified_open_file};
use super::install::{InstalledLock, read_json, write_json};
use super::{Artifact, invalid};
use crate::error::{CoreError, ErrorCode};
use crate::modpack::{
    gamefs::{Content, GameDir},
    state::{Ownership, PackState},
    transfer::Expected,
};
use crate::storage::map_private_error;

const FILE: &str = "verification-cache.json";

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VerificationCache {
    #[serde(rename = "schema", alias = "schema_version")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    size: u64,
    stamp: Vec<u64>,
}

impl VerificationCache {
    /// The caller holds the instance installation lock through load, audit and save.
    pub fn load(root: &Path, installed: &InstalledLock) -> Result<Self, CoreError> {
        let schema_version = if installed.pack.is_some() {
            4
        } else if installed.plan.loader.is_some() {
            3
        } else {
            2
        };
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
            .filter(|cache| {
                cache.schema_version == schema_version && cache.manifest_hash == manifest_hash
            })
            .unwrap_or_else(|| Self {
                schema_version,
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
        self.verify_expected(directory, leaf, key, artifact, executable, None)
    }

    /// Read-only receipt lookup, safe to run from several threads. `Some(true)` is a
    /// receipt hit with the file still unchanged, `Some(false)` a missing or resized
    /// file, and `None` a file that needs the full check of `verify`.
    pub fn check(
        &self,
        directory: &PrivateDir,
        leaf: &str,
        key: &str,
        artifact: &Artifact,
        executable: bool,
    ) -> Result<Option<bool>, CoreError> {
        #[cfg(target_os = "linux")]
        if executable {
            return Ok(None);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = executable;
        let file = match directory.open_file(leaf) {
            Ok(file) => file,
            Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Some(false));
            }
            Err(error) => return Err(map_private_error(error)),
        };
        let input = native_file(&file)?;
        let metadata = input.metadata().map_err(io)?;
        if metadata.len() != artifact.size {
            return Ok(Some(false));
        }
        let before = stamp(&metadata);
        let hit = self.entries.get(key).is_some_and(|receipt| {
            receipt.sha1 == artifact.sha1
                && receipt.sha256.is_none()
                && receipt.size == artifact.size
                && receipt.stamp == before
        });
        if !hit {
            return Ok(None);
        }
        // Recheck the retained protected handle even when content reads are skipped.
        file.verify().map_err(map_private_error)?;
        if stamp(&input.metadata().map_err(io)?) != before {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        Ok(Some(true))
    }

    pub fn verify_sha256(
        &mut self,
        directory: &PrivateDir,
        leaf: &str,
        key: &str,
        artifact: &Artifact,
        sha256: &str,
    ) -> Result<bool, CoreError> {
        self.verify_expected(directory, leaf, key, artifact, false, Some(sha256))
    }

    fn verify_expected(
        &mut self,
        directory: &PrivateDir,
        leaf: &str,
        key: &str,
        artifact: &Artifact,
        executable: bool,
        sha256: Option<&str>,
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
        let mut input = native_file(&file)?;
        let metadata = input.metadata().map_err(io)?;
        if metadata.len() != artifact.size {
            return Ok(false);
        }
        let before = stamp(&metadata);
        if self.entries.get(key).is_some_and(|receipt| {
            receipt.sha1 == artifact.sha1
                && receipt.sha256.as_deref() == sha256
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
        if let Some(expected) = sha256 {
            input.seek(SeekFrom::Start(0)).map_err(io)?;
            let mut hash = Sha256::new();
            let mut buffer = [0; 64 * 1024];
            let mut count = 0u64;
            loop {
                let size = input.read(&mut buffer).map_err(io)?;
                if size == 0 {
                    break;
                }
                count += size as u64;
                if count > artifact.size {
                    return Ok(false);
                }
                hash.update(&buffer[..size]);
            }
            file.verify().map_err(map_private_error)?;
            if count != artifact.size || format!("{:x}", hash.finalize()) != expected {
                return Ok(false);
            }
        }
        // Never attach an old hash result to metadata changed during the read.
        if stamp(&input.metadata().map_err(io)?) != before {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        self.entries.insert(
            key.into(),
            Receipt {
                sha1: artifact.sha1.clone(),
                sha256: sha256.map(str::to_owned),
                size: artifact.size,
                stamp: before,
            },
        );
        self.dirty = true;
        Ok(true)
    }

    /// Managed pack content files in the game directory. A receipt is reused only
    /// while the file's size, times and identity are exactly as when it was hashed;
    /// any change, and any file without a receipt, is hashed in full.
    pub fn verify_pack(
        &mut self,
        game: &GameDir,
        pack: &PackState,
        cancel: &Cancellation,
    ) -> Result<bool, CoreError> {
        for file in pack
            .files
            .iter()
            .filter(|file| file.ownership == Ownership::Content)
        {
            cancel.check()?;
            let expected = Expected::file(&file.desired, file.size);
            let id = expected
                .sha512
                .clone()
                .or_else(|| expected.sha1.clone())
                .ok_or_else(invalid)?;
            let key = format!("pack/{}", file.path);
            let Some(before) = game.stamp(&file.path)? else {
                return Ok(false);
            };
            if self.entries.get(&key).is_some_and(|receipt| {
                receipt.sha1 == id && receipt.size == file.size && receipt.stamp == before
            }) {
                #[cfg(test)]
                {
                    self.reused += 1;
                }
                continue;
            }
            #[cfg(test)]
            {
                self.hashed += 1;
            }
            if game.compare(&file.path, &expected, cancel)? != Content::Matches {
                return Ok(false);
            }
            // Never attach an old hash result to metadata changed during the read.
            if game.stamp(&file.path)?.as_ref() != Some(&before) {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
            self.entries.insert(
                key,
                Receipt {
                    sha1: id,
                    sha256: None,
                    size: file.size,
                    stamp: before,
                },
            );
            self.dirty = true;
        }
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

#[cfg(test)]
mod pack_tests {
    use super::*;
    use crate::modpack::{
        Hashes,
        gamefs::tests::game,
        state::{ManagedFile, PackIdentity},
    };
    use sha2::Sha512;
    use uuid::Uuid;

    fn pack(body: &[u8]) -> PackState {
        let hashes = Hashes {
            sha1: None,
            sha512: Some(format!("{:x}", Sha512::digest(body))),
        };
        PackState {
            schema: 1,
            id: Uuid::nil(),
            source: PackIdentity::Custom {
                version_id: Uuid::new_v4(),
                published_at: "2026-10-25T20:54:25Z".into(),
                label: None,
            },
            minecraft_version: "1.21.1".into(),
            loader: "neoforge".into(),
            loader_version: "21.1.264".into(),
            files: vec![ManagedFile {
                path: "mods/a.jar".into(),
                ownership: Ownership::Content,
                desired: hashes.clone(),
                size: body.len() as u64,
                baseline: Some(hashes),
            }],
            consumed_defaults: Default::default(),
            skipped_configuration: Vec::new(),
            pending: Vec::new(),
        }
    }

    #[test]
    fn unchanged_pack_files_are_not_hashed_again_but_any_change_is() {
        let (root, game) = game();
        let body = b"pack mod bytes".to_vec();
        let path = root.join("minecraft/mods/a.jar");
        assert!(std::fs::create_dir_all(path.parent().unwrap_or(&root)).is_ok());
        assert!(std::fs::write(&path, &body).is_ok());
        let (state, cancel) = (pack(&body), Cancellation::default());
        let mut cache = VerificationCache::default();
        assert_eq!(cache.verify_pack(&game, &state, &cancel).ok(), Some(true));
        assert_eq!((cache.hashed, cache.reused), (1, 0));
        // The receipt is reused while size, times and identity are unchanged.
        assert_eq!(cache.verify_pack(&game, &state, &cancel).ok(), Some(true));
        assert_eq!((cache.hashed, cache.reused), (1, 1));
        // Same size, different bytes, different time: hashed again and rejected.
        assert!(std::fs::write(&path, b"PACK MOD BYTES").is_ok());
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_modified(later))
                .is_ok()
        );
        assert_eq!(cache.verify_pack(&game, &state, &cancel).ok(), Some(false));
        assert_eq!(cache.hashed, 2);
        // A touched but identical file is hashed once more, then accepted again.
        assert!(std::fs::write(&path, &body).is_ok());
        assert_eq!(cache.verify_pack(&game, &state, &cancel).ok(), Some(true));
        assert_eq!(cache.hashed, 3);
        // A receipt never covers a file that is gone.
        assert!(std::fs::remove_file(&path).is_ok());
        assert_eq!(cache.verify_pack(&game, &state, &cancel).ok(), Some(false));
        // Receipts do not survive a different expected hash.
        assert!(std::fs::write(&path, &body).is_ok());
        let mut other = pack(b"another expected file");
        other.files[0].size = body.len() as u64;
        assert_eq!(cache.verify_pack(&game, &other, &cancel).ok(), Some(false));
        let _ = std::fs::remove_dir_all(&root);
    }
}
