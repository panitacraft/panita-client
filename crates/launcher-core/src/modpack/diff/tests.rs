use super::*;
use crate::modpack::state::PackIdentity;
use uuid::Uuid;

fn hashes(n: u64) -> Hashes {
    Hashes {
        sha1: Some(format!("{n:040x}")),
        sha512: Some(format!("{n:0128x}")),
    }
}

fn managed(path: &str, ownership: Ownership, desired: u64, baseline: Option<u64>) -> ManagedFile {
    ManagedFile {
        path: path.into(),
        ownership,
        desired: hashes(desired),
        size: 10,
        baseline: baseline.map(hashes),
    }
}

fn state(files: Vec<ManagedFile>, consumed: &[&str]) -> PackState {
    PackState {
        schema: 1,
        id: Uuid::new_v4(),
        source: PackIdentity::Custom {
            version_id: Uuid::new_v4(),
            published_at: "2026-10-25T20:54:25Z".into(),
            label: None,
        },
        minecraft_version: "1.21.1".into(),
        loader: "neoforge".into(),
        loader_version: "21.1.248".into(),
        files,
        consumed_defaults: consumed.iter().map(|path| (*path).to_owned()).collect(),
        skipped_configuration: Vec::new(),
        pending: Vec::new(),
    }
}

fn want(path: &str, ownership: Ownership, desired: u64) -> Incoming {
    Incoming {
        path: path.into(),
        ownership,
        desired: hashes(desired),
        size: 10,
        adopt_equal: false,
        source: 0,
    }
}

/// What each path holds: absent unless listed.
fn run(old: &PackState, incoming: Vec<Incoming>, disk: &[(&str, Local)]) -> Diff {
    let disk: BTreeMap<String, Local> = disk
        .iter()
        .map(|(path, local)| ((*path).to_owned(), local.clone()))
        .collect();
    diff(old, incoming, &mut |path| {
        Ok(disk.get(path).cloned().unwrap_or(Local::Absent))
    })
    .unwrap_or_else(|_| panic!("diff"))
}

fn file(n: u64) -> Local {
    Local::File(hashes(n))
}

fn ops(result: &Diff) -> Vec<(&str, Op, bool)> {
    result
        .steps
        .iter()
        .map(|step| (step.path.as_str(), step.op, step.backup))
        .collect()
}

use Ownership::{Content, Defaults, Enforced, Override};

#[test]
fn unchanged_replaced_added_and_removed_content_follow_the_baseline() {
    let old = state(
        vec![
            managed("mods/same.jar", Content, 1, Some(1)),
            managed("mods/new-version.jar", Content, 2, Some(2)),
            managed("mods/dropped.jar", Content, 3, Some(3)),
        ],
        &[],
    );
    let result = run(
        &old,
        vec![
            want("mods/same.jar", Content, 1),
            want("mods/new-version.jar", Content, 20),
            want("mods/added.jar", Content, 4),
        ],
        &[
            ("mods/same.jar", file(1)),
            ("mods/new-version.jar", file(2)),
            ("mods/dropped.jar", file(3)),
        ],
    );
    assert_eq!(
        ops(&result),
        [
            ("mods/new-version.jar", Op::Replace, false),
            ("mods/added.jar", Op::Write, false),
            ("mods/dropped.jar", Op::Remove, false),
        ]
    );
    assert!(result.conflicts.is_empty() && result.released.is_empty());
    let paths: Vec<_> = result.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        ["mods/same.jar", "mods/new-version.jar", "mods/added.jar"]
    );
    // Every resulting content entry records its new bytes as the baseline.
    for entry in &result.files {
        assert_eq!(entry.baseline.as_ref(), Some(&entry.desired));
    }
}

