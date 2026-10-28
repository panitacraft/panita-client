use std::collections::BTreeMap;
use std::sync::Arc;

use super::super::tests::{
    ARCHIVE, BASE, World, archive_json, cleanup, code, curseforge_json, exists, installed,
    manifest, mod_json, read, run, server, state, world,
};
use super::*;
use crate::{
    minecraft::download::Cancellation,
    modpack::{archive::tests::build, content::ProgressSink},
    registry::InstanceRegistry,
    test_http::Server,
};

fn platform() -> Platform {
    Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform"))
}

fn quiet() -> ProgressSink {
    Arc::new(|_| {})
}

fn base_quiet() -> Progress {
    Arc::new(|_| {})
}

fn pack(world: &World) -> PackState {
    installed(world)
        .and_then(|lock| lock.pack)
        .unwrap_or_else(|| panic!("pack state"))
}

fn record_source(world: &World) -> Option<CatalogSource> {
    InstanceRegistry::new(world.paths.clone())
        .get(world.id)
        .unwrap_or_default()
        .map(|record| record.resolved_source)
}

fn request(world: &World, next: &Manifest, move_aside: bool) -> UpdateRequest {
    UpdateRequest {
        from_version: pin_of(&pack(world)),
        to_version: next.version_id.to_string(),
        backup_worlds: false,
        move_aside,
    }
}

/// The catalog answer an update starts from: the entry now pins `next`.
fn remember(world: &World, next: &Manifest) {
    let registry = InstanceRegistry::new(world.paths.clone());
    let mut catalog = registry
        .get(world.id)
        .unwrap_or_default()
        .map(|record| record.last_catalog)
        .unwrap_or_else(|| panic!("record"));
    catalog.minecraft_version = Some(next.minecraft_version.clone());
    catalog.source = CatalogSource::Custom {
        minecraft_version: next.minecraft_version.clone(),
        loader: next.loader.clone(),
        current_version_id: Some(next.version_id),
        label: next.label.clone(),
        published_at: Some(next.published_at.clone()),
    };
    assert!(registry.remember_catalog(&[catalog]).is_ok());
}

async fn update_with(
    world: &World,
    plan: Plan,
    request: UpdateRequest,
    server: &Server,
) -> Result<UpdateOutcome, CoreError> {
    let downloader = PackDownloader::for_tests(server.base());
    run_update(
        &world.paths,
        world.id,
        &platform(),
        request,
        plan,
        &downloader,
        BaseSource::Recorded,
        &Cancellation::default(),
        base_quiet(),
        quiet(),
    )
    .await
}

async fn update_to(
    world: &World,
    next: &Manifest,
    server: &Server,
    move_aside: bool,
) -> Result<UpdateOutcome, CoreError> {
    let plan = Plan::custom(next).unwrap_or_else(|_| panic!("plan"));
    remember(world, next);
    update_with(world, plan, request(world, next, move_aside), server).await
}

fn updated(outcome: Result<UpdateOutcome, CoreError>) -> UpdateReport {
    match outcome {
        Ok(UpdateOutcome::Updated(report)) => report,
        Ok(UpdateOutcome::Blocked(blocked)) => {
            panic!(
                "blocked: {:?} {:?}",
                blocked.conflicts,
                blocked.files.iter().map(|f| &f.path).collect::<Vec<_>>()
            )
        }
        Err(error) => panic!("update: {:?}", error.code()),
    }
}

fn blocked(outcome: Result<UpdateOutcome, CoreError>) -> Blocked {
    match outcome {
        Ok(UpdateOutcome::Blocked(blocked)) => blocked,
        Ok(UpdateOutcome::Updated(_)) => panic!("expected the update to be blocked"),
        Err(error) => panic!("update: {:?}", error.code()),
    }
}

fn routes(entries: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    entries
        .iter()
        .map(|(name, body)| {
            let path = if name.starts_with('/') {
                (*name).to_owned()
            } else {
                format!("{BASE}{name}")
            };
            (path, body.to_vec())
        })
        .collect()
}

fn game(world: &World, path: &str) -> std::path::PathBuf {
    world.root.join("minecraft").join(path)
}

