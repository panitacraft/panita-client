use super::*;
use crate::minecraft::download::ensure_parent;
use crate::neoforge::plan::PlannedInput;

pub(crate) fn fixture_at(paths: &AppPaths) -> Result<(ProcessorPlan, StagedLoader), CoreError> {
    fixture_for(
        paths,
        "1.21.1",
        &crate::minecraft::JavaRequirement {
            component: "java-runtime-delta".into(),
            major_version: 21,
        },
    )
}

fn fixture_for(
    paths: &AppPaths,
    minecraft: &str,
    java: &crate::minecraft::JavaRequirement,
) -> Result<(ProcessorPlan, StagedLoader), CoreError> {
    let mut profile: LaunchProfile =
        serde_json::from_slice(include_bytes!("../fixtures/21.1.264-version.json"))
            .map_err(|_| invalid())?;
    profile.inherits_from = minecraft.into();
    let flag = profile
        .arguments
        .game
        .iter()
        .position(|arg| arg == "--fml.mcVersion")
        .ok_or_else(invalid)?;
    profile.arguments.game[flag + 1] = minecraft.into();
    let operation = Uuid::new_v4();
    let root = paths.root().join(format!("libraries/neoforge/{operation}"));
    PrivateDir::ensure(paths.root())
        .map_err(map_private_error)?
        .ensure_dir("libraries")
        .map_err(map_private_error)?
        .ensure_dir("neoforge")
        .map_err(map_private_error)?
        .create_dir(operation.to_string())
        .map_err(map_private_error)?;
    let mut inputs = Vec::new();
    let client = b"synthetic original client";
    let (directory, leaf) = ensure_parent(&root, "minecraft-client.jar")?;
    directory
        .write_atomic_durable(leaf, client)
        .map_err(map_private_error)?;
    inputs.push(PlannedInput {
        destination: "minecraft-client.jar".into(),
        source: InputSource::Mojang(Artifact {
            url: "https://piston-data.mojang.com/fixture".into(),
            sha1: format!("{:x}", Sha1::digest(client)),
            size: client.len() as u64,
            path: None,
        }),
    });
    for library in &mut profile.libraries {
        let bytes = library.name.as_bytes();
        library.downloads.artifact.sha1 = format!("{:x}", Sha1::digest(bytes));
        library.downloads.artifact.size = bytes.len() as u64;
        let destination = format!("libraries/{}", library.downloads.artifact.path);
        let (directory, leaf) = ensure_parent(&root, &destination)?;
        directory
            .write_atomic_durable(leaf, bytes)
            .map_err(map_private_error)?;
        inputs.push(PlannedInput {
            destination,
            source: InputSource::Library(library.downloads.artifact.clone()),
        });
    }
    let universal = Coordinate::parse("net.neoforged:neoforge:21.1.264:universal")?;
    let bytes = b"synthetic universal input";
    let destination = format!("libraries/{}", universal.path());
    let (directory, leaf) = ensure_parent(&root, &destination)?;
    directory
        .write_atomic_durable(leaf, bytes)
        .map_err(map_private_error)?;
    inputs.push(PlannedInput {
        destination,
        source: InputSource::Library(LibraryArtifact {
            path: universal.path().into(),
            url: format!("{MAVEN_ROOT}{}", universal.path()),
            sha1: format!("{:x}", Sha1::digest(bytes)),
            size: bytes.len() as u64,
        }),
    });
    let derived = [
        "net.neoforged:neoforge:21.1.264:client".to_owned(),
        format!("net.minecraft:client:{minecraft}-20240808.144430:srg"),
        format!("net.minecraft:client:{minecraft}-20240808.144430:extra"),
    ]
    .iter()
    .map(|name| {
        Coordinate::parse(name).map(|coordinate| format!("libraries/{}", coordinate.path()))
    })
    .collect::<Result<BTreeSet<_>, _>>()?;
    let mut files = Vec::new();
    for path in &derived {
        let (directory, leaf) = ensure_parent(&root, path)?;
        directory
            .write_atomic_durable(leaf, b"synthetic derived output")
            .map_err(map_private_error)?;
        files.push(crate::neoforge::staging::DerivedArtifact {
            path: path.clone(),
            sha256: format!("{:x}", Sha256::digest(b"synthetic derived output")),
            size: 24,
        });
    }
    let plan = ProcessorPlan {
        identity: InstallerIdentity {
            minecraft: minecraft.into(),
            version: "21.1.264".into(),
            sha1: "1".repeat(40),
            size: 1,
        },
        java: java.clone(),
        inputs,
        processors: Vec::new(),
        launch: profile,
        derived,
    };
    Ok((
        plan,
        StagedLoader {
            root,
            operation,
            files,
        },
    ))
}

