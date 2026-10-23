use super::*;
use crate::minecraft::verification_cache::VerificationCache;
use std::path::PathBuf;

struct Fixture {
    paths: AppPaths,
    id: Uuid,
    root: PathBuf,
    installed: InstalledLock,
}

impl Fixture {
    fn new() -> Self {
        let (paths, accounts, id) = crate::minecraft::play::tests::fixture();
        drop(accounts);
        crate::minecraft::play::tests::complete_installation(&paths, id);
        let root = metadata_root(&paths, id).unwrap_or_else(|error| panic!("root: {error}"));
        let installed = read_json(&root, "installed.json")
            .unwrap_or_else(|error| panic!("manifest: {error}"))
            .unwrap_or_else(|| panic!("missing manifest"));
        Self {
            paths,
            id,
            root,
            installed,
        }
    }

    fn audit(&self) -> VerificationCache {
        let _lease = crate::minecraft::process::LaunchLease::acquire(&self.paths, self.id)
            .unwrap_or_else(|error| panic!("lease: {error}"));
        let mut cache = VerificationCache::load(&self.root, &self.installed)
            .unwrap_or_else(|error| panic!("cache: {error}"));
        verify_manifest_inner(
            &self.paths,
            self.id,
            &self.root,
            &self.installed,
            &Cancellation::default(),
            Some(&mut cache),
        )
        .unwrap_or_else(|error| panic!("audit: {error}"));
        cache.save(&self.root);
        cache
    }

    fn client(&self) -> PathBuf {
        self.paths
            .root()
            .join(&self.installed.plan.artifacts[1].destination)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(std::fs::remove_dir_all(self.paths.root()).is_ok());
    }
}

#[test]
fn play_receipts_survive_reload_and_rehash_only_changed_content() {
    let fixture = Fixture::new();
    let first = fixture.audit();
    assert_eq!(first.reused, 0);
    assert!(first.hashed >= 3);
    let second = fixture.audit();
    assert_eq!(second.hashed, 0);
    assert_eq!(second.reused, first.hashed);

    // A timestamp-only change must rehash, then refresh the receipt without repair.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(fixture.client())
        .unwrap_or_else(|_| panic!("client"));
    let later = std::time::SystemTime::now() + Duration::from_secs(60);
    file.set_times(std::fs::FileTimes::new().set_modified(later))
        .unwrap_or_else(|_| panic!("timestamp"));
    drop(file);
    let touched = fixture.audit();
    assert_eq!(touched.hashed, 1);
    assert_eq!(touched.reused, first.hashed - 1);
    assert_eq!(fixture.audit().hashed, 0);
}

