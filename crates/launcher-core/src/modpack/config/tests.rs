use super::*;
use crate::modpack::{
    archive::{Limits, preflight, tests::build},
    gamefs::tests::game,
    state::PackIdentity,
    state::PackState,
};
use std::io::Cursor;
use uuid::Uuid;

fn opened(entries: &[(&str, &[u8])]) -> Opened<Cursor<Vec<u8>>> {
    preflight(Cursor::new(build(entries)), Limits::PACK).unwrap_or_else(|_| panic!("archive"))
}

fn state(id: Uuid, outcomes: &[&Outcome]) -> PackState {
    PackState {
        schema: 1,
        id,
        source: PackIdentity::Custom {
            version_id: Uuid::new_v4(),
            published_at: "2026-10-25T20:54:25Z".into(),
            label: None,
        },
        minecraft_version: "1.21.1".into(),
        loader: "neoforge".into(),
        loader_version: "21.1.264".into(),
        files: outcomes.iter().flat_map(|o| o.files.clone()).collect(),
        consumed_defaults: outcomes
            .iter()
            .flat_map(|o| o.consumed_defaults.clone())
            .collect(),
        skipped_configuration: outcomes.iter().flat_map(|o| o.skipped.clone()).collect(),
        pending: Vec::new(),
    }
}

fn read(root: &std::path::Path, path: &str) -> Vec<u8> {
    std::fs::read(root.join("minecraft").join(path)).unwrap_or_default()
}

