//! The workflow against an in-memory stand-in for Minecraft Services. Like the real
//! service, it re-encodes every uploaded skin, so these tests also prove that a skin is
//! recognised by its pixels and not by the bytes of its file.
use std::collections::HashMap;
use std::sync::Mutex;

use image::{
    ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};

use super::*;
use crate::authentication::profile_services::{RemoteCape, RemoteSkin};
use texture::encode_for_tests;

const PROFILE_ID: &str = "5627dd98e6be3c21b8a8e92344183641";
const CAPE_A: &str = "11111111-1111-4111-8111-111111111111";
const CAPE_B: &str = "22222222-2222-4222-8222-222222222222";

fn profile() -> JavaProfile {
    JavaProfile {
        id: Uuid::parse_str(PROFILE_ID).unwrap_or_else(|_| panic!("fixture id")),
        name: "PlayerOne".to_owned(),
    }
}

struct World {
    paths: AppPaths,
    manager: SkinManager,
}

impl World {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("panita-skins-{}", Uuid::new_v4()));
        let paths = AppPaths::new(&root).unwrap_or_else(|_| panic!("paths"));
        Self {
            manager: SkinManager::new(paths.clone()),
            paths,
        }
    }

    fn index_text(&self) -> String {
        std::fs::read_to_string(self.paths.root().join("skins.json")).unwrap_or_default()
    }

    fn skin_files(&self) -> Vec<String> {
        names(&self.paths.root().join("skins"))
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.paths.root());
    }
}

fn names(directory: &std::path::Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

/// The same pixels in a different file, as a service that re-encodes would store them.
fn reencoded(png: &[u8]) -> Vec<u8> {
    let rgba = image::load_from_memory(png)
        .unwrap_or_else(|_| panic!("decodes"))
        .into_rgba8();
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::NoFilter)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .unwrap_or_else(|_| panic!("encodes"));
    out
}

#[derive(Default)]
struct Service {
    profile: Option<RemoteProfile>,
    textures: HashMap<String, Vec<u8>>,
    uploads: Vec<(SkinModel, usize)>,
    cape_calls: Vec<Option<String>>,
    downloads: Vec<String>,
    offline: bool,
    cape_fails: bool,
    counter: u64,
}

struct Fake(Mutex<Service>);

impl Fake {
    fn new() -> Self {
        Self(Mutex::new(Service {
            profile: Some(RemoteProfile {
                id: profile().id,
                name: "PlayerOne".to_owned(),
                skin: None,
                capes: Vec::new(),
            }),
            ..Service::default()
        }))
    }

    fn with<T>(&self, change: impl FnOnce(&mut Service) -> T) -> T {
        change(&mut self.0.lock().unwrap_or_else(|poison| poison.into_inner()))
    }

    /// The account already wears a skin and owns capes.
    fn wearing(self, shade: u8, model: SkinModel, capes: &[(&str, u8, bool)]) -> Self {
        self.with(|service| {
            let png = encode_for_tests(64, 64, shade);
            service.counter += 1;
            let hash = format!("a{:063x}", service.counter);
            service.textures.insert(hash.clone(), reencoded(&png));
            let mut owned = Vec::new();
            for (id, cape_shade, active) in capes {
                service.counter += 1;
                let key = format!("b{:063x}", service.counter);
                service
                    .textures
                    .insert(key.clone(), encode_for_tests(64, 32, *cape_shade));
                owned.push(RemoteCape {
                    id: (*id).to_owned(),
                    alias: format!("Capa {cape_shade}"),
                    texture: key,
                    active: *active,
                });
            }
            let remote = service
                .profile
                .as_mut()
                .unwrap_or_else(|| panic!("profile"));
            remote.skin = Some(RemoteSkin {
                texture: hash,
                model,
            });
            remote.capes = owned;
        });
        self
    }
}

impl SkinRemote for Fake {
    async fn profile(&self) -> Result<RemoteProfile, CoreError> {
        self.with(|service| {
            if service.offline {
                return Err(CoreError::new(ErrorCode::NetworkUnavailable));
            }
            service
                .profile
                .clone()
                .ok_or_else(|| CoreError::new(ErrorCode::NetworkUnavailable))
        })
    }

    async fn upload_skin(&self, model: SkinModel, png: &[u8]) -> Result<RemoteProfile, CoreError> {
        self.with(|service| {
            service.uploads.push((model, png.len()));
            service.counter += 1;
            let hash = format!("a{:063x}", service.counter);
            service.textures.insert(hash.clone(), reencoded(png));
            let remote = service
                .profile
                .as_mut()
                .unwrap_or_else(|| panic!("profile"));
            remote.skin = Some(RemoteSkin {
                texture: hash,
                model,
            });
            Ok(remote.clone())
        })
    }

