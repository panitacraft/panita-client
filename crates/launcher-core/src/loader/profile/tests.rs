use super::*;
use crate::{
    minecraft::{
        install::{InstalledLock, metadata_root, read_json},
        play::tests::{complete_installation, fixture},
    },
    test_http::{Reply, Server},
};
use serde_json::{Value, json};

const FABRIC: &str = "https://maven.fabricmc.net/";
const QUILT: &str = "https://maven.quiltmc.org/repository/release/";

fn java() -> JavaRequirement {
    JavaRequirement {
        component: "java-runtime-delta".into(),
        major_version: 21,
    }
}

/// Library name, its Maven path and its bytes.
fn libraries() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    vec![
        (
            "net.fabricmc:sponge-mixin:0.15.5+mixin.0.8.7",
            "net/fabricmc/sponge-mixin/0.15.5+mixin.0.8.7/sponge-mixin-0.15.5+mixin.0.8.7.jar",
            b"mixin jar".to_vec(),
        ),
        (
            "org.ow2.asm:asm:9.8",
            "org/ow2/asm/asm/9.8/asm-9.8.jar",
            b"asm jar".to_vec(),
        ),
        (
            "net.fabricmc:fabric-loader:0.16.14",
            "net/fabricmc/fabric-loader/0.16.14/fabric-loader-0.16.14.jar",
            b"loader jar".to_vec(),
        ),
    ]
}

fn entry(name: &str, root: &str, body: &[u8], hashed: bool) -> Value {
    let mut library = json!({"name": name, "url": root});
    if hashed {
        library["sha1"] = json!(format!("{:x}", Sha1::digest(body)));
        library["sha256"] = json!(format!("{:x}", Sha256::digest(body)));
        library["sha512"] = json!(format!("{:x}", Sha512::digest(body)));
        library["size"] = json!(body.len());
    }
    library
}

fn profile(libraries: Vec<Value>) -> Value {
    json!({"id": "fabric-loader-0.16.14-1.21.1", "inheritsFrom": "1.21.1",
        "releaseTime": "2026-10-27T10:29:50+0000", "type": "release",
        "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
        "arguments": {"game": [], "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main "]},
        "libraries": libraries})
}

fn good() -> Value {
    profile(
        libraries()
            .iter()
            .map(|(name, _, body)| entry(name, FABRIC, body, true))
            .collect(),
    )
}

fn decoded(value: &Value) -> Result<(Launch, Vec<Wanted>), CoreError> {
    decode(
        value.to_string().as_bytes(),
        Flavor::Fabric,
        "1.21.1",
        "0.16.14",
    )
}

fn code<T>(result: Result<T, CoreError>) -> Option<ErrorCode> {
    result.err().map(|error| error.code())
}

