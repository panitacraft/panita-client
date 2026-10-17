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
            assert_eq!(resolved.kind, "release");
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

#[test]
fn cache_mapping_preserves_legacy_locks_and_qualifies_asset_index_names() {
    use super::plan::{InstallPlan, PlannedArtifact, asset_index_cache_path, library_cache_path};
    let version = fixture("1.21.1")
        .resolve(&Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform")))
        .unwrap_or_else(|_| panic!("version"));
    let library = version.classpath[0].clone();
    let legacy_library = format!(
        "libraries/{}",
        library.path.as_ref().unwrap_or_else(|| panic!("path"))
    );
    let legacy_index = format!("assets/indexes/{}.json", version.assets.id);
    let index = version.assets.clone();
    let mut plan = InstallPlan {
        schema_version: 1,
        version,
        platform: "windows".into(),
        architecture: "amd64".into(),
        runtime_root: String::new(),
        java_executable: String::new(),
        documents: Vec::new(),
        artifacts: vec![
            PlannedArtifact {
                destination: legacy_library.clone(),
                artifact: library.clone(),
                executable: false,
            },
            PlannedArtifact {
                destination: legacy_index,
                artifact: index.artifact.clone(),
                executable: false,
            },
        ],
    };
    assert_eq!(
        plan.library_path(&library).ok().as_deref(),
        Some(legacy_library.as_str())
    );
    assert_eq!(
        plan.asset_index_name().ok().as_deref(),
        Some(index.id.as_str())
    );
    plan.artifacts[0].destination =
        library_cache_path(&library).unwrap_or_else(|_| panic!("content path"));
    plan.artifacts[1].destination =
        asset_index_cache_path(&index).unwrap_or_else(|_| panic!("index path"));
    assert!(plan.library_path(&library).is_ok());
    assert_eq!(
        plan.asset_index_name().ok(),
        Some(format!("{}-{}", index.id, index.artifact.sha1))
    );
    let mut changed = index.clone();
    changed.artifact.sha1 = "0".repeat(40);
    assert_ne!(
        asset_index_cache_path(&index).ok(),
        asset_index_cache_path(&changed).ok()
    );
}
