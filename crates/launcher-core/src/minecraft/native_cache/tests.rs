use super::*;
use sha1::{Digest, Sha1};

fn fixture() -> (AppPaths, std::path::PathBuf, InstalledLock) {
    let (paths, accounts, id) = crate::minecraft::play::tests::fixture();
    drop(accounts);
    crate::minecraft::play::tests::complete_installation(&paths, id);
    let root = install::metadata_root(&paths, id).unwrap_or_else(|_| panic!("root"));
    let installed = read_json(&root, "installed.json")
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("manifest"));
    (paths, root, installed)
}

fn generation(root: &Path, installed: &InstalledLock, legacy: bool) -> InstalledLock {
    let mut next = installed.clone();
    next.operation = Uuid::new_v4();
    next.native_directory = if legacy {
        None
    } else {
        Some(format!("natives/{}", next.operation))
    };
    let relative = next.native_directory().unwrap_or_default();
    let native_root = root.join(&relative);
    let mut parent = private_fs::PrivateDir::open(root).unwrap_or_else(|_| panic!("root"));
    for part in relative.split('/') {
        parent = parent
            .ensure_dir(part)
            .unwrap_or_else(|error| panic!("generation root: {error}"));
    }
    drop(parent);
    next.native_files = vec![super::super::natives::NativeFile {
        path: "nested/lib.dll".into(),
        size: 6,
        sha1: format!("{:x}", Sha1::digest(b"native")),
    }];
    let (directory, leaf) = super::super::download::ensure_parent(&native_root, "nested/lib.dll")
        .unwrap_or_else(|_| panic!("native directory"));
    directory
        .write_atomic_durable(leaf, b"native")
        .unwrap_or_else(|_| panic!("native"));
    record(root, &next).unwrap_or_else(|error| panic!("receipt: {error}"));
    next
}