#[test]
fn a_profile_is_bound_to_the_requested_versions_and_approved_repositories() {
    let (launch, wanted) = decoded(&good()).unwrap_or_else(|_| panic!("profile"));
    assert_eq!(launch.id, "fabric-loader-0.16.14-1.21.1");
    assert_eq!(
        launch.main_class,
        "net.fabricmc.loader.impl.launch.knot.KnotClient"
    );
    assert_eq!(
        launch.jvm,
        ["-DFabricMcEmu= net.minecraft.client.main.Main "]
    );
    assert_eq!(wanted.len(), 3);
    assert_eq!(
        wanted[1].url,
        "https://maven.fabricmc.net/org/ow2/asm/asm/9.8/asm-9.8.jar"
    );

    // Quilt names no hashes at all and may use its own repository.
    let quilt = json!({"id": "quilt-loader-0.27.1-1.21.1", "inheritsFrom": "1.21.1",
        "type": "release", "mainClass": "org.quiltmc.loader.impl.launch.knot.KnotClient",
        "arguments": {"game": []},
        "libraries": [entry("org.quiltmc:quilt-loader:0.27.1", QUILT, b"x", false),
            entry("org.ow2.asm:asm:9.7.1", FABRIC, b"x", false)]});
    let (launch, wanted) = decode(
        quilt.to_string().as_bytes(),
        Flavor::Quilt,
        "1.21.1",
        "0.27.1",
    )
    .unwrap_or_else(|_| panic!("quilt"));
    assert!(launch.jvm.is_empty() && wanted.iter().all(|w| w.sha1.is_none()));

    let mutate = |change: &dyn Fn(&mut Value)| {
        let mut value = good();
        change(&mut value);
        value
    };
    let hostile = [
        mutate(&|v| v["id"] = json!("fabric-loader-0.99.0-1.21.1")),
        mutate(&|v| v["inheritsFrom"] = json!("1.20.1")),
        mutate(&|v| v["mainClass"] = json!("not a class name")),
        mutate(&|v| v["mainClass"] = json!("")),
        mutate(&|v| v["libraries"] = json!([])),
        mutate(&|v| v["libraries"][0]["url"] = json!("http://maven.fabricmc.net/")),
        mutate(&|v| v["libraries"][0]["url"] = json!("https://evil.example/")),
        mutate(&|v| v["libraries"][0]["url"] = json!("https://maven.fabricmc.net/../")),
        mutate(&|v| v["libraries"][0]["name"] = json!("../../evil:lib:1")),
        mutate(&|v| v["libraries"][0]["name"] = json!("not-a-coordinate")),
        mutate(&|v| v["libraries"][0]["sha1"] = json!("zz")),
        mutate(&|v| v["libraries"][0]["sha256"] = json!("abc")),
        mutate(&|v| v["libraries"][0]["size"] = json!(0)),
        mutate(&|v| v["libraries"][0]["size"] = json!(u64::MAX)),
        mutate(&|v| v["arguments"]["jvm"] = json!([{"rules": [], "value": "x"}])),
        mutate(&|v| v["arguments"]["game"] = json!(["bad\u{0007}argument"])),
        mutate(&|v| {
            let first = v["libraries"][0].clone();
            if let Some(list) = v["libraries"].as_array_mut() {
                list.push(first);
            }
        }),
    ];
    for (number, value) in hostile.iter().enumerate() {
        assert!(decoded(value).is_err(), "case {number} was accepted");
    }
    assert!(
        decode(
            &vec![b' '; MAX_METADATA_BYTES + 1],
            Flavor::Fabric,
            "1.21.1",
            "0.16.14"
        )
        .is_err()
    );
}

