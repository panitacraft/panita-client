//! Skin and cape manager for the selected Microsoft account.
//!
//! The manager keeps a local library (saved looks, the skin the account wears and its
//! capes, as PNG files plus a small index) and keeps it in step with Minecraft Services.
//! Every remote step goes through [`SkinRemote`], so the account's access token never
//! leaves the crate and the whole workflow can be tested without a network.
//!
//! A "look" is a skin, its body model and the cape that goes with it. The account's
//! current look is compared with the saved ones by the pixels of the skin, so a skin the
//! service re-encoded is still recognised as the one that was uploaded.
mod library;
mod texture;

#[cfg(test)]
mod tests;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::authentication::JavaProfile;
use crate::authentication::profile_services::{RemoteProfile, valid_cape_id};
use crate::error::{CoreError, ErrorCode};
use crate::paths::AppPaths;
use crate::sessions::AccountController;
use library::{
    CachedCape, DiskLook, DiskProfile, DiskProfileCape, DiskSkin, Library, MAX_LOOKS, clean_name,
    invalid,
};
use texture::{Kind, Texture};

/// The body model of a skin: wide arms (Steve) or slim arms (Alex).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkinModel {
    #[default]
    Classic,
    Slim,
}

impl SkinModel {
    /// The spelling the upload form of the service expects.
    pub(crate) fn service_name(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Slim => "slim",
        }
    }
}

/// What a texture request names: a saved or worn skin by pixel hash, or a cape by identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureKind {
    Skin,
    Cape,
}

/// What the account wears, as of the last synchronisation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentLook {
    /// Whether the account has a skin of its own; without one the game shows a default.
    pub custom: bool,
    /// Pixel hash of the cached skin, absent while the image could not be fetched.
    pub texture: Option<String>,
    pub model: SkinModel,
    pub cape: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapeView {
    pub id: String,
    pub alias: String,
    /// Whether its image is cached and can be previewed.
    pub cached: bool,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookView {
    pub id: Uuid,
    pub name: String,
    pub model: SkinModel,
    pub texture: String,
    pub cape: Option<String>,
    pub created_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinsView {
    pub profile_id: Uuid,
    pub profile_name: String,
    /// Absent until the profile has been synchronised once.
    pub current: Option<CurrentLook>,
    pub capes: Vec<CapeView>,
    /// Newest first.
    pub library: Vec<LookView>,
    /// The saved look equal to the current one, if the current one is saved.
    pub matching_look: Option<Uuid>,
    pub synced_ms: Option<u64>,
    /// Some images could not be kept locally yet; a later synchronisation retries them.
    pub incomplete: bool,
}

/// The skin a profile last wore, as the library knows it, for drawing accounts without asking a service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WornSkin {
    pub profile_id: Uuid,
    /// Pixel hash of the cached skin; absent when the profile wears the default skin or its image is not kept yet.
    pub texture: Option<String>,
    pub model: SkinModel,
}

/// A look that was just added, with the library as it is now.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub id: Uuid,
    pub view: SkinsView,
}

/// The cape to wear, or none. Absent from a request means "leave the cape as it is".
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapeChange {
    pub id: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRequest {
    /// A saved look whose skin and model replace the current ones.
    #[serde(default)]
    pub look_id: Option<Uuid>,
    /// An explicit cape choice. Without one, applying a look also applies its own cape.
    #[serde(default)]
    pub cape_change: Option<CapeChange>,
}

/// The remote side of the workflow, as the manager needs it.
pub(crate) trait SkinRemote {
    async fn profile(&self) -> Result<RemoteProfile, CoreError>;
    async fn upload_skin(&self, model: SkinModel, png: &[u8]) -> Result<RemoteProfile, CoreError>;
    async fn choose_cape(&self, cape: Option<&str>) -> Result<RemoteProfile, CoreError>;
    async fn download_texture(&self, hash: &str) -> Result<Vec<u8>, CoreError>;
}

/// Minecraft Services as the selected Microsoft account.
struct AccountRemote<'a>(&'a AccountController);

