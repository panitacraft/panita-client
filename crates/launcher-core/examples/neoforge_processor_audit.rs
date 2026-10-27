//! Development-only processor audit in a new isolated scratch root. Downloads
//! public inputs and runs verified Java processors; never opens a player profile,
//! reads accounts, checks the catalog, publishes an installation, or starts a game.
use launcher_core::{
    minecraft::{
        Platform,
        download::{Cancellation, Downloader},
        plan,
    },
    neoforge::{
        InstallerIdentity, MAX_INSTALLER_BYTES,
        plan::ProcessorPlan,
        staging::{self, StagingProgress},
    },
    paths::AppPaths,
};
use std::io::Read;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        return Err("usage: neoforge_processor_audit <installer.jar> <minecraft> <neoforge> <sha1> <size> <new-absolute-scratch-root>".into());
    }
    if std::env::consts::ARCH != "x86_64" {
        return Err("this native audit currently requires x86_64".into());
    }
    let identity = InstallerIdentity {
        minecraft: args[1].clone(),
        version: args[2].clone(),
        sha1: args[3].clone(),
        size: args[4].parse()?,
    };
    identity.url()?;
    let mut installer = Vec::new();
    std::fs::File::open(&args[0])?
        .take(MAX_INSTALLER_BYTES as u64 + 1)
        .read_to_end(&mut installer)?;
    let paths = AppPaths::new(&args[5])?;
    match std::fs::symlink_metadata(paths.root()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err("scratch root must be new; existing roots are preserved".into()),
    }
    let platform = Platform::x64(std::env::consts::OS, "")?;
    let cancellation = Cancellation::default();
    let downloader = Downloader::new()?;
    let base = plan::resolve(&downloader, &identity.minecraft, &platform, &cancellation).await?;
    let version = base
        .documents
        .iter()
        .find(|document| {
            document
                .source
                .url
                .ends_with(&format!("/{}.json", identity.minecraft))
        })
        .ok_or("missing verified game version source")?;
    let game = downloader
        .document(&version.source.url, &cancellation)
        .await?;
    let plan = ProcessorPlan::resolve(&installer, &identity, &game, &version.source)?;
    println!(
        "Resolved {} input files and {} local processors; Java {}",
        plan.inputs().len(),
        plan.processors().len(),
        plan.java().major_version
    );
    let result =
        staging::prepare(
            &paths,
            &plan,
            &installer,
            &base,
            &cancellation,
            &|event| match event {
                StagingProgress::Runtime { completed, total }
                    if completed % 50 == 0 || completed == total =>
                {
                    println!("Verified Java files: {completed}/{total}")
                }
                StagingProgress::Input { completed, total }
                    if completed % 10 == 0 || completed == total =>
                {
                    println!("Verified inputs: {completed}/{total}")
                }
                StagingProgress::Processor { completed, total } => {
                    println!("Verified processor outputs: {completed}/{total}")
                }
                _ => {}
            },
        )
        .await?;
    println!(
        "Completed isolated operation {}: {} derived files in {}",
        result.operation(),
        result.derived_files().len(),
        result.directory().display()
    );
    println!("No instance installation was published and no Minecraft game was launched.");
    Ok(())
}
