//! Extract only bounded regular files into a private generation directory.
use super::download::{Cancellation, ensure_parent, io};
use super::{invalid, safe_path};
use crate::{error::CoreError, storage::map_private_error};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::io::{Read, Seek, Write};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeFile {
    pub path: String,
    pub sha1: String,
    pub size: u64,
}

pub fn extract(
    reader: impl Read + Seek,
    root: &Path,
    excludes: &[String],
    cancel: &Cancellation,
) -> Result<Vec<NativeFile>, CoreError> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|_| invalid())?;
    if archive.len() > 10_000 {
        return Err(invalid());
    }
    let mut seen = HashSet::new();
    let mut total = 0u64;
    let mut files = Vec::new();
    for index in 0..archive.len() {
        cancel.check()?;
        let mut entry = archive.by_index(index).map_err(|_| invalid())?;
        let name = entry.name().trim_end_matches('/').to_owned();
        safe_path(&name)?;
        if name == ".generation.json" {
            return Err(invalid());
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(invalid());
        }
        if let Some(mode) = entry.unix_mode()
            && mode & 0o170000 != 0
            && mode & 0o170000 != 0o100000
            && mode & 0o170000 != 0o040000
        {
            return Err(invalid());
        }
        if entry.is_dir() {
            continue;
        }
        total = total.checked_add(entry.size()).ok_or_else(invalid)?;
        if total > 512 * 1024 * 1024 || entry.size() > 128 * 1024 * 1024 {
            return Err(invalid());
        }
        if excludes.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        let (directory, leaf) = ensure_parent(root, &name)?;
        // Different archives may contain identical files, but never overwrite conflicting natives.
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes.len() as u64 != entry.size() {
            return Err(invalid());
        }
        let sha1 = format!("{:x}", Sha1::digest(&bytes));
        match directory.open_file(&leaf) {
            Ok(mut existing) => {
                let mut previous = Vec::new();
                Read::by_ref(&mut existing)
                    .take(bytes.len() as u64 + 1)
                    .read_to_end(&mut previous)
                    .map_err(io)?;
                if previous != bytes {
                    return Err(invalid());
                }
            }
            Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut output = directory.atomic_durable(&leaf).map_err(map_private_error)?;
                output.write_all(&bytes).map_err(io)?;
                output.commit().map_err(map_private_error)?;
            }
            Err(error) => return Err(map_private_error(error)),
        }
        files.push(NativeFile {
            path: name,
            sha1,
            size: bytes.len() as u64,
        });
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn archive(name: &str) -> Cursor<Vec<u8>> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap_or_else(|_| panic!("entry"));
        writer
            .write_all(b"native")
            .unwrap_or_else(|_| panic!("write"));
        let mut cursor = writer.finish().unwrap_or_else(|_| panic!("finish"));
        cursor.set_position(0);
        cursor
    }
    #[test]
    fn confined_extraction_rejects_traversal_and_preserves_conflicting_files() {
        let root = std::env::temp_dir().join(format!("panita-natives-{}", uuid::Uuid::new_v4()));
        private_fs::PrivateDir::ensure(&root).unwrap_or_else(|_| panic!("root"));
        for name in ["../escape", "a/../../escape", "C:/escape", "CON.dll"] {
            assert!(extract(archive(name), &root, &[], &Cancellation::default()).is_err());
        }
        assert!(
            extract(
                archive("a/native.dll"),
                &root,
                &[],
                &Cancellation::default()
            )
            .is_ok()
        );
        let (directory, leaf) =
            ensure_parent(&root, "a/native.dll").unwrap_or_else(|_| panic!("parent"));
        directory
            .write_atomic_durable(leaf, b"existing user file")
            .unwrap_or_else(|_| panic!("existing"));
        assert!(
            extract(
                archive("a/native.dll"),
                &root,
                &[],
                &Cancellation::default()
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(root.join("a/native.dll")).ok().as_deref(),
            Some(b"existing user file".as_slice())
        );
    }
    #[test]
    fn archive_symlinks_and_case_collisions_are_rejected() {
        let root = std::env::temp_dir().join(format!("panita-natives-{}", uuid::Uuid::new_v4()));
        private_fs::PrivateDir::ensure(&root).unwrap_or_else(|_| panic!("root"));
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink(
                "link",
                "../outside",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap_or_else(|_| panic!("symlink"));
        let mut bytes = writer.finish().unwrap_or_else(|_| panic!("archive"));
        bytes.set_position(0);
        assert!(extract(bytes, &root, &[], &Cancellation::default()).is_err());
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in ["a.dll", "A.dll"] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap_or_else(|_| panic!("file"));
            writer
                .write_all(b"native")
                .unwrap_or_else(|_| panic!("write"));
        }
        let mut bytes = writer.finish().unwrap_or_else(|_| panic!("archive"));
        bytes.set_position(0);
        assert!(extract(bytes, &root, &[], &Cancellation::default()).is_err());
    }
}
