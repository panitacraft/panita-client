use super::*;
use crate::{
    minecraft::play::tests::{complete_installation, fixture},
    modpack::archive::tests::build,
    test_http::{Reply, Server},
};
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::collections::BTreeMap;

const BASE: &str = "/data/project/versions/version/";
const ARCHIVE: &str = "/storage/v1/object/public/modpacks/by-hash/sha256-abc/defaults.zip";

struct World {
    paths: AppPaths,
    id: Uuid,
    root: std::path::PathBuf,
    base: InstalledLock,
}

fn world() -> World {
    let (paths, accounts, id) = fixture();
    drop(accounts);
    complete_installation(&paths, id);
    let root = metadata_root(&paths, id).unwrap_or_else(|_| panic!("root"));
    let base = read_json::<InstalledLock>(&root, "installed.json")
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("base"));
    World {
        paths,
        id,
        root,
        base,
    }
}

fn mod_json(path: &str, body: &[u8]) -> Value {
    let name = path.rsplit('/').next().unwrap_or_default();
    json!({"type": if path.starts_with("mods/") { "mod" } else if path.starts_with("shaderpacks/") { "shaderpack" } else { "resourcepack" },
        "source": "modrinth", "path": path, "name": name,
        "env": {"client": "required", "server": "required"},
        "url": format!("https://cdn.modrinth.com/data/project/versions/version/{name}"),
        "hashes": {"sha1": format!("{:x}", Sha1::digest(body)), "sha512": format!("{:x}", Sha512::digest(body))},
        "size": body.len()})
}

fn curseforge_json(path: &str, body: &[u8], manual: bool) -> Value {
    let mut value = json!({"type": "mod", "source": "curseforge", "path": path, "name": "CrazyPhone",
        "env": {"client": "required", "server": "required"},
        "hashes": {"sha1": format!("{:x}", Sha1::digest(body))}, "size": body.len(),
        "curseforge": {"project_id": 1646289, "file_id": 9089413}});
    if manual {
        value["manual"] = json!(true);
    } else {
        value["url"] = json!("https://edge.forgecdn.net/files/1/2/cf.jar");
    }
    value
}

fn archive_json(bytes: &[u8]) -> Value {
    json!({"url": format!("https://jqleaywgcoedgvglrvhc.supabase.co{ARCHIVE}"),
        "sha256": format!("{:x}", Sha256::digest(bytes)), "size": bytes.len()})
}

fn manifest(id: Uuid, files: Vec<Value>, defaults: Option<Value>) -> Manifest {
    let version = Uuid::new_v4();
    let document = json!({"data": {
        "instance_id": id, "version_id": version, "number": 2, "label": "0.2",
        "published_at": "2026-10-25T20:54:25Z", "minecraft_version": "1.21.1",
        "loader": "neoforge", "loader_version": "21.1.264", "files": files,
        "archives": {"defaults": defaults, "enforced": null}}});
    Manifest::parse(document.to_string().as_bytes(), id, version)
        .unwrap_or_else(|_| panic!("manifest"))
}

fn server(routes: BTreeMap<String, Vec<u8>>) -> Server {
    Server::start(move |request| match routes.get(&request.path) {
        Some(body) => Reply::ok(body.clone()),
        None => Reply::status(404),
    })
}

async fn run(
    world: &World,
    manifest: &Manifest,
    server: &Server,
    cancel: &Cancellation,
) -> Result<Outcome, CoreError> {
    let downloader = PackDownloader::for_tests(server.base());
    publish(Publish {
        paths: &world.paths,
        id: world.id,
        root: &world.root,
        base: world.base.clone(),
        manifest,
        downloader: &downloader,
        cancel,
        progress: Arc::new(|_| {}),
    })
    .await
}

fn read(world: &World, path: &str) -> Vec<u8> {
    std::fs::read(world.root.join("minecraft").join(path)).unwrap_or_default()
}

fn exists(world: &World, path: &str) -> bool {
    world.root.join("minecraft").join(path).exists()
}

fn installed(world: &World) -> Option<InstalledLock> {
    read_json::<InstalledLock>(&world.root, "installed.json").unwrap_or_default()
}

fn state(world: &World) -> Option<InstallationState> {
    InstanceRegistry::new(world.paths.clone())
        .get(world.id)
        .unwrap_or_default()
        .map(|record| record.state)
}

fn cleanup(world: &World) {
    let _ = std::fs::remove_dir_all(world.paths.root());
}

fn code<T>(result: Result<T, CoreError>) -> Option<ErrorCode> {
    result.err().map(|error| error.code())
}

use std::sync::Arc;