fn put(world: &World, path: &str, body: &[u8]) {
    let target = game(world, path);
    if let Some(parent) = target.parent() {
        assert!(std::fs::create_dir_all(parent).is_ok());
    }
    assert!(std::fs::write(target, body).is_ok());
}

fn count(directory: &std::path::Path) -> usize {
    std::fs::read_dir(directory).map_or(0, |entries| entries.count())
}

fn no_transaction(world: &World) {
    assert!(!world.root.join("transaction.json").exists());
    assert_eq!(count(&world.root.join(TRANSIENT)), 0);
}

struct Pair {
    first: Manifest,
    second: Manifest,
    server: Server,
}

const A1: &[u8] = b"mod a, first version";
const A2: &[u8] = b"mod a, second version";
const B: &[u8] = b"mod b, never changes";
const C: &[u8] = b"mod c, dropped later";
const D: &[u8] = b"mod d, added later";
const OPTIONS: &[u8] = b"fov:90";
const CONFIG: &[u8] = b"a = 1\n";

/// Version one installed for real; version two ready to be served.
async fn installed_pair(world: &World) -> Pair {
    let defaults = build(&[("options.txt", OPTIONS), ("config/a.toml", CONFIG)]);
    let first = manifest(
        world.id,
        vec![
            mod_json("mods/a.jar", A1),
            mod_json("mods/b.jar", B),
            mod_json("mods/c.jar", C),
        ],
        Some(archive_json(&defaults)),
    );
    let second = manifest(
        world.id,
        vec![
            mod_json("mods/a.jar", A2),
            mod_json("mods/b.jar", B),
            mod_json("mods/d.jar", D),
        ],
        Some(archive_json(&defaults)),
    );
    let initial = server(routes(&[
        ("a.jar", A1),
        ("b.jar", B),
        ("c.jar", C),
        (ARCHIVE, &defaults),
    ]));
    let outcome = run(world, &first, &initial, &Cancellation::default()).await;
    assert!(matches!(
        outcome,
        Ok(crate::modpack::install::Outcome::Installed(_))
    ));
    let next = server(routes(&[("a.jar", A2), ("d.jar", D), (ARCHIVE, &defaults)]));
    Pair {
        first,
        second,
        server: next,
    }
}

#[tokio::test]
async fn an_update_replaces_adds_and_removes_unedited_content_and_publishes_the_new_pin() {
    let world = world();
    let pair = installed_pair(&world).await;
    // Player data and another instance must come through untouched.
    put(&world, "saves/world/level.dat", b"a world");
    put(&world, "mods/mine.jar", b"the player's own mod");
    let other = world
        .paths
        .root()
        .join("instances/other/minecraft/saves/level.dat");
    assert!(std::fs::create_dir_all(other.parent().unwrap_or(&world.root)).is_ok());
    assert!(std::fs::write(&other, b"another instance").is_ok());

    let report = updated(update_to(&world, &pair.second, &pair.server, false).await);
    assert_eq!((report.added, report.replaced, report.removed), (1, 1, 1));
    assert_eq!(read(&world, "mods/a.jar"), A2);
    assert_eq!(read(&world, "mods/b.jar"), B);
    assert_eq!(read(&world, "mods/d.jar"), D);
    assert!(!exists(&world, "mods/c.jar"));
    assert_eq!(read(&world, "mods/mine.jar"), b"the player's own mod");
    assert_eq!(read(&world, "saves/world/level.dat"), b"a world");
    assert_eq!(
        std::fs::read(&other).unwrap_or_default(),
        b"another instance"
    );
    assert_eq!(read(&world, "options.txt"), OPTIONS);
    // Only what changed was downloaded: not b, c or the cached archive.
    let requested: Vec<_> = pair.server.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(requested.len(), 2, "{requested:?}");

    let lock = pack(&world);
    assert_eq!(pin_of(&lock), pair.second.version_id.to_string());
    let content: Vec<_> = lock
        .files
        .iter()
        .filter(|file| file.ownership == Ownership::Content)
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(content, ["mods/a.jar", "mods/b.jar", "mods/d.jar"]);
    assert!(matches!(
        record_source(&world),
        Some(CatalogSource::Custom { current_version_id: Some(version), .. })
            if version == pair.second.version_id
    ));
    assert_eq!(state(&world), Some(InstallationState::Installed));
    no_transaction(&world);
    cleanup(&world);
}