#[test]
fn cleanup_preserves_active_modified_unknown_and_untracked_native_data() {
    let (paths, root, installed) = fixture();
    let active = generation(&root, &installed, false);
    write_json(&root, "installed.json", &active).unwrap_or_else(|_| panic!("active"));
    let obsolete = generation(&root, &active, true);
    let edited = generation(&root, &active, false);
    let unknown = generation(&root, &active, false);
    let edited_path = root
        .join(edited.native_directory().unwrap_or_default())
        .join("nested/lib.dll");
    let parent =
        private_fs::PrivateDir::open(edited_path.parent().unwrap_or_else(|| panic!("parent")))
            .unwrap_or_else(|_| panic!("edited parent"));
    parent
        .write_atomic_durable("lib.dll", b"edited")
        .unwrap_or_else(|_| panic!("edit"));
    let unknown_path = root
        .join(unknown.native_directory().unwrap_or_default())
        .join("user.txt");
    std::fs::write(&unknown_path, b"preserve unknown").unwrap_or_else(|_| panic!("unknown"));
    let untracked = root.join(format!("natives-{}", Uuid::new_v4()));
    private_fs::PrivateDir::ensure(&untracked)
        .unwrap_or_else(|_| panic!("untracked"))
        .write_atomic_durable("keep.dll", b"untracked")
        .unwrap_or_else(|_| panic!("untracked file"));
    let world = root.join("minecraft/player-marker.txt");
    std::fs::write(&world, b"world").unwrap_or_else(|_| panic!("player data"));
    let report = cleanup(&paths, active.instance_id, &Cancellation::default())
        .unwrap_or_else(|error| panic!("cleanup: {error}"));
    assert_eq!(report.cleaned_generations, 1);
    assert_eq!(report.retained_generations, 2);
    assert_eq!(report.untracked_generations, 1);
    assert!(
        !root
            .join(obsolete.native_directory().unwrap_or_default())
            .exists()
    );
    assert!(
        root.join(active.native_directory().unwrap_or_default())
            .join("nested/lib.dll")
            .exists()
    );
    assert_eq!(std::fs::read(edited_path).unwrap_or_default(), b"edited");
    assert_eq!(
        std::fs::read(unknown_path).unwrap_or_default(),
        b"preserve unknown"
    );
    assert_eq!(std::fs::read(world).unwrap_or_default(), b"world");
    assert_eq!(
        cleanup(&paths, active.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("again"))
            .cleaned_generations,
        0
    );
    drop(parent);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn cleanup_rejects_busy_cancelled_foreign_and_hard_linked_ownership() {
    let (paths, root, installed) = fixture();
    let foreign = generation(&root, &installed, false);
    let hard_linked = generation(&root, &installed, false);
    let foreign_root = root.join(foreign.native_directory().unwrap_or_default());
    let mut receipt: Generation = read_json(&foreign_root, RECEIPT)
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("receipt"));
    receipt.id = Uuid::new_v4();
    write_json(
        &root,
        &format!(
            "{}/{RECEIPT}",
            foreign.native_directory().unwrap_or_default()
        ),
        &receipt,
    )
    .unwrap_or_else(|_| panic!("foreign receipt"));
    let source = root
        .join(hard_linked.native_directory().unwrap_or_default())
        .join("nested/lib.dll");
    let alias = root.join("hard-link-alias.dll");
    std::fs::hard_link(&source, &alias).unwrap_or_else(|_| panic!("hard link"));
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
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("cleanup"));
    assert_eq!(report.retained_generations, 2);
    assert!(source.exists());
    assert!(foreign_root.join("nested/lib.dll").exists());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn cleanup_resumes_after_file_removal_and_does_not_create_empty_generations() {
    let (paths, root, installed) = fixture();
    record(&root, &installed).unwrap_or_else(|_| panic!("empty native set"));
    assert!(!root.join("natives").exists());
    let obsolete = generation(&root, &installed, false);
    super::super::maintenance::natives(&root, &obsolete, true)
        .unwrap_or_else(|_| panic!("interrupted cleanup"));
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("resume"));
    assert_eq!(report.cleaned_generations, 1);
    assert!(
        !root
            .join(obsolete.native_directory().unwrap_or_default())
            .exists()
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn unsafe_receipts_and_unresolved_transactions_cannot_authorize_cleanup() {
    let (paths, root, installed) = fixture();
    let forged = generation(&root, &installed, false);
    let forged_root = root.join(forged.native_directory().unwrap_or_default());
    let mut receipt: Generation = read_json(&forged_root, RECEIPT)
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("receipt"));
    receipt.files[0].path = "../../minecraft/player-marker.txt".into();
    write_json(
        &root,
        &format!(
            "{}/{RECEIPT}",
            forged.native_directory().unwrap_or_default()
        ),
        &receipt,
    )
    .unwrap_or_else(|_| panic!("unsafe receipt"));
    let marker = root.join("minecraft/player-marker.txt");
    std::fs::write(&marker, b"preserve").unwrap_or_else(|_| panic!("marker"));
    assert_eq!(
        cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("preserving cleanup"))
            .retained_generations,
        1
    );
    assert_eq!(std::fs::read(&marker).unwrap_or_default(), b"preserve");
    assert!(forged_root.join("nested/lib.dll").exists());
    let valid = generation(&root, &installed, true);
    let private = private_fs::PrivateDir::open(&root).unwrap_or_else(|_| panic!("root"));
    private
        .write_atomic_durable("transaction.json", b"{broken")
        .unwrap_or_else(|_| panic!("pending recovery"));
    assert!(cleanup(&paths, installed.instance_id, &Cancellation::default()).is_err());
    assert!(
        root.join(valid.native_directory().unwrap_or_default())
            .join("nested/lib.dll")
            .exists()
    );
    assert_eq!(
        std::fs::read(root.join("transaction.json")).unwrap_or_default(),
        b"{broken"
    );
    drop(private);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

fn runtime_fixture() -> (AppPaths, std::path::PathBuf, InstalledLock) {
    use std::io::{Cursor, Write};
    let (paths, root, mut installed) = fixture();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("windows/x64/lwjgl.dll", b"native".as_slice()),
        (
            "com/sun/jna/win32-x86-64/jnidispatch.dll",
            b"jna".as_slice(),
        ),
    ] {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap_or_else(|_| panic!("entry"));
        writer.write_all(bytes).unwrap_or_else(|_| panic!("bytes"));
    }
    let bytes = writer
        .finish()
        .unwrap_or_else(|_| panic!("zip"))
        .into_inner();
    let artifact = super::super::Artifact {
        url: "https://libraries.minecraft.net/runtime-fixture.jar".into(),
        path: Some("test/native.jar".into()),
        sha1: format!("{:x}", Sha1::digest(&bytes)),
        size: bytes.len() as u64,
    };
    let destination =
        super::super::plan::library_cache_path(&artifact).unwrap_or_else(|_| panic!("cache"));
    let (parent, leaf) = super::super::download::ensure_parent(paths.root(), &destination)
        .unwrap_or_else(|_| panic!("parent"));
    parent
        .write_atomic_durable(&leaf, &bytes)
        .unwrap_or_else(|_| panic!("cache"));
    installed.plan.version.classpath.push(artifact.clone());
    installed
        .plan
        .artifacts
        .push(super::super::plan::PlannedArtifact {
            artifact,
            destination,
            executable: false,
        });
    write_json(&root, "installed.json", &installed).unwrap_or_else(|_| panic!("manifest"));
    (paths, root, installed)
}