    async fn choose_cape(&self, cape: Option<&str>) -> Result<RemoteProfile, CoreError> {
        self.with(|service| {
            service.cape_calls.push(cape.map(str::to_owned));
            if service.cape_fails {
                return Err(CoreError::new(ErrorCode::ServiceUnavailable));
            }
            let remote = service
                .profile
                .as_mut()
                .unwrap_or_else(|| panic!("profile"));
            for owned in &mut remote.capes {
                owned.active = cape == Some(owned.id.as_str());
            }
            Ok(remote.clone())
        })
    }

    async fn download_texture(&self, hash: &str) -> Result<Vec<u8>, CoreError> {
        self.with(|service| {
            service.downloads.push(hash.to_owned());
            if service.offline {
                return Err(CoreError::new(ErrorCode::NetworkUnavailable));
            }
            service
                .textures
                .get(hash)
                .cloned()
                .ok_or_else(|| CoreError::new(ErrorCode::InvalidData))
        })
    }
}

fn upload_file(shade: u8) -> Vec<u8> {
    encode_for_tests(64, 64, shade)
}

#[tokio::test]
async fn a_sync_keeps_the_skin_and_every_cape_and_reuses_them_afterwards() {
    let world = World::new();
    let fake = Fake::new().wearing(5, SkinModel::Slim, &[(CAPE_A, 1, true), (CAPE_B, 2, false)]);
    let view = world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync"));
    let current = view.current.clone().unwrap_or_else(|| panic!("current"));
    assert!(current.custom);
    assert_eq!(current.model, SkinModel::Slim);
    assert_eq!(current.cape.as_deref(), Some(CAPE_A));
    assert!(current.texture.is_some());
    assert_eq!(view.capes.len(), 2);
    assert!(view.capes.iter().all(|cape| cape.cached));
    assert!(
        view.capes
            .iter()
            .any(|cape| cape.id == CAPE_A && cape.active)
    );
    assert!(!view.incomplete);
    assert!(view.synced_ms.is_some());
    assert_eq!(world.skin_files().len(), 1);
    assert_eq!(fake.with(|service| service.downloads.len()), 3);

    // A second profile refresh finds everything already cached.
    let again = world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync again"));
    assert_eq!(fake.with(|service| service.downloads.len()), 3);
    assert_eq!(again.capes, view.capes);
    let bytes = world
        .manager
        .texture(TextureKind::Cape, CAPE_A)
        .unwrap_or_else(|_| panic!("cape image"));
    assert!(texture::prepare(&bytes, Kind::Cape).is_ok());
}

#[tokio::test]
async fn the_current_look_can_be_saved_once_and_is_then_recognised() {
    let world = World::new();
    let fake = Fake::new().wearing(5, SkinModel::Classic, &[(CAPE_A, 1, true)]);
    let view = world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync"));
    assert_eq!(view.matching_look, None);
    assert!(view.library.is_empty());

    let saved = world
        .manager
        .save_current(&profile(), "  Mi aspecto \u{7}")
        .unwrap_or_else(|_| panic!("save"));
    assert_eq!(saved.view.matching_look, Some(saved.id));
    assert_eq!(saved.view.library.len(), 1);
    assert_eq!(saved.view.library[0].name, "Mi aspecto");
    assert_eq!(saved.view.library[0].cape.as_deref(), Some(CAPE_A));
    // The skin is shared between the saved look and the worn one: one file.
    assert_eq!(world.skin_files().len(), 1);

    // Changing only the cape makes the current look differ from the saved one again.
    let changed = world
        .manager
        .apply_with(
            &fake,
            ApplyRequest {
                look_id: None,
                cape_change: Some(CapeChange { id: None }),
            },
        )
        .await
        .unwrap_or_else(|_| panic!("hide the cape"));
    assert_eq!(changed.matching_look, None);
    assert_eq!(changed.current.map(|current| current.cape), Some(None));
}