#[tokio::test]
async fn edited_content_blocks_the_update_until_the_player_agrees_and_is_then_set_aside() {
    let world = world();
    let pair = installed_pair(&world).await;
    put(&world, "mods/a.jar", b"the player's edited mod");
    let before = installed(&world).map(|lock| lock.operation);

    let stopped = blocked(update_to(&world, &pair.second, &pair.server, false).await);
    assert_eq!(
        stopped.conflicts,
        [Conflict {
            path: "mods/a.jar".into(),
            reason: diff::Reason::Edited
        }]
    );
    // Nothing happened: files, pointer and journal are as they were.
    assert_eq!(read(&world, "mods/a.jar"), b"the player's edited mod");
    assert_eq!(pin_of(&pack(&world)), pair.first.version_id.to_string());
    assert_eq!(installed(&world).map(|lock| lock.operation), before);
    assert!(!exists(&world, "mods/d.jar"));
    no_transaction(&world);

    let report = updated(update_to(&world, &pair.second, &pair.server, true).await);
    assert_eq!(report.backed_up, ["mods/a.jar"]);
    assert_eq!(read(&world, "mods/a.jar"), A2);
    // The player's version is kept, never deleted, in the permanent backups.
    let kept = std::fs::read_dir(world.root.join(BACKUPS))
        .ok()
        .and_then(|mut entries| entries.next())
        .and_then(Result::ok)
        .map(|entry| std::fs::read(entry.path().join("mods/a.jar")).unwrap_or_default());
    assert_eq!(kept.as_deref(), Some(&b"the player's edited mod"[..]));
    cleanup(&world);
}

#[tokio::test]
async fn a_player_file_where_the_pack_adds_one_is_a_collision_that_can_be_set_aside() {
    let world = world();
    let pair = installed_pair(&world).await;
    put(&world, "mods/d.jar", b"the player already had a d");
    let stopped = blocked(update_to(&world, &pair.second, &pair.server, false).await);
    assert_eq!(
        stopped.conflicts,
        [Conflict {
            path: "mods/d.jar".into(),
            reason: diff::Reason::Occupied
        }]
    );
    assert_eq!(read(&world, "mods/d.jar"), b"the player already had a d");
    // A folder in the way cannot be moved aside, even when the player agrees.
    assert!(std::fs::remove_file(game(&world, "mods/d.jar")).is_ok());
    assert!(std::fs::create_dir_all(game(&world, "mods/d.jar")).is_ok());
    let hard = blocked(update_to(&world, &pair.second, &pair.server, true).await);
    assert_eq!(hard.conflicts[0].reason, diff::Reason::Blocked);
    assert!(game(&world, "mods/d.jar").is_dir());
    assert_eq!(pin_of(&pack(&world)), pair.first.version_id.to_string());
    cleanup(&world);
}

#[tokio::test]
async fn a_dropped_file_the_player_edited_is_kept_and_an_unedited_one_is_removed() {
    let world = world();
    let pair = installed_pair(&world).await;
    put(&world, "mods/c.jar", b"the player changed c");
    let report = updated(update_to(&world, &pair.second, &pair.server, false).await);
    assert_eq!(report.released, ["mods/c.jar"]);
    assert_eq!(read(&world, "mods/c.jar"), b"the player changed c");
    assert!(
        !pack(&world)
            .files
            .iter()
            .any(|file| file.path == "mods/c.jar")
    );
    cleanup(&world);
}

