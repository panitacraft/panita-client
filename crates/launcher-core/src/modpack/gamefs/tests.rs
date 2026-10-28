use super::*;
use sha1::{Digest as _, Sha1};
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
