//! Safe reading of pack configuration archives. Every entry is checked before
//! anything is written, and extraction enforces its byte budget while streaming:
//! a ZIP header can lie about sizes, so declared sizes only gate preflight.
use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom, Write};

use sha1::Sha1;
use sha2::{Digest, Sha512};

use super::{
    Hashes, MAX_ARCHIVE_BYTES, MAX_FILES, game_path, invalid, modrinth::directory_entry_count,
    path_key,
};
use crate::{
    error::CoreError,
    minecraft::{
        download::{Cancellation, io},
        safe_unicode_path,
    },
};

/// Executable or native content never belongs in a configuration archive.
const FORBIDDEN_EXTENSIONS: [&str; 10] = [
    "jar", "exe", "dll", "bat", "cmd", "ps1", "sh", "so", "dylib", "msi",
];
/// Pack content has its own manifest entries; an archive cannot place it.
const MANAGED_ROOTS: [&str; 3] = ["mods", "resourcepacks", "shaderpacks"];

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub entries: usize,
    pub bytes: u64,
}

impl Limits {
    pub const PACK: Self = Self {
        entries: MAX_FILES,
        bytes: MAX_ARCHIVE_BYTES,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub index: usize,
    /// The game-directory path this entry is written to.
    pub path: String,
    /// The entry's own name inside the archive.
    pub source: String,
    pub size: u64,
}

/// Maps one archive name (without a trailing slash) to the game path it is written
/// to, or `None` for an entry that is never extracted. The structural checks of
/// `preflight_with` apply to every entry regardless of what the rule decides.
pub type Rule<'a> = &'a dyn Fn(&str, bool) -> Result<Option<String>, CoreError>;

/// The rule for configuration archives: the entry name is the game path and nothing
/// executable or pack-managed may appear.
fn configuration_rule(path: &str, directory: bool) -> Result<Option<String>, CoreError> {
    game_path(path)?;
    if MANAGED_ROOTS.contains(&path_key(path.split('/').next().unwrap_or_default()).as_str()) {
        return Err(invalid());
    }
    if !directory && forbidden_extension(path, &FORBIDDEN_EXTENSIONS) {
        return Err(invalid());
    }
    Ok(Some(path.to_owned()))
}

/// True when the file name ends in one of the listed extensions, ignoring case.
pub(crate) fn forbidden_extension(path: &str, list: &[&str]) -> bool {
    path_key(path)
        .rsplit_once('.')
        .is_some_and(|(_, extension)| list.contains(&extension))
}

pub struct Opened<R: Read + Seek> {
    archive: zip::ZipArchive<R>,
    pub entries: Vec<ArchiveEntry>,
    limits: Limits,
}

/// What streaming one entry produced; `sha1` and `sha512` cover its exact bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Digested {
    pub size: u64,
    pub hashes: Hashes,
}

fn has_zip64_extra(extra: Option<&[u8]>) -> bool {
    let Some(mut data) = extra else {
        return false;
    };
    while data.len() >= 4 {
        let id = u16::from_le_bytes([data[0], data[1]]);
        let length = u16::from_le_bytes([data[2], data[3]]) as usize;
        if id == 0x0001 {
            return true;
        }
        let Some(rest) = data.get(4 + length..) else {
            return true;
        };
        data = rest;
    }
    !data.is_empty()
}

/// Validate the whole directory of a configuration archive without reading data.
pub fn preflight<R: Read + Seek>(reader: R, limits: Limits) -> Result<Opened<R>, CoreError> {
    preflight_with(reader, limits, &configuration_rule)
}