#[tokio::test]
async fn defaults_are_seeded_once_and_deleted_ones_are_never_recreated() {
    let world = world();
    let defaults = build(&[("options.txt", OPTIONS), ("config/a.toml", CONFIG)]);
    let first = manifest(
        world.id,
        vec![mod_json("mods/a.jar", A1)],
        Some(archive_json(&defaults)),
    );
    let initial = server(routes(&[("a.jar", A1), (ARCHIVE, &defaults)]));
    assert!(
        run(&world, &first, &initial, &Cancellation::default())
            .await
            .is_ok()
    );
    // The player deletes one default and edits the other.
    assert!(std::fs::remove_file(game(&world, "options.txt")).is_ok());
    put(&world, "config/a.toml", b"a = 99\n");

    // The next version offers new values for both and one brand new default.
    let newer = build(&[
        ("options.txt", b"fov:110"),
        ("config/a.toml", b"a = 2\n"),
        ("config/new.toml", b"new = 1\n"),
    ]);
    let archive_path = "/storage/v1/object/public/modpacks/by-hash/sha256-abc/defaults.zip";
    let second = manifest(
        world.id,
        vec![mod_json("mods/a.jar", A1)],
        Some(archive_json(&newer)),
    );
    let next = server(routes(&[(archive_path, &newer)]));
    let report = updated(update_to(&world, &second, &next, false).await);
    assert_eq!(read(&world, "config/a.toml"), b"a = 99\n");
    assert!(!exists(&world, "options.txt"));
    assert_eq!(read(&world, "config/new.toml"), b"new = 1\n");
    assert_eq!(report.added, 1);
    let lock = pack(&world);
    assert!(lock.consumed_defaults.contains("options.txt"));
    assert!(lock.consumed_defaults.contains("config/new.toml"));
    cleanup(&world);
}

#[tokio::test]
async fn an_update_that_needs_a_manual_file_changes_nothing_until_it_is_supplied() {
    let world = world();
    let pair = installed_pair(&world).await;
    let manual: &[u8] = b"a manual download";
    let defaults = build(&[("options.txt", OPTIONS), ("config/a.toml", CONFIG)]);
    let third = manifest(
        world.id,
        vec![
            mod_json("mods/a.jar", A2),
            curseforge_json("mods/manual.jar", manual, true),
        ],
        Some(archive_json(&defaults)),
    );
    let stopped = blocked(update_to(&world, &third, &pair.server, false).await);
    assert_eq!(
        stopped
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["mods/manual.jar"]
    );
    // Nothing was written or published: the previous version is intact.
    assert_eq!(read(&world, "mods/a.jar"), A1);
    assert!(exists(&world, "mods/c.jar") && !exists(&world, "mods/manual.jar"));
    assert_eq!(pin_of(&pack(&world)), pair.first.version_id.to_string());
    no_transaction(&world);

    // The player places the file by hand; the next attempt adopts it as pack content.
    put(&world, "mods/manual.jar", manual);
    let report = updated(update_to(&world, &third, &pair.server, false).await);
    assert!(report.removed >= 1);
    assert_eq!(read(&world, "mods/manual.jar"), manual);
    assert!(
        pack(&world)
            .files
            .iter()
            .any(|file| file.path == "mods/manual.jar" && file.ownership == Ownership::Content)
    );
    cleanup(&world);
}

#[tokio::test]
async fn a_withdrawn_download_blocks_the_update_before_any_change() {
    let world = world();
    let pair = installed_pair(&world).await;
    let defaults = build(&[("options.txt", OPTIONS), ("config/a.toml", CONFIG)]);
    let gone: &[u8] = b"withdrawn by its author";
    let third = manifest(
        world.id,
        vec![
            mod_json("mods/a.jar", A2),
            curseforge_json("mods/gone.jar", gone, false),
        ],
        Some(archive_json(&defaults)),
    );
    // The CDN answers 404 for the withdrawn file; the others are served.
    let stopped = blocked(update_to(&world, &third, &pair.server, false).await);
    assert_eq!(stopped.files.len(), 1);
    assert_eq!(read(&world, "mods/a.jar"), A1);
    assert!(!exists(&world, "mods/gone.jar"));
    assert_eq!(pin_of(&pack(&world)), pair.first.version_id.to_string());
    no_transaction(&world);
    cleanup(&world);
}

