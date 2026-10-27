use super::*;
use serde_json::{Value, json};

pub(super) fn fixture() -> (Uuid, Uuid, Value) {
    let instance = Uuid::new_v4();
    let version = Uuid::new_v4();
    (
        instance,
        version,
        json!({"data": {
            "instance_id": instance, "version_id": version, "number": 2, "label": null,
            "published_at": "2026-10-25T20:54:25Z", "minecraft_version": "1.21.1",
            "loader": "neoforge", "loader_version": "21.1.264",
            "files": [{"type": "mod", "source": "modrinth", "path": "mods/example.jar",
                "name": "Example", "env": {"client": "optional", "server": "unsupported"},
                "url": "https://cdn.modrinth.com/data/project/versions/version/example.jar",
                "hashes": {"sha1": "a".repeat(40), "sha512": "b".repeat(128)}, "size": 100}],
            "archives": {"defaults": null, "enforced": null}
        }}),
    )
}

#[test]
fn manifests_bind_pins_and_reject_hostile_or_incomplete_content() {
    let (id, version, value) = fixture();
    let parse = |value: &Value| Manifest::parse(value.to_string().as_bytes(), id, version);
    let manifest = parse(&value).unwrap_or_else(|_| panic!("manifest"));
    assert!(manifest.supported_loader());
    assert_eq!(manifest.files[0].env.client, Environment::Optional);
    assert!(Manifest::parse(value.to_string().as_bytes(), Uuid::new_v4(), version).is_err());
    for (field, replacement) in [
        ("loader_version", json!("../bad")),
        ("number", json!(0)),
        ("published_at", json!("invalid")),
        ("minecraft_version", Value::Null),
        ("version_id", json!(Uuid::nil())),
        ("label", json!("a".repeat(33))),
    ] {
        let mut bad = value.clone();
        bad["data"][field] = replacement;
        assert!(parse(&bad).is_err(), "{field}");
    }
    for (field, replacement) in [
        ("path", json!("mods/CON.jar")),
        ("path", json!("mods/../escape.jar")),
        ("path", json!("mods\\escape.jar")),
        ("path", json!("mods/sub/file.jar")),
        ("path", json!("resourcepacks/file.zip")),
        ("path", json!("mods/invalid:stream")),
        ("size", json!(MAX_FILE_BYTES + 1)),
        ("hashes", json!({})),
        ("hashes", json!({"sha1": "a".repeat(40)})),
        ("manual", json!(true)),
        ("url", Value::Null),
        ("url", json!("https://evil.test/file")),
        ("env", json!({"client": "future", "server": "required"})),
    ] {
        let mut bad = value.clone();
        bad["data"]["files"][0][field] = replacement;
        assert!(parse(&bad).is_err(), "{field}");
    }
    let mut duplicate = value.clone();
    let mut file = duplicate["data"]["files"][0].clone();
    file["path"] = json!("mods/EXAMPLE.jar");
    duplicate["data"]["files"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("files"))
        .push(file);
    assert!(parse(&duplicate).is_err());
    let mut future = value.clone();
    future["data"]["loader"] = json!("future-loader");
    assert!(
        !parse(&future)
            .unwrap_or_else(|_| panic!("future loader"))
            .supported_loader()
    );
    let mut manual = value.clone();
    manual["data"]["files"][0]["source"] = json!("curseforge");
    manual["data"]["files"][0]["manual"] = json!(true);
    manual["data"]["files"][0]["url"] = Value::Null;
    manual["data"]["files"][0]["curseforge"] = json!({"project_id": 123, "file_id": 456});
    assert!(parse(&manual).is_ok());
}

#[test]
fn initial_urls_and_redirects_have_separate_source_policies() {
    for raw in [
        "http://cdn.modrinth.com/data/p/versions/v/a.jar",
        "https://cdn.modrinth.com.evil.test/data/p/versions/v/a.jar",
        "https://user@cdn.modrinth.com/data/p/versions/v/a.jar",
        "https://cdn.modrinth.com:444/data/p/versions/v/a.jar",
        "https://cdn.modrinth.com/data/p/versions/v/%2e%2e",
        "https://cdn.modrinth.com/data/p/versions/v/a%2fb.jar",
        "https://cdn.modrinth.com/data/p/versions/v/a.jar?token=x",
        "https://cdn.modrinth.com/data/p/versions/v/a.jar#fragment",
    ] {
        assert!(download_url(raw, &Origin::Modrinth).is_err(), "{raw}");
    }
    assert!(
        download_url(
            "https://cdn.modrinth.com/data/p/versions/v/a%20b.jar",
            &Origin::Modrinth
        )
        .is_ok()
    );
    assert!(
        download_url(
            "https://other.supabase.co/storage/v1/object/public/modpacks/a.zip",
            &Origin::Upload
        )
        .is_err()
    );
    assert!(
        download_url(
            "https://release-assets.githubusercontent.com/asset",
            &Origin::Github
        )
        .is_err()
    );
    assert!(
        github_redirect(
            "https://release-assets.githubusercontent.com/asset?signature=x",
            1
        )
        .is_ok()
    );
    assert!(github_redirect("https://evilgithubusercontent.com/asset", 1).is_err());
    assert!(github_redirect("https://release-assets.githubusercontent.com/asset", 4).is_err());
}