#[tokio::test]
async fn a_pack_installs_content_defaults_and_publishes_schema_five() {
    let world = world();
    let (a, b, pack) = (
        b"mod a".to_vec(),
        b"mod b optional".to_vec(),
        b"resource pack".to_vec(),
    );
    let defaults = build(&[("options.txt", b"fov:90"), ("config/a.toml", b"a = 1\n")]);
    let mut optional = mod_json("mods/b.jar", &b);
    optional["env"] = json!({"client": "optional", "server": "unsupported"});
    let mut server_only = mod_json("mods/server-only.jar", b"never installed");
    server_only["env"] = json!({"client": "unsupported", "server": "required"});
    let manifest = manifest(
        world.id,
        vec![
            mod_json("mods/a.jar", &a),
            optional,
            server_only,
            mod_json("resourcepacks/p.zip", &pack),
        ],
        Some(archive_json(&defaults)),
    );
    let routes = BTreeMap::from([
        (format!("{BASE}a.jar"), a.clone()),
        (format!("{BASE}b.jar"), b.clone()),
        (format!("{BASE}p.zip"), pack.clone()),
        (ARCHIVE.to_owned(), defaults),
    ]);
    let server = server(routes);
    let outcome = run(&world, &manifest, &server, &Cancellation::default()).await;
    let Ok(Outcome::Installed(report)) = outcome else {
        panic!("install: {:?}", code(outcome));
    };
    assert!(report.awaiting.is_empty() && report.skipped_configuration.is_empty());
    assert_eq!(read(&world, "mods/a.jar"), a);
    assert_eq!(read(&world, "mods/b.jar"), b);
    assert_eq!(read(&world, "resourcepacks/p.zip"), pack);
    assert_eq!(read(&world, "options.txt"), b"fov:90");
    assert_eq!(read(&world, "config/a.toml"), b"a = 1\n");
    // Client-unsupported entries are neither downloaded nor installed.
    assert!(!exists(&world, "mods/server-only.jar"));
    assert!(
        !server
            .requests()
            .iter()
            .any(|r| r.path.ends_with("server-only.jar"))
    );
    let lock = installed(&world).unwrap_or_else(|| panic!("installed"));
    let Some(pack_state) = &lock.pack else {
        panic!("pack state");
    };
    assert_eq!(pack_state.files.len(), 5);
    assert!(pack_state.consumed_defaults.contains("options.txt"));
    assert!(pack_state.pending.is_empty());
    let on_disk = std::fs::read_to_string(world.root.join("installed.json")).unwrap_or_default();
    assert!(on_disk.contains("\"schema\": 5") && on_disk.contains("\"pack\""));
    assert!(!world.root.join("transaction.json").exists());
    assert_eq!(state(&world), Some(InstallationState::Installed));
    let game = GameDir::open(&world.root.join("minecraft")).unwrap_or_else(|_| panic!("game"));
    assert!(verify_files(&game, pack_state, &Cancellation::default()).is_ok());
    // A second instance pass reuses the cache; nothing is downloaded again.
    let before = server.requests().len();
    let again = world_copy(&world);
    let _ = run(&again, &manifest, &server, &Cancellation::default()).await;
    assert_eq!(server.requests().len(), before);
    cleanup(&world);
}

/// Another empty game directory under the same data root shares its cache.
fn world_copy(world: &World) -> World {
    let other = world.root.join("other-instance");
    assert!(private_fs::PrivateDir::ensure(&other).is_ok());
    assert!(private_fs::PrivateDir::ensure(other.join("minecraft")).is_ok());
    World {
        paths: world.paths.clone(),
        id: world.id,
        root: other,
        base: world.base.clone(),
    }
}

