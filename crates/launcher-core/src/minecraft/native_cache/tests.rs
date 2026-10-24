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