#[test]
fn edited_content_is_a_conflict_only_when_the_pack_changes_it() {
    let old = state(
        vec![
            managed("mods/edited-changed.jar", Content, 1, Some(1)),
            managed("mods/edited-untouched.jar", Content, 2, Some(2)),
            managed("mods/edited-dropped.jar", Content, 3, Some(3)),
        ],
        &[],
    );
    let result = run(
        &old,
        vec![
            want("mods/edited-changed.jar", Content, 10),
            want("mods/edited-untouched.jar", Content, 2),
        ],
        &[
            ("mods/edited-changed.jar", file(99)),
            ("mods/edited-untouched.jar", file(98)),
            ("mods/edited-dropped.jar", file(97)),
        ],
    );
    assert_eq!(
        result.conflicts,
        [Conflict {
            path: "mods/edited-changed.jar".into(),
            reason: Reason::Edited
        }]
    );
    // Resolving moves the player's file aside, then installs the pack's.
    assert_eq!(
        ops(&result),
        [("mods/edited-changed.jar", Op::Replace, true)]
    );
    // The pack did not change this one: the player's edit stays and is reported.
    assert_eq!(result.edited, ["mods/edited-untouched.jar"]);
    let kept = result
        .files
        .iter()
        .find(|file| file.path == "mods/edited-untouched.jar")
        .map(|file| (file.desired.clone(), file.baseline.clone()));
    assert_eq!(kept, Some((hashes(2), Some(hashes(2)))));
    // A file the pack dropped that the player changed becomes theirs, never deleted.
    assert_eq!(result.released, ["mods/edited-dropped.jar"]);
    assert!(!result.unresolvable());
}

#[test]
fn a_player_file_where_the_pack_adds_one_is_a_collision() {
    let old = state(Vec::new(), &[]);
    let result = run(
        &old,
        vec![
            want("mods/new.jar", Content, 5),
            want("mods/blocked.jar", Content, 6),
        ],
        &[
            ("mods/new.jar", file(5)),
            ("mods/blocked.jar", Local::Unsafe),
        ],
    );
    // Identical bytes are still the player's file unless they came from a manual import.
    assert_eq!(
        result.conflicts,
        [
            Conflict {
                path: "mods/new.jar".into(),
                reason: Reason::Occupied
            },
            Conflict {
                path: "mods/blocked.jar".into(),
                reason: Reason::Blocked
            }
        ]
    );
    assert!(result.unresolvable());

    let mut manual = want("mods/new.jar", Content, 5);
    manual.adopt_equal = true;
    let adopted = run(&old, vec![manual], &[("mods/new.jar", file(5))]);
    assert!(adopted.conflicts.is_empty() && adopted.steps.is_empty());
    assert_eq!(adopted.files.len(), 1);
}

#[test]
fn a_missing_managed_content_file_is_restored_and_a_missing_dropped_one_forgotten() {
    let old = state(
        vec![
            managed("mods/lost.jar", Content, 1, Some(1)),
            managed("mods/gone.jar", Content, 2, Some(2)),
        ],
        &[],
    );
    let result = run(&old, vec![want("mods/lost.jar", Content, 1)], &[]);
    assert_eq!(ops(&result), [("mods/lost.jar", Op::Write, false)]);
    assert!(result.released.is_empty() && result.conflicts.is_empty());
    assert_eq!(result.files.len(), 1);
}

#[test]
fn unedited_configuration_follows_the_pack_and_edited_configuration_is_kept_whole() {
    let old = state(
        vec![
            managed("config/untouched.toml", Override, 1, Some(1)),
            managed("config/edited.toml", Override, 2, Some(2)),
            managed("config/enforced.toml", Enforced, 3, Some(3)),
        ],
        &["config/untouched.toml", "config/edited.toml"],
    );
    let result = run(
        &old,
        vec![
            want("config/untouched.toml", Override, 10),
            want("config/edited.toml", Override, 20),
            want("config/enforced.toml", Enforced, 30),
        ],
        &[
            ("config/untouched.toml", file(1)),
            ("config/edited.toml", file(77)),
            ("config/enforced.toml", file(3)),
        ],
    );
    assert_eq!(
        ops(&result),
        [
            ("config/untouched.toml", Op::Replace, false),
            ("config/enforced.toml", Op::Replace, false)
        ]
    );
    // The edited file stays untouched; both the pack's version and the last published
    // baseline are recorded, so nothing claims an exact match.
    let edited = result
        .files
        .iter()
        .find(|file| file.path == "config/edited.toml")
        .unwrap_or_else(|| panic!("edited"));
    assert_eq!(edited.desired, hashes(20));
    assert_eq!(edited.baseline, Some(hashes(2)));
    assert_eq!(
        result.skipped,
        [SkippedConfiguration {
            path: "config/edited.toml".into(),
            desired: hashes(20)
        }]
    );
    assert!(result.conflicts.is_empty());
}