#[test]
fn defaults_apply_only_to_absent_paths_and_record_exact_hashes() {
    let (root, game) = game();
    let mut archive = opened(&[("options.txt", b"fov:90"), ("config/a.toml", b"a = 1\n")]);
    let mut budget = archive.budget();
    let outcome = apply(
        &mut archive,
        Kind::Defaults,
        &game,
        &BTreeSet::new(),
        &mut budget,
        &Cancellation::default(),
    );
    let outcome = outcome.unwrap_or_else(|_| panic!("defaults"));
    assert_eq!(read(&root, "options.txt"), b"fov:90");
    assert_eq!(read(&root, "config/a.toml"), b"a = 1\n");
    assert_eq!(outcome.written, ["options.txt", "config/a.toml"]);
    assert!(outcome.skipped.is_empty());
    assert_eq!(outcome.consumed_defaults.len(), 2);
    for file in &outcome.files {
        assert_eq!(file.ownership, Ownership::Defaults);
        assert_eq!(file.baseline.as_ref(), Some(&file.desired));
    }
    let id = Uuid::new_v4();
    assert!(state(id, &[&outcome]).validate(id).is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_existing_default_is_kept_whole_reported_and_still_consumed() {
    let (root, game) = game();
    assert!(std::fs::create_dir_all(root.join("minecraft/config")).is_ok());
    assert!(std::fs::write(root.join("minecraft/config/a.toml"), b"a = 'player edit'\n").is_ok());
    let mut archive = opened(&[("config/a.toml", b"a = 1\n"), ("options.txt", b"fov:90")]);
    let mut budget = archive.budget();
    let outcome = apply(
        &mut archive,
        Kind::Defaults,
        &game,
        &BTreeSet::new(),
        &mut budget,
        &Cancellation::default(),
    )
    .unwrap_or_else(|_| panic!("defaults"));
    assert_eq!(read(&root, "config/a.toml"), b"a = 'player edit'\n");
    assert_eq!(read(&root, "options.txt"), b"fov:90");
    assert_eq!(outcome.skipped.len(), 1);
    assert_eq!(outcome.skipped[0].path, "config/a.toml");
    assert_eq!(outcome.written, ["options.txt"]);
    // The desired pack content is recorded, not the player's edited bytes.
    let skipped = outcome.files.iter().find(|f| f.path == "config/a.toml");
    assert!(
        skipped.is_some_and(|f| f.baseline.is_none() && f.desired == outcome.skipped[0].desired)
    );
    assert!(outcome.consumed_defaults.contains("config/a.toml"));
    let id = Uuid::new_v4();
    assert!(state(id, &[&outcome]).validate(id).is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_consumed_default_is_never_recreated_after_the_player_deleted_it() {
    let (root, game) = game();
    let consumed = BTreeSet::from(["Options.TXT".to_owned()]);
    let mut archive = opened(&[("options.txt", b"fov:90"), ("config/new.toml", b"n = 1\n")]);
    let mut budget = archive.budget();
    let outcome = apply(
        &mut archive,
        Kind::Defaults,
        &game,
        &consumed,
        &mut budget,
        &Cancellation::default(),
    )
    .unwrap_or_else(|_| panic!("defaults"));
    assert!(read(&root, "options.txt").is_empty());
    assert_eq!(game.entry("options.txt").ok(), Some(Entry::Absent));
    assert_eq!(read(&root, "config/new.toml"), b"n = 1\n");
    assert_eq!(outcome.written, ["config/new.toml"]);
    assert!(outcome.files.iter().all(|f| f.path != "options.txt"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn enforced_files_are_written_when_absent_and_kept_when_present() {
    let (root, game) = game();
    assert!(std::fs::create_dir_all(root.join("minecraft/config")).is_ok());
    assert!(std::fs::write(root.join("minecraft/config/server.toml"), b"edited").is_ok());
    let mut archive = opened(&[
        ("config/server.toml", b"pack"),
        ("config/other.toml", b"other"),
    ]);
    let mut budget = archive.budget();
    let outcome = apply(
        &mut archive,
        Kind::Enforced,
        &game,
        &BTreeSet::new(),
        &mut budget,
        &Cancellation::default(),
    )
    .unwrap_or_else(|_| panic!("enforced"));
    assert_eq!(read(&root, "config/server.toml"), b"edited");
    assert_eq!(read(&root, "config/other.toml"), b"other");
    assert!(outcome.consumed_defaults.is_empty());
    assert_eq!(outcome.skipped.len(), 1);
    // A change to one file never skips the other files.
    assert_eq!(outcome.written, ["config/other.toml"]);
    assert!(
        outcome
            .files
            .iter()
            .all(|f| f.ownership == Ownership::Enforced)
    );
    let id = Uuid::new_v4();
    assert!(state(id, &[&outcome]).validate(id).is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_folder_at_a_destination_is_skipped_not_replaced() {
    let (root, game) = game();
    assert!(std::fs::create_dir_all(root.join("minecraft/options.txt")).is_ok());
    let mut archive = opened(&[("options.txt", b"fov:90")]);
    let mut budget = archive.budget();
    let outcome = apply(
        &mut archive,
        Kind::Defaults,
        &game,
        &BTreeSet::new(),
        &mut budget,
        &Cancellation::default(),
    )
    .unwrap_or_else(|_| panic!("defaults"));
    assert!(root.join("minecraft/options.txt").is_dir());
    assert_eq!(outcome.skipped.len(), 1);
    assert!(outcome.written.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn archives_cannot_offer_the_same_path_twice() {
    let first = opened(&[("config/a.toml", b"1"), ("options.txt", b"2")]);
    let same = opened(&[("CONFIG/A.TOML", b"3")]);
    let other = opened(&[("config/b.toml", b"4")]);
    assert!(ensure_disjoint(&first.entries, &same.entries).is_err());
    assert!(ensure_disjoint(&first.entries, &other.entries).is_ok());
}

#[test]
fn an_exhausted_budget_or_cancellation_publishes_no_partial_file() {
    let (root, game) = game();
    let mut archive = opened(&[
        ("config/a.toml", &[b'a'; 50]),
        ("config/b.toml", &[b'b'; 50]),
    ]);
    let mut budget = 70;
    let result = apply(
        &mut archive,
        Kind::Defaults,
        &game,
        &BTreeSet::new(),
        &mut budget,
        &Cancellation::default(),
    );
    assert!(result.is_err());
    // The first file fit and was published whole; the second was never published.
    assert_eq!(read(&root, "config/a.toml"), vec![b'a'; 50]);
    assert_eq!(game.entry("config/b.toml").ok(), Some(Entry::Absent));
    let leftovers = std::fs::read_dir(root.join("minecraft/config"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
                .count()
        })
        .unwrap_or(99);
    assert_eq!(leftovers, 0);

    let cancel = Cancellation::default();
    cancel.cancel();
    let mut archive = opened(&[("c.txt", b"c")]);
    let mut budget = archive.budget();
    assert_eq!(
        apply(
            &mut archive,
            Kind::Enforced,
            &game,
            &BTreeSet::new(),
            &mut budget,
            &cancel
        )
        .err()
        .map(|e| e.code()),
        Some(crate::error::ErrorCode::OperationCancelled)
    );
    assert_eq!(game.entry("c.txt").ok(), Some(Entry::Absent));
    let _ = std::fs::remove_dir_all(&root);
}