#[tokio::test]
async fn manual_and_withdrawn_files_wait_in_one_list_and_finish_offline() {
    let world = world();
    let (plain, manual_body, gone_body) = (
        b"plain".to_vec(),
        b"manual jar".to_vec(),
        b"withdrawn jar".to_vec(),
    );
    let manifest = manifest(
        world.id,
        vec![
            mod_json("mods/plain.jar", &plain),
            curseforge_json("mods/manual.jar", &manual_body, true),
            curseforge_json("mods/gone.jar", &gone_body, false),
        ],
        None,
    );
    // The CurseForge CDN answers 404 for the withdrawn file.
    let server = server(BTreeMap::from([(
        format!("{BASE}plain.jar"),
        plain.clone(),
    )]));
    let outcome = run(&world, &manifest, &server, &Cancellation::default()).await;
    let Ok(Outcome::Installed(report)) = outcome else {
        panic!("install: {:?}", code(outcome));
    };
    let mut waiting: Vec<_> = report
        .awaiting
        .iter()
        .map(|f| (f.path.as_str(), f.reason))
        .collect();
    waiting.sort();
    assert_eq!(
        waiting,
        [
            ("mods/gone.jar", PendingReason::Withdrawn),
            ("mods/manual.jar", PendingReason::Manual)
        ]
    );
    assert_eq!(read(&world, "mods/plain.jar"), plain);
    assert_eq!(state(&world), Some(InstallationState::AwaitingFiles));
    assert_eq!(
        pending_files(&world.paths, world.id).map(|p| p.len()).ok(),
        Some(2)
    );

    // From here no network, session or catalog is involved.
    let cancel = Cancellation::default();
    let picked = world.root.join("picked.jar");
    assert!(std::fs::write(&picked, &manual_body).is_ok());
    let left = import_pending(&world.paths, world.id, "mods/manual.jar", &picked, &cancel).await;
    assert_eq!(left.map(|l| l.len()).ok(), Some(1));
    assert_eq!(read(&world, "mods/manual.jar"), manual_body);
    assert_eq!(state(&world), Some(InstallationState::AwaitingFiles));
    // A wrong file is rejected and keeps the list as it was.
    assert!(std::fs::write(&picked, b"not the expected version").is_ok());
    assert_eq!(
        code(import_pending(&world.paths, world.id, "mods/gone.jar", &picked, &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert_eq!(
        pending_files(&world.paths, world.id).map(|p| p.len()).ok(),
        Some(1)
    );
    // The player places the last file by hand and presses Verificar.
    assert!(std::fs::write(world.root.join("minecraft/mods/gone.jar"), &gone_body).is_ok());
    let left = refresh_pending(&world.paths, world.id, &cancel);
    assert_eq!(left.map(|l| l.len()).ok(), Some(0));
    assert_eq!(state(&world), Some(InstallationState::Installed));
    let lock = installed(&world).unwrap_or_else(|| panic!("installed"));
    assert!(
        lock.pack
            .is_some_and(|p| p.pending.is_empty() && p.files.len() == 3)
    );
    assert!(
        import_pending(&world.paths, world.id, "mods/none.jar", &picked, &cancel)
            .await
            .is_err()
    );
    cleanup(&world);
}

#[tokio::test]
async fn existing_player_files_block_content_and_never_get_overwritten() {
    let world = world();
    let body = b"pack mod".to_vec();
    let manifest = manifest(world.id, vec![mod_json("mods/a.jar", &body)], None);
    assert!(std::fs::create_dir_all(world.root.join("minecraft/mods")).is_ok());
    assert!(std::fs::write(world.root.join("minecraft/mods/a.jar"), b"player's own mod").is_ok());
    let server = server(BTreeMap::from([(format!("{BASE}a.jar"), body)]));
    let outcome = run(&world, &manifest, &server, &Cancellation::default()).await;
    assert_eq!(
        outcome.ok(),
        Some(Outcome::Conflicts(vec!["mods/a.jar".into()]))
    );
    assert_eq!(read(&world, "mods/a.jar"), b"player's own mod");
    // Preflight stops before any download, journal or published pointer.
    assert!(server.requests().is_empty());
    assert!(!world.root.join("transaction.json").exists());
    assert!(installed(&world).is_some_and(|lock| lock.pack.is_none()));
    cleanup(&world);
}

#[tokio::test]
async fn existing_configuration_is_kept_whole_and_reported() {
    let world = world();
    let body = b"pack mod".to_vec();
    let defaults = build(&[("options.txt", b"fov:90"), ("config/a.toml", b"a = 1\n")]);
    let manifest = manifest(
        world.id,
        vec![mod_json("mods/a.jar", &body)],
        Some(archive_json(&defaults)),
    );
    assert!(std::fs::write(world.root.join("minecraft/options.txt"), b"fov:110").is_ok());
    let server = server(BTreeMap::from([
        (format!("{BASE}a.jar"), body),
        (ARCHIVE.to_owned(), defaults),
    ]));
    let outcome = run(&world, &manifest, &server, &Cancellation::default()).await;
    let Ok(Outcome::Installed(report)) = outcome else {
        panic!("install");
    };
    assert_eq!(report.skipped_configuration, ["options.txt"]);
    assert_eq!(read(&world, "options.txt"), b"fov:110");
    assert_eq!(read(&world, "config/a.toml"), b"a = 1\n");
    cleanup(&world);
}

#[tokio::test]
async fn a_failure_after_the_journal_removes_only_what_the_operation_created() {
    let world = world();
    let body = b"pack mod".to_vec();
    // The directory claims 100 bytes while the entry inflates far beyond it.
    let mut lying = build(&[("config/big.txt", &vec![b'0'; 10_000])]);
    let at = lying
        .windows(4)
        .position(|w| w == b"PK\x01\x02")
        .unwrap_or_default()
        + 24;
    lying[at..at + 4].copy_from_slice(&100u32.to_le_bytes());
    let manifest = manifest(
        world.id,
        vec![mod_json("mods/a.jar", &body)],
        Some(archive_json(&lying)),
    );
    assert!(std::fs::create_dir_all(world.root.join("minecraft/saves")).is_ok());
    assert!(std::fs::write(world.root.join("minecraft/saves/keep.dat"), b"world").is_ok());
    let server = server(BTreeMap::from([
        (format!("{BASE}a.jar"), body),
        (ARCHIVE.to_owned(), lying),
    ]));
    let outcome = run(&world, &manifest, &server, &Cancellation::default()).await;
    assert!(outcome.is_err());
    assert!(!exists(&world, "mods/a.jar"));
    assert!(!exists(&world, "config/big.txt"));
    assert!(!world.root.join("transaction.json").exists());
    assert!(installed(&world).is_some_and(|lock| lock.pack.is_none()));
    assert_eq!(read(&world, "saves/keep.dat"), b"world");
    cleanup(&world);
}

#[tokio::test]
async fn cancellation_before_the_commit_leaves_no_trace() {
    let world = world();
    let body = b"pack mod".to_vec();
    let manifest = manifest(world.id, vec![mod_json("mods/a.jar", &body)], None);
    let server = server(BTreeMap::from([(format!("{BASE}a.jar"), body)]));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        code(run(&world, &manifest, &server, &cancel).await),
        Some(ErrorCode::OperationCancelled)
    );
    assert!(!exists(&world, "mods/a.jar"));
    assert!(!world.root.join("transaction.json").exists());
    cleanup(&world);
}

fn journal(world: &World, created: Vec<Created>) -> Journal {
    Journal {
        schema_version: 4,
        kind: KIND.into(),
        instance_id: world.id,
        operation: world.base.operation,
        active: true,
        created,
    }
}

#[tokio::test]
async fn an_interrupted_uncommitted_install_rolls_back_only_matching_created_files() {
    let world = world();
    let body = b"created by the operation".to_vec();
    let theirs = b"player replaced this one".to_vec();
    assert!(std::fs::create_dir_all(world.root.join("minecraft/mods")).is_ok());
    assert!(std::fs::write(world.root.join("minecraft/mods/ours.jar"), &body).is_ok());
    assert!(std::fs::write(world.root.join("minecraft/mods/theirs.jar"), &theirs).is_ok());
    assert!(std::fs::write(world.root.join("minecraft/config.txt"), b"declared size").is_ok());
    let created = vec![
        Created {
            path: "mods/ours.jar".into(),
            size: body.len() as u64,
            sha1: Some(format!("{:x}", Sha1::digest(&body))),
            sha512: None,
        },
        // A different file now sits where this operation planned one: it is kept.
        Created {
            path: "mods/theirs.jar".into(),
            size: 3,
            sha1: Some("a".repeat(40)),
            sha512: None,
        },
        Created {
            path: "config.txt".into(),
            size: 13,
            sha1: None,
            sha512: None,
        },
        Created {
            path: "mods/never-written.jar".into(),
            size: 1,
            sha1: Some("b".repeat(40)),
            sha512: None,
        },
    ];
    assert!(write_json(&world.root, "transaction.json", &journal(&world, created)).is_ok());
    // While the intent is active, Play-side reads refuse the installation.
    assert!(crate::minecraft::install::verify_installed(&world.paths, world.id).is_err());
    assert_eq!(
        recover_locked(&world.paths, world.id, &world.root).ok(),
        Some(true)
    );
    assert!(!exists(&world, "mods/ours.jar"));
    assert!(!exists(&world, "config.txt"));
    assert_eq!(read(&world, "mods/theirs.jar"), theirs);
    assert!(!world.root.join("transaction.json").exists());
    assert_eq!(state(&world), Some(InstallationState::Failed));
    assert_eq!(
        recover_locked(&world.paths, world.id, &world.root).ok(),
        Some(false)
    );
    cleanup(&world);
}

#[tokio::test]
async fn an_interrupted_committed_install_rolls_forward_after_an_audit() {
    let world = world();
    let body = b"pack mod".to_vec();
    let manifest = manifest(world.id, vec![mod_json("mods/a.jar", &body)], None);
    let server = server(BTreeMap::from([(format!("{BASE}a.jar"), body.clone())]));
    assert!(matches!(
        run(&world, &manifest, &server, &Cancellation::default()).await,
        Ok(Outcome::Installed(_))
    ));
    // The pointer is durable but the journal was never retired.
    let created = vec![Created {
        path: "mods/a.jar".into(),
        size: body.len() as u64,
        sha1: None,
        sha512: None,
    }];
    assert!(
        write_json(
            &world.root,
            "transaction.json",
            &journal(&world, created.clone())
        )
        .is_ok()
    );
    assert_eq!(
        recover_locked(&world.paths, world.id, &world.root).ok(),
        Some(true)
    );
    assert_eq!(
        read(&world, "mods/a.jar"),
        body,
        "a committed file is never rolled back"
    );
    assert_eq!(state(&world), Some(InstallationState::Installed));
    assert!(!world.root.join("transaction.json").exists());
    // A damaged file blocks the roll-forward and keeps the intent for a later retry.
    assert!(write_json(&world.root, "transaction.json", &journal(&world, created)).is_ok());
    assert!(std::fs::write(world.root.join("minecraft/mods/a.jar"), b"damaged!").is_ok());
    assert!(recover_locked(&world.paths, world.id, &world.root).is_err());
    assert!(world.root.join("transaction.json").exists());
    cleanup(&world);
}

#[tokio::test]
async fn journals_and_schema_five_documents_reject_inconsistent_data() {
    let world = world();
    let mut bad = journal(&world, Vec::new());
    bad.instance_id = Uuid::new_v4();
    assert!(!journal_inactive(
        serde_json::to_value(&bad).unwrap_or_default(),
        world.id
    ));
    let mut inactive = journal(&world, Vec::new());
    inactive.active = false;
    assert!(journal_inactive(
        serde_json::to_value(&inactive).unwrap_or_default(),
        world.id
    ));
    inactive.created.push(Created {
        path: "../escape".into(),
        size: 1,
        sha1: None,
        sha512: None,
    });
    assert!(inactive.validate(world.id).is_err());

    // Disk schema 4 is vanilla and schema 5 is exactly a pack installation.
    let vanilla = serde_json::to_value(&world.base).unwrap_or_default();
    assert_eq!(vanilla["schema"], 4);
    let mut tampered = vanilla.clone();
    tampered["pack"] = json!({"schema": 1});
    assert!(serde_json::from_value::<InstalledLock>(tampered).is_err());
    let mut fake = vanilla.clone();
    fake["schema"] = json!(5);
    assert!(serde_json::from_value::<InstalledLock>(fake).is_err());
    assert!(serde_json::from_value::<InstalledLock>(vanilla).is_ok());
    cleanup(&world);
}

// ---- Pinned Modrinth packs -------------------------------------------------

const PACK_PATH: &str = "/data/project/versions/version/pack.mrpack";

fn mrpack_catalog(id: Uuid, loader: Option<&str>, minecraft: Option<&str>) -> CatalogInstance {
    CatalogInstance {
        id,
        slug: "pixelmon".into(),
        name: "Pixelmon".into(),
        description: "Pack".into(),
        minecraft_version: minecraft.map(str::to_owned),
        enabled: true,
        restricted: false,
        min_launcher_version: None,
        icon_url: None,
        banner_url: None,
        sort_order: 1,
        updated_at: "2026-10-27T00:00:00Z".into(),
        source: CatalogSource::Modrinth {
            project_id: "project".into(),
            pack_version_id: "version".into(),
            loader: loader.map(str::to_owned),
        },
    }
}

fn modrinth_url(name: &str) -> String {
    format!("https://cdn.modrinth.com{BASE}{name}")
}

fn listed(path: &str, body: &[u8], downloads: &[String]) -> Value {
    json!({"path": path, "env": {"client": "required", "server": "required"},
        "hashes": {"sha1": format!("{:x}", Sha1::digest(body)), "sha512": format!("{:x}", Sha512::digest(body))},
        "downloads": downloads, "fileSize": body.len()})
}

fn neoforge() -> Value {
    json!({"minecraft": "1.21.1", "neoforge": "21.1.248"})
}

fn index_document(files: Vec<Value>, dependencies: Value) -> Vec<u8> {
    json!({"formatVersion": 1, "game": "minecraft", "versionId": "9.4.1", "name": "Fixture pack",
        "files": files, "dependencies": dependencies})
    .to_string()
    .into_bytes()
}

fn mrpack(index: &[u8], entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut all: Vec<(&str, &[u8])> = vec![("modrinth.index.json", index)];
    all.extend_from_slice(entries);
    build(&all)
}

fn archive_expectation(archive: &[u8]) -> Expected {
    Expected {
        size: archive.len() as u64,
        sha1: Some(format!("{:x}", Sha1::digest(archive))),
        sha256: None,
        sha512: Some(format!("{:x}", Sha512::digest(archive))),
    }
}

/// Download the archive the way an installation does, then reduce it to a plan.
async fn plan_for(
    world: &World,
    catalog: &CatalogInstance,
    archive: &[u8],
    server: &Server,
) -> Result<Plan, CoreError> {
    let downloader = PackDownloader::for_tests(server.base());
    let expected = archive_expectation(archive);
    let artifact = PackArtifact {
        url: modrinth_url("pack.mrpack"),
        hashes: Hashes {
            sha1: expected.sha1.clone(),
            sha512: expected.sha512.clone(),
        },
        size: expected.size,
    };
    let request = Request {
        url: artifact.url.clone(),
        origin: Origin::Modrinth,
        expected: expected.clone(),
        fallbacks: Vec::new(),
    };
    let cancel = Cancellation::default();
    content::ensure_all(
        &world.paths,
        &downloader,
        &[request],
        &cancel,
        Arc::new(|_| {}),
    )
    .await?;
    let inspected = mrpack::inspect(&world.paths, &expected, &cancel)?;
    Plan::mrpack(
        catalog, "project", "version", &artifact, expected, inspected,
    )
}

async fn run_plan(
    world: &World,
    plan: Plan,
    server: &Server,
    cancel: &Cancellation,
) -> Result<Outcome, CoreError> {
    let downloader = PackDownloader::for_tests(server.base());
    publish_plan(PlanPublish {
        paths: &world.paths,
        id: world.id,
        root: &world.root,
        base: world.base.clone(),
        plan,
        downloader: &downloader,
        cancel,
        progress: Arc::new(|_| {}),
    })
    .await
}

struct Fixture {
    archive: Vec<u8>,
    index: Vec<u8>,
    server: Server,
}

/// A pack with a mirrored mod, an optional mod, a server-only mod, a shader and
/// every override layer.
fn pixelmon_like() -> Fixture {
    let (a, b, shader) = (
        b"mod a".to_vec(),
        b"optional mod".to_vec(),
        b"shader pack".to_vec(),
    );
    let mut optional = listed("mods/b.jar", &b, &[modrinth_url("b.jar")]);
    optional["env"] = json!({"client": "optional", "server": "unsupported"});
    let mut server_only = listed(
        "mods/server-only.jar",
        b"never installed",
        &[modrinth_url("server-only.jar")],
    );
    server_only["env"] = json!({"client": "unsupported", "server": "required"});
    let index = index_document(
        vec![
            // The first mirror is gone; the second one serves the file.
            listed(
                "mods/a.jar",
                &a,
                &[modrinth_url("gone.jar"), modrinth_url("a.jar")],
            ),
            optional,
            server_only,
            listed("shaderpacks/s.zip", &shader, &[modrinth_url("s.zip")]),
        ],
        neoforge(),
    );
    let archive = mrpack(
        &index,
        &[
            ("overrides/options.txt", b"fov:90"),
            ("overrides/config/a.toml", b"common = 1\n"),
            ("client-overrides/config/a.toml", b"client = 1\n"),
            ("overrides/mods/bundled.jar", b"bundled"),
            ("server-overrides/config/server.toml", b"server\n"),
        ],
    );
    let server = server(BTreeMap::from([
        (PACK_PATH.to_owned(), archive.clone()),
        (format!("{BASE}a.jar"), a),
        (format!("{BASE}b.jar"), b),
        (format!("{BASE}s.zip"), shader),
    ]));
    Fixture {
        archive,
        index,
        server,
    }
}

#[tokio::test]
async fn a_pinned_mrpack_installs_files_and_overrides_and_records_its_identity() {
    let world = world();
    let fixture = pixelmon_like();
    let catalog = mrpack_catalog(world.id, Some("neoforge"), None);
    let plan = plan_for(&world, &catalog, &fixture.archive, &fixture.server)
        .await
        .unwrap_or_else(|error| panic!("plan: {:?}", error.code()));
    assert_eq!(plan.minecraft_version, "1.21.1");
    assert_eq!(
        (plan.loader.as_str(), plan.loader_version.as_str()),
        ("neoforge", "21.1.248")
    );
    let outcome = run_plan(
        &world,
        plan.clone(),
        &fixture.server,
        &Cancellation::default(),
    )
    .await;
    let Ok(Outcome::Installed(report)) = outcome else {
        panic!("install: {:?}", code(outcome));
    };
    assert!(report.awaiting.is_empty() && report.skipped_configuration.is_empty());
    assert_eq!(read(&world, "mods/a.jar"), b"mod a");
    assert_eq!(read(&world, "mods/b.jar"), b"optional mod");
    assert_eq!(read(&world, "shaderpacks/s.zip"), b"shader pack");
    assert_eq!(read(&world, "options.txt"), b"fov:90");
    // The client layer wins over the common one; server overrides never apply.
    assert_eq!(read(&world, "config/a.toml"), b"client = 1\n");
    assert_eq!(read(&world, "mods/bundled.jar"), b"bundled");
    assert!(!exists(&world, "config/server.toml"));
    assert!(!exists(&world, "mods/server-only.jar"));
    let requested = |name: &str| {
        fixture
            .server
            .requests()
            .iter()
            .any(|request| request.path.ends_with(name))
    };
    assert!(requested("gone.jar") && requested("a.jar"));
    assert!(!requested("server-only.jar"));

    let lock = installed(&world).unwrap_or_else(|| panic!("installed"));
    let Some(pack) = &lock.pack else {
        panic!("pack state");
    };
    assert_eq!(
        pack.source,
        PackIdentity::Modrinth {
            project_id: "project".into(),
            version_id: "version".into(),
            archive_sha512: format!("{:x}", Sha512::digest(&fixture.archive)),
            index_sha256: format!("{:x}", Sha256::digest(&fixture.index)),
        }
    );
    assert_eq!(
        (pack.loader.as_str(), pack.loader_version.as_str()),
        ("neoforge", "21.1.248")
    );
    let owner = |path: &str| {
        pack.files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.ownership.clone())
    };
    for content in [
        "mods/a.jar",
        "mods/b.jar",
        "shaderpacks/s.zip",
        "mods/bundled.jar",
    ] {
        assert_eq!(owner(content), Some(Ownership::Content), "{content}");
    }
    for configuration in ["config/a.toml", "options.txt"] {
        assert_eq!(
            owner(configuration),
            Some(Ownership::Override),
            "{configuration}"
        );
    }
    assert_eq!(pack.files.len(), 6);
    assert!(pack.consumed_defaults.contains("config/a.toml"));
    assert!(pack.consumed_defaults.contains("options.txt"));
    assert!(!pack.consumed_defaults.contains("mods/bundled.jar"));
    let on_disk = std::fs::read_to_string(world.root.join("installed.json")).unwrap_or_default();
    assert!(on_disk.contains("\"schema\": 5") && on_disk.contains("\"modrinth\""));
    assert!(!world.root.join("transaction.json").exists());
    assert_eq!(state(&world), Some(InstallationState::Installed));
    let game = GameDir::open(&world.root.join("minecraft")).unwrap_or_else(|_| panic!("game"));
    assert!(verify_files(&game, pack, &Cancellation::default()).is_ok());

    // A second instance reuses the cache: the archive and every file stay downloaded once.
    let before = fixture.server.requests().len();
    let again = world_copy(&world);
    let second = run_plan(&again, plan, &fixture.server, &Cancellation::default()).await;
    assert!(matches!(second, Ok(Outcome::Installed(_))));
    assert_eq!(fixture.server.requests().len(), before);
    assert_eq!(read(&again, "config/a.toml"), b"client = 1\n");
    cleanup(&world);
}

#[tokio::test]
async fn player_files_in_the_way_block_content_but_never_replace_configuration() {
    let world = world();
    let fixture = pixelmon_like();
    let catalog = mrpack_catalog(world.id, None, Some("1.21.1"));
    let plan = plan_for(&world, &catalog, &fixture.archive, &fixture.server)
        .await
        .unwrap_or_else(|error| panic!("plan: {:?}", error.code()));
    let game = world.root.join("minecraft");
    assert!(std::fs::create_dir_all(game.join("mods")).is_ok());
    assert!(std::fs::create_dir_all(game.join("config")).is_ok());
    assert!(std::fs::write(game.join("mods/a.jar"), b"the player's own mod").is_ok());
    assert!(std::fs::write(game.join("mods/bundled.jar"), b"the player's other mod").is_ok());
    assert!(std::fs::write(game.join("config/a.toml"), b"player = 1\n").is_ok());
    let before = fixture.server.requests().len();
    let outcome = run_plan(
        &world,
        plan.clone(),
        &fixture.server,
        &Cancellation::default(),
    )
    .await;
    // Both a downloaded file and a bundled content file collide: nothing is written.
    assert_eq!(
        outcome.ok(),
        Some(Outcome::Conflicts(vec![
            "mods/a.jar".into(),
            "mods/bundled.jar".into()
        ]))
    );
    assert_eq!(fixture.server.requests().len(), before);
    assert_eq!(read(&world, "mods/a.jar"), b"the player's own mod");
    assert!(!exists(&world, "mods/b.jar") && !exists(&world, "options.txt"));
    assert!(!world.root.join("transaction.json").exists());
    assert!(installed(&world).is_some_and(|lock| lock.pack.is_none()));

    // With the content paths free, only the existing configuration is kept and reported.
    assert!(std::fs::remove_file(game.join("mods/a.jar")).is_ok());
    assert!(std::fs::remove_file(game.join("mods/bundled.jar")).is_ok());
    let outcome = run_plan(&world, plan, &fixture.server, &Cancellation::default()).await;
    let Ok(Outcome::Installed(report)) = outcome else {
        panic!("install: {:?}", code(outcome));
    };
    assert_eq!(report.skipped_configuration, ["config/a.toml"]);
    assert_eq!(read(&world, "config/a.toml"), b"player = 1\n");
    assert_eq!(read(&world, "options.txt"), b"fov:90");
    let lock = installed(&world).unwrap_or_else(|| panic!("installed"));
    let skipped = lock.pack.as_ref().map(|pack| {
        pack.files
            .iter()
            .find(|file| file.path == "config/a.toml")
            .map(|file| file.baseline.is_none())
    });
    assert_eq!(skipped, Some(Some(true)));
    cleanup(&world);
}

#[tokio::test]
async fn unsupported_or_contradictory_pins_never_become_a_plan() {
    let world = world();
    let catalog = mrpack_catalog(world.id, Some("neoforge"), None);
    for (dependencies, expected) in [
        (
            json!({"minecraft": "1.21.1", "forge": "52.0.0"}),
            ErrorCode::UnsupportedInstance,
        ),
        // The catalog names NeoForge, so an index pinning another loader disagrees.
        (
            json!({"minecraft": "1.21.1", "fabric-loader": "0.16.0"}),
            ErrorCode::InvalidData,
        ),
        (
            json!({"minecraft": "1.21.1", "neoforge": "21.1.248", "extra": "1"}),
            ErrorCode::UnsupportedInstance,
        ),
        (
            json!({"minecraft": "1.21.1"}),
            ErrorCode::UnsupportedInstance,
        ),
        (
            json!({"minecraft": "1.21.1", "neoforge": "21.1.248", "forge": "52.0.0"}),
            ErrorCode::InvalidData,
        ),
    ] {
        let archive = mrpack(&index_document(Vec::new(), dependencies.clone()), &[]);
        let server = server(BTreeMap::from([(PACK_PATH.to_owned(), archive.clone())]));
        assert_eq!(
            code(plan_for(&world, &catalog, &archive, &server).await),
            Some(expected),
            "{dependencies}"
        );
    }
    // The catalog's informational Minecraft version may agree with the index, not override it.
    let archive = mrpack(&index_document(Vec::new(), neoforge()), &[]);
    let server = server(BTreeMap::from([(PACK_PATH.to_owned(), archive.clone())]));
    let mismatch = mrpack_catalog(world.id, Some("neoforge"), Some("1.20.1"));
    assert_eq!(
        code(plan_for(&world, &mismatch, &archive, &server).await),
        Some(ErrorCode::InvalidData)
    );
    let agreeing = mrpack_catalog(world.id, Some("neoforge"), Some("1.21.1"));
    assert!(plan_for(&world, &agreeing, &archive, &server).await.is_ok());
    // Fabric and Quilt packs become plans for their own loader, whatever the catalog
    // leaves unsaid; the version comes from the index.
    for (key, kind) in [
        ("fabric-loader", crate::loader::Kind::Fabric),
        ("quilt-loader", crate::loader::Kind::Quilt),
    ] {
        let archive = mrpack(
            &index_document(Vec::new(), json!({"minecraft": "1.21.1", key: "0.16.14"})),
            &[],
        );
        let served = self::server(BTreeMap::from([(PACK_PATH.to_owned(), archive.clone())]));
        for named in [None, Some(kind.name())] {
            let catalog = mrpack_catalog(world.id, named, None);
            let plan = plan_for(&world, &catalog, &archive, &served).await;
            let plan = plan.unwrap_or_else(|error| panic!("{key}: {:?}", error.code()));
            assert_eq!(plan.kind, kind);
            assert_eq!(
                (plan.loader.as_str(), plan.loader_version.as_str()),
                (kind.name(), "0.16.14")
            );
        }
        let mismatched = mrpack_catalog(world.id, Some("neoforge"), None);
        assert_eq!(
            code(plan_for(&world, &mismatched, &archive, &served).await),
            Some(ErrorCode::InvalidData)
        );
    }
    cleanup(&world);
}

#[tokio::test]
async fn a_damaged_download_or_cancellation_installs_nothing() {
    let world = world();
    let fixture = pixelmon_like();
    let catalog = mrpack_catalog(world.id, Some("neoforge"), None);
    let plan = plan_for(&world, &catalog, &fixture.archive, &fixture.server)
        .await
        .unwrap_or_else(|error| panic!("plan: {:?}", error.code()));
    // Every mirror of one file serves the wrong bytes.
    let (archive, a_body) = (fixture.archive.clone(), b"corrupt".to_vec());
    let hostile = server(BTreeMap::from([
        (PACK_PATH.to_owned(), archive),
        (format!("{BASE}a.jar"), a_body),
        (format!("{BASE}b.jar"), b"optional mod".to_vec()),
        (format!("{BASE}s.zip"), b"shader pack".to_vec()),
    ]));
    assert_eq!(
        code(run_plan(&world, plan.clone(), &hostile, &Cancellation::default()).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert!(!exists(&world, "mods/a.jar") && !exists(&world, "options.txt"));
    assert!(!world.root.join("transaction.json").exists());
    assert!(installed(&world).is_some_and(|lock| lock.pack.is_none()));

    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert_eq!(
        code(run_plan(&world, plan, &fixture.server, &cancelled).await),
        Some(ErrorCode::OperationCancelled)
    );
    assert!(!exists(&world, "mods/a.jar") && !exists(&world, "config/a.toml"));
    cleanup(&world);
}
