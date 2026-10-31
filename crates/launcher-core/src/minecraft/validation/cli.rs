//! Development native qualification in an explicit account-free scratch root.
//! Build with --features loader-validation. This is not a supported player CLI.
use crate::{
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

pub async fn run(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() == 7 && args[0] == "prepare" {
        return super::prepare::run(&args[1..]).await;
    }
    if args.len() != 2 {
        return Err(
            "usage: --loader-validation <install|verify|launch> <absolute-phase-9-scratch-root>; prepare <installer.jar> <minecraft> <neoforge> <sha1> <size> <new-root>"
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
