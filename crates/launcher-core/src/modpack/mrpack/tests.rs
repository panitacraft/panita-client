use super::*;
use crate::{
    modpack::{
        Origin,
        archive::tests::build,
        content::{ContentKey, Request, ensure},
        transfer::PackDownloader,
    },
    test_http::{Reply, Server},
};
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::Sha512;
use uuid::Uuid;

fn root() -> AppPaths {
    AppPaths::new(std::env::temp_dir().join(format!("panita-mrpack-{}", Uuid::new_v4())))
        .unwrap_or_else(|_| panic!("root"))
}

fn document(files: Vec<Value>) -> Vec<u8> {
    json!({"formatVersion": 1, "game": "minecraft", "versionId": "1.0", "name": "Fixture",
        "files": files, "dependencies": {"minecraft": "1.21.1", "neoforge": "21.1.248"}})
    .to_string()
    .into_bytes()
}

fn listed(path: &str) -> Value {
    json!({"path": path, "env": {"client": "required", "server": "required"},
        "hashes": {"sha1": "a".repeat(40), "sha512": "b".repeat(128)},
        "downloads": ["https://cdn.modrinth.com/data/p/versions/v/file.jar"], "fileSize": 10})
}

fn pack(files: Vec<Value>, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let index = document(files);
    let mut all: Vec<(&str, &[u8])> = vec![("modrinth.index.json", &index)];
    all.extend_from_slice(entries);
    build(&all)
}

fn expected(bytes: &[u8]) -> Expected {
    Expected {
        size: bytes.len() as u64,
        sha1: Some(format!("{:x}", Sha1::digest(bytes))),
        sha256: None,
        sha512: Some(format!("{:x}", Sha512::digest(bytes))),
    }
}

/// Put the archive in the shared cache the way an installation does.
async fn cached(paths: &AppPaths, bytes: &[u8]) -> Expected {
    let expected = expected(bytes);
    let body = bytes.to_vec();
    let server = Server::start(move |_| Reply::ok(body.clone()));
    let downloader = PackDownloader::for_tests(server.base());
    let request = Request {
        url: "https://cdn.modrinth.com/data/p/versions/v/pack.mrpack".into(),
        origin: Origin::Modrinth,
        expected: expected.clone(),
        fallbacks: Vec::new(),
    };
    let result = ensure(
        paths,
        &downloader,
        &request,
        &Cancellation::default(),
        &|_| {},
    )
    .await;
    assert!(result.is_ok());
    expected
}

#[test]
fn mod_jars_are_placeable_only_directly_under_mods_and_executables_never() {
    for allowed in [
        "mods/a.jar",
        "MODS/A.JAR",
        "config/a.toml",
        "options.txt",
        "mods/readme",
        "mods/nested/readme.txt",
        "resourcepacks/pack.zip",
        "shaderpacks/shader.zip",
        "kubejs/startup/script.js",
    ] {
        assert!(placeable(allowed), "{allowed}");
    }
    for refused in [
        "mods/nested/a.jar",
        "config/a.jar",
        "a.jar",
        "tools/run.sh",
        "tools/run.SH",
        "bin/helper.exe",
        "bin/native.dll",
        "bin/lib.so",
        "bin/lib.dylib",
        "setup.bat",
        "setup.cmd",
        "setup.ps1",
        "setup.msi",
    ] {
        assert!(!placeable(refused), "{refused}");
    }
    assert!(is_content("mods/a.jar") && is_content("ResourcePacks/p.zip"));
    assert!(is_content("shaderpacks/s.zip"));
    assert!(!is_content("config/a.toml") && !is_content("options.txt"));
}

