use super::*;
use crate::modpack::{
    CurseforgeFile, Environments, Hashes,
    gamefs::tests::{expected, game},
};

fn pending(path: &str, body: &[u8]) -> PendingFile {
    PendingFile {
        path: path.into(),
        name: "CrazyPhone".into(),
        project_id: 1646289,
        file_id: 9089413,
        sha1: expected(body).sha1.unwrap_or_default(),
        size: body.len() as u64,
        reason: PendingReason::Manual,
    }
}

fn picked(root: &std::path::Path, name: &str, body: &[u8]) -> std::path::PathBuf {
    let folder = root.join("downloads");
    assert!(std::fs::create_dir_all(&folder).is_ok());
    let path = folder.join(name);
    assert!(std::fs::write(&path, body).is_ok());
    path
}

fn code<T>(result: Result<T, CoreError>) -> Option<ErrorCode> {
    result.err().map(|error| error.code())
}

#[test]
fn pending_files_need_exact_identifiers_and_a_content_folder() {
    let good = pending("mods/crazyphone.jar", b"jar");
    assert!(good.validate().is_ok());
    assert_eq!(
        good.source_url(),
        "https://www.curseforge.com/projects/1646289"
    );
    assert_eq!(good.folder(), "mods");
    assert_eq!(good.file_name(), "crazyphone.jar");
    assert_eq!(good.file_type(), FileType::Mod);
    assert_eq!(
        pending("shaderpacks/a.zip", b"z").file_type(),
        FileType::Shaderpack
    );
    for bad in [
        PendingFile {
            path: "config/a.toml".into(),
            ..good.clone()
        },
        PendingFile {
            path: "mods/sub/a.jar".into(),
            ..good.clone()
        },
        PendingFile {
            path: "mods/../a.jar".into(),
            ..good.clone()
        },
        PendingFile {
            path: "mods/CON.jar".into(),
            ..good.clone()
        },
        PendingFile {
            project_id: 0,
            ..good.clone()
        },
        PendingFile {
            file_id: 0,
            ..good.clone()
        },
        PendingFile {
            sha1: "A".repeat(40),
            ..good.clone()
        },
        PendingFile {
            sha1: "a".repeat(39),
            ..good.clone()
        },
        PendingFile {
            name: String::new(),
            ..good.clone()
        },
        PendingFile {
            size: crate::modpack::MAX_FILE_BYTES + 1,
            ..good.clone()
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
}

#[test]
fn manifest_entries_become_pending_only_when_curseforge_ids_and_sha1_exist() {
    let file = ManifestFile {
        file_type: FileType::Mod,
        source: Origin::Curseforge,
        path: "mods/crazyphone.jar".into(),
        name: "CrazyPhone".into(),
        env: Environments::default(),
        hashes: Hashes {
            sha1: Some("d".repeat(40)),
            sha512: None,
        },
        size: 2_300_000,
        url: None,
        manual: true,
        curseforge: Some(CurseforgeFile {
            project_id: 1646289,
            file_id: 9089413,
        }),
    };
    let manual = PendingFile::from_manifest(&file, PendingReason::Manual);
    assert_eq!(
        manual.map(|p| (p.project_id, p.file_id, p.reason)).ok(),
        Some((1646289, 9089413, PendingReason::Manual))
    );
    let withdrawn = PendingFile::from_manifest(
        &ManifestFile {
            manual: false,
            url: Some("https://edge.forgecdn.net/files/1/2/a.jar".into()),
            ..file.clone()
        },
        PendingReason::Withdrawn,
    );
    assert_eq!(
        withdrawn.map(|p| p.reason).ok(),
        Some(PendingReason::Withdrawn)
    );
    for bad in [
        ManifestFile {
            curseforge: None,
            ..file.clone()
        },
        ManifestFile {
            hashes: Hashes {
                sha1: None,
                sha512: Some("b".repeat(128)),
            },
            ..file.clone()
        },
        ManifestFile {
            source: Origin::Modrinth,
            ..file.clone()
        },
    ] {
        assert!(PendingFile::from_manifest(&bad, PendingReason::Manual).is_err());
    }
}

#[tokio::test]
async fn the_destination_reports_missing_wrong_blocked_and_satisfied() {
    let (root, game) = game();
    let cancel = Cancellation::default();
    let body = b"the right jar";
    let file = pending("mods/crazyphone.jar", body);
    assert_eq!(check(&game, &file, &cancel).ok(), Some(Placement::Missing));
    // The player drops a different version into the folder.
    assert!(std::fs::create_dir_all(root.join("minecraft/mods")).is_ok());
    assert!(std::fs::write(root.join("minecraft/mods/crazyphone.jar"), b"older version").is_ok());
    assert_eq!(
        check(&game, &file, &cancel).ok(),
        Some(Placement::WrongVersion)
    );
    // The same file name with the exact bytes satisfies it without any import.
    assert!(std::fs::write(root.join("minecraft/mods/crazyphone.jar"), body).is_ok());
    assert_eq!(
        check(&game, &file, &cancel).ok(),
        Some(Placement::Satisfied)
    );
    let blocked = pending("mods/blocked.jar", body);
    assert!(std::fs::create_dir_all(root.join("minecraft/mods/blocked.jar")).is_ok());
    assert_eq!(
        check(&game, &blocked, &cancel).ok(),
        Some(Placement::Blocked)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn a_right_file_is_verified_cached_and_placed_without_network_or_session() {
    let (root, game) = game();
    let paths = AppPaths::new(root.clone()).unwrap_or_else(|_| panic!("paths"));
    let cancel = Cancellation::default();
    let body = b"the right jar for this pack".to_vec();
    let file = pending("mods/crazyphone.jar", &body);
    let source = picked(&root, "crazyphone-1.8.0-beta.5.jar", &body);
    assert!(import(&paths, &game, &file, &source, &cancel).await.is_ok());
    assert_eq!(
        std::fs::read(root.join("minecraft/mods/crazyphone.jar")).unwrap_or_default(),
        body
    );
    assert_eq!(
        check(&game, &file, &cancel).ok(),
        Some(Placement::Satisfied)
    );
    // The picked file is untouched and a verified cache entry exists for reuse.
    assert_eq!(std::fs::read(&source).unwrap_or_default(), body);
    assert!(root.join("content/sha1").is_dir());
    // Importing again is a no-op, not a conflict.
    assert!(import(&paths, &game, &file, &source, &cancel).await.is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn wrong_files_are_rejected_and_nothing_is_placed() {
    let (root, game) = game();
    let paths = AppPaths::new(root.clone()).unwrap_or_else(|_| panic!("paths"));
    let cancel = Cancellation::default();
    let body = b"the right jar for this pack".to_vec();
    let file = pending("mods/crazyphone.jar", &body);
    // Same size, different bytes: only the hash can tell them apart.
    let imposter = vec![b'x'; body.len()];
    let wrong_hash = picked(&root, "imposter.jar", &imposter);
    assert_eq!(
        code(import(&paths, &game, &file, &wrong_hash, &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    let wrong_size = picked(&root, "short.jar", b"short");
    assert_eq!(
        code(import(&paths, &game, &file, &wrong_size, &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    let mut longer = body.clone();
    longer.push(0);
    let too_long = picked(&root, "long.jar", &longer);
    assert_eq!(
        code(import(&paths, &game, &file, &too_long, &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert_eq!(
        game.entry("mods/crazyphone.jar").ok(),
        Some(crate::modpack::gamefs::Entry::Absent)
    );
    // A missing or non-file selection is invalid data, not a panic.
    assert!(
        import(&paths, &game, &file, &root.join("missing.jar"), &cancel)
            .await
            .is_err()
    );
    assert!(
        import(&paths, &game, &file, &root.join("downloads"), &cancel)
            .await
            .is_err()
    );
    // After the rejections the right file still imports.
    let right = picked(&root, "right.jar", &body);
    assert!(import(&paths, &game, &file, &right, &cancel).await.is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn an_existing_different_file_is_reported_never_replaced() {
    let (root, game) = game();
    let paths = AppPaths::new(root.clone()).unwrap_or_else(|_| panic!("paths"));
    let cancel = Cancellation::default();
    let body = b"the right jar for this pack".to_vec();
    let file = pending("mods/crazyphone.jar", &body);
    assert!(std::fs::create_dir_all(root.join("minecraft/mods")).is_ok());
    assert!(std::fs::write(root.join("minecraft/mods/crazyphone.jar"), b"older version").is_ok());
    let right = picked(&root, "right.jar", &body);
    assert_eq!(
        code(import(&paths, &game, &file, &right, &cancel).await),
        Some(ErrorCode::PackConflict)
    );
    assert_eq!(
        std::fs::read(root.join("minecraft/mods/crazyphone.jar")).unwrap_or_default(),
        b"older version"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn cancellation_stops_an_import_without_publishing() {
    let (root, game) = game();
    let paths = AppPaths::new(root.clone()).unwrap_or_else(|_| panic!("paths"));
    let cancel = Cancellation::default();
    let body = b"the right jar for this pack".to_vec();
    let file = pending("mods/crazyphone.jar", &body);
    let right = picked(&root, "right.jar", &body);
    cancel.cancel();
    assert_eq!(
        code(import(&paths, &game, &file, &right, &cancel).await),
        Some(ErrorCode::OperationCancelled)
    );
    assert_eq!(
        game.entry("mods/crazyphone.jar").ok(),
        Some(crate::modpack::gamefs::Entry::Absent)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[tokio::test]
async fn a_linked_selection_is_rejected() {
    let (root, game) = game();
    let paths = AppPaths::new(root.clone()).unwrap_or_else(|_| panic!("paths"));
    let body = b"the right jar for this pack".to_vec();
    let file = pending("mods/crazyphone.jar", &body);
    let real = picked(&root, "real.jar", &body);
    let link = root.join("downloads/link.jar");
    assert!(std::os::unix::fs::symlink(&real, &link).is_ok());
    assert!(
        import(&paths, &game, &file, &link, &Cancellation::default())
            .await
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn state_with(
    pending: Vec<PendingFile>,
    files: Vec<crate::modpack::state::ManagedFile>,
) -> crate::modpack::state::PackState {
    use crate::modpack::state::*;
    PackState {
        schema: 1,
        id: uuid::Uuid::nil(),
        source: PackIdentity::Custom {
            version_id: uuid::Uuid::new_v4(),
            published_at: "2026-10-25T20:54:25Z".into(),
            label: None,
        },
        minecraft_version: "1.21.1".into(),
        loader: "neoforge".into(),
        loader_version: "21.1.264".into(),
        files,
        consumed_defaults: Default::default(),
        skipped_configuration: Vec::new(),
        pending,
    }
}

#[test]
fn the_pending_list_is_part_of_the_validated_state() {
    use crate::modpack::state::*;
    let id = uuid::Uuid::new_v4();
    let valid = |mut state: PackState| {
        state.id = id;
        state.validate(id).is_ok()
    };
    let hashes = Hashes {
        sha1: Some("a".repeat(40)),
        sha512: Some("b".repeat(128)),
    };
    let installed = ManagedFile {
        path: "mods/other.jar".into(),
        ownership: Ownership::Content,
        desired: hashes.clone(),
        size: 5,
        baseline: Some(hashes),
    };
    let one = pending("mods/crazyphone.jar", b"a");
    assert!(valid(state_with(
        vec![one.clone()],
        vec![installed.clone()]
    )));
    // A file cannot be both installed and still pending.
    let same = pending("mods/OTHER.jar", b"a");
    assert!(!valid(state_with(vec![same], vec![installed.clone()])));
    // Two pending entries cannot share a destination, even by case.
    let duplicate = PendingFile {
        path: "mods/CRAZYPHONE.jar".into(),
        ..one.clone()
    };
    assert!(!valid(state_with(
        vec![one.clone(), duplicate],
        vec![installed]
    )));
    assert!(!valid(state_with(
        vec![PendingFile {
            project_id: 0,
            ..one.clone()
        }],
        Vec::new()
    )));
    // The pending list survives a round trip and an empty list is not written.
    let state = {
        let mut s = state_with(vec![one], Vec::new());
        s.id = id;
        s
    };
    let bytes = serde_json::to_vec(&state).unwrap_or_default();
    assert_eq!(PackState::decode(&bytes, id).ok(), Some(state.clone()));
    let mut empty = state;
    empty.pending.clear();
    assert!(
        !String::from_utf8_lossy(&serde_json::to_vec(&empty).unwrap_or_default())
            .contains("pending")
    );
}