#[tokio::test]
async fn an_uploaded_skin_is_recognised_after_the_service_reencodes_it() {
    let world = World::new();
    let fake = Fake::new().wearing(1, SkinModel::Classic, &[(CAPE_A, 1, false)]);
    world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync"));
    let added = world
        .manager
        .add_look(
            &profile(),
            "Nueva",
            SkinModel::Slim,
            Some(CAPE_A.to_owned()),
            &upload_file(9),
        )
        .unwrap_or_else(|_| panic!("add"));
    let applied = world
        .manager
        .apply_with(
            &fake,
            ApplyRequest {
                look_id: Some(added.id),
                cape_change: None,
            },
        )
        .await
        .unwrap_or_else(|_| panic!("apply"));
    assert_eq!(
        fake.with(|service| service.uploads.iter().map(|u| u.0).collect::<Vec<_>>()),
        [SkinModel::Slim]
    );
    // The look's own cape went along with it.
    assert_eq!(
        fake.with(|service| service.cape_calls.clone()),
        [Some(CAPE_A.to_owned())]
    );
    assert_eq!(applied.matching_look, Some(added.id));
    let current = applied.current.unwrap_or_else(|| panic!("current"));
    assert_eq!(current.model, SkinModel::Slim);
    assert_eq!(current.cape.as_deref(), Some(CAPE_A));
    // The old worn skin is gone from the cache, the new one is shared with the look.
    assert_eq!(world.skin_files().len(), 1);
}

#[tokio::test]
async fn a_looks_cape_follows_what_the_account_owns() {
    let world = World::new();
    let fake = Fake::new().wearing(1, SkinModel::Classic, &[(CAPE_A, 1, true)]);
    world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync"));
    let unowned = "33333333-3333-4333-8333-333333333333";
    let foreign = world
        .manager
        .add_look(
            &profile(),
            "Ajena",
            SkinModel::Classic,
            Some(unowned.to_owned()),
            &upload_file(3),
        )
        .unwrap_or_else(|_| panic!("foreign"));
    let bare = world
        .manager
        .add_look(
            &profile(),
            "Sin capa",
            SkinModel::Classic,
            None,
            &upload_file(4),
        )
        .unwrap_or_else(|_| panic!("bare"));

    // A cape the account does not own cannot be worn, so the current one stays.
    world
        .manager
        .apply_with(
            &fake,
            ApplyRequest {
                look_id: Some(foreign.id),
                cape_change: None,
            },
        )
        .await
        .unwrap_or_else(|_| panic!("foreign apply"));
    assert!(fake.with(|service| service.cape_calls.is_empty()));

    // A look without a cape hides the one being worn.
    let hidden = world
        .manager
        .apply_with(
            &fake,
            ApplyRequest {
                look_id: Some(bare.id),
                cape_change: None,
            },
        )
        .await
        .unwrap_or_else(|_| panic!("bare apply"));
    assert_eq!(fake.with(|service| service.cape_calls.clone()), [None]);
    assert_eq!(hidden.matching_look, Some(bare.id));
}