#[test]
fn cached_play_rejects_changed_and_missing_files_without_publishing_or_repairing() {
    let fixture = Fixture::new();
    fixture.audit();
    let before = std::fs::read(fixture.root.join("verification-cache.json"))
        .unwrap_or_else(|_| panic!("receipt"));
    let original = std::fs::read(fixture.client()).unwrap_or_else(|_| panic!("client"));
    let replacement = vec![b'x'; original.len()];
    let artifact = &fixture.installed.plan.artifacts[1];
    let (directory, leaf) = ensure_parent(fixture.paths.root(), &artifact.destination)
        .unwrap_or_else(|_| panic!("directory"));
    directory
        .write_atomic_durable(&leaf, &replacement)
        .unwrap_or_else(|_| panic!("change"));
    let lease = crate::minecraft::process::LaunchLease::acquire(&fixture.paths, fixture.id)
        .unwrap_or_else(|_| panic!("lease"));
    assert_eq!(
        lease.verify_installation().err().map(|error| error.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert_eq!(
        std::fs::read(fixture.client()).unwrap_or_default(),
        replacement
    );
    std::fs::remove_file(fixture.client()).unwrap_or_else(|_| panic!("remove fixture file"));
    assert_eq!(
        lease.verify_installation().err().map(|error| error.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert_eq!(
        std::fs::read(fixture.root.join("verification-cache.json")).unwrap_or_default(),
        before
    );
}

#[test]
fn invalid_receipts_and_new_installation_generations_require_fresh_checks() {
    let mut fixture = Fixture::new();
    fixture.audit();
    for invalid in [b"{broken".as_slice(), br#"{"schema_version":999}"#] {
        private_fs::PrivateDir::open(&fixture.root)
            .unwrap_or_else(|_| panic!("root"))
            .write_atomic_durable("verification-cache.json", invalid)
            .unwrap_or_else(|_| panic!("invalid receipt"));
        assert!(fixture.audit().hashed >= 3);
    }
    let mut unsupported: serde_json::Value = read_json(&fixture.root, "verification-cache.json")
        .unwrap_or_else(|_| panic!("receipt"))
        .unwrap_or_else(|| panic!("missing receipt"));
    unsupported["schema_version"] = 999.into();
    write_json(&fixture.root, "verification-cache.json", &unsupported)
        .unwrap_or_else(|_| panic!("unsupported cache"));
    assert!(fixture.audit().hashed >= 3);
    fixture.installed.operation = Uuid::new_v4();
    assert!(fixture.audit().hashed >= 3);
}

#[test]
fn cached_native_generation_rechecks_changes_and_preserves_other_generations() {
    use sha1::{Digest, Sha1};
    let mut fixture = Fixture::new();
    fixture.installed.native_directory = Some(format!("natives/{}", fixture.installed.operation));
    let relative = format!(
        "{}/owned.dll",
        fixture.installed.native_directory().unwrap_or_default()
    );
    let (directory, leaf) =
        ensure_parent(&fixture.root, &relative).unwrap_or_else(|_| panic!("native directory"));
    directory
        .write_atomic_durable(&leaf, b"native")
        .unwrap_or_else(|_| panic!("native"));
    fixture.installed.native_files.push(NativeFile {
        path: "owned.dll".into(),
        size: 6,
        sha1: format!("{:x}", Sha1::digest(b"native")),
    });
    write_json(&fixture.root, "installed.json", &fixture.installed)
        .unwrap_or_else(|_| panic!("native manifest"));
    let (old_directory, old_leaf) = ensure_parent(&fixture.root, "natives-legacy/keep.dll")
        .unwrap_or_else(|_| panic!("old native directory"));
    old_directory
        .write_atomic_durable(old_leaf, b"preserve")
        .unwrap_or_else(|_| panic!("old native"));
    let first = fixture.audit();
    assert_eq!(fixture.audit().reused, first.hashed);
    directory
        .write_atomic_durable(leaf, b"broken")
        .unwrap_or_else(|_| panic!("changed native"));
    let lease = crate::minecraft::process::LaunchLease::acquire(&fixture.paths, fixture.id)
        .unwrap_or_else(|_| panic!("lease"));
    assert_eq!(
        lease.verify_installation().err().map(|error| error.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert_eq!(
        std::fs::read(fixture.root.join("natives-legacy/keep.dll")).unwrap_or_default(),
        b"preserve"
    );
}

#[test]
fn redirected_cache_receipt_is_rejected_instead_of_trusted_or_overwritten() {
    let fixture = Fixture::new();
    fixture.audit();
    let cache = fixture.root.join("verification-cache.json");
    let alias = fixture.root.join("receipt-alias.json");
    std::fs::hard_link(&cache, &alias).unwrap_or_else(|_| panic!("fixture receipt alias"));
    assert!(VerificationCache::load(&fixture.root, &fixture.installed).is_err());
    assert_eq!(
        std::fs::read(cache).unwrap_or_default(),
        std::fs::read(&alias).unwrap_or_default()
    );
    std::fs::remove_file(alias).unwrap_or_else(|_| panic!("remove fixture alias"));
}

#[test]
fn cached_hits_still_reject_hard_links_and_cancellation_publishes_nothing() {
    let fixture = Fixture::new();
    let lease = crate::minecraft::process::LaunchLease::acquire(&fixture.paths, fixture.id)
        .unwrap_or_else(|_| panic!("lease"));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        lease
            .verify_installation_with_cancel(&cancel)
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::OperationCancelled)
    );
    assert!(!fixture.root.join("verification-cache.json").exists());
    drop(lease);
    fixture.audit();
    let alias = fixture.root.join("client-alias.jar");
    std::fs::hard_link(fixture.client(), &alias).unwrap_or_else(|_| panic!("fixture hard link"));
    let lease = crate::minecraft::process::LaunchLease::acquire(&fixture.paths, fixture.id)
        .unwrap_or_else(|_| panic!("lease"));
    assert!(lease.verify_installation().is_err());
    std::fs::remove_file(alias).unwrap_or_else(|_| panic!("remove fixture alias"));
}

#[cfg(windows)]
#[test]
fn explicit_full_audit_detects_corruption_even_when_cached_metadata_is_preserved() {
    use std::io::Write;
    let fixture = Fixture::new();
    fixture.audit();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(fixture.client())
        .unwrap_or_else(|_| panic!("client"));
    let modified = file
        .metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or_else(|_| panic!("metadata"));
    file.write_all(b"X").unwrap_or_else(|_| panic!("change"));
    file.sync_all().unwrap_or_else(|_| panic!("sync"));
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap_or_else(|_| panic!("restore timestamp"));
    drop(file);
    // Document the deliberate cache-trust boundary, then prove full audit bypasses it.
    assert_eq!(fixture.audit().hashed, 0);
    assert_eq!(
        verify_installed(&fixture.paths, fixture.id)
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
}
