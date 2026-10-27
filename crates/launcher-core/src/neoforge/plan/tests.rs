use super::*;
use std::io::Write;

fn resolve(change: impl FnOnce(&mut serde_json::Value)) -> Result<ProcessorPlan, CoreError> {
    let mut profile: serde_json::Value =
        serde_json::from_slice(include_bytes!("../fixtures/21.1.264-install-profile.json"))
            .map_err(|_| invalid())?;
    change(&mut profile);
    let profile = serde_json::to_vec(&profile).map_err(|_| invalid())?;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("install_profile.json", profile.as_slice()),
        (
            "version.json",
            include_bytes!("../fixtures/21.1.264-version.json").as_slice(),
        ),
        ("data/client.lzma", b"synthetic patch".as_slice()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .map_err(|_| invalid())?;
        zip.write_all(bytes).map_err(|_| invalid())?;
    }
    let installer = zip.finish().map_err(|_| invalid())?.into_inner();
    let identity = InstallerIdentity {
        minecraft: "1.21.1".into(),
        version: "21.1.264".into(),
        sha1: format!("{:x}", Sha1::digest(&installer)),
        size: installer.len() as u64,
    };
    let game = include_bytes!("../../minecraft/fixtures/1.21.1.json");
    let game_source = Artifact { url: "https://piston-meta.mojang.com/v1/packages/cedfc3b6dcbca34e2b478d498bf1d56a8fa2f404/1.21.1.json".into(),
        sha1: "cedfc3b6dcbca34e2b478d498bf1d56a8fa2f404".into(), size: game.len() as u64, path: None };
    ProcessorPlan::resolve(&installer, &identity, game, &game_source)
}

#[test]
fn builds_ordered_graph_with_verified_mapping_download_instead_of_network_processor()
-> Result<(), CoreError> {
    let plan = resolve(|_| {})?;
    assert_eq!(plan.processors.len(), 5);
    assert_eq!(plan.java.major_version, 21);
    assert_eq!(plan.derived.len(), 6);
    let mappings = plan
        .inputs
        .iter()
        .find(|input| input.destination.ends_with("-mappings.txt"))
        .ok_or_else(invalid)?;
    assert!(matches!(mappings.source, InputSource::Mojang(_)));
    assert!(
        plan.inputs
            .iter()
            .any(|input| matches!(input.source, InputSource::Embedded { .. }))
    );
    assert!(
        plan.processors[0]
            .outputs
            .keys()
            .any(|path| path.contains("/neoform/"))
    );
    assert!(
        plan.processors[4]
            .outputs
            .keys()
            .any(|path| path.ends_with("neoforge-21.1.264-client.jar"))
    );
    assert!(plan.processors.iter().all(|step| {
        !step
            .args
            .contains(&ProcessorArgument::Literal("DOWNLOAD_MOJMAPS".into()))
    }));
    Ok(())
}

#[test]
fn rejects_output_aliases_unavailable_inputs_reordered_steps_and_unknown_tasks() {
    for change in 0..7 {
        let result = resolve(|profile| match change {
            0 => profile["data"]["PATCHED"]["client"] = profile["data"]["MC_SRG"]["client"].clone(),
            1 => {
                profile["data"]["MAPPINGS"]["client"] =
                    serde_json::json!("[net.neoforged:neoforge:21.1.264:universal]")
            }
            2 => profile["processors"][3]["args"][1] = serde_json::json!("UNKNOWN"),
            3 => profile["processors"][3]["args"][3] = serde_json::json!("[unknown:input:1]"),
            4 => profile["processors"]
                .as_array_mut()
                .unwrap_or_else(|| panic!("processors"))
                .swap(3, 5),
            5 => profile["processors"][3]["args"][5] = serde_json::json!("{ROOT}/../outside"),
            _ => profile["processors"][4]["args"][3] = serde_json::json!("1.21.2"),
        });
        assert!(result.is_err(), "accepted mutation {change}");
    }
}

#[test]
fn destination_tree_rejects_file_directory_aliases() {
    assert!(validate_destinations(["libraries/a", "libraries/a/b.jar"].into_iter()).is_err());
    assert!(validate_destinations(["libraries/a.jar", "libraries/b.jar"].into_iter()).is_ok());
}
