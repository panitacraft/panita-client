use super::*;
use std::io::Write;
use zip::write::SimpleFileOptions;

const INSTALL: &[u8] = include_bytes!("fixtures/21.1.264-install-profile.json");
const LAUNCH: &[u8] = include_bytes!("fixtures/21.1.264-version.json");

fn metadata() -> Result<InstallerMetadata, Box<dyn std::error::Error>> {
    Ok(InstallerMetadata {
        install: serde_json::from_slice(INSTALL)?,
        launch: serde_json::from_slice(LAUNCH)?,
    })
}

fn identity(bytes: &[u8]) -> InstallerIdentity {
    InstallerIdentity {
        minecraft: "1.21.1".into(),
        version: "21.1.264".into(),
        sha1: format!("{:x}", Sha1::digest(bytes)),
        size: bytes.len() as u64,
    }
}

fn archive(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, SimpleFileOptions::default())?;
        zip.write_all(bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

#[test]
fn resolves_maven_classifiers_and_extensions_without_path_aliases() -> Result<(), CoreError> {
    assert_eq!(
        Coordinate::parse("net.neoforged:neoforge:21.1.264:client")?.path(),
        "net/neoforged/neoforge/21.1.264/neoforge-21.1.264-client.jar"
    );
    assert_eq!(
        Coordinate::parse("net.neoforged:neoform:1.21.1-20240808.144430@zip")?.path(),
        "net/neoforged/neoform/1.21.1-20240808.144430/neoform-1.21.1-20240808.144430.zip"
    );
    for bad in [
        "a:b",
        "a:b:c:d:e",
        "a..b:c:d",
        "a:b:../x",
        "a:b:c@../zip",
        "a:b:c@@jar",
        "a:b:c:",
        "a:b:CON",
        "a:b:c@",
        "a/b:c:d",
        "a:b:c%2f",
    ] {
        assert!(Coordinate::parse(bad).is_err(), "accepted {bad}");
    }
    Ok(())
}

#[test]
fn scopes_library_hosts_and_binds_exact_coordinate_path() -> Result<(), CoreError> {
    let path = Coordinate::parse("a:b:1")?;
    for root in [MAVEN_ROOT, "https://libraries.minecraft.net/"] {
        assert!(library_url(&format!("{root}{}", path.path()), path.path()).is_ok());
    }
    for root in [
        "http://maven.neoforged.net/releases/",
        "https://maven.neoforged.net.evil/releases/",
        "https://user@maven.neoforged.net/releases/",
        "https://maven.neoforged.net:443/releases/",
        "https://maven.neoforged.net/snapshots/",
    ] {
        assert!(library_url(&format!("{root}{}", path.path()), path.path()).is_err());
    }
    for suffix in ["?token=x", "#fragment", "/../other.jar"] {
        assert!(library_url(&format!("{MAVEN_ROOT}{}{suffix}", path.path()), path.path()).is_err());
    }
    // The vanilla boundary remains Mojang-only.
    assert!(crate::minecraft::official_url(&format!("{MAVEN_ROOT}{}", path.path())).is_err());
    Ok(())
}

#[test]
fn validates_public_candidate_and_retains_order_and_exact_loader_pin()
-> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        format!("{:x}", Sha1::digest(LAUNCH)),
        "cb85b8f15baa7928ee6397e66b9ff5a4d75b4689"
    );
    let metadata = metadata()?;
    metadata.validate(&identity(b"fixture"))?;
    let processors: Vec<_> = metadata
        .install
        .processors
        .iter()
        .filter(|p| p.runs_on_client())
        .collect();
    assert_eq!(processors.len(), 6);
    assert_eq!(processors[0].args[1], "MCP_DATA");
    assert_eq!(processors[1].args[1], "DOWNLOAD_MOJMAPS");
    assert_eq!(
        processors[5].jar,
        "net.neoforged.installertools:binarypatcher:2.1.2:fatjar"
    );
    assert_eq!(
        metadata.launch.main_class,
        "cpw.mods.bootstraplauncher.BootstrapLauncher"
    );
    assert_eq!(
        &metadata.launch.arguments.game[..2],
        &["--fml.neoForgeVersion", "21.1.264"]
    );
    let mut wrong_pin = identity(b"fixture");
    wrong_pin.version = "21.1.248".into();
    assert!(metadata.validate(&wrong_pin).is_err());
    wrong_pin.version = "21.1.264".into();
    wrong_pin.minecraft = "1.21.2".into();
    assert!(metadata.validate(&wrong_pin).is_err());
    Ok(())
}