/// Validate the whole directory without reading any entry data. Duplicate names,
/// links, encryption, Zip64 and file/folder conflicts are rejected for every entry;
/// `rule` decides which entries are extracted and where they go.
pub fn preflight_with<R: Read + Seek>(
    mut reader: R,
    limits: Limits,
    rule: Rule<'_>,
) -> Result<Opened<R>, CoreError> {
    let length = reader.seek(SeekFrom::End(0)).map_err(io)?;
    let start = length.saturating_sub(22 + u64::from(u16::MAX));
    reader.seek(SeekFrom::Start(start)).map_err(io)?;
    let mut tail = Vec::new();
    reader
        .by_ref()
        .take(length - start)
        .read_to_end(&mut tail)
        .map_err(io)?;
    let declared = directory_entry_count(&tail, start)?;
    reader.rewind().map_err(io)?;
    if declared > limits.entries {
        return Err(invalid());
    }
    let mut archive = zip::ZipArchive::new(reader).map_err(|_| invalid())?;
    // The reader maps names; a different count means duplicate names were hidden.
    if archive.len() != declared {
        return Err(invalid());
    }
    let mut names = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut entries = Vec::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(|_| invalid())?;
        if entry.encrypted()
            || entry.is_symlink()
            || entry.unix_mode().is_some_and(|mode| {
                matches!(mode & 0o170000, 0o010000 | 0o020000 | 0o060000 | 0o140000)
            })
            || has_zip64_extra(entry.extra_data())
        {
            return Err(invalid());
        }
        let raw = entry.name().to_owned();
        let directory = entry.is_dir();
        let path = raw.trim_end_matches('/');
        safe_unicode_path(path)?;
        let lower = path_key(path);
        if !names.insert(lower.clone()) {
            return Err(invalid());
        }
        let destination = rule(path, directory)?;
        if directory {
            continue;
        }
        files.insert(lower);
        let Some(destination) = destination else {
            continue;
        };
        total = total.checked_add(entry.size()).ok_or_else(invalid)?;
        if total > limits.bytes || entry.compressed_size() > limits.bytes {
            return Err(invalid());
        }
        entries.push(ArchiveEntry {
            index,
            path: destination,
            source: path.to_owned(),
            size: entry.size(),
        });
    }
    // A name cannot be both a file and a folder, in either order.
    for name in &names {
        for (end, _) in name.match_indices('/') {
            if files.contains(&name[..end]) {
                return Err(invalid());
            }
        }
    }
    Ok(Opened {
        archive,
        entries,
        limits,
    })
}

impl<R: Read + Seek> Opened<R> {
    /// Stream one entry through its declared size. `budget` is shared by every call
    /// on this archive, so the archive total is bounded by what actually decompresses.
    /// `output`, when present, receives the bytes; the CRC is checked at end of entry.
    pub fn stream(
        &mut self,
        entry: &ArchiveEntry,
        budget: &mut u64,
        cancel: &Cancellation,
        mut output: Option<&mut dyn Write>,
    ) -> Result<Digested, CoreError> {
        let mut file = self.archive.by_index(entry.index).map_err(|_| invalid())?;
        if file.encrypted() || file.is_symlink() || file.size() != entry.size {
            return Err(invalid());
        }
        let (mut sha1, mut sha512) = (Sha1::new(), Sha512::new());
        let mut count = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            cancel.check()?;
            let read = file.by_ref().take(entry.size + 1 - count).read(&mut buffer);
            let read = read.map_err(|_| invalid())?;
            if read == 0 {
                break;
            }
            count += read as u64;
            if count > entry.size || read as u64 > *budget {
                return Err(invalid());
            }
            *budget -= read as u64;
            sha1.update(&buffer[..read]);
            sha512.update(&buffer[..read]);
            if let Some(output) = output.as_deref_mut() {
                output.write_all(&buffer[..read]).map_err(io)?;
            }
        }
        if count != entry.size {
            return Err(invalid());
        }
        Ok(Digested {
            size: count,
            hashes: Hashes {
                sha1: Some(format!("{:x}", sha1.finalize())),
                sha512: Some(format!("{:x}", sha512.finalize())),
            },
        })
    }

    pub fn budget(&self) -> u64 {
        self.limits.bytes
    }
}

#[cfg(test)]
pub(crate) mod tests;