#[test]
fn deleted_defaults_and_overrides_are_never_recreated_but_enforced_files_are() {
    let old = state(
        vec![
            managed("config/default.toml", Defaults, 1, Some(1)),
            managed("config/override.toml", Override, 2, Some(2)),
            managed("config/enforced.toml", Enforced, 3, Some(3)),
        ],
        &[
            "config/default.toml",
            "config/override.toml",
            "config/forgotten.toml",
        ],
    );
    let result = run(
        &old,
        vec![
            want("config/default.toml", Defaults, 10),
            want("config/override.toml", Override, 20),
            want("config/enforced.toml", Enforced, 30),
            // Offered once before and since deleted: its history remembers that.
            want("config/forgotten.toml", Defaults, 40),
        ],
        &[],
    );
    assert_eq!(ops(&result), [("config/enforced.toml", Op::Write, false)]);
    assert!(result.consumed.contains("config/default.toml"));
    assert!(result.consumed.contains("config/override.toml"));
    // Every offered path is remembered, so a later deletion is respected too.
    assert!(result.consumed.contains("config/forgotten.toml"));
    assert_eq!(result.files.len(), 1);

    // A default never offered before is applied to an absent path.
    let fresh = run(
        &state(Vec::new(), &[]),
        vec![want("config/new.toml", Defaults, 5)],
        &[],
    );
    assert_eq!(ops(&fresh), [("config/new.toml", Op::Write, false)]);
    assert!(fresh.consumed.contains("config/new.toml"));
}

#[test]
fn an_existing_player_file_at_a_new_configuration_path_is_kept_and_reported() {
    let old = state(Vec::new(), &[]);
    let result = run(
        &old,
        vec![
            want("config/theirs.toml", Defaults, 1),
            want("config/same.toml", Defaults, 2),
            want("config/folder", Override, 3),
        ],
        &[
            ("config/theirs.toml", file(50)),
            ("config/same.toml", file(2)),
            ("config/folder", Local::Unsafe),
        ],
    );
    assert!(result.steps.is_empty() && result.conflicts.is_empty());
    let skipped: Vec<_> = result.skipped.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(skipped, ["config/theirs.toml", "config/folder"]);
    // Bytes that already equal the pack's are recorded as installed without a write.
    let same = result
        .files
        .iter()
        .find(|file| file.path == "config/same.toml")
        .map(|file| file.baseline.clone());
    assert_eq!(same, Some(Some(hashes(2))));
}

#[test]
fn dropped_configuration_stops_being_managed_but_is_never_deleted() {
    let old = state(
        vec![managed("config/old.toml", Override, 1, Some(1))],
        &["config/old.toml"],
    );
    let result = run(&old, Vec::new(), &[("config/old.toml", file(1))]);
    assert!(result.steps.is_empty());
    assert_eq!(result.released, ["config/old.toml"]);
    assert!(result.files.is_empty());
    // The history survives every version.
    assert!(result.consumed.contains("config/old.toml"));
}

#[test]
fn paths_match_regardless_of_case_and_the_result_is_a_valid_pack_state() {
    let old = state(vec![managed("mods/Case.jar", Content, 1, Some(1))], &[]);
    let result = run(
        &old,
        vec![
            want("mods/case.jar", Content, 1),
            want("config/a.toml", Defaults, 2),
        ],
        &[("mods/case.jar", file(1))],
    );
    assert_eq!(ops(&result), [("config/a.toml", Op::Write, false)]);
    let mut next = state(result.files.clone(), &[]);
    next.consumed_defaults = result.consumed.clone();
    next.skipped_configuration = result.skipped.clone();
    assert!(next.validate(next.id).is_ok());
}

#[test]
fn hash_comparison_needs_one_shared_algorithm_and_no_disagreement() {
    let both = hashes(1);
    let only_sha1 = Hashes {
        sha1: both.sha1.clone(),
        sha512: None,
    };
    let only_sha512 = Hashes {
        sha1: None,
        sha512: both.sha512.clone(),
    };
    assert!(same(&both, &only_sha1) && same(&only_sha512, &both));
    assert!(!same(&only_sha1, &only_sha512));
    assert!(!same(&both, &hashes(2)));
    let mixed = Hashes {
        sha1: both.sha1.clone(),
        sha512: hashes(2).sha512,
    };
    assert!(!same(&both, &mixed));
}