#[tokio::test]
async fn an_explicit_cape_the_account_does_not_own_changes_nothing() {
    let world = World::new();
    let fake = Fake::new().wearing(1, SkinModel::Classic, &[(CAPE_A, 1, false)]);
    world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync"));
    let added = world
        .manager
        .add_look(
            &profile(),
            "Otra",
            SkinModel::Classic,
            None,
            &upload_file(8),
        )
        .unwrap_or_else(|_| panic!("add"));
    let result = world
        .manager
        .apply_with(
            &fake,
            ApplyRequest {
                look_id: Some(added.id),
                cape_change: Some(CapeChange {
                    id: Some(CAPE_B.to_owned()),
                }),
            },
        )
        .await;
    assert_eq!(
        result.err().map(|error| error.code()),
        Some(ErrorCode::InvalidData)
    );
    assert!(fake.with(|service| service.uploads.is_empty() && service.cape_calls.is_empty()));
    for bad in ["../x", "a/b", ""] {
        let result = world
            .manager
            .apply_with(
                &fake,
                ApplyRequest {
                    look_id: None,
                    cape_change: Some(CapeChange {
                        id: Some(bad.to_owned()),
                    }),
                },
            )
            .await;
        assert_eq!(
            result.err().map(|error| error.code()),
            Some(ErrorCode::InvalidData),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn a_failed_second_step_still_records_the_first() {
    let world = World::new();
    let fake = Fake::new().wearing(1, SkinModel::Classic, &[(CAPE_A, 1, false)]);
    world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync"));
    let added = world
        .manager
        .add_look(
            &profile(),
            "Con capa",
            SkinModel::Slim,
            Some(CAPE_A.to_owned()),
            &upload_file(6),
        )
        .unwrap_or_else(|_| panic!("add"));
    fake.with(|service| service.cape_fails = true);
    let result = world
        .manager
        .apply_with(
            &fake,
            ApplyRequest {
                look_id: Some(added.id),
                cape_change: None,
            },
        )
        .await;
    assert_eq!(
        result.err().map(|error| error.code()),
        Some(ErrorCode::ServiceUnavailable)
    );
    // The skin did change on the service, so the cache must say so.
    let view = world
        .manager
        .view(&profile())
        .unwrap_or_else(|_| panic!("view"));
    let current = view.current.unwrap_or_else(|| panic!("current"));
    assert_eq!(current.model, SkinModel::Slim);
    assert_eq!(current.cape, None);
}

#[tokio::test]
async fn images_that_could_not_be_fetched_are_reported_and_retried() {
    let world = World::new();
    let fake = Fake::new().wearing(5, SkinModel::Classic, &[(CAPE_A, 1, true)]);
    fake.with(|service| service.textures.clear());
    let view = world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync"));
    assert!(view.incomplete);
    let current = view.current.unwrap_or_else(|| panic!("current"));
    assert!(current.custom);
    assert_eq!(current.texture, None);
    assert!(view.capes.iter().all(|cape| !cape.cached));
    assert_eq!(
        world
            .manager
            .save_current(&profile(), "x")
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::InvalidData)
    );

    let healed = Fake::new().wearing(5, SkinModel::Classic, &[(CAPE_A, 1, true)]);
    let view = world
        .manager
        .sync_with(&healed)
        .await
        .unwrap_or_else(|_| panic!("sync again"));
    assert!(!view.incomplete);
    assert!(
        view.current
            .is_some_and(|current| current.texture.is_some())
    );
}

#[tokio::test]
async fn an_account_without_a_skin_of_its_own_has_nothing_to_save() {
    let world = World::new();
    let fake = Fake::new();
    let view = world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync"));
    let current = view.current.unwrap_or_else(|| panic!("current"));
    assert!(!current.custom);
    assert_eq!(current.texture, None);
    assert!(!view.incomplete);
    assert!(world.manager.save_current(&profile(), "x").is_err());
}

#[tokio::test]
async fn an_offline_service_fails_without_touching_the_library() {
    let world = World::new();
    let fake = Fake::new().wearing(5, SkinModel::Classic, &[]);
    world
        .manager
        .sync_with(&fake)
        .await
        .unwrap_or_else(|_| panic!("sync"));
    let before = world.index_text();
    fake.with(|service| service.offline = true);
    let result = world.manager.sync_with(&fake).await;
    assert_eq!(
        result.err().map(|error| error.code()),
        Some(ErrorCode::NetworkUnavailable)
    );
    assert_eq!(world.index_text(), before);
    // The saved library stays usable without a connection.
    assert!(world.manager.view(&profile()).is_ok());
}

#[test]
fn deleting_a_look_removes_its_file_unless_the_account_still_wears_it() {
    let world = World::new();
    let first = world
        .manager
        .add_look(&profile(), "Una", SkinModel::Classic, None, &upload_file(1))
        .unwrap_or_else(|_| panic!("first"));
    let second = world
        .manager
        .add_look(&profile(), "Dos", SkinModel::Classic, None, &upload_file(1))
        .unwrap_or_else(|_| panic!("second"));
    // Identical pixels share one file.
    assert_eq!(world.skin_files().len(), 1);
    world
        .manager
        .delete_look(&profile(), first.id)
        .unwrap_or_else(|_| panic!("delete"));
    assert_eq!(world.skin_files().len(), 1);
    let view = world
        .manager
        .delete_look(&profile(), second.id)
        .unwrap_or_else(|_| panic!("delete"));
    assert!(view.library.is_empty());
    assert!(world.skin_files().is_empty());
    assert_eq!(
        world
            .manager
            .delete_look(&profile(), second.id)
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::InvalidData)
    );
}

#[test]
fn editing_renames_switches_the_model_and_can_replace_the_file() {
    let world = World::new();
    let added = world
        .manager
        .add_look(
            &profile(),
            "Vieja",
            SkinModel::Classic,
            None,
            &upload_file(1),
        )
        .unwrap_or_else(|_| panic!("add"));
    let old = added.view.library[0].texture.clone();
    let renamed = world
        .manager
        .edit_look(
            &profile(),
            added.id,
            "Nueva",
            SkinModel::Slim,
            Some(CAPE_A.to_owned()),
            None,
        )
        .unwrap_or_else(|_| panic!("rename"));
    assert_eq!(renamed.library[0].name, "Nueva");
    assert_eq!(renamed.library[0].model, SkinModel::Slim);
    assert_eq!(renamed.library[0].cape.as_deref(), Some(CAPE_A));
    assert_eq!(renamed.library[0].texture, old);
    let replaced = world
        .manager
        .edit_look(
            &profile(),
            added.id,
            "Nueva",
            SkinModel::Slim,
            None,
            Some(&upload_file(2)),
        )
        .unwrap_or_else(|_| panic!("replace"));
    assert_ne!(replaced.library[0].texture, old);
    assert_eq!(world.skin_files().len(), 1, "the replaced file is removed");
    assert!(
        world
            .manager
            .edit_look(
                &profile(),
                Uuid::new_v4(),
                "x",
                SkinModel::Classic,
                None,
                None
            )
            .is_err()
    );
    assert!(
        world
            .manager
            .edit_look(
                &profile(),
                added.id,
                "x",
                SkinModel::Classic,
                None,
                Some(b"junk")
            )
            .is_err()
    );
}

#[test]
fn names_files_and_capacity_are_bounded() {
    let world = World::new();
    for name in ["", "   ", "\u{7}", &"x".repeat(33)] {
        assert_eq!(
            world
                .manager
                .add_look(&profile(), name, SkinModel::Classic, None, &upload_file(1))
                .err()
                .map(|e| e.code()),
            Some(ErrorCode::InvalidData),
            "{name:?}"
        );
    }
    assert_eq!(
        world
            .manager
            .add_look(&profile(), "ok", SkinModel::Classic, None, b"not a png")
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::InvalidSkin)
    );
    assert_eq!(
        world
            .manager
            .add_look(
                &profile(),
                "ok",
                SkinModel::Classic,
                Some("../x".to_owned()),
                &upload_file(1)
            )
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::InvalidData)
    );
    let library = Library::open(&world.paths).unwrap_or_else(|_| panic!("library"));
    library
        .update(|index, _| {
            for number in 0..MAX_LOOKS {
                index.looks.push(DiskLook {
                    id: Uuid::new_v4(),
                    name: format!("Look {number}"),
                    model: SkinModel::Classic,
                    texture: "0".repeat(64),
                    cape: None,
                    created_ms: number as u64,
                });
            }
            Ok(())
        })
        .unwrap_or_else(|_| panic!("fill"));
    assert_eq!(
        world
            .manager
            .add_look(
                &profile(),
                "one more",
                SkinModel::Classic,
                None,
                &upload_file(1)
            )
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::SkinLibraryFull)
    );
}