#[tokio::test]
async fn the_request_must_match_what_is_installed_and_what_the_plan_is() {
    let world = world();
    let pair = installed_pair(&world).await;
    let plan = || Plan::custom(&pair.second).unwrap_or_else(|_| panic!("plan"));
    let mut wrong_from = request(&world, &pair.second, false);
    wrong_from.from_version = "somewhere-else".into();
    assert_eq!(
        code(update_with(&world, plan(), wrong_from, &pair.server).await),
        Some(ErrorCode::UpdateSelectionChanged)
    );
    let mut wrong_to = request(&world, &pair.second, false);
    wrong_to.to_version = pair.first.version_id.to_string();
    assert_eq!(
        code(update_with(&world, plan(), wrong_to, &pair.server).await),
        Some(ErrorCode::UpdateSelectionChanged)
    );
    // Another loader kind needs a reinstall, never an in-place update.
    let mut other_loader = plan();
    other_loader.kind = crate::loader::Kind::Fabric;
    other_loader.loader = "fabric".into();
    assert_eq!(
        code(
            update_with(
                &world,
                other_loader,
                request(&world, &pair.second, false),
                &pair.server
            )
            .await
        ),
        Some(ErrorCode::UnsupportedInstance)
    );
    assert_eq!(read(&world, "mods/a.jar"), A1);
    no_transaction(&world);
    cleanup(&world);
}

#[tokio::test]
async fn a_busy_instance_refuses_every_maintenance_operation() {
    let world = world();
    let pair = installed_pair(&world).await;
    let _held = try_lock(&world.root, ".installation.lock")
        .ok()
        .flatten()
        .unwrap_or_else(|| panic!("lock"));
    assert_eq!(
        code(update_to(&world, &pair.second, &pair.server, false).await),
        Some(ErrorCode::OperationBusy)
    );
    assert_eq!(
        code(uninstall(&world.paths, world.id, &Cancellation::default())),
        Some(ErrorCode::OperationBusy)
    );
    assert_eq!(read(&world, "mods/a.jar"), A1);
    cleanup(&world);
}

fn interrupt_at(stage: Stage, crash: bool) {
    INTERRUPT.with(|cell| cell.set(Some(Interrupt { stage, crash })));
}

fn clear_interrupt() {
    INTERRUPT.with(|cell| cell.set(None));
}

#[tokio::test]
async fn a_failure_before_the_pointer_rolls_every_file_back() {
    let world = world();
    let pair = installed_pair(&world).await;
    put(&world, "mods/mine.jar", b"the player's own mod");
    let before = installed(&world).map(|lock| lock.operation);
    interrupt_at(Stage::AfterApply, false);
    let failed = update_to(&world, &pair.second, &pair.server, false).await;
    clear_interrupt();
    assert_eq!(code(failed), Some(ErrorCode::StorageUnavailable));
    assert_eq!(read(&world, "mods/a.jar"), A1);
    assert_eq!(read(&world, "mods/c.jar"), C);
    assert!(!exists(&world, "mods/d.jar"));
    assert_eq!(read(&world, "mods/mine.jar"), b"the player's own mod");
    assert_eq!(pin_of(&pack(&world)), pair.first.version_id.to_string());
    assert_eq!(installed(&world).map(|lock| lock.operation), before);
    assert_eq!(state(&world), Some(InstallationState::Installed));
    no_transaction(&world);
    cleanup(&world);
}

#[tokio::test]
async fn a_crash_after_the_files_changed_is_undone_by_recovery_even_with_a_set_aside_player_file() {
    let world = world();
    let pair = installed_pair(&world).await;
    put(&world, "mods/a.jar", b"the player's edited mod");
    interrupt_at(Stage::AfterApply, true);
    let crashed = update_to(&world, &pair.second, &pair.server, true).await;
    clear_interrupt();
    assert!(crashed.is_err());
    // The process died with the new files in place and the journal still active.
    assert_eq!(read(&world, "mods/a.jar"), A2);
    assert!(world.root.join("transaction.json").exists());
    assert_eq!(pin_of(&pack(&world)), pair.first.version_id.to_string());

    assert!(crate::minecraft::install::recover_locked(&world.paths, world.id, &world.root).is_ok());
    assert_eq!(read(&world, "mods/a.jar"), b"the player's edited mod");
    assert_eq!(read(&world, "mods/c.jar"), C);
    assert!(!exists(&world, "mods/d.jar"));
    no_transaction(&world);
    assert_eq!(count(&world.root.join(BACKUPS)), 0);
    // Recovery is repeatable once nothing is pending.
    assert!(crate::minecraft::install::recover_locked(&world.paths, world.id, &world.root).is_ok());
    cleanup(&world);
}