#[tokio::test]
async fn libraries_must_match_a_profile_hash_or_the_maven_sidecar() {
    let body = b"library bytes".to_vec();
    let sha1 = format!("{:x}", Sha1::digest(&body));
    let served = body.clone();
    let sidecar = format!("{sha1}  library.jar\n");
    let server = Server::start(move |request| match request.path.as_str() {
        "/org/ow2/asm/asm/9.8/asm-9.8.jar" => Reply::ok(served.clone()),
        "/org/ow2/asm/asm/9.8/asm-9.8.jar.sha1" => Reply::ok(sidecar.clone()),
        "/org/ow2/asm/asm/9.9/asm-9.9.jar" => Reply::ok(b"other bytes".to_vec()),
        "/org/ow2/asm/asm/9.9/asm-9.9.jar.sha1" => Reply::ok(sidecar.clone()),
        "/org/ow2/asm/asm/9.7/asm-9.7.jar" => Reply::ok(served.clone()),
        "/org/ow2/asm/asm/9.7/asm-9.7.jar.sha1" => Reply::ok("not a hash".to_owned()),
        _ => Reply::status(404),
    });
    let maven = Maven::for_tests(server.base()).unwrap_or_else(|_| panic!("maven"));
    let cancel = Cancellation::default();
    let wanted = |version: &str, sha1: Option<&str>, size: Option<u64>| Wanted {
        path: format!("org/ow2/asm/asm/{version}/asm-{version}.jar"),
        url: format!("{FABRIC}org/ow2/asm/asm/{version}/asm-{version}.jar"),
        sha1: sha1.map(str::to_owned),
        sha256: None,
        sha512: None,
        size,
    };
    // A hash in the profile needs no sidecar; the sidecar identifies the rest.
    assert_eq!(
        download(
            &maven,
            &wanted("9.8", Some(&sha1), Some(body.len() as u64)),
            &cancel
        )
        .await
        .ok(),
        Some(body.clone())
    );
    assert_eq!(
        download(&maven, &wanted("9.8", None, None), &cancel)
            .await
            .ok(),
        Some(body.clone())
    );
    // Wrong bytes, a wrong size or a malformed sidecar never verify.
    assert_eq!(
        code(download(&maven, &wanted("9.9", None, None), &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert_eq!(
        code(download(&maven, &wanted("9.8", Some(&sha1), Some(1)), &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert_eq!(
        code(download(&maven, &wanted("9.7", None, None), &cancel).await),
        Some(ErrorCode::InvalidData)
    );
    assert_eq!(
        code(download(&maven, &wanted("9.8", Some(&"0".repeat(40)), None), &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    // A missing library is an unavailable service, not a silent skip.
    assert!(
        download(&maven, &wanted("1.0", Some(&sha1), None), &cancel)
            .await
            .is_err()
    );
}

struct World {
    paths: AppPaths,
    base: InstallPlan,
}

fn world() -> World {
    let (paths, accounts, id) = fixture();
    drop(accounts);
    complete_installation(&paths, id);
    let root = metadata_root(&paths, id).unwrap_or_else(|_| panic!("root"));
    let installed = read_json::<InstalledLock>(&root, "installed.json")
        .unwrap_or_default()
        .unwrap_or_else(|| panic!("installed"));
    World {
        paths,
        base: installed.plan,
    }
}

fn items() -> Vec<(String, String, Vec<u8>)> {
    libraries()
        .into_iter()
        .map(|(_, path, body)| (path.to_owned(), format!("{FABRIC}{path}"), body))
        .collect()
}

fn launch() -> Launch {
    let (launch, _) = decoded(&good()).unwrap_or_else(|_| panic!("profile"));
    Launch {
        id: "fabric-loader-0.16.14-26.3".into(),
        ..launch
    }
}

fn published(world: &World) -> ProfileRef {
    publish(
        &world.paths,
        Flavor::Fabric,
        "26.3",
        "0.16.14",
        java(),
        launch(),
        items(),
        &Cancellation::default(),
    )
    .unwrap_or_else(|error| panic!("publish: {:?}", error.code()))
}

fn cleanup(world: &World) {
    let _ = std::fs::remove_dir_all(world.paths.root());
}

#[test]
fn a_published_generation_is_marked_audited_and_ordered_on_the_classpath() {
    let world = world();
    let reference = published(&world);
    let cancel = Cancellation::default();
    assert!(reference.validate("26.3").is_ok());
    assert!(reference.verify(&world.paths, &cancel).is_ok());
    assert_eq!(reference.kind(), "fabric");
    assert!(reference.directory().starts_with("libraries/fabric/"));
    assert_eq!(reference.profile_id(), "fabric-loader-0.16.14-26.3");
    assert_eq!(reference.files().len(), 3);
    // Every file is stored under the generation, named by its Maven path.
    let root = world.paths.root().join(reference.directory());
    assert!(
        root.join("libraries/org/ow2/asm/asm/9.8/asm-9.8.jar")
            .is_file()
    );
    assert!(root.join(MARKER).is_file());

    // The loader's libraries follow the game's own, and the game jar comes last.
    let classpath = reference
        .classpath(&world.paths, &world.base)
        .unwrap_or_else(|_| panic!("classpath"));
    assert_eq!(classpath.len(), 4);
    assert!(classpath[0].ends_with("sponge-mixin-0.15.5+mixin.0.8.7.jar"));
    assert!(classpath[3].to_string_lossy().contains("26.3"));

    // The loader record survives the plan's JSON round trip as a profile loader.
    let value = serde_json::to_value(LoaderRef::Profile(reference.clone()))
        .unwrap_or_else(|_| panic!("encode"));
    assert_eq!(value["type"], "fabric");
    let decoded: LoaderRef = serde_json::from_value(value).unwrap_or_else(|_| panic!("decode"));
    assert!(matches!(&decoded, LoaderRef::Profile(profile) if profile.version() == "0.16.14"));
    assert!(decoded.verify(&world.paths, &cancel).is_ok());
    assert_eq!(
        decoded.library_directory(&world.paths),
        world.paths.root().join("libraries")
    );
    cleanup(&world);
}

#[test]
fn damage_a_missing_marker_or_a_forged_record_fail_the_audit() {
    let world = world();
    let reference = published(&world);
    let cancel = Cancellation::default();
    let root = world.paths.root().join(reference.directory());
    let library = root.join("libraries/org/ow2/asm/asm/9.8/asm-9.8.jar");
    assert!(std::fs::write(&library, b"tampered").is_ok());
    assert_eq!(
        code(reference.verify(&world.paths, &cancel)),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert!(std::fs::remove_file(&library).is_ok());
    assert!(reference.verify(&world.paths, &cancel).is_err());

    // A record that no longer matches its marker is refused.
    let reference = published(&world);
    let mut forged = reference.clone();
    forged.version = "0.99.0".into();
    forged.profile.id = "fabric-loader-0.99.0-26.3".into();
    assert!(forged.verify_marker(&world.paths).is_err());
    let marker = world.paths.root().join(reference.directory()).join(MARKER);
    assert!(std::fs::remove_file(marker).is_ok());
    assert!(reference.verify(&world.paths, &cancel).is_err());

    // Records that break their own invariants never validate.
    let intact = published(&world);
    let mutate = |change: &dyn Fn(&mut ProfileRef)| {
        let mut copy = intact.clone();
        change(&mut copy);
        copy
    };
    for (name, bad) in [
        ("other game", mutate(&|r| r.minecraft = "1.21.1".into())),
        ("loader name", mutate(&|r| r.loader = "forge".into())),
        ("profile id", mutate(&|r| r.profile.id = "other".into())),
        ("class", mutate(&|r| r.profile.main_class = "a b".into())),
        ("no files", mutate(&|r| r.files.clear())),
        (
            "file outside libraries",
            mutate(&|r| r.files[0].path = "other/x.jar".into()),
        ),
        (
            "traversal",
            mutate(&|r| r.files[0].path = "libraries/../x.jar".into()),
        ),
        ("hash", mutate(&|r| r.files[0].sha1 = "z".into())),
        ("size", mutate(&|r| r.files[0].size = 0)),
        (
            "source",
            mutate(&|r| {
                let path = r.files[0].path.clone();
                r.sources.insert(path, "https://evil.example/x.jar".into());
            }),
        ),
        ("sources", mutate(&|r| r.sources.clear())),
        ("java", mutate(&|r| r.java.major_version = 0)),
        ("nil id", mutate(&|r| r.id = Uuid::nil())),
        ("schema", mutate(&|r| r.schema = 9)),
    ] {
        assert!(bad.validate("26.3").is_err(), "{name}");
    }
    cleanup(&world);
}

#[tokio::test]
async fn repair_reproduces_exact_bytes_in_a_fresh_generation_and_refuses_anything_else() {
    let world = world();
    let reference = published(&world);
    let cancel = Cancellation::default();
    let originals: std::collections::BTreeMap<String, Vec<u8>> = items()
        .into_iter()
        .map(|(path, _, body)| (format!("/{path}"), body))
        .collect();
    let faithful = originals.clone();
    let server = Server::start(move |request| match faithful.get(&request.path) {
        Some(body) => Reply::ok(body.clone()),
        None => Reply::status(404),
    });
    let maven = Maven::for_tests(server.base()).unwrap_or_else(|_| panic!("maven"));
    let library = world
        .paths
        .root()
        .join(reference.directory())
        .join("libraries/org/ow2/asm/asm/9.8/asm-9.8.jar");
    assert!(std::fs::write(&library, b"tampered").is_ok());
    assert!(reference.verify(&world.paths, &cancel).is_err());
    let repaired = reference
        .repair_with(&world.paths, &maven, &cancel)
        .await
        .unwrap_or_else(|error| panic!("repair: {:?}", error.code()));
    assert_ne!(repaired.directory(), reference.directory());
    assert!(repaired.verify(&world.paths, &cancel).is_ok());
    assert_eq!(repaired.files().len(), reference.files().len());
    for (before, after) in reference.files().iter().zip(repaired.files()) {
        assert_eq!((&before.path, &before.sha256), (&after.path, &after.sha256));
    }

    // A host that now serves different bytes cannot replace the recorded ones.
    let mut changed = originals;
    changed.insert(
        "/org/ow2/asm/asm/9.8/asm-9.8.jar".into(),
        b"different bytes".to_vec(),
    );
    let hostile = Server::start(move |request| match changed.get(&request.path) {
        Some(body) => Reply::ok(body.clone()),
        None => Reply::status(404),
    });
    let maven = Maven::for_tests(hostile.base()).unwrap_or_else(|_| panic!("maven"));
    assert_eq!(
        code(reference.repair_with(&world.paths, &maven, &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    cleanup(&world);
}
