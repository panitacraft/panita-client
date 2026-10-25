use super::*;

fn fixture() -> (AppPaths, std::path::PathBuf, InstalledLock) {
    let (paths, accounts, id) = crate::minecraft::play::tests::fixture();
    drop(accounts);
    crate::minecraft::play::tests::complete_installation(&paths, id);
    let root = install::metadata_root(&paths, id).unwrap_or_else(|_| panic!("root"));
    let mut installed: InstalledLock = read_json(&root, "installed.json")
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("manifest"));
    installed.native_directory = Some(format!("natives/{}", installed.operation));
    super::super::install::write_json(&root, "installed.json", &installed)
        .unwrap_or_else(|_| panic!("manifest"));
    (paths, root, installed)
}

fn generation(root: &Path, name: &str) -> std::path::PathBuf {
    let directory = root.join(name);
    std::fs::create_dir_all(directory.join("nested/extra")).unwrap_or_else(|_| panic!("directory"));
    std::fs::write(directory.join("lwjgl.dll"), b"runtime native")
        .unwrap_or_else(|_| panic!("native"));
    std::fs::write(directory.join("nested/extra/edited.dll"), b"edited content")
        .unwrap_or_else(|_| panic!("edited"));
    std::fs::write(
        directory.join(".generation.json"),
        b"{corrupt legacy receipt",
    )
    .unwrap_or_else(|_| panic!("receipt"));
    std::fs::write(directory.join("jna123.dll.x"), b"").unwrap_or_else(|_| panic!("marker"));
    directory
}

#[test]
fn manual_cleanup_removes_untracked_and_modified_generations_but_preserves_active_and_player_data()
{
    let (paths, root, installed) = fixture();
    let active = generation(&root, &installed.native_directory().unwrap_or_default());
    let old = generation(&root, &format!("natives/{}", Uuid::new_v4()));
    let legacy = generation(&root, &format!("natives-{}", Uuid::new_v4()));
    let extra = generation(&root, "natives/old-runtime");
    let game = root.join("minecraft");
    std::fs::create_dir_all(game.join("saves/world")).unwrap_or_else(|_| panic!("world"));
    std::fs::write(game.join("saves/world/level.dat"), b"world")
        .unwrap_or_else(|_| panic!("world"));
    std::fs::write(game.join("options.txt"), b"settings").unwrap_or_else(|_| panic!("options"));
    std::fs::write(
        root.join("natives-personal.txt"),
        b"outside native namespace",
    )
    .unwrap_or_else(|_| panic!("outside"));
    let cache = paths.root().join(&installed.plan.artifacts[0].destination);
    let original_cache = std::fs::read(&cache).unwrap_or_default();
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|error| panic!("cleanup: {error}"));
    assert_eq!(report.cleaned_generations, 3);
    assert_eq!(report.retained_generations, 0);
    assert_eq!(report.untracked_generations, 0);
    assert_eq!(report.details.len(), 1);
    assert_eq!(report.details[0].reason, "active");
    assert!(!old.exists() && !legacy.exists() && !extra.exists());
    assert_eq!(
        std::fs::read(active.join("lwjgl.dll")).unwrap_or_default(),
        b"runtime native"
    );
    assert_eq!(
        std::fs::read(game.join("saves/world/level.dat")).unwrap_or_default(),
        b"world"
    );
    assert_eq!(
        std::fs::read(game.join("options.txt")).unwrap_or_default(),
        b"settings"
    );
    assert_eq!(std::fs::read(cache).unwrap_or_default(), original_cache);
    assert!(root.join("natives-personal.txt").exists());
    assert_eq!(
        cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("again"))
            .cleaned_generations,
        0
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn busy_cancelled_and_unresolved_operations_never_delete_native_folders() {
    let (paths, root, installed) = fixture();
    let old = generation(&root, &format!("natives/{}", Uuid::new_v4()));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        cleanup(&paths, installed.instance_id, &cancel)
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::OperationCancelled)
    );
    let lease = super::super::process::LaunchLease::acquire(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("lease"));
    assert_eq!(
        cleanup(&paths, installed.instance_id, &Cancellation::default())
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::OperationBusy)
    );
    drop(lease);
    let private = private_fs::PrivateDir::open(&root).unwrap_or_else(|_| panic!("root"));
    private
        .write_atomic_durable("transaction.json", b"{broken")
        .unwrap_or_else(|_| panic!("intent"));
    drop(private);
    assert!(cleanup(&paths, installed.instance_id, &Cancellation::default()).is_err());
    assert!(old.join("lwjgl.dll").exists());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn uninstall_removes_all_generations_including_active_and_keeps_worlds_and_hardlink_aliases() {
    let (paths, root, installed) = fixture();
    let active = generation(&root, &installed.native_directory().unwrap_or_default());
    let legacy = generation(&root, &format!("natives-{}", Uuid::new_v4()));
    let old = generation(&root, "natives/untracked-extra");
    let alias = root.join("minecraft/player-owned.bin");
    std::fs::hard_link(active.join("lwjgl.dll"), &alias).unwrap_or_else(|_| panic!("hard link"));
    std::fs::write(root.join("minecraft/options.txt"), b"settings")
        .unwrap_or_else(|_| panic!("options"));
    let outcome = super::super::maintenance::uninstall(
        &paths,
        installed.instance_id,
        &Cancellation::default(),
    )
    .unwrap_or_else(|error| panic!("uninstall: {error}"));
    let report = outcome
        .native_cleanup
        .unwrap_or_else(|| panic!("cleanup report"));
    assert_eq!(report.cleaned_generations, 3);
    assert_eq!(report.retained_generations, 0);
    assert!(!active.exists() && !legacy.exists() && !old.exists());
    assert!(!root.join("natives").exists());
    assert!(!root.join("transaction.json").exists());
    assert_eq!(std::fs::read(alias).unwrap_or_default(), b"runtime native");
    assert_eq!(
        std::fs::read(root.join("minecraft/options.txt")).unwrap_or_default(),
        b"settings"
    );
    let tombstone: InstalledLock = read_json(&root, "installed.json")
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("tombstone"));
    assert_eq!(tombstone.schema_version, 3);
    assert!(
        super::super::maintenance::uninstall(
            &paths,
            installed.instance_id,
            &Cancellation::default()
        )
        .is_ok()
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[cfg(windows)]
#[test]
fn blocked_uninstall_keeps_recovery_intent_until_every_native_is_removed() {
    use std::os::windows::fs::OpenOptionsExt;
    let (paths, root, installed) = fixture();
    let active = generation(&root, &installed.native_directory().unwrap_or_default());
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(active.join("lwjgl.dll"))
        .unwrap_or_else(|_| panic!("lock file"));
    let outcome = super::super::maintenance::uninstall(
        &paths,
        installed.instance_id,
        &Cancellation::default(),
    )
    .unwrap_or_else(|_| panic!("uninstall"));
    let report = outcome.native_cleanup.unwrap_or_else(|| panic!("report"));
    assert_eq!(report.retained_generations, 1);
    assert_eq!(report.details[0].reason, "accessFailure");
    assert!(root.join("transaction.json").exists());
    assert_eq!(
        install::recover_locked(&paths, installed.instance_id, &root)
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::NativeCleanupIncomplete)
    );
    drop(locked);
    let retried = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("retry cleanup"));
    assert_eq!(retried.cleaned_generations, 1);
    assert!(!root.join("natives").exists());
    assert!(!root.join("transaction.json").exists());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