#[tokio::test]
async fn a_crash_after_the_pointer_is_finished_forward() {
    let world = world();
    let pair = installed_pair(&world).await;
    interrupt_at(Stage::AfterPointer, true);
    let crashed = update_to(&world, &pair.second, &pair.server, false).await;
    clear_interrupt();
    assert!(crashed.is_err());
    assert_eq!(pin_of(&pack(&world)), pair.second.version_id.to_string());
    assert!(world.root.join("transaction.json").exists());
    assert!(matches!(
        record_source(&world),
        Some(CatalogSource::Vanilla { .. })
    ));

    assert!(crate::minecraft::install::recover_locked(&world.paths, world.id, &world.root).is_ok());
    assert!(matches!(
        record_source(&world),
        Some(CatalogSource::Custom { current_version_id: Some(version), .. })
            if version == pair.second.version_id
    ));
    assert_eq!(read(&world, "mods/a.jar"), A2);
    assert_eq!(state(&world), Some(InstallationState::Installed));
    no_transaction(&world);

    // The same interruption without a crash still reports the update as done.
    let world = self::world();
    let pair = installed_pair(&world).await;
    interrupt_at(Stage::AfterPointer, false);
    let outcome = update_to(&world, &pair.second, &pair.server, false).await;
    clear_interrupt();
    assert!(matches!(outcome, Ok(UpdateOutcome::Updated(_))));
    no_transaction(&world);
    cleanup(&world);
}

#[tokio::test]
async fn cancellation_before_any_change_leaves_no_trace() {
    let world = world();
    let pair = installed_pair(&world).await;
    let plan = Plan::custom(&pair.second).unwrap_or_else(|_| panic!("plan"));
    let downloader = PackDownloader::for_tests(pair.server.base());
    let cancel = Cancellation::default();
    cancel.cancel();
    let outcome = run_update(
        &world.paths,
        world.id,
        &platform(),
        request(&world, &pair.second, false),
        plan,
        &downloader,
        BaseSource::Recorded,
        &cancel,
        base_quiet(),
        quiet(),
    )
    .await;
    assert_eq!(code(outcome), Some(ErrorCode::OperationCancelled));
    assert_eq!(read(&world, "mods/a.jar"), A1);
    no_transaction(&world);
    cleanup(&world);
}

async fn repair_with(
    world: &World,
    manifest: &Manifest,
    server: &Server,
) -> Result<RepairReport, CoreError> {
    let downloader = PackDownloader::for_tests(server.base());
    run_repair(
        &world.paths,
        world.id,
        &platform(),
        Plan::custom(manifest)?,
        &downloader,
        BaseSource::Recorded,
        &Cancellation::default(),
        base_quiet(),
        quiet(),
    )
    .await
}

#[tokio::test]
async fn repair_restores_content_and_sets_changed_files_aside_but_never_touches_configuration() {
    let world = world();
    let defaults = build(&[("options.txt", OPTIONS), ("config/a.toml", CONFIG)]);
    let first = manifest(
        world.id,
        vec![mod_json("mods/a.jar", A1), mod_json("mods/b.jar", B)],
        Some(archive_json(&defaults)),
    );
    let served = server(routes(&[("a.jar", A1), ("b.jar", B), (ARCHIVE, &defaults)]));
    assert!(
        run(&world, &first, &served, &Cancellation::default())
            .await
            .is_ok()
    );
    let before = pack(&world);
    let operation = installed(&world).map(|lock| lock.operation);
    // A missing mod, a changed mod, and a player-edited configuration file.
    assert!(std::fs::remove_file(game(&world, "mods/a.jar")).is_ok());
    put(&world, "mods/b.jar", b"b, damaged or swapped by the player");
    put(&world, "options.txt", b"fov:30");

    let report = repair_with(&world, &first, &served)
        .await
        .unwrap_or_else(|error| panic!("repair: {:?}", error.code()));
    assert_eq!(report.restored, 2);
    assert_eq!(report.backed_up, ["mods/b.jar"]);
    assert_eq!(read(&world, "mods/a.jar"), A1);
    assert_eq!(read(&world, "mods/b.jar"), B);
    // Configuration is the player's; repair never resets it.
    assert_eq!(read(&world, "options.txt"), b"fov:30");
    // The pin, history and ownership records are exactly what they were.
    let after = pack(&world);
    assert_eq!(after.source, before.source);
    assert_eq!(after.files, before.files);
    assert_eq!(after.consumed_defaults, before.consumed_defaults);
    assert_ne!(installed(&world).map(|lock| lock.operation), operation);
    let kept = std::fs::read_dir(world.root.join(BACKUPS))
        .ok()
        .and_then(|mut entries| entries.next())
        .and_then(Result::ok)
        .map(|entry| std::fs::read(entry.path().join("mods/b.jar")).unwrap_or_default());
    assert_eq!(
        kept.as_deref(),
        Some(&b"b, damaged or swapped by the player"[..])
    );
    no_transaction(&world);
    cleanup(&world);
}

