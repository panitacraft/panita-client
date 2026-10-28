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
    if args.len() == 2 && matches!(args[0].as_str(), "verify" | "repair") {
        let paths = AppPaths::new(&args[1])?;
        let directory = private_fs::PrivateDir::open(paths.root())?;
        let mut bytes = Vec::new();
        directory
            .open_file("prepared-plan.json")?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("prepared plan exceeds limit".into());
        }
        let mut base: plan::InstallPlan = serde_json::from_slice(&bytes)?;
        base.validate()?;
        let loader = base.loader.as_ref().ok_or("missing loader plan")?;
        let cancel = Cancellation::default();
        if args[0] == "repair" {
            let replacement = loader.repair(&paths, &base, &cancel).await?;
            base.loader = Some(replacement);
            let mut bytes = serde_json::to_vec_pretty(&base)?;
            bytes.push(b'\n');
            directory.write_atomic_durable("prepared-plan.json", &bytes)?;
        }
        let loader = base.loader.as_ref().ok_or("loader")?;
        loader.verify(&paths, &cancel)?;
        println!(
            "Verified exact {} / NeoForge {} loader generation: {}",
            loader.minecraft(),
            loader.version(),
            loader.directory()
        );
        println!(
            "This scratch plan is not a complete game installation or a player Play authorization."
        );
        return Ok(());
    }
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
    let mut base =
        plan::resolve(&downloader, &identity.minecraft, &platform, &cancellation).await?;
    base.allocate_runtime(&paths)?;
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
    let loader =
        launcher_core::neoforge::generation::publish(&paths, &plan, result, &cancellation)?;
    base.schema_version = 2;
    base.loader = Some(loader.into());
    base.validate()?;
    let mut bytes = serde_json::to_vec_pretty(&base)?;
    bytes.push(b'\n');
    private_fs::PrivateDir::open(paths.root())?
        .write_atomic_durable("prepared-plan.json", &bytes)?;
    println!("Saved the exact prepared plan for independent restart/repair verification.");
    println!("No instance installation was published and no Minecraft game was launched.");
    Ok(())
}
