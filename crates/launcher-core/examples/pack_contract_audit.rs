//! Inspect public pack inputs without reading a player profile or writing game data.
use launcher_core::{
    catalog,
    modpack::{
        Manifest,
        modrinth::{resolve_version, verified_index},
    },
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() != 4 {
        return Err("usage: pack_contract_audit <catalog.json> <custom-manifest.json> <modrinth-version.json> <pack.mrpack>".into());
    }
    let read = |index: usize, limit: u64| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use std::io::Read;
        let file = std::fs::File::open(&arguments[index])?;
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err("input exceeds bound".into());
        }
        Ok(bytes)
    };
    let catalog = catalog::parse_list(&read(0, 512 * 1024)?)?;
    let custom_bytes = read(1, launcher_core::modpack::MAX_DOCUMENT_BYTES as u64)?;
    let custom_json: serde_json::Value = serde_json::from_slice(&custom_bytes)?;
    let instance = serde_json::from_value(custom_json["data"]["instance_id"].clone())?;
    let version = serde_json::from_value(custom_json["data"]["version_id"].clone())?;
    let custom = Manifest::parse(&custom_bytes, instance, version)?;
    custom.bind_current(
        catalog
            .iter()
            .find(|entry| entry.id == instance)
            .ok_or("custom target absent")?,
    )?;
    println!(
        "Custom: {} files, Minecraft {}, {} {}",
        custom.files.len(),
        custom.minecraft_version,
        custom.loader,
        custom.loader_version
    );
    let version_bytes = read(2, launcher_core::modpack::MAX_DOCUMENT_BYTES as u64)?;
    let metadata: serde_json::Value = serde_json::from_slice(&version_bytes)?;
    let project = metadata["project_id"].as_str().ok_or("missing project")?;
    let version = metadata["id"].as_str().ok_or("missing version")?;
    let artifact = resolve_version(&version_bytes, project, version)?;
    let index = verified_index(
        &read(3, launcher_core::modpack::modrinth::MAX_MRPACK_BYTES)?,
        &artifact,
    )?;
    println!(
        "Modrinth: {} files, dependencies {:?}",
        index.files.len(),
        index.dependencies
    );
    if !index.supported_loader() || !custom.supported_loader() {
        return Err("unsupported acceptance loader".into());
    }
    Ok(())
}