#[tokio::test]
async fn repair_only_accepts_the_plan_of_the_installed_pin() {
    let world = world();
    let pair = installed_pair(&world).await;
    assert_eq!(
        code(repair_with(&world, &pair.second, &pair.server).await),
        Some(ErrorCode::UpdateSelectionChanged)
    );
    // A file supplied by hand cannot be refetched: its loss needs the player.
    cleanup(&world);
}

#[tokio::test]
async fn a_failed_repair_rolls_back_and_a_crash_is_recovered() {
    let world = world();
    let pair = installed_pair(&world).await;
    let served = server(routes(&[("a.jar", A1), ("b.jar", B), ("c.jar", C)]));
    assert!(std::fs::remove_file(game(&world, "mods/a.jar")).is_ok());
    put(&world, "mods/b.jar", b"changed b");
    interrupt_at(Stage::AfterApply, true);
    let crashed = repair_with(&world, &pair.first, &served).await;
    clear_interrupt();
    assert!(crashed.is_err());
    assert_eq!(read(&world, "mods/a.jar"), A1);
    assert!(crate::minecraft::install::recover_locked(&world.paths, world.id, &world.root).is_ok());
    // Back to exactly the damaged state the repair started from.
    assert!(!exists(&world, "mods/a.jar"));
    assert_eq!(read(&world, "mods/b.jar"), b"changed b");
    no_transaction(&world);
    assert_eq!(count(&world.root.join(BACKUPS)), 0);
    cleanup(&world);
}

#[tokio::test]
async fn uninstall_removes_only_unedited_pack_content_and_keeps_everything_else() {
    let world = world();
    let pair = installed_pair(&world).await;
    put(&world, "mods/b.jar", b"b, edited by the player");
    put(&world, "mods/mine.jar", b"the player's own mod");
    put(&world, "saves/world/level.dat", b"a world");
    put(&world, "options.txt", b"fov:30");
    let report = uninstall(&world.paths, world.id, &Cancellation::default())
        .unwrap_or_else(|error| panic!("uninstall: {:?}", error.code()));
    assert_eq!((report.removed, report.kept), (2, 1));
    assert!(!exists(&world, "mods/a.jar") && !exists(&world, "mods/c.jar"));
    assert_eq!(read(&world, "mods/b.jar"), b"b, edited by the player");
    assert_eq!(read(&world, "mods/mine.jar"), b"the player's own mod");
    assert_eq!(read(&world, "saves/world/level.dat"), b"a world");
    assert_eq!(read(&world, "options.txt"), b"fov:30");
    assert_eq!(state(&world), Some(InstallationState::NotInstalled));
    // The record keeps its pin, and the tombstone keeps only configuration history.
    assert!(matches!(
        record_source(&world),
        Some(CatalogSource::Vanilla { .. })
    ));
    let tombstone = installed(&world).unwrap_or_else(|| panic!("tombstone"));
    assert_eq!(tombstone.schema_version, 3);
    let kept = tombstone.pack.unwrap_or_else(|| panic!("pack"));
    assert!(
        kept.files
            .iter()
            .all(|file| file.ownership != Ownership::Content)
    );
    assert!(kept.consumed_defaults.contains("options.txt"));
    no_transaction(&world);
    let _ = pair;
    cleanup(&world);
}