impl SkinRemote for AccountRemote<'_> {
    async fn profile(&self) -> Result<RemoteProfile, CoreError> {
        let client = self.0.auth_client();
        self.0
            .with_microsoft_session(|session| async move { client.service_profile(&session).await })
            .await
    }

    async fn upload_skin(&self, model: SkinModel, png: &[u8]) -> Result<RemoteProfile, CoreError> {
        let client = self.0.auth_client();
        self.0
            .with_microsoft_session(|session| async move {
                client.upload_skin(&session, model, png).await
            })
            .await
    }

    async fn choose_cape(&self, cape: Option<&str>) -> Result<RemoteProfile, CoreError> {
        let client = self.0.auth_client();
        self.0
            .with_microsoft_session(
                |session| async move { client.choose_cape(&session, cape).await },
            )
            .await
    }

    async fn download_texture(&self, hash: &str) -> Result<Vec<u8>, CoreError> {
        self.0.auth_client().download_texture(hash).await
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn check_cape(cape: Option<&str>) -> Result<(), CoreError> {
    match cape {
        Some(id) if !valid_cape_id(id) => Err(invalid()),
        _ => Ok(()),
    }
}

fn owns(state: &RemoteProfile, cape: &str) -> bool {
    state.capes.iter().any(|owned| owned.id == cape)
}

fn worn_cape(state: &RemoteProfile) -> Option<&str> {
    state
        .capes
        .iter()
        .find(|cape| cape.active)
        .map(|cape| cape.id.as_str())
}

/// Decode a skin file the interface read from the file picker and sent as base64. The
/// length is bounded before decoding; the image itself is validated when it is used.
pub fn decode_upload(encoded: &str) -> Result<Vec<u8>, CoreError> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    if encoded.len() > texture::MAX_INPUT_BYTES.div_ceil(3) * 4 {
        return Err(CoreError::new(ErrorCode::InvalidSkin));
    }
    STANDARD
        .decode(encoded)
        .map_err(|_| CoreError::new(ErrorCode::InvalidSkin))
}

