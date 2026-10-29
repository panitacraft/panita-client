use super::*;
use rustix::io::{FdFlags, fcntl_getfd};
use std::os::unix::{
    ffi::OsStrExt,
    fs::{PermissionsExt, symlink},
};

fn each(mut test: impl FnMut(PrivateDir, &Path)) {
    for resolver in [
        Resolver::Auto,
        Resolver::OpenAt2,
        Resolver::Walk,
        Resolver::Unavailable,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let dir = PrivateDir::acquire(&root, true, resolver).unwrap();
        test(dir, &root);
    }
}
fn chmod(p: &Path, mode: u32) {
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn relative_directory_open_preserves_missing_insecure_and_redirected_entries() {
    each(|dir, root| {
        let child = dir.create_dir("child").unwrap();
        child
            .create("proof")
            .unwrap()
            .write_all(b"retained")
            .unwrap();
        dir.open_dir("child").unwrap().open_file("proof").unwrap();
        assert!(dir.open_dir("missing").is_err());
        assert!(!root.join("missing").exists());
        chmod(&root.join("child"), 0o755);
        dir.open_dir("child").unwrap();
        assert_eq!(
            std::fs::metadata(root.join("child"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
        chmod(&root.join("child"), 0o700);
        symlink(root.join("child"), root.join("redirect")).unwrap();
        assert!(dir.open_dir("redirect").is_err());
        assert!(root.join("redirect").is_symlink());
        assert!(dir.open_dir("../child").is_err());
        let renamed = root.with_file_name("renamed-private");
        std::fs::rename(root, &renamed).unwrap();
        dir.open_dir("child").unwrap().open_file("proof").unwrap();
    });
}

#[test]
fn proc_publication_preserves_inode_permissions_and_renamed_parent() {
    each(|dir, root| {
        let mut temp = dir
            .anonymous_temp()
            .unwrap()
            .expect("O_TMPFILE test filesystem");
        temp.write_all(b"complete private contents").unwrap();
        let renamed = root.with_file_name("renamed-private");
        std::fs::rename(root, &renamed).unwrap();
        dir.link_temp_via_proc(&temp, OsStr::new("linked")).unwrap();
        temp.links = 1;
        let mut linked = dir.open_file("linked").unwrap();
        assert!(linked.same_object(&temp));
        linked.verify().unwrap();
        let mut contents = Vec::new();
        linked.read_to_end(&mut contents).unwrap();
        assert_eq!(contents, b"complete private contents");
        assert!(!root.exists());
        assert_eq!(
            std::fs::metadata(renamed.join("linked"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o600
        );
    });
}

#[test]
fn proc_publication_preserves_existing_file_and_redirected_destination() {
    each(|dir, root| {
        let temp = dir
            .anonymous_temp()
            .unwrap()
            .expect("O_TMPFILE test filesystem");
        dir.create("existing").unwrap().write_all(b"keep").unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside");
        std::fs::write(&outside_file, b"outside contents").unwrap();
        symlink(&outside_file, root.join("redirect")).unwrap();
        for name in ["existing", "redirect"] {
            let error = dir.link_temp_via_proc(&temp, OsStr::new(name)).unwrap_err();
            assert!(
                matches!(error, Error::Io(error) if error.kind() == io::ErrorKind::AlreadyExists)
            );
            temp.verify().unwrap();
        }
        assert_eq!(std::fs::read(root.join("existing")).unwrap(), b"keep");
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside contents");
        assert!(root.join("redirect").is_symlink());
    });
}

#[test]
fn proc_publication_rejects_loosened_private_temp_but_accepts_readonly_public_parent() {
    each(|dir, root| {
        let temp = dir
            .anonymous_temp()
            .unwrap()
            .expect("O_TMPFILE test filesystem");
        fs::fchmod(&temp.file, Mode::from_raw_mode(0o644)).unwrap();
        assert!(matches!(
            dir.link_temp_via_proc(&temp, OsStr::new("unsafe-temp")),
            Err(Error::InsecurePermissions { .. })
        ));
        assert!(!root.join("unsafe-temp").exists());
        fs::fchmod(&temp.file, Mode::from_raw_mode(0o600)).unwrap();
        chmod(root, 0o755);
        dir.verify_private_namespace().unwrap();
        dir.link_temp_via_proc(&temp, OsStr::new("safe-private-file"))
            .unwrap();
        // The synthetic publication added a link to the still-anonymous handle.
        let mut temp = temp;
        temp.links = 1;
        dir.open_file("safe-private-file")
            .unwrap()
            .verify()
            .unwrap();
        chmod(root, 0o700);
        temp.verify().unwrap();
    });
}

#[test]
fn symlink_and_directory_replacement_stress() {
    each(|dir, root| {
        let outside = tempfile::tempdir().unwrap();
        let outside_root = outside.path().join("private");
        let outside_dir = PrivateDir::ensure(&outside_root).unwrap();
        outside_dir
            .create("data")
            .unwrap()
            .write_all(b"outside")
            .unwrap();
        dir.create_dir("slot").unwrap();
        dir.create("slot/data")
            .unwrap()
            .write_all(b"inside")
            .unwrap();
        symlink(&outside_root, root.join("bad")).unwrap();
        crate::stress::redirect(&dir, root, &outside_root);
    });
}

#[test]
fn data_directory_creation_accepts_accessible_parents_while_namespace_checks_stay_strict() {
    let temp = tempfile::tempdir().unwrap();
    chmod(temp.path(), 0o777);
    let root = temp.path().join("private");
    PrivateDir::ensure_data(&root).unwrap();
    let parent = PrivateDir::open_data(temp.path()).unwrap();
    assert!(parent.verify_private_namespace().is_err());
    assert_eq!(
        std::fs::metadata(temp.path()).unwrap().permissions().mode() & 0o7777,
        0o777
    );
    chmod(temp.path(), 0o1777);
    PrivateDir::ensure(&root).unwrap();
    chmod(temp.path(), 0o777);
    // Opening an already-private root does not require repairing its ancestors.
    PrivateDir::ensure(&root).unwrap();
    let mut stat = fs::fstat(std::fs::File::open(temp.path()).unwrap()).unwrap();
    stat.st_uid = rustix::process::geteuid().as_raw().wrapping_add(1).max(1);
    stat.st_mode = 0o1777;
    assert!(matches!(
        creation_parent_stat(&stat),
        Err(Error::UntrustedCreationParent)
    ));
}

#[test]
fn round_trip_and_exclusive_creation() {
    each(|dir, root| {
        let child = dir.create_dir("nested").unwrap();
        let mut file = dir.create("nested/key").unwrap();
        file.write_all(b"secret").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut data = String::new();
        file.read_to_string(&mut data).unwrap();
        assert_eq!(data, "secret");
        assert!(dir.create("nested/key").is_err());
        assert!(dir.create_dir("nested").is_err());
        dir.ensure_dir("nested").unwrap();
        assert_eq!(std::fs::read(root.join("nested/key")).unwrap(), b"secret");
        for fd in [dir.as_fd(), child.as_fd(), file.as_fd()] {
            assert!(fcntl_getfd(fd).unwrap().contains(FdFlags::CLOEXEC));
        }
        assert_eq!(
            std::fs::metadata(root.join("nested/key"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(root.join("nested"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
    });
}
#[test]
fn strict_modes_and_no_implicit_repair() {
    each(|dir, root| {
        dir.create("key").unwrap();
        for mode in [0o644, 0o400, 0o4600, 0o2600, 0o1600] {
            chmod(&root.join("key"), mode);
            assert!(matches!(
                dir.open_file("key"),
                Err(Error::InsecurePermissions { .. })
            ));
            assert_eq!(
                std::fs::metadata(root.join("key"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                mode
            );
        }
        let nested = dir.create_dir("nested").unwrap();
        nested.create("key").unwrap();
        for mode in [0o755, 0o4700, 0o2700, 0o1700] {
            chmod(&root.join("nested"), mode);
            dir.ensure_dir("nested").unwrap();
            dir.open_file("nested/key").unwrap().verify().unwrap();
        }
        chmod(root, 0o755);
        PrivateDir::ensure(root).unwrap();
        dir.verify_private_namespace().unwrap();
        dir.create("other").unwrap().verify().unwrap();
    });
}
#[test]
fn redirects_and_unexpected_kinds() {
    each(|dir, root| {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"outside").unwrap();
        symlink(outside.path(), root.join("link")).unwrap();
        symlink(outside.path().join("secret"), root.join("file-link")).unwrap();
        assert!(matches!(
            dir.open_file("file-link"),
            Err(Error::PathRedirect)
        ));
        assert!(dir.open_file("link/secret").is_err());
        assert!(dir.create("link/new").is_err());
        assert!(!outside.path().join("new").exists());
        assert!(PrivateDir::open(root.join("link")).is_err());
        dir.create_dir("directory").unwrap();
        assert!(matches!(
            dir.open_file("directory"),
            Err(Error::WrongObjectKind)
        ));
        fs::mknodat(&dir, "fifo", FileType::Fifo, Mode::from_raw_mode(0o600), 0).unwrap();
        assert!(matches!(dir.open_file("fifo"), Err(Error::WrongObjectKind)));
        let socket = std::os::unix::net::UnixListener::bind(root.join("socket")).unwrap();
        assert!(dir.open_file("socket").is_err());
        drop(socket);
        symlink("/proc/self/fd/0", root.join("magic")).unwrap();
        assert!(matches!(dir.open_file("magic"), Err(Error::PathRedirect)));
    });
}
#[test]
fn retained_identity_survives_namespace_replacement() {
    each(|dir, root| {
        let mut file = dir.create("key").unwrap();
        file.write_all(b"original").unwrap();
        std::fs::rename(root.join("key"), root.join("old-key")).unwrap();
        let mut other = dir.create("key").unwrap();
        other.write_all(b"replacement").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut s = String::new();
        file.read_to_string(&mut s).unwrap();
        assert_eq!(s, "original");
        let old = root.with_file_name("renamed");
        std::fs::rename(root, &old).unwrap();
        std::fs::create_dir(root).unwrap();
        chmod(root, 0o700);
        dir.create("through-capability").unwrap();
        assert!(old.join("through-capability").exists());
        assert!(!root.join("through-capability").exists());
        let wrong = fs::fstat(&other).unwrap();
        assert!(matches!(
            validate(&file, false, Some(identity(&wrong))),
            Err(Error::IdentityChanged)
        ));
    });
}
#[test]
fn hard_links_are_checked_again_before_io() {
    each(|dir, root| {
        let mut file = dir.create("key").unwrap();
        std::fs::hard_link(root.join("key"), root.join("alias")).unwrap();
        assert!(matches!(
            dir.open_file("key"),
            Err(Error::MultipleHardLinks { links: 2 })
        ));
        assert!(matches!(
            file.verify(),
            Err(Error::MultipleHardLinks { links: 2 })
        ));
        assert!(file.write_all(b"blocked").is_err());
        assert!(file.read(&mut [0]).is_err());
        assert!(std::fs::read(root.join("alias")).unwrap().is_empty());
    });
}
#[test]
fn non_utf8_names_and_invalid_paths() {
    each(|dir, _| {
        let name = OsStr::from_bytes(b"key-\xff");
        dir.create(name).unwrap();
        dir.open_file(name).unwrap();
        for p in ["../escape", "/absolute", "a/../b", "", "a/./b", "C:\\key"] {
            assert!(matches!(dir.create(p), Err(Error::InvalidRelativePath)));
        }
    });
}
#[test]
fn wrong_owner() {
    // chown requires privilege; normal-user CI exercises the metadata policy below.
    let temp = tempfile::tempdir().unwrap();
    let dir = PrivateDir::ensure(temp.path().join("root")).unwrap();
    let file = dir.create("key").unwrap();
    let mut metadata = fs::fstat(&file).unwrap();
    metadata.st_uid = metadata.st_uid.wrapping_add(1);
    assert!(matches!(
        validate_stat(&metadata, false, None),
        Err(Error::WrongOwner { .. })
    ));
    if rustix::process::geteuid().is_root() {
        fs::fchown(&file, Some(rustix::process::Uid::from_raw(12345)), None).unwrap();
        assert!(matches!(file.verify(), Err(Error::WrongOwner { .. })));
        assert!(matches!(file.harden(), Err(Error::WrongOwner { .. })));
    }
}
#[test]
fn restrictive_umask_subprocess() {
    if std::env::var_os("PRIVATE_FS_UMASK_CHILD").is_some() {
        // Only this isolated process changes its umask. The library never does.
        let temp = tempfile::tempdir().unwrap();
        rustix::process::umask(Mode::from_raw_mode(0o200));
        let dir = PrivateDir::ensure(temp.path().join("root")).unwrap();
        let file = dir.create("key").unwrap();
        file.verify().unwrap();
        assert_eq!(
            rustix::process::umask(Mode::from_raw_mode(0o200)),
            Mode::from_raw_mode(0o200)
        );
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "unix::tests::restrictive_umask_subprocess"])
        .env("PRIVATE_FS_UMASK_CHILD", "1")
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn parent_modes_are_preserved_and_missing_parents_fail() {
    each(|dir, root| {
        let parent = root.parent().unwrap();
        chmod(parent, 0o755);
        PrivateDir::ensure(root).unwrap();
        assert_eq!(
            std::fs::metadata(parent).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        assert!(PrivateDir::ensure(parent.join("absent/root")).is_err());
        assert!(!parent.join("absent").exists());
        assert!(dir.create_dir("absent/nested").is_err());
        assert!(dir.open_file("missing").is_err());
        assert!(!root.join("missing").exists());
        let alias = parent.join("alias");
        symlink(root, &alias).unwrap();
        assert!(PrivateDir::ensure(alias.join("new-root")).is_err());
        assert!(!root.join("new-root").exists());
    });
}

#[test]
fn mutable_permissions_are_checked_before_io() {
    each(|dir, root| {
        let mut file = dir.create("key").unwrap();
        chmod(&root.join("key"), 0o644);
        assert!(file.write(b"blocked").is_err());
        assert!(file.read(&mut [0]).is_err());
        assert!(file.seek(SeekFrom::Start(0)).is_err());
        assert!(file.sync_all().is_err());
        assert!(file.sync_data().is_err());
    });
}

#[test]
fn normalization_never_repairs_untrusted_properties() {
    each(|dir, root| {
        let file = dir.create("key").unwrap();
        chmod(&root.join("key"), 0o400);
        let stat = fs::fstat(&file).unwrap();
        let wrong = Identity {
            ino: stat.st_ino.wrapping_add(1),
            dev: stat.st_dev,
        };
        assert!(matches!(
            normalize_created(&file, false, wrong),
            Err(Error::IdentityChanged)
        ));
        assert_eq!(fs::fstat(&file).unwrap().st_mode & 0o7777, 0o400);
        std::fs::hard_link(root.join("key"), root.join("alias")).unwrap();
        assert!(matches!(
            normalize_created(&file, false, identity(&stat)),
            Err(Error::MultipleHardLinks { .. })
        ));
        assert_eq!(fs::fstat(&file).unwrap().st_mode & 0o7777, 0o400);
    });
}

#[test]
fn hardening_is_explicit_and_does_not_repair_hard_links() {
    each(|dir, root| {
        let file = dir.create("key").unwrap();
        chmod(&root.join("key"), 0o644);
        assert!(matches!(
            file.verify(),
            Err(Error::InsecurePermissions { .. })
        ));
        file.harden().unwrap();
        file.verify().unwrap();
        chmod(root, 0o755);
        dir.create("ordinary-parent").unwrap();
        dir.harden().unwrap();
        assert_eq!(
            std::fs::metadata(root).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        dir.create("allowed").unwrap();
        std::fs::hard_link(root.join("key"), root.join("alias")).unwrap();
        chmod(&root.join("key"), 0o644);
        assert!(matches!(
            file.harden(),
            Err(Error::MultipleHardLinks { links: 2 })
        ));
        assert_eq!(
            std::fs::metadata(root.join("alias"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o644
        );
    });
}

#[test]
fn ordinary_data_preserves_modes_and_accepts_foreign_owner_metadata_without_losing_guards() {
    each(|dir, root| {
        chmod(root, 0o755);
        std::fs::write(root.join("pack.zip"), b"pack data").unwrap();
        chmod(&root.join("pack.zip"), 0o644);
        let data = PrivateDir::open_data(root).unwrap();
        data.open_file("pack.zip").unwrap().verify().unwrap();
        assert!(dir.open_file("pack.zip").is_err());
        data.write_atomic_durable("new.dat", b"ordinary bytes")
            .unwrap();
        data.open_file("new.dat").unwrap().verify().unwrap();
        assert_eq!(
            std::fs::metadata(root.join("pack.zip"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o644
        );
        let file = data.open_file("pack.zip").unwrap();
        let mut stat = fs::fstat(&file).unwrap();
        stat.st_uid = stat.st_uid.wrapping_add(1);
        validate_data_stat(&stat, false, None, 1).unwrap();
        assert!(matches!(
            validate_stat(&stat, false, None),
            Err(Error::WrongOwner { .. })
        ));
        std::fs::hard_link(root.join("pack.zip"), root.join("alias")).unwrap();
        assert!(matches!(
            file.verify(),
            Err(Error::MultipleHardLinks { .. })
        ));
        assert!(data.open_file("pack.zip").is_err());
        symlink("new.dat", root.join("redirect")).unwrap();
        assert!(data.open_file("redirect").is_err());
        assert!(data.open_file("../escape").is_err());
        chmod(root, 0o777);
        assert!(data.verify_private_namespace().is_err());
        data.create("allowed-data").unwrap();
    });
}