#[test]
fn verifies_archive_before_decoding_and_rejects_unsafe_entries()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = archive(&[("install_profile.json", INSTALL), ("version.json", LAUNCH)])?;
    let expected = identity(&bytes);
    assert!(InstallerMetadata::decode(&bytes, &expected).is_ok());
    let mut corrupt = bytes.clone();
    corrupt[0] ^= 1;
    assert!(InstallerMetadata::decode(&corrupt, &expected).is_err());
    let mut wrong_size = expected.clone();
    wrong_size.size += 1;
    assert!(InstallerMetadata::decode(&bytes, &wrong_size).is_err());
    for bad in [
        "../escape",
        "/absolute",
        "data/CON",
        "data/hidden:stream",
        "data\\escape",
        "VERSION.JSON",
    ] {
        let bytes = archive(&[
            ("install_profile.json", INSTALL),
            ("version.json", LAUNCH),
            (bad, b"x"),
        ])?;
        assert!(
            InstallerMetadata::decode(&bytes, &identity(&bytes)).is_err(),
            "accepted {bad}"
        );
    }
    let missing = archive(&[("install_profile.json", INSTALL)])?;
    assert!(InstallerMetadata::decode(&missing, &identity(&missing)).is_err());
    let oversized = vec![b' '; MAX_METADATA_BYTES + 1];
    let bytes = archive(&[
        ("install_profile.json", &oversized),
        ("version.json", LAUNCH),
    ])?;
    assert!(InstallerMetadata::decode(&bytes, &identity(&bytes)).is_err());
    Ok(())
}

#[test]
fn rejects_malformed_processor_references_and_future_operational_fields()
-> Result<(), Box<dyn std::error::Error>> {
    let base = metadata()?;
    let expected = identity(b"fixture");
    for change in 0..9 {
        let mut metadata = base.clone();
        let processor = &mut metadata.install.processors[3];
        match change {
            0 => processor.jar = "unknown:processor:1".into(),
            1 => processor.classpath.push("unknown:library:1".into()),
            2 => processor.args.push("{UNDEFINED}".into()),
            3 => processor.args.push("prefix{ROOT".into()),
            4 => processor.sides = Some(vec!["future".into()]),
            5 => {
                metadata.install.data.insert(
                    "ROOT".into(),
                    SideData {
                        client: "'x'".into(),
                        server: "'x'".into(),
                    },
                );
            }
            6 => metadata.install.spec = 2,
            7 => {
                metadata
                    .install
                    .data
                    .get_mut("BINPATCH")
                    .ok_or("missing patch")?
                    .client = "/data/../escape".into()
            }
            _ => {
                processor
                    .outputs
                    .insert("{PATCHED}".into(), "wrong-hash".into());
            }
        }
        assert!(
            metadata.validate(&expected).is_err(),
            "accepted mutation {change}"
        );
    }
    let mut processor =
        serde_json::json!({"jar":"a:b:1","classpath":[],"args":[],"future_execution":true});
    assert!(serde_json::from_value::<Processor>(processor.clone()).is_err());
    processor
        .as_object_mut()
        .ok_or("object")?
        .remove("future_execution");
    assert!(serde_json::from_value::<Processor>(processor).is_ok());
    Ok(())
}

#[test]
fn rejects_conflicting_libraries_paths_hashes_and_download_origins()
-> Result<(), Box<dyn std::error::Error>> {
    let base = metadata()?;
    for change in 0..5 {
        let mut metadata = base.clone();
        let library = &mut metadata.install.libraries[0];
        match change {
            0 => library.downloads.artifact.sha1 = "0".into(),
            1 => library.downloads.artifact.path = "other/file.jar".into(),
            2 => library.downloads.artifact.url = "https://evil.example/file.jar".into(),
            3 => library.downloads.artifact.size = 0,
            _ => {
                let duplicate = library.clone();
                metadata.install.libraries.push(duplicate);
            }
        }
        assert!(metadata.validate(&identity(b"fixture")).is_err());
    }
    let mut metadata = base;
    let mut conflict = metadata.install.libraries[0].clone();
    conflict.downloads.artifact.sha1 = "0".repeat(40);
    metadata.launch.libraries.push(conflict);
    assert!(metadata.validate(&identity(b"fixture")).is_err());
    Ok(())
}

#[test]
fn rejects_duplicate_variables_outputs_and_changed_launch_pins()
-> Result<(), Box<dyn std::error::Error>> {
    let profile = String::from_utf8(INSTALL.to_vec())?;
    let duplicate = profile.replacen(
        "\"data\": {",
        "\"data\": {\"MAPPINGS\":{\"client\":\"'x'\",\"server\":\"'x'\"},",
        1,
    );
    assert_ne!(duplicate, profile);
    assert!(serde_json::from_str::<InstallProfile>(&duplicate).is_err());
    let duplicate = r#"{"jar":"a:b:1","classpath":[],"args":[],"outputs":{"x":"a","x":"b"}}"#;
    assert!(serde_json::from_str::<Processor>(duplicate).is_err());
    let mut changed = metadata()?;
    changed.launch.arguments.game[1] = "21.1.248".into();
    assert!(changed.validate(&identity(b"fixture")).is_err());
    let mut changed = metadata()?;
    changed.launch.arguments.game.push("--fml.mcVersion".into());
    changed.launch.arguments.game.push("1.21.1".into());
    assert!(changed.validate(&identity(b"fixture")).is_err());
    let mut changed = metadata()?;
    changed
        .install
        .decorations
        .insert("future_execution".into(), serde_json::json!(true));
    assert!(changed.validate(&identity(b"fixture")).is_err());
    Ok(())
}
