use super::*;

fn fixture(id: &str) -> VersionMetadata {
    let (bytes, hash) = match id {
        "26.3" => (
            include_bytes!("fixtures/26.3.json").as_slice(),
            "702fe59163c6ee6578607daa85811d9bc9c7cc40",
        ),
        _ => (
            include_bytes!("fixtures/1.21.1.json").as_slice(),
            "cedfc3b6dcbca34e2b478d498bf1d56a8fa2f404",
        ),
    };
    VersionMetadata::decode(
        bytes,
        &Artifact {
            url: format!("https://piston-meta.mojang.com/v1/packages/{hash}/{id}.json"),
            sha1: hash.into(),
            size: bytes.len() as u64,
            path: None,
        },
        id,
    )
    .unwrap_or_else(|error| panic!("fixture: {error}"))
}

#[test]
fn both_pins_resolve_for_each_target_without_foreign_native_jars() {
    for id in ["26.3", "1.21.1"] {
        for os in ["windows", "linux"] {
            let platform = Platform::x64(os, "10.0.26100").unwrap_or_else(|_| panic!("platform"));
            let resolved = fixture(id)
                .resolve(&platform)
                .unwrap_or_else(|error| panic!("{id}/{os}: {error}"));
            assert_eq!(
                resolved.java.major_version,
                if id == "26.3" { 25 } else { 21 }
            );
            assert!(resolved.classpath.len() > 30);
            let native_paths: Vec<_> = resolved
                .classpath
                .iter()
                .filter_map(|item| item.path.as_deref())
                .filter(|path| path.contains("natives-"))
                .collect();
            assert!(!native_paths.is_empty());
            for path in native_paths {
                assert!(path.ends_with(&format!("natives-{os}.jar")), "{path}");
            }
            assert!(resolved.natives.is_empty());
            let metadata = fixture(id);
            assert_eq!(
                resolved.classpath[0],
                metadata
                    .libraries
                    .iter()
                    .find(|library| allowed(&library.rules, &platform).unwrap_or(false))
                    .unwrap_or_else(|| panic!("applicable library"))
                    .downloads
                    .artifact
                    .clone()
                    .unwrap_or_else(|| panic!("artifact"))
            );
        }
    }
}

#[test]
fn ordered_rules_features_regex_and_version_ranges() {
    let rules: Vec<Rule> = serde_json::from_str(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"windows"}},{"action":"allow","os":{"name":"windows","arch":"amd64","version":"10\\..*","versionRange":{"min":"10.0.17134","max":"11"}},"features":{"custom":true}}]"#).unwrap_or_else(|_| panic!("rules"));
    let mut platform =
        Platform::x64("windows", "10.0.26100").unwrap_or_else(|_| panic!("platform"));
    assert!(!allowed(&rules, &platform).unwrap_or(true));
    platform.features.insert("custom".into(), true);
    assert!(allowed(&rules, &platform).unwrap_or(false));
    platform.os_version = "11.0".into();
    assert!(!allowed(&rules, &platform).unwrap_or(true));
    let invalid_rule: Vec<Rule> =
        serde_json::from_str(r#"[{"action":"allow","os":{"version":"["}}]"#)
            .unwrap_or_else(|_| panic!("rule"));
    assert!(allowed(&invalid_rule, &platform).is_err());
    assert!(serde_json::from_str::<Vec<Rule>>(r#"[{"action":"allow","unknown":true}]"#).is_err());
}

#[test]
fn substitution_preserves_spaces_and_rejects_unresolved_placeholders() {
    let replacements =
        BTreeMap::from([("game_directory".into(), "C:\\Users\\Test User\\game".into())]);
    assert_eq!(
        substitute("${game_directory}", &replacements)
            .ok()
            .as_deref(),
        Some("C:\\Users\\Test User\\game")
    );
    assert!(substitute("${missing}", &replacements).is_err());
    assert!(substitute("${unfinished", &replacements).is_err());
}

#[test]
fn paths_urls_hashes_and_case_collisions_are_rejected() {
    for path in [
        "../escape",
        "/root",
        "a\\b",
        "a:stream",
        "a//b",
        "con.jar",
        "a/LPT1",
        "a./b",
        "a/b ",
        "a/../b",
    ] {
        assert!(safe_path(path).is_err(), "{path}");
    }
    for url in [
        "http://libraries.minecraft.net/a",
        "https://libraries.minecraft.net.evil.test/a",
        "https://user@libraries.minecraft.net/a",
        "https://libraries.minecraft.net:444/a",
    ] {
        assert!(official_url(url).is_err());
    }
    let mut metadata = fixture("1.21.1");
    let library = metadata
        .libraries
        .iter()
        .find(|library| library.rules.is_empty())
        .unwrap_or_else(|| panic!("unconditional library"))
        .clone();
    metadata.libraries.push(library);
    assert!(
        metadata
            .resolve(&Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")))
            .is_err()
    );
    assert!(
        VersionMetadata::decode(
            b"{}",
            &Artifact {
                url: VERSION_MANIFEST.into(),
                sha1: "0".repeat(40),
                size: 2,
                path: None
            },
            "1.21.1"
        )
        .is_err()
    );
}

#[test]
fn inheritance_replaces_library_versions_preserving_classpath_position() {
    let parent = fixture("1.21.1");
    let mut child = parent.clone();
    child.id = "child".into();
    child.inherits_from = Some(parent.id.clone());
    child.libraries.truncate(1);
    let original = child.libraries[0].name.clone();
    let mut parts: Vec<_> = original.split(':').collect();
    parts[2] = "999";
    child.libraries[0].name = parts.join(":");
    let merged = child
        .inherit(parent.clone())
        .unwrap_or_else(|_| panic!("merge"));
    assert_eq!(merged.libraries.len(), parent.libraries.len());
    assert!(merged.libraries[0].name.contains(":999"));
    assert_eq!(merged.libraries[1].name, parent.libraries[1].name);
}