pub(crate) fn published(paths: &AppPaths) -> Result<GenerationRef, CoreError> {
    let (plan, staged) = fixture_at(paths)?;
    publish(paths, &plan, staged, &Cancellation::default())
}

pub(crate) fn published_for(
    paths: &AppPaths,
    minecraft: &str,
    java: &crate::minecraft::JavaRequirement,
) -> Result<GenerationRef, CoreError> {
    // Deliberately synthetic contract for installation-integrity tests only.
    let (plan, staged) = fixture_for(paths, minecraft, java)?;
    publish(paths, &plan, staged, &Cancellation::default())
}

fn paths() -> Result<AppPaths, CoreError> {
    AppPaths::new(std::env::temp_dir().join(format!("panita-generation-{}", Uuid::new_v4())))
}

#[test]
fn published_generation_round_trips_and_rejects_changed_files_and_missing_markers()
-> Result<(), Box<dyn std::error::Error>> {
    let paths = paths()?;
    let generation = published(&paths)?;
    generation.verify(&paths, &Cancellation::default())?;
    let disk = crate::storage_json::pretty(&generation)?;
    assert!(
        String::from_utf8_lossy(&disk)
            .starts_with("{\n  \"schema\": 1,\n  \"type\": \"neoforge\",\n  \"id\"")
    );
    let loaded: GenerationRef = serde_json::from_slice(&disk)?;
    loaded.verify(&paths, &Cancellation::default())?;
    let output = loaded.derived.iter().next().ok_or("output")?;
    std::fs::write(
        paths.root().join(loaded.directory()).join(output),
        b"modified derived content",
    )?;
    assert!(loaded.verify(&paths, &Cancellation::default()).is_err());
    std::fs::remove_file(paths.root().join(loaded.directory()).join(MARKER))?;
    assert!(loaded.verify_marker(&paths).is_err());
    Ok(())
}

#[test]
fn interrupted_publication_and_cancellation_leave_no_completion_marker()
-> Result<(), Box<dyn std::error::Error>> {
    for cancelled in [false, true] {
        let paths = paths()?;
        let (plan, staged) = fixture_at(&paths)?;
        let root = staged.root.clone();
        let cancellation = Cancellation::default();
        if cancelled {
            cancellation.cancel();
        } else {
            std::fs::write(root.join(&staged.files[0].path), b"damaged output")?;
        }
        assert!(publish(&paths, &plan, staged, &cancellation).is_err());
        assert!(!root.join(MARKER).exists());
    }
    Ok(())
}

#[test]
fn unknown_markers_and_future_state_are_preserved() -> Result<(), Box<dyn std::error::Error>> {
    let paths = paths()?;
    let (plan, staged) = fixture_at(&paths)?;
    let root = staged.root.clone();
    let directory = PrivateDir::open_data(&root)?;
    directory.write_atomic_durable(MARKER, b"{\"schema\":900}")?;
    assert!(publish(&paths, &plan, staged, &Cancellation::default()).is_err());
    assert_eq!(std::fs::read(root.join(MARKER))?, b"{\"schema\":900}");
    let generation = published(&paths)?;
    let mut value = serde_json::to_value(generation)?;
    value["schema"] = serde_json::json!(900);
    let future: GenerationRef = serde_json::from_value(value)?;
    assert!(future.validate("1.21.1").is_err());
    Ok(())
}

#[test]
fn repair_requires_exact_reproduction_and_preserves_the_previous_generation()
-> Result<(), CoreError> {
    let paths = paths()?;
    let previous = published(&paths)?;
    let mut replacement = published(&paths)?;
    previous.verify_replacement(&replacement)?;
    replacement.files[0].sha256 = "0".repeat(64);
    assert!(previous.verify_replacement(&replacement).is_err());
    previous.verify(&paths, &Cancellation::default())?;
    Ok(())
}