fn retire(root: &Path, old: &InstalledLock) -> InstalledLock {
    let mut next = old.clone();
    next.operation = Uuid::new_v4();
    next.native_directory = Some(format!("natives/{}", next.operation));
    next.native_files.clear();
    write_json(root, "installed.json", &next).unwrap_or_else(|_| panic!("retire"));
    next
}

#[test]
fn tracked_runtime_files_and_jna_markers_are_reclaimed_but_preexisting_files_stay() {
    let (paths, root, installed) = runtime_fixture();
    let relative = installed.native_directory().unwrap_or_default();
    let (private, _) =
        super::super::download::ensure_parent(&root, &format!("{relative}/placeholder"))
            .unwrap_or_else(|_| panic!("generation"));
    private
        .write_atomic_durable("lwjgl.dll", b"native")
        .unwrap_or_else(|_| panic!("baseline"));
    let observation = capture(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("capture"))
        .unwrap_or_else(|| panic!("tracking"))
        .publish()
        .unwrap_or_else(|_| panic!("publish"));
    private
        .write_atomic_durable("jna123.dll", b"jna")
        .unwrap_or_else(|_| panic!("jna"));
    private
        .write_atomic_durable("jna123.dll.x", b"")
        .unwrap_or_else(|_| panic!("marker"));
    private
        .write_atomic_durable("jna999.dll.x", b"")
        .unwrap_or_else(|_| panic!("unknown marker"));
    observation
        .finish()
        .unwrap_or_else(|_| panic!("completed execution"));
    let (late, _) =
        super::super::download::ensure_parent(&root, &format!("{relative}/late/lwjgl.dll"))
            .unwrap_or_else(|_| panic!("idle file"));
    late.write_atomic_durable("lwjgl.dll", b"native")
        .unwrap_or_else(|_| panic!("idle native"));
    drop(late);
    // Subsequent runs retain prior observations but cannot claim files added while idle.
    capture(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("again"))
        .unwrap_or_else(|| panic!("tracking"))
        .publish()
        .unwrap_or_else(|_| panic!("publish again"))
        .finish()
        .unwrap_or_else(|_| panic!("finish again"));
    retire(&root, &installed);
    // Closing the game releases creation handles before idle-only cleanup.
    drop(private);
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|error| panic!("cleanup {error}"));
    assert_eq!(report.retained_generations, 1);
    assert_eq!(report.details[0].reason, "unknownFiles");
    assert!(root.join(&relative).join("lwjgl.dll").exists());
    assert!(root.join(&relative).join("late/lwjgl.dll").exists());
    assert!(!root.join(&relative).join("jna123.dll").exists());
    assert!(!root.join(&relative).join("jna123.dll.x").exists());
    assert!(root.join(&relative).join("jna999.dll.x").exists());
    let again = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("repeat"));
    assert_eq!(again.retained_generations, 1);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn runtime_cleanup_removes_whole_obsolete_generations_and_preserves_active_ones() {
    let (paths, root, installed) = runtime_fixture();
    let observation = capture(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("capture"))
        .unwrap_or_else(|| panic!("tracking"))
        .publish()
        .unwrap_or_else(|_| panic!("publish"));
    let relative = installed.native_directory().unwrap_or_default();
    let private =
        private_fs::PrivateDir::open(root.join(&relative)).unwrap_or_else(|_| panic!("generation"));
    private
        .write_atomic_durable("lwjgl.dll", b"native")
        .unwrap_or_else(|_| panic!("runtime"));
    assert_eq!(
        cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("active"))
            .cleaned_generations,
        0
    );
    observation
        .finish()
        .unwrap_or_else(|_| panic!("completed execution"));
    retire(&root, &installed);
    drop(private);
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|error| panic!("cleanup {error}"));
    assert_eq!(report.cleaned_generations, 1);
    assert!(!root.join(relative).exists());
    assert_eq!(
        cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("again"))
            .cleaned_generations,
        0
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn corrupt_libraries_and_modified_runtime_files_cannot_be_adopted() {
    let (paths, root, installed) = runtime_fixture();
    let observation = capture(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("capture"))
        .unwrap_or_else(|| panic!("tracking"))
        .publish()
        .unwrap_or_else(|_| panic!("publish"));
    let relative = installed.native_directory().unwrap_or_default();
    let private =
        private_fs::PrivateDir::open(root.join(&relative)).unwrap_or_else(|_| panic!("generation"));
    private
        .write_atomic_durable("lwjgl.dll", b"modified")
        .unwrap_or_else(|_| panic!("runtime"));
    observation
        .finish()
        .unwrap_or_else(|_| panic!("completed execution"));
    retire(&root, &installed);
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("modified"));
    assert_eq!(report.retained_generations, 1);
    assert_eq!(
        std::fs::read(root.join(&relative).join("lwjgl.dll")).unwrap_or_default(),
        b"modified"
    );
    let library = &installed
        .plan
        .artifacts
        .last()
        .unwrap_or_else(|| panic!("library"))
        .destination;
    let (parent, leaf) = super::super::download::existing_parent(paths.root(), library)
        .unwrap_or_else(|_| panic!("parent"))
        .unwrap_or_else(|| panic!("cache"));
    parent
        .write_atomic_durable(leaf, b"corrupt")
        .unwrap_or_else(|_| panic!("corrupt"));
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("corrupt cleanup"));
    assert_eq!(report.details[0].reason, "unverifiedLibraries");
    assert!(root.join(&relative).join("lwjgl.dll").exists());
    drop(private);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn failed_spawn_inventory_does_not_leave_an_empty_generation() {
    let (paths, root, installed) = runtime_fixture();
    capture(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("capture"))
        .unwrap_or_else(|| panic!("tracking"))
        .abort();
    assert!(
        !root
            .join(installed.native_directory().unwrap_or_default())
            .exists()
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn unfinished_runs_idle_files_and_replaced_files_are_not_adopted() {
    for scenario in ["unfinished", "idle", "replaced"] {
        let (paths, root, installed) = runtime_fixture();
        let observation = capture(&paths, installed.instance_id)
            .unwrap_or_else(|_| panic!("capture"))
            .unwrap_or_else(|| panic!("tracking"))
            .publish()
            .unwrap_or_else(|_| panic!("publish"));
        let relative = installed.native_directory().unwrap_or_default();
        let private = private_fs::PrivateDir::open(root.join(&relative))
            .unwrap_or_else(|_| panic!("generation"));
        if scenario == "idle" {
            observation.finish().unwrap_or_else(|_| panic!("finish"));
            private
                .write_atomic_durable("lwjgl.dll", b"native")
                .unwrap_or_else(|_| panic!("idle"));
        } else {
            private
                .write_atomic_durable("lwjgl.dll", b"native")
                .unwrap_or_else(|_| panic!("runtime"));
            if scenario == "replaced" {
                observation.finish().unwrap_or_else(|_| panic!("finish"));
                private
                    .write_atomic_durable("lwjgl.dll", b"native")
                    .unwrap_or_else(|_| panic!("replace"));
            }
        }
        drop(private);
        retire(&root, &installed);
        let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("cleanup"));
        assert_eq!(report.cleaned_generations, 0, "{scenario}");
        assert_eq!(
            std::fs::read(root.join(relative).join("lwjgl.dll")).unwrap_or_default(),
            b"native",
            "{scenario}"
        );
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }
}

#[test]
fn runtime_child_fixture() {
    if !std::env::args().any(|arg| arg == "--exact") {
        return;
    }
    let game = std::env::current_dir().unwrap_or_else(|_| panic!("game"));
    let root = game.parent().unwrap_or_else(|| panic!("instance"));
    let installed: InstalledLock = read_json(root, "installed.json")
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("manifest"));
    std::fs::write(
        root.join(installed.native_directory().unwrap_or_default())
            .join("lwjgl.dll"),
        b"native",
    )
    .unwrap_or_else(|_| panic!("runtime extraction"));
    std::process::exit(3);
}