fn redirect(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link.components().collect::<std::path::PathBuf>())
            .arg(target.components().collect::<std::path::PathBuf>())
            .output()
            .unwrap_or_else(|_| panic!("junction"));
        assert!(
            output.status.success(),
            "junction creation: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap_or_else(|_| panic!("symlink"));
}

fn remove_redirect(link: &Path) {
    #[cfg(windows)]
    std::fs::remove_dir(link).unwrap_or_else(|_| panic!("remove junction only"));
    #[cfg(unix)]
    std::fs::remove_file(link).unwrap_or_else(|_| panic!("remove symlink only"));
}

#[test]
fn cleanup_never_follows_redirected_containers_generations_or_nested_entries() {
    for kind in ["container", "generation", "nested"] {
        let (paths, root, installed) = fixture();
        let outside = root.join("minecraft/saves/world");
        std::fs::create_dir_all(&outside).unwrap_or_else(|_| panic!("world directory"));
        std::fs::write(outside.join("level.dat"), b"preserve world")
            .unwrap_or_else(|_| panic!("world"));
        let link = match kind {
            "container" => root.join("natives"),
            "generation" => {
                std::fs::create_dir_all(root.join("natives"))
                    .unwrap_or_else(|_| panic!("container"));
                root.join(format!("natives/{}", Uuid::new_v4()))
            }
            _ => generation(&root, &format!("natives/{}", Uuid::new_v4())).join("redirect"),
        };
        redirect(&link, &outside);
        let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("cleanup"));
        assert!(report.retained_generations > 0, "{kind}");
        assert_eq!(
            std::fs::read(outside.join("level.dat")).unwrap_or_default(),
            b"preserve world"
        );
        remove_redirect(&link);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }
}

#[test]
fn cleanup_resumes_after_partial_directory_removal_without_a_receipt() {
    let (paths, root, installed) = fixture();
    let old = generation(&root, &format!("natives/{}", Uuid::new_v4()));
    std::fs::remove_file(old.join(".generation.json")).unwrap_or_else(|_| panic!("no receipt"));
    std::fs::remove_file(old.join("lwjgl.dll")).unwrap_or_else(|_| panic!("partial removal"));
    assert_eq!(
        cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("resume"))
            .cleaned_generations,
        1
    );
    assert!(!old.exists());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}
