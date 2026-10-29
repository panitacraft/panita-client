//! Development validation harness for pack installation; not a product CLI.
//! It reads the public catalog without credentials and writes only inside the
//! explicitly supplied, new, isolated root. It never reads account or session files.
use std::sync::Arc;

use launcher_core::{
    accounts::{AccountInfo, AccountKind, AccountRepository},
    catalog::{CatalogInstance, CatalogSource},
    minecraft::{
        Platform,
        download::Cancellation,
        install,
        launch::{LaunchIdentity, build_command},
        process::{LaunchLease, ProcessEventKind},
    },
    modpack::{
        gamefs::GameDir,
        install::{
            Installed, Outcome, UpdateOutcome, UpdateRequest, import_pending, install_any,
            install_catalog, pending_files, repair_catalog, uninstall_pack, update_catalog,
            verify_files,
        },
    },
    operation::{OperationId, ProcessSessionId},
    paths::AppPaths,
    registry::InstanceRegistry,
    sessions::AccountController,
    settings::GlobalSettings,
    storage::PrivateStore,
};
use uuid::Uuid;

fn usage() -> Box<dyn std::error::Error> {
    "usage: pack_install install <root> <instance> | \
     install-modrinth <root> <new-instance-uuid> <project> <version> [loader] | \
     update-modrinth <root> <instance> <project> <version> [move-aside] | \
     repair-modrinth <root> <instance> | uninstall-pack <root> <instance> | \
     pending <root> <instance> | \
     import <root> <instance> <manifest-path> <file> | verify <root> <instance> | \
     launch <root> <instance> <seconds> | play-flow <root> <instance> <seconds> | play-check <root> <instance>; <root> must be a new directory named phase-10-*, phase-11-* or phase-12-*"
        .into()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err(usage());
    }
    let paths = AppPaths::new(&args[1])?;
    let name = paths
        .root()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !["phase-10-", "phase-11-", "phase-12-"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return Err(
            "expected an isolated phase-10-*, phase-11-* or phase-12-* scratch root".into(),
        );
    }
    let id = Uuid::parse_str(&args[2])?;
    let platform = Platform::x64(std::env::consts::OS, "")?;
    let cancel = Cancellation::default();
    match args[0].as_str() {
        "install" => {
            if std::fs::symlink_metadata(paths.root()).is_ok() {
                return Err("the scratch root must be new; existing roots are preserved".into());
            }
            let base = Arc::new(
                |progress: launcher_core::minecraft::download::DownloadProgress| {
                    if progress.bytes == progress.total && progress.total > 50 * 1024 * 1024 {
                        eprintln!("base artifact done ({} MiB)", progress.total / 1024 / 1024);
                    }
                },
            );
            let last = std::sync::Mutex::new(std::time::Instant::now());
            let pack = Arc::new(move |p: launcher_core::modpack::content::PackProgress| {
                let mut last = last.lock().unwrap_or_else(|poison| poison.into_inner());
                if last.elapsed().as_secs() >= 5 || p.files_done == p.files_total {
                    *last = std::time::Instant::now();
                    eprintln!(
                        "pack files {}/{} ({} of {} MiB, {} reused)",
                        p.files_done,
                        p.files_total,
                        p.bytes_done / 1024 / 1024,
                        p.bytes_total / 1024 / 1024,
                        p.reused_files
                    );
                }
            });
            let started = std::time::Instant::now();
            match install_any(&paths, id, &platform, &cancel, base, pack, None).await? {
                Installed::Pack(Outcome::Installed(report)) => println!(
                    "{}",
                    serde_json::json!({"result": "installed", "files": report.files,
                        "awaiting": report.awaiting.iter().map(|f| &f.path).collect::<Vec<_>>(),
                        "skipped_configuration": report.skipped_configuration,
                        "seconds": started.elapsed().as_secs()})
                ),
                Installed::Pack(Outcome::Conflicts(paths)) => {
                    println!(
                        "{}",
                        serde_json::json!({"result": "conflicts", "paths": paths})
                    )
                }
                Installed::Vanilla => println!("{}", serde_json::json!({"result": "vanilla"})),
            }
        }
        // Install an arbitrary public Modrinth pin without a catalog entry, for
        // validating loaders the Panita catalog does not offer yet.
        "install-modrinth" => {
            if std::fs::symlink_metadata(paths.root()).is_ok() {
                return Err("the scratch root must be new; existing roots are preserved".into());
            }
            let (project, version) = (
                args.get(3).ok_or_else(usage)?.clone(),
                args.get(4).ok_or_else(usage)?.clone(),
            );
            let catalog = CatalogInstance {
                id,
                slug: "modrinth-test".into(),
                name: "Modrinth test pack".into(),
                description: "Development validation".into(),
                minecraft_version: None,
                enabled: true,
                restricted: false,
                min_launcher_version: None,
                icon_url: None,
                banner_url: None,
                sort_order: 1,
                updated_at: "2026-10-28T00:00:00Z".into(),
                source: CatalogSource::Modrinth {
                    project_id: project,
                    pack_version_id: version,
                    loader: args.get(5).cloned(),
                },
            };
            let base = Arc::new(|_: launcher_core::minecraft::download::DownloadProgress| {});
            let pack = Arc::new(|p: launcher_core::modpack::content::PackProgress| {
                if p.files_done == p.files_total {
                    eprintln!("pack files {}/{}", p.files_done, p.files_total);
                }
            });
            let started = std::time::Instant::now();
            match install_catalog(&paths, catalog, &platform, &cancel, base, pack, None).await? {
                Outcome::Installed(report) => println!(
                    "{}",
                    serde_json::json!({"result": "installed", "files": report.files,
                        "skipped_configuration": report.skipped_configuration,
                        "seconds": started.elapsed().as_secs()})
                ),
                Outcome::Conflicts(paths) => {
                    println!(
                        "{}",
                        serde_json::json!({"result": "conflicts", "paths": paths})
                    )
                }
            }
        }
        // Update an installed Modrinth pack to another pinned version, the way the
        // launcher's update workflow does once the catalog names that version.
        "update-modrinth" | "repair-modrinth" => {
            let record = InstanceRegistry::new(paths.clone())
                .get(id)?
                .ok_or("not installed")?;
            let mut catalog = record.last_catalog.clone();
            let base = Arc::new(|_: launcher_core::minecraft::download::DownloadProgress| {});
            let pack = Arc::new(|p: launcher_core::modpack::content::PackProgress| {
                if p.files_done == p.files_total {
                    eprintln!("pack files {}/{}", p.files_done, p.files_total);
                }
            });
            let started = std::time::Instant::now();
            if args[0] == "repair-modrinth" {
                let report =
                    repair_catalog(&paths, id, &platform, &cancel, base, pack, catalog, None)
                        .await?;
                println!(
                    "{}",
                    serde_json::json!({"result": "repaired", "restored": report.restored,
                        "backed_up": report.backed_up, "seconds": started.elapsed().as_secs()})
                );
            } else {
                let (project, version) = (
                    args.get(3).ok_or_else(usage)?.clone(),
                    args.get(4).ok_or_else(usage)?.clone(),
                );
                // Read the recorded pin without auditing: the player may have edited files.
                let document: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
                    paths.instance(id)?.join("installed.json"),
                )?)?;
                let from = document["pack"]["source"]["version_id"]
                    .as_str()
                    .ok_or("not a pack installation")?
                    .to_owned();
                catalog.source = CatalogSource::Modrinth {
                    project_id: project,
                    pack_version_id: version.clone(),
                    loader: None,
                };
                let request = UpdateRequest {
                    from_version: from,
                    to_version: version,
                    backup_worlds: false,
                    move_aside: args.get(5).is_some_and(|flag| flag == "move-aside"),
                };
                match update_catalog(
                    &paths, id, &platform, request, &cancel, base, pack, catalog, None,
                )
                .await?
                {
                    UpdateOutcome::Updated(report) => println!(
                        "{}",
                        serde_json::json!({"result": "updated", "added": report.added,
                            "replaced": report.replaced, "removed": report.removed,
                            "released": report.released, "kept_configuration": report.skipped_configuration,
                            "backed_up": report.backed_up, "seconds": started.elapsed().as_secs()})
                    ),
                    UpdateOutcome::Blocked(blocked) => println!(
                        "{}",
                        serde_json::json!({"result": "blocked",
                            "conflicts": blocked.conflicts.iter().map(|c| (&c.path, format!("{:?}", c.reason))).collect::<Vec<_>>(),
                            "files": blocked.files.iter().map(|f| &f.path).collect::<Vec<_>>()})
                    ),
                }
            }
        }
        "uninstall-pack" => {
            let report = uninstall_pack(&paths, id, &cancel)?;
            println!(
                "{}",
                serde_json::json!({"result": "uninstalled", "removed": report.removed, "kept": report.kept})
            );
        }
        "pending" => {
            for file in pending_files(&paths, id)? {
                println!("{} -> {}", file.path, file.source_url());
            }
        }
        "import" => {
            let remaining = import_pending(
                &paths,
                id,
                &args[3],
                std::path::Path::new(&args[4]),
                &cancel,
            )
            .await?;
            println!("remaining: {}", remaining.len());
        }
        "verify" => {
            let installed = install::verify_installed(&paths, id)?;
            let pack = installed.pack.as_ref().ok_or("not a pack installation")?;
            let game = GameDir::open(&paths.game_directory(id)?)?;
            verify_files(&game, pack, &cancel)?;
            println!(
                "{}",
                serde_json::json!({"verified": true, "schema_version": installed.schema_version,
                    "minecraft": pack.minecraft_version, "loader": pack.loader,
                    "loader_version": pack.loader_version, "managed_files": pack.files.len(),
                    "pending": pack.pending.len(), "consumed_defaults": pack.consumed_defaults.len(),
                    "skipped_configuration": pack.skipped_configuration.len()})
            );
        }
        // Time what Play does before starting Java: the cached installation audit
        // (twice, to show receipt reuse) and the managed pack file check.
        "play-check" => {
            for round in 1..=2 {
                let lease = LaunchLease::acquire(&paths, id)?;
                let started = std::time::Instant::now();
                let installed = lease.verify_installation()?;
                let audit = started.elapsed().as_millis();
                let started = std::time::Instant::now();
                let pack = installed.pack.as_ref().ok_or("not a pack installation")?;
                verify_files(&GameDir::open(lease.game_directory())?, pack, &cancel)?;
                println!(
                    "round {round}: installation audit {audit} ms, pack files {} ms",
                    started.elapsed().as_millis()
                );
            }
        }
        // The production Play path with an offline test account: catalog check, installation
        // audit with receipts, authorization and process creation, timed stage by stage.
        "play-flow" => {
            let seconds: u64 = args.get(3).ok_or_else(usage)?.parse()?;
            let flow = std::time::Instant::now();
            let accounts = Arc::new(AccountController::new(
                AccountRepository::new(PrivateStore::open(&paths)?),
                ProcessSessionId::new(),
            )?);
            let _ = accounts.add_offline("Phase10_Check");
            let pending = launcher_core::minecraft::play::prepare_play(
                paths.clone(),
                id,
                accounts.clone(),
                platform.clone(),
                ProcessSessionId::new(),
                cancel.clone(),
                None,
            )
            .await?;
            eprintln!(
                "prepare_play (catalog + audit): {} ms",
                flow.elapsed().as_millis()
            );
            let started = std::time::Instant::now();
            let game = pending.start(None).await?;
            eprintln!(
                "start (authorize + spawn): {} ms",
                started.elapsed().as_millis()
            );
            let spawned = std::time::Instant::now();
            let (mut seen, until) = (
                0usize,
                std::time::Instant::now() + std::time::Duration::from_secs(seconds),
            );
            let mut early = false;
            while std::time::Instant::now() < until {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                let events = game.events();
                for event in events.iter().skip(seen) {
                    if let ProcessEventKind::Output { text, .. } = &event.kind {
                        if !early && text.contains("Trying GL version") {
                            early = true;
                            eprintln!(
                                "early loading window: {:.1} s after spawn, {:.1} s after the Play request",
                                spawned.elapsed().as_secs_f32(),
                                flow.elapsed().as_secs_f32()
                            );
                        }
                        if text.contains("Setting user") {
                            eprintln!(
                                "Minecraft window: {:.1} s after spawn, {:.1} s after the Play request",
                                spawned.elapsed().as_secs_f32(),
                                flow.elapsed().as_secs_f32()
                            );
                        }
                    }
                }
                seen = events.len();
            }
            let _ = game.force_stop().await;
        }
        "launch" => {
            let seconds: u64 = args.get(3).ok_or_else(usage)?.parse()?;
            let lease = LaunchLease::acquire(&paths, id)?;
            let installed = lease.verify_installation()?;
            let pack = installed.pack.as_ref().ok_or("not a pack installation")?;
            verify_files(&GameDir::open(lease.game_directory())?, pack, &cancel)?;
            let settings = InstanceRegistry::new(paths.clone()).load_settings(id)?;
            let mut global = GlobalSettings::load(&PrivateStore::open(&paths)?)?;
            // Experiment knob: extra JVM arguments separated by spaces (development only).
            if let Ok(extra) = std::env::var("PACK_JVM_ARGS") {
                global.extra_jvm_args = extra.split_whitespace().map(str::to_owned).collect();
            }
            // A synthetic offline identity; no account file is read or written.
            let account = AccountInfo {
                account_id: Uuid::new_v4(),
                kind: AccountKind::Offline,
                username: "Phase10_Check".into(),
                profile_id: Uuid::from_u128(1),
                oauth_client_id: None,
            };
            let command = build_command(
                &paths,
                lease.game_directory(),
                &installed,
                &platform,
                &global,
                &settings,
                LaunchIdentity {
                    account: &account,
                    minecraft_token: None,
                    xuid: None,
                },
            )?;
            let game = lease.spawn(command, ProcessSessionId::new(), OperationId::new())?;
            let spawned = std::time::Instant::now();
            eprintln!("Minecraft started; watching its output for {seconds} s");
            let (mut seen, until) = (
                0usize,
                std::time::Instant::now() + std::time::Duration::from_secs(seconds),
            );
            while std::time::Instant::now() < until {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let events = game.events();
                for event in events.iter().skip(seen) {
                    if let ProcessEventKind::Output { text, .. } = &event.kind {
                        for (marker, label) in [
                            ("Setting user", "window"),
                            ("NeoForge mod loading", "mod loading"),
                            ("Game took", "title screen"),
                        ] {
                            if text.contains(marker) {
                                eprintln!(
                                    "{label}: {:.1} s after spawn",
                                    spawned.elapsed().as_secs_f32()
                                );
                            }
                        }
                        println!("{text}");
                    }
                }
                seen = events.len();
            }
            eprintln!("stopping the game after the observation window");
            let _ = game.force_stop().await;
        }
        _ => return Err(usage()),
    }
    Ok(())
}
