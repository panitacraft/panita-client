use super::*;
use crate::minecraft::download::ensure_parent;
use uuid::Uuid;

fn root() -> AppPaths {
    AppPaths::new(std::env::temp_dir().join(format!("panita-store-{}", Uuid::new_v4())))
        .unwrap_or_else(|_| panic!("root"))
}

fn put(paths: &AppPaths, relative: &str, bytes: &[u8]) {
    let (directory, leaf) =
        ensure_parent(paths.root(), relative).unwrap_or_else(|_| panic!("parent"));
    assert!(directory.write_atomic_durable(leaf, bytes).is_ok());
}

fn fixture(paths: &AppPaths) {
    put(paths, "content/sha512/ab/abcdef-5", b"12345");
    put(paths, "content/sha512/ab/abc999-3", b"123");
    put(paths, "content/sha512/cd/cdef01-4", b"1234");
    put(paths, "content/sha1/ff/ff0011-2", b"12");
    // Other caches and instance data must survive.
    put(paths, "libraries/aa/aabb.jar", b"library");
    put(paths, "instances/slug/minecraft/mods/a.jar", b"mod");
}

#[test]
fn size_adds_entries_and_missing_caches_are_empty() {
    let paths = root();
    let cancel = Cancellation::default();
    assert_eq!(size(&paths, &cancel).ok(), Some(0));
    assert_eq!(clear(&paths, &cancel).ok(), Some(ClearReport::default()));
    fixture(&paths);
    assert_eq!(size(&paths, &cancel).ok(), Some(14));
    let _ = std::fs::remove_dir_all(paths.root());
}

#[test]
fn clearing_removes_only_cache_entries_and_reports_them() {
    let paths = root();
    let cancel = Cancellation::default();
    fixture(&paths);
    let report = clear(&paths, &cancel).unwrap_or_else(|_| panic!("clear"));
    assert_eq!(report.removed_entries, 4);
    assert_eq!(report.removed_bytes, 14);
    assert_eq!(report.retained_entries, 0);
    assert!(!paths.root().join("content").exists());
    assert_eq!(size(&paths, &cancel).ok(), Some(0));
    // Other caches and instances are untouched.
    assert_eq!(
        std::fs::read(paths.root().join("libraries/aa/aabb.jar")).unwrap_or_default(),
        b"library"
    );
    assert_eq!(
        std::fs::read(paths.root().join("instances/slug/minecraft/mods/a.jar")).unwrap_or_default(),
        b"mod"
    );
    // Clearing twice is harmless, and the cache can be filled again afterwards.
    assert_eq!(clear(&paths, &cancel).ok(), Some(ClearReport::default()));
    put(&paths, "content/sha512/ab/abcdef-5", b"12345");
    assert_eq!(size(&paths, &cancel).ok(), Some(5));
    let _ = std::fs::remove_dir_all(paths.root());
}

#[test]
fn a_shard_with_a_running_writer_is_kept_and_reported() {
    let paths = root();
    let cancel = Cancellation::default();
    fixture(&paths);
    let held = try_lock(paths.root(), &shard_lock("content/sha512/ab")).unwrap_or_default();
    assert!(held.is_some());
    let report = clear(&paths, &cancel).unwrap_or_else(|_| panic!("clear"));
    assert_eq!(report.removed_entries, 2);
    assert_eq!(report.retained_entries, 2);
    assert!(
        report
            .details
            .iter()
            .any(|d| d.directory == "content/sha512/ab" && d.reason == "inUse")
    );
    assert!(paths.root().join("content/sha512/ab/abcdef-5").exists());
    assert!(!paths.root().join("content/sha512/cd").exists());
    drop(held);
    let report = clear(&paths, &cancel).unwrap_or_else(|_| panic!("clear"));
    assert_eq!(report.removed_entries, 2);
    assert!(!paths.root().join("content").exists());
    let _ = std::fs::remove_dir_all(paths.root());
}

#[test]
fn unknown_folders_stay_and_cancellation_stops_before_deleting() {
    let paths = root();
    fixture(&paths);
    put(&paths, "content/future/aa/entry", b"?");
    let report = clear(&paths, &Cancellation::default()).unwrap_or_else(|_| panic!("clear"));
    assert!(
        report
            .details
            .iter()
            .any(|d| d.directory == "future" && d.reason == "unknown")
    );
    assert!(paths.root().join("content/future/aa/entry").exists());
    assert!(!paths.root().join("content/sha512").exists());
    let _ = std::fs::remove_dir_all(paths.root());

    let paths = root();
    fixture(&paths);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(clear(&paths, &cancel).is_err());
    assert_eq!(size(&paths, &Cancellation::default()).ok(), Some(14));
    let _ = std::fs::remove_dir_all(paths.root());
}

#[cfg(unix)]
#[test]
fn links_inside_the_cache_are_never_followed_or_removed() {
    let paths = root();
    fixture(&paths);
    let outside = std::env::temp_dir().join(format!("panita-outside-{}", Uuid::new_v4()));
    assert!(std::fs::write(&outside, b"keep me").is_ok());
    assert!(
        std::os::unix::fs::symlink(&outside, paths.root().join("content/sha512/ab/link-1")).is_ok()
    );
    let report = clear(&paths, &Cancellation::default()).unwrap_or_else(|_| panic!("clear"));
    assert!(report.details.iter().any(|d| d.reason == "unsafePath"));
    assert_eq!(std::fs::read(&outside).unwrap_or_default(), b"keep me");
    let _ = std::fs::remove_file(outside);
    let _ = std::fs::remove_dir_all(paths.root());
}