#[tokio::test]
async fn owned_child_exit_publishes_runtime_identity_before_releasing_lease() {
    let (paths, root, installed) = runtime_fixture();
    let command = super::super::launch::LaunchCommand {
        executable: std::env::current_exe().unwrap_or_else(|_| panic!("test executable")),
        directory: paths
            .game_directory(installed.instance_id)
            .unwrap_or_else(|_| panic!("game")),
        args: vec![
            "--exact".into(),
            "minecraft::native_cache::tests::runtime_child_fixture".into(),
            "--nocapture".into(),
        ],
        redactions: vec![],
    };
    let game = super::super::process::LaunchLease::acquire(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("lease"))
        .spawn(
            command,
            crate::operation::ProcessSessionId::new(),
            crate::operation::OperationId::new(),
        )
        .unwrap_or_else(|_| panic!("spawn"));
    let exit = tokio::time::timeout(std::time::Duration::from_secs(15), game.wait())
        .await
        .unwrap_or_else(|_| panic!("timeout"))
        .unwrap_or_else(|_| panic!("wait"));
    assert_eq!(exit.code, Some(3));
    assert!(!game.events().iter().any(|event| matches!(
        event.kind,
        super::super::process::ProcessEventKind::NativeTrackingUnavailable
    )));
    retire(&root, &installed);
    assert_eq!(
        cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("cleanup"))
            .cleaned_generations,
        1
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn matching_untracked_files_and_runtime_hard_links_remain_unowned() {
    let (paths, root, installed) = runtime_fixture();
    let observation = capture(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("capture"))
        .unwrap_or_else(|| panic!("tracking"))
        .publish()
        .unwrap_or_else(|_| panic!("publish"));
    let relative = installed.native_directory().unwrap_or_default();
    let private =
        private_fs::PrivateDir::open(root.join(&relative)).unwrap_or_else(|_| panic!("generation"));
    private
        .write_atomic_durable("lwjgl.dll", b"native")
        .unwrap_or_else(|_| panic!("runtime"));
    let alias = root.join("keep-alias.dll");
    std::fs::hard_link(root.join(&relative).join("lwjgl.dll"), &alias)
        .unwrap_or_else(|_| panic!("link"));
    let unknown = format!("natives/{}", Uuid::new_v4());
    let (untracked, _) =
        super::super::download::ensure_parent(&root, &format!("{unknown}/lwjgl.dll"))
            .unwrap_or_else(|_| panic!("untracked"));
    untracked
        .write_atomic_durable("lwjgl.dll", b"native")
        .unwrap_or_else(|_| panic!("matching DLL"));
    drop(untracked);
    drop(private);
    observation
        .finish()
        .unwrap_or_else(|_| panic!("completed execution"));
    retire(&root, &installed);
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("cleanup"));
    assert_eq!(report.untracked_generations, 1);
    assert_eq!(report.retained_generations, 1);
    assert!(
        report
            .details
            .iter()
            .any(|detail| detail.directory == unknown && detail.reason == "missingOwnership")
    );
    assert_eq!(std::fs::read(alias).unwrap_or_default(), b"native");
    assert!(root.join(&unknown).join("lwjgl.dll").exists());
    assert!(!root.join(&unknown).join(RECEIPT).exists());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn runtime_inventory_survives_interrupted_removal() {
    let (paths, root, mut installed) = runtime_fixture();
    let relative = installed.native_directory().unwrap_or_default();
    let (base, _) = super::super::download::ensure_parent(&root, &format!("{relative}/base.dll"))
        .unwrap_or_else(|_| panic!("base directory"));
    base.write_atomic_durable("base.dll", b"native")
        .unwrap_or_else(|_| panic!("base"));
    drop(base);
    installed
        .native_files
        .push(super::super::natives::NativeFile {
            path: "base.dll".into(),
            sha1: format!("{:x}", Sha1::digest(b"native")),
            size: 6,
        });
    write_json(&root, "installed.json", &installed).unwrap_or_else(|_| panic!("base manifest"));
    record(&root, &installed).unwrap_or_else(|_| panic!("base ownership"));
    let observation = capture(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("capture"))
        .unwrap_or_else(|| panic!("tracking"))
        .publish()
        .unwrap_or_else(|_| panic!("publish"));
    let relative = installed.native_directory().unwrap_or_default();
    let private =
        private_fs::PrivateDir::open(root.join(&relative)).unwrap_or_else(|_| panic!("generation"));
    private
        .write_atomic_durable("lwjgl.dll", b"native")
        .unwrap_or_else(|_| panic!("runtime"));
    drop(private);
    observation
        .finish()
        .unwrap_or_else(|_| panic!("completed execution"));
    let mut receipt: Generation = read_json(&root.join(&relative), RECEIPT)
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("receipt"));
    runtime::extend(
        &paths,
        &root,
        &mut receipt,
        &Cancellation::default(),
        &mut runtime::ArchiveCache::new(),
    )
    .unwrap_or_else(|_| panic!("inventory"));
    record(&root, &installed).unwrap_or_else(|_| panic!("preserve extended runtime inventory"));
    let mut recorded = installed.clone();
    recorded.native_files = receipt.files;
    super::super::maintenance::natives(&root, &recorded, true)
        .unwrap_or_else(|_| panic!("interrupted removal"));
    assert!(root.join(&relative).join(RECEIPT).exists());
    retire(&root, &installed);
    assert_eq!(
        cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("resume"))
            .cleaned_generations,
        1
    );
    assert!(!root.join(relative).exists());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn future_runtime_receipts_are_preserved_without_rewriting() {
    let (paths, root, installed) = runtime_fixture();
    capture(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("capture"))
        .unwrap_or_else(|| panic!("tracking"))
        .publish()
        .unwrap_or_else(|_| panic!("publish"));
    let relative = installed.native_directory().unwrap_or_default();
    let mut receipt: Generation = read_json(&root.join(&relative), RECEIPT)
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("receipt"));
    receipt.schema = 999;
    write_json(&root, &format!("{relative}/{RECEIPT}"), &receipt)
        .unwrap_or_else(|_| panic!("future"));
    let before = std::fs::read(root.join(&relative).join(RECEIPT)).unwrap_or_default();
    assert!(capture(&paths, installed.instance_id).is_err());
    retire(&root, &installed);
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("cleanup"));
    assert_eq!(report.details[0].reason, "invalidOwnership");
    assert_eq!(
        std::fs::read(root.join(&relative).join(RECEIPT)).unwrap_or_default(),
        before
    );
    let mut invalid = serde_json::to_value(&receipt).unwrap_or_else(|_| panic!("value"));
    invalid["schema"] = serde_json::json!(2);
    invalid["runtime"]["baseline"] = serde_json::json!(["../player-data"]);
    write_json(&root, &format!("{relative}/{RECEIPT}"), &invalid)
        .unwrap_or_else(|_| panic!("invalid baseline"));
    let before = std::fs::read(root.join(&relative).join(RECEIPT)).unwrap_or_default();
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("invalid cleanup"));
    assert_eq!(report.details[0].reason, "invalidOwnership");
    assert_eq!(
        std::fs::read(root.join(&relative).join(RECEIPT)).unwrap_or_default(),
        before
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[cfg(windows)]
#[test]
fn open_creation_handles_report_access_failure_and_cleanup_can_be_retried() {
    let (paths, root, installed) = runtime_fixture();
    let relative = installed.native_directory().unwrap_or_default();
    let (held, _) =
        super::super::download::ensure_parent(&root, &format!("{relative}/placeholder"))
            .unwrap_or_else(|_| panic!("generation"));
    let observation = capture(&paths, installed.instance_id)
        .unwrap_or_else(|_| panic!("capture"))
        .unwrap_or_else(|| panic!("tracking"))
        .publish()
        .unwrap_or_else(|_| panic!("publish"));
    held.write_atomic_durable("lwjgl.dll", b"native")
        .unwrap_or_else(|_| panic!("runtime"));
    observation
        .finish()
        .unwrap_or_else(|_| panic!("completed execution"));
    retire(&root, &installed);
    let report = cleanup(&paths, installed.instance_id, &Cancellation::default())
        .unwrap_or_else(|_| panic!("retained"));
    assert_eq!(report.details[0].reason, "accessFailure");
    assert!(root.join(&relative).join("lwjgl.dll").exists());
    drop(held);
    assert_eq!(
        cleanup(&paths, installed.instance_id, &Cancellation::default())
            .unwrap_or_else(|_| panic!("retry"))
            .cleaned_generations,
        1
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}
