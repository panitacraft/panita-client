//! Development validation harness; this is not a supported product CLI.
//! All output and game files go to the explicitly supplied isolated data root.
use launcher_core::{
    error::CoreError,
    minecraft::{
        Platform,
        download::{Cancellation, Downloader},
        install, plan,
    },
    paths::AppPaths,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("usage: vanilla_install <plan|install|verify|recover|migrate> <absolute-isolated-root> <version|instance-uuid> <windows|linux> <os-version>".into());
    }
    let paths = AppPaths::new(&args[1])?;
    let platform = Platform::x64(&args[3], &args[4])?;
    let cancel = Cancellation::default();
    if args[0] == "migrate" {
        let id = uuid::Uuid::parse_str(&args[2])?;
        let directory =
            launcher_core::registry::InstanceRegistry::new(paths).prepare_instance(id)?;
        println!(
            "{}",
            serde_json::json!({"instance": id, "directory": directory})
        );
        return Ok(());
    }
    if args[0] == "recover" {
        let id = uuid::Uuid::parse_str(&args[2])?;
        install::recover(&paths, id, &cancel).await?;
        let record = launcher_core::registry::InstanceRegistry::new(paths).get(id)?;
        println!(
            "{}",
            serde_json::json!({ "instance": id, "state": record.map(|record| record.state) })
        );
        return Ok(());
    }
    let plan = match args[0].as_str() {
        "plan" => plan::resolve(&Downloader::new()?, &args[2], &platform, &cancel).await?,
        "install" => {
            if std::env::consts::OS != platform.os || std::env::consts::ARCH != "x86_64" {
                return Err("installation validation requires the native target platform".into());
            }
            let counts = std::sync::Arc::new((
                std::sync::atomic::AtomicUsize::new(0),
                std::sync::atomic::AtomicUsize::new(0),
            ));
            let progress_counts = counts.clone();
            let progress = std::sync::Arc::new(
                move |progress: launcher_core::minecraft::download::DownloadProgress| {
                    if progress.bytes == progress.total {
                        let counter = if progress.reused {
                            &progress_counts.0
                        } else {
                            &progress_counts.1
                        };
                        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let files = progress_counts.0.load(std::sync::atomic::Ordering::SeqCst)
                            + progress_counts.1.load(std::sync::atomic::Ordering::SeqCst);
                        if files.is_multiple_of(250) {
                            eprintln!("Verified {files} artifacts");
                        }
                    }
                },
            );
            let installed = install::install(
                &paths,
                uuid::Uuid::parse_str(&args[2])?,
                &platform,
                &cancel,
                progress,
            )
            .await?;
            eprintln!(
                "{}",
                serde_json::json!({ "reused_artifacts": counts.0.load(std::sync::atomic::Ordering::SeqCst), "downloaded_artifacts": counts.1.load(std::sync::atomic::Ordering::SeqCst) })
            );
            installed.plan
        }
        "verify" => install::verify_installed(&paths, uuid::Uuid::parse_str(&args[2])?)?.plan,
        _ => return Err("unknown validation command".into()),
    };
    if args[0] == "plan" {
        let root = private_fs::PrivateDir::ensure(paths.root())
            .map_err(|_| CoreError::new(launcher_core::error::ErrorCode::StorageUnavailable))?;
        root.write_atomic_durable(
            format!("plan-{}-{}.json", plan.version.id, plan.platform),
            &serde_json::to_vec_pretty(&plan)?,
        )?;
    }
    println!(
        "{}",
        serde_json::json!({ "version": plan.version.id, "platform": plan.platform, "java_major": plan.version.java.major_version, "artifacts": plan.artifacts.len(), "bytes": plan.artifacts.iter().map(|file| file.artifact.size).sum::<u64>(), "metadata_documents": plan.documents.len(), "instance_directory": if args[0] == "plan" { None } else { Some(paths.instance(uuid::Uuid::parse_str(&args[2])?)?) } })
    );
    Ok(())
}
