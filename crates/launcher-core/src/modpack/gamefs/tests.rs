use super::*;
use sha1::Sha1;
use sha2::Sha512;

pub(crate) fn game() -> (std::path::PathBuf, GameDir) {
    let root = std::env::temp_dir().join(format!("panita-gamefs-{}", Uuid::new_v4()));
    let path = root.join("minecraft");
    assert!(private_fs::PrivateDir::ensure(&root).is_ok());
    assert!(private_fs::PrivateDir::ensure(&path).is_ok());
    let game = GameDir::open(&path).unwrap_or_else(|_| panic!("game directory"));
    (root, game)
}

pub(crate) fn expected(body: &[u8]) -> Expected {
    Expected {
        size: body.len() as u64,
        sha1: Some(format!("{:x}", Sha1::digest(body))),
        sha256: None,
        sha512: Some(format!("{:x}", Sha512::digest(body))),
    }
}

fn code<T>(result: Result<T, CoreError>) -> Option<ErrorCode> {
    result.err().map(|error| error.code())
}

#[test]
fn new_files_are_created_absent_only_with_their_parents() {
    let (root, game) = game();
    let cancel = Cancellation::default();
    let body = b"first".to_vec();
    assert_eq!(game.entry("mods/a.jar").ok(), Some(Entry::Absent));
    assert!(
        game.write_new("mods/a.jar", |w| w.write_all(&body).map_err(io))
            .is_ok()
    );
    assert_eq!(game.entry("mods/a.jar").ok(), Some(Entry::File));
    assert_eq!(
        std::fs::read(root.join("minecraft/mods/a.jar")).unwrap_or_default(),
        body
    );
    assert_eq!(
        game.compare("mods/a.jar", &expected(&body), &cancel).ok(),
        Some(Content::Matches)
    );
    assert_eq!(
        game.compare("mods/a.jar", &expected(b"other"), &cancel)
            .ok(),
        Some(Content::Differs)
    );
    assert_eq!(
        game.compare("mods/none.jar", &expected(b"x"), &cancel).ok(),
        Some(Content::Absent)
    );
    // An existing file, even identical, is never replaced.
    assert_eq!(
        code(game.write_new("mods/a.jar", |w| w.write_all(b"second").map_err(io))),
        Some(ErrorCode::PackConflict)
    );
    assert_eq!(
        std::fs::read(root.join("minecraft/mods/a.jar")).unwrap_or_default(),
        body
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_failed_fill_publishes_nothing_and_leaves_no_temporary() {
    let (root, game) = game();
    let result = game.write_new("config/example.toml", |w| {
        w.write_all(b"partial").map_err(io)?;
        Err(CoreError::new(ErrorCode::OperationCancelled))
    });
    assert_eq!(code(result), Some(ErrorCode::OperationCancelled));
    assert_eq!(game.entry("config/example.toml").ok(), Some(Entry::Absent));
    let leftovers = std::fs::read_dir(root.join("minecraft/config"))
        .map(|entries| entries.flatten().count())
        .unwrap_or(99);
    assert_eq!(leftovers, 0);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn player_created_folders_and_files_are_handled_as_ordinary_data() {
    let (root, game) = game();
    // A folder the game or the player made has ordinary permissions.
    assert!(std::fs::create_dir_all(root.join("minecraft/mods")).is_ok());
    assert!(std::fs::write(root.join("minecraft/mods/player.jar"), b"theirs").is_ok());
    assert!(
        game.write_new("mods/pack.jar", |w| w.write_all(b"ours").map_err(io))
            .is_ok()
    );
    assert_eq!(game.entry("mods/player.jar").ok(), Some(Entry::File));
    assert_eq!(
        std::fs::read(root.join("minecraft/mods/player.jar")).unwrap_or_default(),
        b"theirs"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn directories_and_unsafe_paths_are_not_files() {
    let (root, game) = game();
    let cancel = Cancellation::default();
    assert!(std::fs::create_dir_all(root.join("minecraft/mods/folder.jar")).is_ok());
    assert_eq!(game.entry("mods/folder.jar").ok(), Some(Entry::Unsafe));
    assert_eq!(
        game.compare("mods/folder.jar", &expected(b"x"), &cancel)
            .ok(),
        Some(Content::Unsafe)
    );
    assert_eq!(
        code(game.write_new("mods/folder.jar", |_| Ok(()))),
        Some(ErrorCode::PackConflict)
    );
    assert_eq!(
        code(game.remove_file("mods/folder.jar")),
        Some(ErrorCode::PackConflict)
    );
    for bad in [
        "../escape.jar",
        "mods/../../x",
        "saves/world.dat",
        "mods\\x.jar",
        "mods/CON.jar",
        "/abs.jar",
    ] {
        assert!(game.entry(bad).is_err(), "{bad}");
        assert!(game.write_new(bad, |_| Ok(())).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn hard_links_and_linked_parents_are_refused() {
    let (root, game) = game();
    let cancel = Cancellation::default();
    assert!(std::fs::create_dir_all(root.join("minecraft/mods")).is_ok());
    assert!(std::fs::write(root.join("minecraft/mods/real.jar"), b"data").is_ok());
    if std::fs::hard_link(
        root.join("minecraft/mods/real.jar"),
        root.join("minecraft/mods/linked.jar"),
    )
    .is_ok()
    {
        assert_eq!(game.entry("mods/linked.jar").ok(), Some(Entry::Unsafe));
        assert_eq!(
            game.compare("mods/linked.jar", &expected(b"data"), &cancel)
                .ok(),
            Some(Content::Unsafe)
        );
    }
    let outside = root.join("outside");
    assert!(std::fs::create_dir_all(&outside).is_ok());
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, root.join("minecraft/resourcepacks")).is_ok();
    #[cfg(windows)]
    let linked =
        std::os::windows::fs::symlink_dir(&outside, root.join("minecraft/resourcepacks")).is_ok();
    if linked {
        assert_eq!(
            code(game.write_new("resourcepacks/pack.zip", |w| w.write_all(b"x").map_err(io))),
            Some(ErrorCode::PackConflict)
        );
        assert!(!outside.join("pack.zip").exists());
        assert_eq!(
            code(game.entry("resourcepacks/pack.zip")),
            Some(ErrorCode::PackConflict)
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_regular_files_are_removed() {
    let (root, game) = game();
    assert!(
        game.write_new("mods/a.jar", |w| w.write_all(b"x").map_err(io))
            .is_ok()
    );
    assert!(game.remove_file("mods/a.jar").is_ok());
    assert_eq!(game.entry("mods/a.jar").ok(), Some(Entry::Absent));
    assert!(game.remove_file("mods/a.jar").is_ok());
    assert!(game.remove_file("shaderpacks/none.zip").is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fingerprints_report_absent_files_unsafe_nodes_and_exact_hashes() {
    let (root, game) = game();
    let cancel = Cancellation::default();
    assert_eq!(
        game.fingerprint("mods/a.jar", &cancel).ok(),
        Some(Local::Absent)
    );
    assert!(
        game.write_new("mods/a.jar", |w| w.write_all(b"bytes").map_err(io))
            .is_ok()
    );
    let wanted = expected(b"bytes");
    assert_eq!(
        game.fingerprint("mods/a.jar", &cancel).ok(),
        Some(Local::File(Hashes {
            sha1: wanted.sha1,
            sha512: wanted.sha512
        }))
    );
    assert!(std::fs::create_dir_all(root.join("minecraft/folder")).is_ok());
    assert_eq!(
        game.fingerprint("folder", &cancel).ok(),
        Some(Local::Unsafe)
    );
    let stopped = Cancellation::default();
    stopped.cancel();
    assert!(game.fingerprint("mods/a.jar", &stopped).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_file_moves_into_the_stash_and_back_without_replacing_anything() {
    let (root, game) = game();
    let operation = Uuid::new_v4();
    let stash =
        Stash::open(&root, ".pack-transaction", operation).unwrap_or_else(|_| panic!("stash"));
    assert!(
        game.write_new("mods/a.jar", |w| w.write_all(b"old").map_err(io))
            .is_ok()
    );
    // Nothing to move for an absent path.
    assert_eq!(game.stash("mods/none.jar", &stash).ok(), Some(false));
    assert_eq!(game.stash("mods/a.jar", &stash).ok(), Some(true));
    assert_eq!(game.entry("mods/a.jar").ok(), Some(Entry::Absent));
    assert_eq!(stash.has("mods/a.jar").ok(), Some(true));
    assert_eq!(
        std::fs::read(
            root.join(".pack-transaction")
                .join(operation.to_string())
                .join("mods/a.jar")
        )
        .unwrap_or_default(),
        b"old"
    );
    // The stash never overwrites an earlier entry, and a restore never replaces a new file.
    assert!(
        game.write_new("mods/a.jar", |w| w.write_all(b"new").map_err(io))
            .is_ok()
    );
    assert_eq!(
        code(game.restore("mods/a.jar", &stash)),
        Some(ErrorCode::PackConflict)
    );
    assert_eq!(
        code(game.stash("mods/a.jar", &stash)),
        Some(ErrorCode::PackConflict)
    );
    assert!(game.remove_file("mods/a.jar").is_ok());
    assert_eq!(game.restore("mods/a.jar", &stash).ok(), Some(true));
    assert_eq!(
        std::fs::read(root.join("minecraft/mods/a.jar")).unwrap_or_default(),
        b"old"
    );
    assert_eq!(stash.has("mods/a.jar").ok(), Some(false));
    assert_eq!(game.restore("mods/a.jar", &stash).ok(), Some(false));

    // A folder is not a file and is never moved.
    assert!(std::fs::create_dir_all(root.join("minecraft/folder")).is_ok());
    assert_eq!(
        code(game.stash("folder", &stash)),
        Some(ErrorCode::PackConflict)
    );
    // Reopening finds the same stash. Discarding needs every handle closed, which
    // Windows enforces, and removes the folder with its contents.
    drop(stash);
    assert!(
        Stash::existing(&root, ".pack-transaction", operation)
            .ok()
            .flatten()
            .is_some()
    );
    assert!(Stash::discard(&root, ".pack-transaction", operation).is_ok());
    assert!(
        Stash::existing(&root, ".pack-transaction", operation)
            .ok()
            .flatten()
            .is_none()
    );
    assert!(Stash::discard(&root, ".pack-transaction", operation).is_ok());
    let _ = std::fs::remove_dir_all(&root);
}
