//! Read-only contract audit of an already downloaded, pinned official installer.
//! Does not contact services, read launcher profiles, extract files, or run Java.
use launcher_core::neoforge::{InstallerIdentity, InstallerMetadata, MAX_INSTALLER_BYTES};
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err(
            "usage: neoforge_contract_audit <installer.jar> <minecraft> <neoforge> <sha1> <size>"
                .into(),
        );
    }
    let expected = InstallerIdentity {
        minecraft: args[1].clone(),
        version: args[2].clone(),
        sha1: args[3].clone(),
        size: args[4].parse()?,
    };
    expected.url()?;
    let mut bytes = Vec::new();
    std::fs::File::open(&args[0])?
        .take(MAX_INSTALLER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let metadata = InstallerMetadata::decode(&bytes, &expected)?;
    println!(
        "Verified Minecraft {} / NeoForge {}: {} installer libraries, {} launch libraries, {} client processors",
        expected.minecraft,
        expected.version,
        metadata.install.libraries.len(),
        metadata.launch.libraries.len(),
        metadata
            .install
            .processors
            .iter()
            .filter(|p| p.runs_on_client())
            .count()
    );
    Ok(())
}