#[test]
fn a_damaged_or_future_index_is_reported_and_left_untouched() {
    let world = World::new();
    world
        .manager
        .add_look(&profile(), "Una", SkinModel::Classic, None, &upload_file(1))
        .unwrap_or_else(|_| panic!("add"));
    let path = world.paths.root().join("skins.json");
    for broken in [
        "{ not json".to_owned(),
        r#"{"schema":2,"looks":[]}"#.to_owned(),
        r#"{"schema":1,"looks":[{"id":"6f1c6f0e-0f0c-4a43-a4a6-0d4b7d1a1c11","name":"x","model":"classic","texture":"../../accounts","cape":null,"created_ms":1}]}"#.to_owned(),
        r#"{"schema":1,"capes":{"../escape":{"texture_key":"abc"}}}"#.to_owned(),
    ] {
        std::fs::write(&path, &broken).unwrap_or_else(|_| panic!("write"));
        assert_eq!(world.manager.view(&profile()).err().map(|e| e.code()), Some(ErrorCode::InvalidData), "{broken}");
        assert!(world.manager.add_look(&profile(), "x", SkinModel::Classic, None, &upload_file(2)).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), broken);
    }
}

#[test]
fn only_validated_names_reach_the_image_folders() {
    let world = World::new();
    world
        .manager
        .view(&profile())
        .unwrap_or_else(|_| panic!("view"));
    for key in [
        "../skins.json",
        "a/b",
        "",
        "ABC",
        &"g".repeat(64),
        &"a".repeat(63),
    ] {
        assert!(
            world.manager.texture(TextureKind::Skin, key).is_err(),
            "{key}"
        );
    }
    for key in ["../x", "a b", ""] {
        assert!(
            world.manager.texture(TextureKind::Cape, key).is_err(),
            "{key}"
        );
    }
    assert!(
        world
            .manager
            .texture(TextureKind::Skin, &"a".repeat(64))
            .is_err(),
        "missing"
    );
}
