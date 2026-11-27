//! The local skin library: a small index document and two folders of PNG files, all under
//! the private `skins/` folder.
//!
//! `skins/skins.json` is written under the same interprocess lock and with the same atomic
//! replacement as the other documents. `skins/` also holds one PNG per distinct skin, named
//! by its pixel hash, and `skins/capes/` one per cape, named by the service's cape
//! identifier. A name read from the index is validated before it becomes a file name, so a
//! damaged or edited index cannot reach outside those folders. Earlier builds kept the
//! index and the capes folder in the application root; `Library::open` moves them in.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

use private_fs::{Error as PrivateFsError, PrivateDir};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::SkinModel;
use crate::authentication::profile_services::{valid_cape_id, valid_hash};
use crate::error::{CoreError, ErrorCode};
use crate::minecraft::world_backup::capability;
use crate::paths::AppPaths;
use crate::storage::{PrivateStore, StoreTransaction, map_io_error, map_private_error};
use crate::storage_json::pretty;

const INDEX: &str = "skins.json";
const SCHEMA: u32 = 1;
/// A player's saved looks. Each needs a file of a few kilobytes at most.
pub(crate) const MAX_LOOKS: usize = 100;
const MAX_PROFILES: usize = 16;
const MAX_REMOTE_TEXTURES: usize = 128;
const MAX_NAME_CHARS: usize = 32;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Index {
    pub schema: u32,
    /// Saved skins. Earlier builds wrote this key as `looks`; it is still read, never written,
    /// and a document carrying both spellings is refused.
    #[serde(default, rename = "skins", alias = "looks")]
    pub looks: Vec<DiskLook>,
    /// Capes whose image is cached, by service identifier.
    #[serde(default)]
    pub capes: BTreeMap<String, CachedCape>,
    /// Service texture hash to the pixel hash of the cached skin, so a skin already kept
    /// is not downloaded again.
    #[serde(default)]
    pub remote_textures: BTreeMap<String, String>,
    /// The last known state of each profile, keyed by its simple-form UUID.
    #[serde(default)]
    pub profiles: BTreeMap<String, DiskProfile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DiskLook {
    pub id: Uuid,
    pub name: String,
    pub model: SkinModel,
    /// Pixel hash; the image is `skins/{texture}.png`.
    pub texture: String,
    pub cape: Option<String>,
    pub created_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct CachedCape {
    /// The service's texture hash of the cached image, to notice a changed cape.
    pub texture_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DiskProfile {
    pub skin: Option<DiskSkin>,
    /// The cape shown now, by identifier.
    pub cape: Option<String>,
    /// Every cape the profile owns.
    pub capes: Vec<DiskProfileCape>,
    pub synced_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DiskSkin {
    pub model: SkinModel,
    /// The service's texture hash.
    pub remote_texture: String,
    /// Pixel hash of the cached image; absent when it could not be fetched yet.
    pub texture: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DiskProfileCape {
    pub id: String,
    pub alias: String,
    pub texture_key: String,
}

pub(crate) fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidData)
}

/// A pixel hash: exactly 64 lowercase hexadecimal characters.
pub(crate) fn valid_pixel_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A name for a saved look: no control characters, 1 to 32 characters once trimmed.
pub(crate) fn clean_name(name: &str) -> Result<String, CoreError> {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned.chars().count() > MAX_NAME_CHARS {
        return Err(invalid());
    }
    Ok(cleaned.to_owned())
}

impl Index {
    pub(crate) fn empty() -> Self {
        Self {
            schema: SCHEMA,
            ..Self::default()
        }
    }

    fn parse(bytes: Option<Vec<u8>>) -> Result<Self, CoreError> {
        let Some(bytes) = bytes else {
            return Ok(Self::empty());
        };
        let index: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        index.validate()?;
        Ok(index)
    }

    /// Everything that becomes a file name or a bound is checked here, so later code can
    /// trust the index. A future schema is refused untouched rather than reinterpreted.
    fn validate(&self) -> Result<(), CoreError> {
        let mut ids = BTreeSet::new();
        if self.schema != SCHEMA
            || self.looks.len() > MAX_LOOKS
            || self.profiles.len() > MAX_PROFILES
            || self.remote_textures.len() > MAX_REMOTE_TEXTURES * 2
        {
            return Err(invalid());
        }
        for look in &self.looks {
            if !ids.insert(look.id)
                || clean_name(&look.name).is_err()
                || !valid_pixel_hash(&look.texture)
                || look.cape.as_deref().is_some_and(|id| !valid_cape_id(id))
            {
                return Err(invalid());
            }
        }
        if self
            .capes
            .iter()
            .any(|(id, cape)| !valid_cape_id(id) || !valid_hash(&cape.texture_key))
            || self
                .remote_textures
                .iter()
                .any(|(remote, pixel)| !valid_hash(remote) || !valid_pixel_hash(pixel))
        {
            return Err(invalid());
        }
        for (key, profile) in &self.profiles {
            let skin_ok = profile.skin.as_ref().is_none_or(|skin| {
                valid_hash(&skin.remote_texture)
                    && skin.texture.as_deref().is_none_or(valid_pixel_hash)
            });
            let capes_ok = profile
                .capes
                .iter()
                .all(|cape| valid_cape_id(&cape.id) && valid_hash(&cape.texture_key));
            if Uuid::parse_str(key).is_err()
                || !skin_ok
                || !capes_ok
                || profile.cape.as_deref().is_some_and(|id| !valid_cape_id(id))
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    pub(crate) fn profile(&self, id: Uuid) -> Option<&DiskProfile> {
        self.profiles.get(&id.simple().to_string())
    }

    /// Pixel hashes of skins something still refers to.
    fn referenced_skins(&self) -> BTreeSet<&str> {
        let looks = self.looks.iter().map(|look| look.texture.as_str());
        let worn = self
            .profiles
            .values()
            .filter_map(|profile| profile.skin.as_ref()?.texture.as_deref());
        looks.chain(worn).collect()
    }

    fn referenced_capes(&self) -> BTreeSet<&str> {
        let looks = self.looks.iter().filter_map(|look| look.cape.as_deref());
        let owned = self
            .profiles
            .values()
            .flat_map(|profile| profile.capes.iter().map(|cape| cape.id.as_str()));
        looks.chain(owned).collect()
    }

    /// Forget what nothing refers to any more, within fixed bounds.
    fn tidy(&mut self) {
        while self.profiles.len() > MAX_PROFILES {
            let oldest = self
                .profiles
                .iter()
                .min_by_key(|(_, profile)| profile.synced_ms)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => self.profiles.remove(&key),
                None => break,
            };
        }
        let skins: BTreeSet<String> = self
            .referenced_skins()
            .into_iter()
            .map(str::to_owned)
            .collect();
        self.remote_textures
            .retain(|_, pixel| skins.contains(pixel));
        if self.remote_textures.len() > MAX_REMOTE_TEXTURES {
            self.remote_textures.clear();
        }
        let capes: BTreeSet<String> = self
            .referenced_capes()
            .into_iter()
            .map(str::to_owned)
            .collect();
        self.capes.retain(|id, _| capes.contains(id));
    }
}

/// The two image folders, opened through private handles that reject redirections.
pub(crate) struct Files {
    skins: PrivateDir,
    capes: PrivateDir,
}

fn not_found(error: &PrivateFsError) -> bool {
    matches!(error, PrivateFsError::Io(io) if io.kind() == std::io::ErrorKind::NotFound)
}

impl Files {
    fn read(directory: &PrivateDir, name: &str) -> Result<Option<Vec<u8>>, CoreError> {
        let mut file = match directory.open_file(Path::new(name)) {
            Ok(file) => file,
            Err(error) if not_found(&error) => return Ok(None),
            Err(error) => return Err(map_private_error(error)),
        };
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(map_io_error)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(invalid());
        }
        file.verify().map_err(map_private_error)?;
        Ok(Some(bytes))
    }

    fn exists(directory: &PrivateDir, name: &str) -> bool {
        directory.open_file(Path::new(name)).is_ok()
    }

    fn write(directory: &PrivateDir, name: &str, bytes: &[u8]) -> Result<(), CoreError> {
        directory
            .write_atomic_durable(name, bytes)
            .map_err(map_private_error)
    }

    pub(crate) fn read_skin(&self, hash: &str) -> Result<Option<Vec<u8>>, CoreError> {
        if !valid_pixel_hash(hash) {
            return Err(invalid());
        }
        Self::read(&self.skins, &format!("{hash}.png"))
    }

    pub(crate) fn read_cape(&self, id: &str) -> Result<Option<Vec<u8>>, CoreError> {
        if !valid_cape_id(id) {
            return Err(invalid());
        }
        Self::read(&self.capes, &format!("{id}.png"))
    }

    pub(crate) fn has_skin(&self, hash: &str) -> bool {
        valid_pixel_hash(hash) && Self::exists(&self.skins, &format!("{hash}.png"))
    }

    pub(crate) fn has_cape(&self, id: &str) -> bool {
        valid_cape_id(id) && Self::exists(&self.capes, &format!("{id}.png"))
    }

    pub(crate) fn write_skin(&self, hash: &str, png: &[u8]) -> Result<(), CoreError> {
        if !valid_pixel_hash(hash) {
            return Err(invalid());
        }
        if self.has_skin(hash) {
            return Ok(());
        }
        Self::write(&self.skins, &format!("{hash}.png"), png)
    }

    pub(crate) fn write_cape(&self, id: &str, png: &[u8]) -> Result<(), CoreError> {
        if !valid_cape_id(id) {
            return Err(invalid());
        }
        Self::write(&self.capes, &format!("{id}.png"), png)
    }

    /// Remove the PNG files nothing refers to. Entries that are not ours are left alone.
    fn prune(
        directory: &PrivateDir,
        keep: &BTreeSet<&str>,
        valid: fn(&str) -> bool,
    ) -> Result<(), CoreError> {
        let handle = capability(directory)?;
        let mut stale = Vec::new();
        for entry in handle.entries().map_err(map_io_error)? {
            let Ok(name) = entry.map_err(map_io_error)?.file_name().into_string() else {
                continue;
            };
            let Some(stem) = name.strip_suffix(".png") else {
                continue;
            };
            if valid(stem) && !keep.contains(stem) {
                stale.push(name);
            }
        }
        for name in stale {
            match handle.remove_file(&name) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(map_io_error(error)),
            }
        }
        Ok(())
    }
}

pub(crate) struct Library {
    store: PrivateStore,
    files: Files,
}

/// Where earlier builds kept the capes folder, inside the application root.
const LEGACY_CAPES: &str = "capes";
/// Same limit as the other documents written through the private store.
const MAX_INDEX_BYTES: usize = 1024 * 1024;

/// Move what earlier builds left in the application root into `skins/`, under the writer
/// lock. Nothing is deleted that is not first present at the destination: an index is
/// removed only once its bytes are published, a cape image only by moving it. A conflicting
/// copy is left where it is and the new location wins.
fn migrate_legacy(
    transaction: &StoreTransaction<'_>,
    root: &PrivateDir,
    skins: &PrivateDir,
) -> Result<(), CoreError> {
    if let Some(old) = Files::read(root, INDEX)? {
        match Files::read(skins, INDEX)? {
            None => {
                Files::write(skins, INDEX, &old)?;
                transaction.remove_verified_file(INDEX)?;
            }
            Some(current) if current == old => transaction.remove_verified_file(INDEX)?,
            Some(_) => {}
        }
    }
    let legacy = match root.open_dir(LEGACY_CAPES) {
        Ok(directory) => directory,
        Err(error) if not_found(&error) => return Ok(()),
        Err(error) => return Err(map_private_error(error)),
    };
    let source = capability(&legacy)?;
    let parent = capability(root)?;
    match skins.open_dir(LEGACY_CAPES) {
        Err(error) if not_found(&error) => {
            // The whole folder moves, with whatever it holds.
            drop(source);
            drop(legacy);
            parent
                .rename(LEGACY_CAPES, &capability(skins)?, LEGACY_CAPES)
                .map_err(map_io_error)
        }
        Err(error) => Err(map_private_error(error)),
        Ok(destination) => {
            // Both exist: move the images that are not already there, then drop the old
            // folder if that left it empty.
            let target = capability(&destination)?;
            let mut moving = Vec::new();
            for entry in source.entries().map_err(map_io_error)? {
                let Ok(name) = entry.map_err(map_io_error)?.file_name().into_string() else {
                    continue;
                };
                let valid = name.strip_suffix(".png").is_some_and(valid_cape_id);
                if valid && target.symlink_metadata(&name).is_err() {
                    moving.push(name);
                }
            }
            for name in moving {
                source.rename(&name, &target, &name).map_err(map_io_error)?;
            }
            let _ = parent.remove_dir(LEGACY_CAPES);
            Ok(())
        }
    }
}

impl Library {
    pub(crate) fn open(paths: &AppPaths) -> Result<Self, CoreError> {
        let store = PrivateStore::open(paths)?;
        let root = PrivateDir::ensure(paths.root()).map_err(map_private_error)?;
        let skins = root.ensure_dir("skins").map_err(map_private_error)?;
        // Take the writer lock only when something from an earlier build is still there.
        if root.open_file(Path::new(INDEX)).is_ok() || root.open_dir(LEGACY_CAPES).is_ok() {
            store.with_lock(|transaction| migrate_legacy(transaction, &root, &skins))?;
        }
        let capes = skins.ensure_dir("capes").map_err(map_private_error)?;
        Ok(Self {
            store,
            files: Files { skins, capes },
        })
    }

    pub(crate) fn files(&self) -> &Files {
        &self.files
    }

    /// The current index, without taking the writer lock: replacements are atomic.
    pub(crate) fn index(&self) -> Result<Index, CoreError> {
        Index::parse(Files::read(&self.files.skins, INDEX)?)
    }

    /// Change the index under the interprocess lock. Nothing is written when `change`
    /// fails, and images nothing refers to are removed after the new index is durable.
    pub(crate) fn update<T>(
        &self,
        change: impl FnOnce(&mut Index, &Files) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        self.store.with_lock(|_| {
            let mut index = Index::parse(Files::read(&self.files.skins, INDEX)?)?;
            let result = change(&mut index, &self.files)?;
            index.tidy();
            index.validate()?;
            let bytes = pretty(&index).map_err(|_| invalid())?;
            if bytes.len() > MAX_INDEX_BYTES {
                return Err(invalid());
            }
            Files::write(&self.files.skins, INDEX, &bytes)?;
            // Clean up is best effort: a failure here only leaves an unreferenced file
            // that the next change removes.
            let _ = Files::prune(
                &self.files.skins,
                &index.referenced_skins(),
                valid_pixel_hash,
            );
            let _ = Files::prune(&self.files.capes, &index.referenced_capes(), valid_cape_id);
            Ok(result)
        })
    }
}
