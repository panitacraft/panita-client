//! Development native qualification in an explicit account-free scratch root.
//! Build with --features loader-validation. This is not a supported player CLI.
use launcher_core::{
    minecraft::{download::Cancellation, validation},
    paths::AppPaths,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err(
            "usage: loader_validation <install|verify|launch> <absolute-phase-9-scratch-root>"
                .into(),
        );
    }
    let paths = AppPaths::new(&args[1])?;
    let cancel = Cancellation::default();
    match args[0].as_str() {
        "install" => {
            let completed = Arc::new(AtomicUsize::new(0));
            let target = validation::install(
                &paths,
                &cancel,
                Arc::new(move |progress| {
                    if progress.bytes == progress.total {
                        let count = completed.fetch_add(1, Ordering::SeqCst) + 1;
                        if count.is_multiple_of(250) {
                            println!("Verified game artifacts: {count}");
                        }
                    }
                }),
            )
            .await?;
            println!(
                "Published isolated loader-only installation {}",
                target.id()
            );
        }
        "verify" => {
            let installed = validation::verify(&paths, &cancel)?;
            println!(
                "Verified installed loader candidate {} / {}",
                installed.instance_id, installed.plan.version.id
            );
        }
        "launch" => {
            let game = validation::launch(&paths, &cancel)?;
            println!(
                "Started isolated offline loader candidate; close its game window after menu inspection."
            );
            let exit = tokio::select! {
                result = game.wait() => result?,
                _ = tokio::time::sleep(Duration::from_secs(600)) => { game.force_stop().await?; game.wait().await? },
            };
            println!(
                "Game exited: success={}, code={:?}",
                exit.success, exit.code
            );
            if !exit.success {
                return Err("validation game failed; retained output remains in the supervisor until this process exits".into());
            }
        }
        _ => return Err("unsupported validation action".into()),
    }
    Ok(())
}
