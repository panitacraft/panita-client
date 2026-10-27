//! Manual pack files: CurseForge files the author does not let other apps download,
//! plus files a CurseForge CDN reports as withdrawn. They form one persisted pending
//! list with exact project and file identifiers. Everything here is local: no session
//! and no network are needed to import a file or re-check the list.
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{
    FileType, MAX_FILE_BYTES, ManifestFile, Origin, content, game_path,
    gamefs::{Content, GameDir},
    hex, invalid, text,
    transfer::Expected,
};
use crate::{
    error::{CoreError, ErrorCode},
    minecraft::download::{Cancellation, io},
    paths::AppPaths,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingReason {
    /// The manifest marks the file as manual: it carries no download URL.
    Manual,
    /// The CDN answered 403, 404 or 410 for a file the manifest lists with a URL.
    Withdrawn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingFile {
    pub path: String,
    pub name: String,
    pub project_id: u64,
    pub file_id: u64,
    pub sha1: String,
    pub size: u64,
    pub reason: PendingReason,
}

/// What the player's folder currently holds for one pending file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Missing,
    /// The expected size and SHA-1 are present at the destination.
    Satisfied,
    /// Another version or unrelated content sits at the destination.
    WrongVersion,
    /// A folder, link or other node occupies the destination.
    Blocked,
}

impl PendingFile {
    pub fn from_manifest(file: &ManifestFile, reason: PendingReason) -> Result<Self, CoreError> {
        let (Origin::Curseforge, Some(ids), Some(sha1)) =
            (&file.source, &file.curseforge, &file.hashes.sha1)
        else {
            return Err(invalid());
        };
        let pending = Self {
            path: file.path.clone(),
            name: file.name.clone(),
            project_id: ids.project_id,
            file_id: ids.file_id,
            sha1: sha1.clone(),
            size: file.size,
            reason,
        };
        pending.validate()?;
        Ok(pending)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        game_path(&self.path)?;
        let parts: Vec<_> = self.path.split('/').collect();
        let folder_ok = matches!(parts.as_slice(), [folder, _]
            if ["mods", "resourcepacks", "shaderpacks"].contains(folder));
        if !folder_ok
            || !text(&self.name, 256)
            || self.project_id == 0
            || self.file_id == 0
            || !hex(&self.sha1, 40)
            || self.size > MAX_FILE_BYTES
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn file_type(&self) -> FileType {
        match self.path.split('/').next() {
            Some("resourcepacks") => FileType::Resourcepack,
            Some("shaderpacks") => FileType::Shaderpack,
            _ => FileType::Mod,
        }
    }

    /// The page where the player finds the file; the file id says which version.
    pub fn source_url(&self) -> String {
        format!("https://www.curseforge.com/projects/{}", self.project_id)
    }

    /// The folder, relative to the game directory, the file must end up in.
    pub fn folder(&self) -> &str {
        self.path.split('/').next().unwrap_or_default()
    }

    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or_default()
    }

    pub fn expected(&self) -> Expected {
        Expected {
            size: self.size,
            sha1: Some(self.sha1.clone()),
            sha256: None,
            sha512: None,
        }
    }
}

/// Inspect the destination without changing anything.
pub fn check(
    game: &GameDir,
    file: &PendingFile,
    cancel: &Cancellation,
) -> Result<Placement, CoreError> {
    Ok(match game.compare(&file.path, &file.expected(), cancel)? {
        Content::Absent => Placement::Missing,
        Content::Matches => Placement::Satisfied,
        Content::Differs => Placement::WrongVersion,
        Content::Unsafe => Placement::Blocked,
    })
}

/// Take a file the player picked, verify its exact size and SHA-1, and place it.
/// The picked file is read once into the verified cache; the copy into the game
/// directory is verified again. A wrong file is rejected without touching anything,
/// and an existing different file at the destination is never replaced.
pub async fn import(
    paths: &AppPaths,
    game: &GameDir,
    file: &PendingFile,
    selected: &Path,
    cancel: &Cancellation,
) -> Result<(), CoreError> {
    file.validate()?;
    match check(game, file, cancel)? {
        Placement::Satisfied => return Ok(()),
        Placement::Missing => {}
        Placement::WrongVersion | Placement::Blocked => {
            return Err(CoreError::new(ErrorCode::PackConflict));
        }
    }
    let metadata = std::fs::symlink_metadata(selected).map_err(io)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid());
    }
    // Cheap rejection of an obviously different file before reading it.
    if metadata.len() != file.size {
        return Err(CoreError::new(ErrorCode::ArtifactMismatch));
    }
    let expected = file.expected();
    let mut input = std::fs::File::open(selected)
        .map_err(io)?
        .take(file.size.saturating_add(1));
    content::import(paths, &expected, &mut input, cancel).await?;
    game.write_new(&file.path, |output| {
        content::stream_verified(paths, &expected, cancel, |chunk| {
            output.write_all(chunk).map_err(io)
        })
    })
}

#[cfg(test)]
mod tests;