#[tokio::test]
async fn verified_overrides_apply_common_then_client_and_skip_server_content() {
    let paths = root();
    let bytes = pack(
        vec![listed("mods/listed.jar")],
        &[
            ("overrides/options.txt", b"fov:90"),
            ("overrides/config/a.toml", b"common = 1\n"),
            ("client-overrides/config/a.toml", b"client = 1\n"),
            ("overrides/mods/bundled.jar", b"jar"),
            ("server-overrides/config/server.toml", b"server\n"),
            ("server-overrides/start.sh", b"#!/bin/sh"),
            ("README.txt", b"about this pack"),
        ],
    );
    let expected = cached(&paths, &bytes).await;
    let cancel = Cancellation::default();
    let inspected = inspect(&paths, &expected, &cancel).unwrap_or_else(|_| panic!("inspect"));
    assert_eq!(inspected.index.files.len(), 1);
    assert_eq!(inspected.overrides, 3);

    let mut opened = layer(
        &paths,
        &expected,
        &inspected.index,
        &inspected.index_sha256,
        &cancel,
    )
    .unwrap_or_else(|_| panic!("layer"));
    let names: Vec<_> = opened.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(names, ["config/a.toml", "mods/bundled.jar", "options.txt"]);
    // The client layer wins for the same destination.
    let entry = opened.entries[0].clone();
    assert!(entry.source.starts_with("client-overrides/"));
    let mut budget = opened.budget();
    let mut body = Vec::new();
    assert!(
        opened
            .stream(&entry, &mut budget, &cancel, Some(&mut body))
            .is_ok()
    );
    assert_eq!(body, b"client = 1\n");
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn hostile_or_conflicting_archives_are_rejected_before_anything_is_written() {
    let paths = root();
    let cancel = Cancellation::default();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "traversal",
            pack(vec![], &[("overrides/../evil.txt", b"x")]),
        ),
        (
            "world data",
            pack(vec![], &[("overrides/saves/world/level.dat", b"x")]),
        ),
        (
            "launcher metadata",
            pack(vec![], &[("overrides/instance.json", b"{}")]),
        ),
        (
            "hidden folder",
            pack(vec![], &[("overrides/.git/config", b"x")]),
        ),
        ("script", pack(vec![], &[("overrides/tools/run.sh", b"x")])),
        (
            "native library",
            pack(vec![], &[("client-overrides/config/x.dll", b"x")]),
        ),
        (
            "jar outside mods",
            pack(vec![], &[("overrides/config/x.jar", b"x")]),
        ),
        (
            "nested mod jar",
            pack(vec![], &[("overrides/mods/sub/x.jar", b"x")]),
        ),
        (
            "override of a downloaded file",
            pack(
                vec![listed("mods/listed.jar")],
                &[("overrides/mods/listed.jar", b"x")],
            ),
        ),
        (
            "case-different override of a downloaded file",
            pack(
                vec![listed("mods/listed.jar")],
                &[("client-overrides/Mods/Listed.jar", b"x")],
            ),
        ),
        (
            "file and folder across layers",
            pack(
                vec![],
                &[
                    ("overrides/config", b"x"),
                    ("client-overrides/config/a.toml", b"x"),
                ],
            ),
        ),
        (
            "file and folder in one layer",
            pack(
                vec![],
                &[
                    ("overrides/config", b"x"),
                    ("overrides/config/a.toml", b"x"),
                ],
            ),
        ),
        (
            "override under a downloaded file",
            pack(
                vec![listed("config/a")],
                &[("overrides/config/a/b.toml", b"x")],
            ),
        ),
        ("missing index", build(&[("overrides/options.txt", b"x")])),
        (
            "index folder",
            build(&[
                ("modrinth.index.json/", b""),
                ("overrides/options.txt", b"x"),
            ]),
        ),
        (
            "downloaded executable",
            pack(vec![listed("tools/run.sh")], &[]),
        ),
    ];
    for (name, bytes) in cases {
        let expected = cached(&paths, &bytes).await;
        assert!(
            inspect(&paths, &expected, &cancel).is_err(),
            "{name} was accepted"
        );
    }
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn a_tampered_cache_entry_never_reaches_the_index() {
    let paths = root();
    let cancel = Cancellation::default();
    let bytes = pack(vec![], &[("overrides/options.txt", b"fov:90")]);
    let expected = cached(&paths, &bytes).await;
    assert!(inspect(&paths, &expected, &cancel).is_ok());

    let key = ContentKey::for_expected(&expected).unwrap_or_else(|_| panic!("key"));
    let path = paths.root().join(key.relative());
    let mut damaged = bytes.clone();
    damaged[10] ^= 0xff;
    assert!(std::fs::write(&path, &damaged).is_ok());
    assert_eq!(
        inspect(&paths, &expected, &cancel).err().map(|e| e.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    // A shorter file of the right name is rejected too.
    assert!(std::fs::write(&path, &bytes[..bytes.len() / 2]).is_ok());
    assert_eq!(
        inspect(&paths, &expected, &cancel).err().map(|e| e.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn reopening_the_layers_requires_the_same_index() {
    let paths = root();
    let cancel = Cancellation::default();
    let bytes = pack(
        vec![listed("mods/listed.jar")],
        &[("overrides/options.txt", b"x")],
    );
    let expected = cached(&paths, &bytes).await;
    let inspected = inspect(&paths, &expected, &cancel).unwrap_or_else(|_| panic!("inspect"));
    assert!(
        layer(
            &paths,
            &expected,
            &inspected.index,
            &inspected.index_sha256,
            &cancel
        )
        .is_ok()
    );

    let mut other = inspected.index.clone();
    other.name = "Another pack".into();
    assert_eq!(
        layer(&paths, &expected, &other, &inspected.index_sha256, &cancel)
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert_eq!(
        layer(
            &paths,
            &expected,
            &inspected.index,
            &"0".repeat(64),
            &cancel
        )
        .err()
        .map(|e| e.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    let _ = std::fs::remove_dir_all(paths.root());
}
