//! Development-only entry point shared with optional validation packages.
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    launcher_core::minecraft::validation::cli::run(std::env::args().skip(1).collect()).await
}