#[tokio::test]
async fn a_reinstall_keeps_the_history_so_a_deleted_default_stays_deleted() {
    let world = world();
    let pair = installed_pair(&world).await;
    assert!(std::fs::remove_file(game(&world, "options.txt")).is_ok());
    assert!(uninstall(&world.paths, world.id, &Cancellation::default()).is_ok());
    assert!(!exists(&world, "mods/a.jar"));
    let again = run(
        &world,
        &pair.first,
        &pair.server_for_first(),
        &Cancellation::default(),
    )
    .await;
    assert!(
        matches!(again, Ok(crate::modpack::install::Outcome::Installed(_))),
        "{:?}",
        code(again)
    );
    assert!(!exists(&world, "options.txt"));
    assert_eq!(read(&world, "config/a.toml"), CONFIG);
    assert_eq!(read(&world, "mods/a.jar"), A1);
    cleanup(&world);
}

impl Pair {
    fn server_for_first(&self) -> Server {
        let defaults = build(&[("options.txt", OPTIONS), ("config/a.toml", CONFIG)]);
        server(routes(&[
            ("a.jar", A1),
            ("b.jar", B),
            ("c.jar", C),
            (ARCHIVE, &defaults),
        ]))
    }
}

#[tokio::test]
async fn an_interrupted_uninstall_finishes_forward_and_can_be_retried() {
    let world = world();
    let _pair = installed_pair(&world).await;
    // Leave the journal as a crash right after it was written would.
    let previous = installed(&world).unwrap_or_else(|| panic!("installed"));
    let journal = Journal {
        schema_version: 5,
        kind: Operation::Uninstall,
        instance_id: world.id,
        operation: Uuid::new_v4(),
        previous_operation: previous.operation,
        active: true,
        backup: None,
        ledger: Vec::new(),
        prepared: None,
    };
    assert!(write_json(&world.root, "transaction.json", &journal).is_ok());
    // A pack uninstall journal is read by the generic recovery, which completes it.
    assert!(crate::minecraft::install::recover_locked(&world.paths, world.id, &world.root).is_ok());
    assert!(!exists(&world, "mods/a.jar"));
    assert_eq!(state(&world), Some(InstallationState::NotInstalled));
    no_transaction(&world);
    cleanup(&world);
}

#[test]
fn journals_are_validated_and_inactive_ones_are_recognized() {
    let id = Uuid::new_v4();
    let journal = |change: &dyn Fn(&mut Journal)| {
        let mut journal = Journal {
            schema_version: 5,
            kind: Operation::Update,
            instance_id: id,
            operation: Uuid::new_v4(),
            previous_operation: Uuid::new_v4(),
            active: true,
            backup: None,
            ledger: vec![Entry {
                path: "mods/a.jar".into(),
                action: Action::Write,
                backup: false,
                size: Some(3),
                sha1: Some("a".repeat(40)),
                sha512: None,
            }],
            prepared: None,
        };
        change(&mut journal);
        journal
    };
    assert!(journal(&|_| {}).validate(id).is_ok());
    for (name, bad) in [
        ("schema", journal(&|j| j.schema_version = 4)),
        ("instance", journal(&|j| j.instance_id = Uuid::new_v4())),
        ("operation", journal(&|j| j.operation = Uuid::nil())),
        ("previous", journal(&|j| j.previous_operation = Uuid::nil())),
        ("path", journal(&|j| j.ledger[0].path = "../x".into())),
        (
            "saves",
            journal(&|j| j.ledger[0].path = "saves/world".into()),
        ),
        ("missing size", journal(&|j| j.ledger[0].size = None)),
        (
            "missing hash",
            journal(&|j| {
                j.ledger[0].sha1 = None;
            }),
        ),
        (
            "removal with hash",
            journal(&|j| {
                j.ledger[0].action = Action::Remove;
                j.ledger[0].size = None;
            }),
        ),
    ] {
        assert!(bad.validate(id).is_err(), "{name}");
    }
    let mut finished = journal(&|j| j.active = false);
    finished.ledger.clear();
    let value = serde_json::to_value(&finished).unwrap_or_default();
    assert!(journal_inactive(value, id));
    let active = serde_json::to_value(journal(&|_| {})).unwrap_or_default();
    assert!(!journal_inactive(active, id));
}
