#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--loader-validation")) {
        validation_mode();
        return;
    }
    panita_desktop::run();
}

fn validation_mode() {
    #[cfg(not(feature = "loader-validation"))]
    std::process::exit(2);
    #[cfg(feature = "loader-validation")]
    {
        // Return before Tauri/profile/OAuth initialization. The optional core CLI
        // accepts only explicit isolated roots and never opens saved accounts.
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "cannot initialize validation runtime".to_owned())
            .and_then(|runtime| {
                runtime
                    .block_on(launcher_core::minecraft::validation::cli::run(
                        std::env::args().skip(2).collect(),
                    ))
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            eprintln!("Loader validation failed: {error}");
            std::process::exit(1);
        }
    }
}