/// The view of one profile from the library alone; it needs no network.
fn build_view(library: &Library, profile: &JavaProfile) -> Result<SkinsView, CoreError> {
    let index = library.index()?;
    let files = library.files();
    let snapshot = index.profile(profile.id);
    let current = snapshot.map(|snapshot| {
        let skin = snapshot.skin.as_ref();
        CurrentLook {
            custom: skin.is_some(),
            texture: skin
                .and_then(|skin| skin.texture.clone())
                .filter(|texture| files.has_skin(texture)),
            model: skin.map_or(SkinModel::Classic, |skin| skin.model),
            cape: snapshot.cape.clone(),
        }
    });
    let capes: Vec<CapeView> = snapshot
        .map(|snapshot| {
            snapshot
                .capes
                .iter()
                .map(|cape| CapeView {
                    id: cape.id.clone(),
                    alias: cape.alias.clone(),
                    cached: files.has_cape(&cape.id),
                    active: snapshot.cape.as_deref() == Some(cape.id.as_str()),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut library_views: Vec<LookView> = index
        .looks
        .iter()
        .map(|look| LookView {
            id: look.id,
            name: look.name.clone(),
            model: look.model,
            texture: look.texture.clone(),
            cape: look.cape.clone(),
            created_ms: look.created_ms,
        })
        .collect();
    library_views.sort_by(|a, b| {
        b.created_ms
            .cmp(&a.created_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    let matching_look = current.as_ref().and_then(|current| {
        let texture = current.texture.as_ref()?;
        library_views
            .iter()
            .find(|look| {
                &look.texture == texture && look.model == current.model && look.cape == current.cape
            })
            .map(|look| look.id)
    });
    let incomplete = current
        .as_ref()
        .is_some_and(|current| current.custom && current.texture.is_none())
        || capes.iter().any(|cape| !cape.cached);
    Ok(SkinsView {
        profile_id: profile.id,
        profile_name: profile.name.clone(),
        synced_ms: snapshot.map(|snapshot| snapshot.synced_ms),
        current,
        capes,
        library: library_views,
        matching_look,
        incomplete,
    })
}

/// Fetch and validate one texture. A failure is not fatal: the image is retried on the
/// next synchronisation and the view reports it as incomplete meanwhile.
async fn fetch<R: SkinRemote>(remote: &R, hash: &str, kind: Kind) -> Option<Texture> {
    let bytes = remote.download_texture(hash).await.ok()?;
    texture::prepare(&bytes, kind).ok()
}

pub struct SkinManager {
    paths: AppPaths,
}

impl SkinManager {
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    /// The library as it is on disk. Needs no network.
    pub fn view(&self, profile: &JavaProfile) -> Result<SkinsView, CoreError> {
        build_view(&Library::open(&self.paths)?, profile)
    }

    /// The PNG of a cached skin or cape, for previewing it.
    pub fn texture(&self, kind: TextureKind, key: &str) -> Result<Vec<u8>, CoreError> {
        let library = Library::open(&self.paths)?;
        let bytes = match kind {
            TextureKind::Skin => library.files().read_skin(key)?,
            TextureKind::Cape => library.files().read_cape(key)?,
        };
        bytes.ok_or_else(invalid)
    }

    /// What every synchronised profile last wore. It reads the library only and never touches the network or
    /// an account, so the accounts list can draw every saved profile at once.
    pub fn worn_skins(&self) -> Result<Vec<WornSkin>, CoreError> {
        let library = Library::open(&self.paths)?;
        let index = library.index()?;
        let files = library.files();
        let mut worn: Vec<WornSkin> = index
            .profiles
            .iter()
            .filter_map(|(key, profile)| {
                let skin = profile.skin.as_ref();
                Some(WornSkin {
                    profile_id: Uuid::parse_str(key).ok()?,
                    texture: skin
                        .and_then(|skin| skin.texture.clone())
                        .filter(|texture| files.has_skin(texture)),
                    model: skin.map_or(SkinModel::Classic, |skin| skin.model),
                })
            })
            .collect();
        worn.sort_by_key(|entry| entry.profile_id);
        Ok(worn)
    }

    /// Add a skin file to the library as a new look.
    pub fn add_look(
        &self,
        profile: &JavaProfile,
        name: &str,
        model: SkinModel,
        cape: Option<String>,
        png: &[u8],
    ) -> Result<Saved, CoreError> {
        let name = clean_name(name)?;
        check_cape(cape.as_deref())?;
        let texture = texture::prepare(png, Kind::UploadSkin)?;
        let library = Library::open(&self.paths)?;
        let id = library.update(|index, files| {
            if index.looks.len() >= MAX_LOOKS {
                return Err(CoreError::new(ErrorCode::SkinLibraryFull));
            }
            files.write_skin(&texture.hash, &texture.png)?;
            let id = Uuid::new_v4();
            index.looks.push(DiskLook {
                id,
                name,
                model,
                texture: texture.hash.clone(),
                cape,
                created_ms: now_ms(),
            });
            Ok(id)
        })?;
        Ok(Saved {
            id,
            view: build_view(&library, profile)?,
        })
    }

    /// Save the look the account wears now, which is not in the library yet.
    pub fn save_current(&self, profile: &JavaProfile, name: &str) -> Result<Saved, CoreError> {
        let name = clean_name(name)?;
        let library = Library::open(&self.paths)?;
        let id = library.update(|index, files| {
            if index.looks.len() >= MAX_LOOKS {
                return Err(CoreError::new(ErrorCode::SkinLibraryFull));
            }
            let (model, texture, cape) = {
                let snapshot = index.profile(profile.id).ok_or_else(invalid)?;
                let skin = snapshot.skin.as_ref().ok_or_else(invalid)?;
                let texture = skin
                    .texture
                    .clone()
                    .filter(|texture| files.has_skin(texture))
                    .ok_or_else(invalid)?;
                (skin.model, texture, snapshot.cape.clone())
            };
            let id = Uuid::new_v4();
            index.looks.push(DiskLook {
                id,
                name,
                model,
                texture,
                cape,
                created_ms: now_ms(),
            });
            Ok(id)
        })?;
        Ok(Saved {
            id,
            view: build_view(&library, profile)?,
        })
    }

    /// Change a saved look. A new file replaces its skin; without one the skin stays.
    pub fn edit_look(
        &self,
        profile: &JavaProfile,
        id: Uuid,
        name: &str,
        model: SkinModel,
        cape: Option<String>,
        png: Option<&[u8]>,
    ) -> Result<SkinsView, CoreError> {
        let name = clean_name(name)?;
        check_cape(cape.as_deref())?;
        let replacement = png
            .map(|bytes| texture::prepare(bytes, Kind::UploadSkin))
            .transpose()?;
        let library = Library::open(&self.paths)?;
        library.update(|index, files| {
            let look = index
                .looks
                .iter_mut()
                .find(|look| look.id == id)
                .ok_or_else(invalid)?;
            if let Some(texture) = &replacement {
                files.write_skin(&texture.hash, &texture.png)?;
                look.texture.clone_from(&texture.hash);
            }
            look.name = name;
            look.model = model;
            look.cape = cape;
            Ok(())
        })?;
        build_view(&library, profile)
    }

    pub fn delete_look(&self, profile: &JavaProfile, id: Uuid) -> Result<SkinsView, CoreError> {
        let library = Library::open(&self.paths)?;
        library.update(|index, _| {
            let before = index.looks.len();
            index.looks.retain(|look| look.id != id);
            if index.looks.len() == before {
                return Err(invalid());
            }
            Ok(())
        })?;
        build_view(&library, profile)
    }

    /// Fetch the account's skin and capes from the service and keep their images.
    pub async fn sync(&self, accounts: &AccountController) -> Result<SkinsView, CoreError> {
        self.sync_with(&AccountRemote(accounts)).await
    }

    /// Put a saved look and/or a cape on the account, then record what the service now says.
    pub async fn apply(
        &self,
        accounts: &AccountController,
        request: ApplyRequest,
    ) -> Result<SkinsView, CoreError> {
        self.apply_with(&AccountRemote(accounts), request).await
    }

    pub(crate) async fn sync_with<R: SkinRemote>(
        &self,
        remote: &R,
    ) -> Result<SkinsView, CoreError> {
        let state = remote.profile().await?;
        self.ingest(remote, &state).await
    }

    pub(crate) async fn apply_with<R: SkinRemote>(
        &self,
        remote: &R,
        request: ApplyRequest,
    ) -> Result<SkinsView, CoreError> {
        let wanted = request.cape_change.map(|change| change.id);
        check_cape(wanted.as_ref().and_then(|id| id.as_deref()))?;
        let look = match request.look_id {
            Some(id) => {
                let library = Library::open(&self.paths)?;
                let look = library
                    .index()?
                    .looks
                    .into_iter()
                    .find(|look| look.id == id)
                    .ok_or_else(invalid)?;
                let png = library
                    .files()
                    .read_skin(&look.texture)?
                    .ok_or_else(invalid)?;
                Some((look, png))
            }
            None => None,
        };
        let mut state = remote.profile().await?;
        // Whatever the service did, the cache must say what it now holds.
        let result = Self::apply_steps(remote, &mut state, look, wanted).await;
        let view = self.ingest(remote, &state).await;
        result?;
        view
    }

    async fn apply_steps<R: SkinRemote>(
        remote: &R,
        state: &mut RemoteProfile,
        look: Option<(DiskLook, Vec<u8>)>,
        mut wanted: Option<Option<String>>,
    ) -> Result<(), CoreError> {
        // Refuse a cape the account does not own before anything changes remotely.
        if let Some(Some(id)) = &wanted
            && !owns(state, id)
        {
            return Err(invalid());
        }
        if let Some((look, png)) = &look {
            *state = remote.upload_skin(look.model, png).await?;
            if wanted.is_none() {
                wanted = match &look.cape {
                    None => Some(None),
                    Some(id) if owns(state, id) => Some(Some(id.clone())),
                    // A cape this account does not own cannot be worn; keep the current one.
                    Some(_) => None,
                };
            }
        }
        match wanted {
            Some(Some(id)) if worn_cape(state) != Some(id.as_str()) => {
                *state = remote.choose_cape(Some(&id)).await?;
            }
            Some(None) if worn_cape(state).is_some() => {
                *state = remote.choose_cape(None).await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Record the service's view of a profile, fetching the images not yet kept.
    async fn ingest<R: SkinRemote>(
        &self,
        remote: &R,
        state: &RemoteProfile,
    ) -> Result<SkinsView, CoreError> {
        let library = Library::open(&self.paths)?;
        let known = library.index()?;
        let files = library.files();
        let mut skin_download = None;
        if let Some(skin) = &state.skin {
            let kept = known
                .remote_textures
                .get(&skin.texture)
                .is_some_and(|pixel| files.has_skin(pixel));
            if !kept && let Some(texture) = fetch(remote, &skin.texture, Kind::RemoteSkin).await {
                skin_download = Some((skin.texture.clone(), texture));
            }
        }
        let mut cape_downloads = Vec::new();
        for cape in &state.capes {
            let kept = files.has_cape(&cape.id)
                && known
                    .capes
                    .get(&cape.id)
                    .is_some_and(|cached| cached.texture_key == cape.texture);
            if !kept && let Some(texture) = fetch(remote, &cape.texture, Kind::Cape).await {
                cape_downloads.push((cape, texture));
            }
        }
        library.update(|index, files| {
            if let Some((remote_hash, texture)) = &skin_download {
                files.write_skin(&texture.hash, &texture.png)?;
                index
                    .remote_textures
                    .insert(remote_hash.clone(), texture.hash.clone());
            }
            for (cape, texture) in &cape_downloads {
                files.write_cape(&cape.id, &texture.png)?;
                index.capes.insert(
                    cape.id.clone(),
                    CachedCape {
                        texture_key: cape.texture.clone(),
                    },
                );
            }
            let pixel = state
                .skin
                .as_ref()
                .and_then(|skin| index.remote_textures.get(&skin.texture).cloned())
                .filter(|pixel| files.has_skin(pixel));
            index.profiles.insert(
                state.id.simple().to_string(),
                DiskProfile {
                    skin: state.skin.as_ref().map(|skin| DiskSkin {
                        model: skin.model,
                        remote_texture: skin.texture.clone(),
                        texture: pixel,
                    }),
                    cape: worn_cape(state).map(str::to_owned),
                    capes: state
                        .capes
                        .iter()
                        .map(|cape| DiskProfileCape {
                            id: cape.id.clone(),
                            alias: cape.alias.clone(),
                            texture_key: cape.texture.clone(),
                        })
                        .collect(),
                    synced_ms: now_ms(),
                },
            );
            Ok(())
        })?;
        build_view(
            &library,
            &JavaProfile {
                id: state.id,
                name: state.name.clone(),
            },
        )
    }
}