fn index() -> Value {
    json!({"formatVersion": 1, "game": "minecraft", "versionId": "pack-1", "name": "Example",
        "dependencies": {"minecraft": "1.21.1", "neoforge": "21.1.264"},
        "files": [{"path": "mods/example.jar", "hashes": {"sha1": "a".repeat(40), "sha512": "b".repeat(128)},
            "downloads": ["https://cdn.modrinth.com/data/p/versions/v/a.jar"], "fileSize": 100}]})
}

#[test]
fn mrpack_index_requires_exact_dependencies_and_both_hashes() {
    let parse = |v: &Value| MrpackIndex::parse(v.to_string().as_bytes());
    let valid = index();
    let duplicate_dependencies = valid.to_string().replace(
        "\"neoforge\":\"21.1.264\"",
        "\"neoforge\":\"21.1.264\",\"neoforge\":\"21.1.265\"",
    );
    assert!(MrpackIndex::parse(duplicate_dependencies.as_bytes()).is_err());
    assert!(
        parse(&valid)
            .unwrap_or_else(|_| panic!("index"))
            .supported_loader()
    );
    for (field, replacement) in [
        ("formatVersion", json!(2)),
        ("game", json!("other")),
        ("dependencies", json!({"neoforge": "21.1.264"})),
        (
            "dependencies",
            json!({"minecraft":"1.21.1", "neoforge":"21.1.264", "forge":"52.0.0"}),
        ),
    ] {
        let mut bad = valid.clone();
        bad[field] = replacement;
        assert!(parse(&bad).is_err(), "{field}");
    }
    for (field, replacement) in [
        ("path", json!("saves/world/level.dat")),
        ("path", json!(".panita/installed.json")),
        ("hashes", json!({"sha1":"a".repeat(40)})),
        ("downloads", json!([])),
        ("downloads", json!(["https://evil.test/file"])),
    ] {
        let mut bad = valid.clone();
        bad["files"][0][field] = replacement;
        assert!(parse(&bad).is_err(), "{field}");
    }
    let mut future = valid;
    future["dependencies"]["future-loader"] = json!("1.0");
    assert!(
        !parse(&future)
            .unwrap_or_else(|_| panic!("future"))
            .supported_loader()
    );
}

#[test]
fn state_retains_deleted_defaults_and_separates_skipped_desired_content() {
    use state::*;
    let id = Uuid::new_v4();
    let hashes = Hashes {
        sha1: None,
        sha512: Some("b".repeat(128)),
    };
    let state = PackState {
        schema: 1,
        id,
        source: PackIdentity::Custom {
            version_id: Uuid::new_v4(),
            published_at: "2026-10-25T20:54:25Z".into(),
            label: None,
        },
        minecraft_version: "1.21.1".into(),
        loader: "neoforge".into(),
        loader_version: "21.1.264".into(),
        files: vec![ManagedFile {
            path: "config/example.toml".into(),
            ownership: Ownership::Enforced,
            desired: hashes.clone(),
            size: 100,
            baseline: None,
        }],
        consumed_defaults: BTreeSet::from(["options.txt".into()]),
        skipped_configuration: vec![SkippedConfiguration {
            path: "config/example.toml".into(),
            desired: hashes,
        }],
        pending: Vec::new(),
    };
    assert!(state.validate(id).is_ok());
    let bytes = serde_json::to_vec(&state).unwrap_or_else(|_| panic!("encode"));
    assert_eq!(
        PackState::decode(&bytes, id).unwrap_or_else(|_| panic!("decode")),
        state
    );
    assert!(PackState::decode(&bytes, Uuid::new_v4()).is_err());
    let mut damaged = state.clone();
    damaged.skipped_configuration.clear();
    assert!(damaged.validate(id).is_err());
    let mut future = serde_json::to_value(&state).unwrap_or_else(|_| panic!("value"));
    future["schema"] = json!(99);
    assert!(PackState::decode(future.to_string().as_bytes(), id).is_err());
    future["schema"] = json!(1);
    future["schema_version"] = json!(1);
    assert!(PackState::decode(future.to_string().as_bytes(), id).is_err());
}
