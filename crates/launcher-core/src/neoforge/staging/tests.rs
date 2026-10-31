use super::*;

fn workspace() -> Result<Workspace, CoreError> {
    let paths = AppPaths::new(
        std::env::temp_dir().join(format!("panita-neoforge-stage-{}", Uuid::new_v4())),
    )?;
    Workspace::create(&paths)
}

#[test]
fn processor_child_fixture() {
    let Ok(mode) = std::env::var("PANITA_PROCESSOR_TEST_MODE") else {
        return;
    };
    match mode.as_str() {
        "fail" => std::process::exit(17),
        "sleep" => std::thread::sleep(Duration::from_secs(30)),
        "marker" => {
            if let Some(path) = std::env::var_os("PANITA_PROCESSOR_TEST_MARKER") {
                std::fs::write(path, b"spawned").unwrap_or_else(|_| panic!("marker"));
            }
        }
        "flood" => {
            for _ in 0..400 {
                println!("{}", "private synthetic processor output".repeat(500));
                eprintln!("{}", "private synthetic processor error".repeat(500));
            }
        }
        _ => {}
    }
}

fn child(mode: &str) -> Result<tokio::process::Command, std::io::Error> {
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .args([
            "--exact",
            "neoforge::staging::tests::processor_child_fixture",
            "--nocapture",
        ])
        .env("PANITA_PROCESSOR_TEST_MODE", mode);
    Ok(command)
}

#[tokio::test]
async fn child_failures_floods_timeouts_and_cancellation_are_bounded_and_reaped()
-> Result<(), Box<dyn std::error::Error>> {
    let cancellation = Cancellation::default();
    let result = run_child(
        child("fail")?,
        &cancellation,
        Duration::from_secs(10),
        false,
    )
    .await?;
    assert!(!result.success);
    let result = run_child(
        child("flood")?,
        &cancellation,
        Duration::from_secs(20),
        false,
    )
    .await?;
    assert!(result.success);
    assert!(result.stderr.is_empty());
    let timed_out = run_child(
        child("sleep")?,
        &cancellation,
        Duration::from_millis(100),
        false,
    )
    .await;
    assert_eq!(
        timed_out.err().map(|error| error.code()),
        Some(ErrorCode::LoaderProcessorTimeout)
    );
    let cancellation = Cancellation::default();
    let cancel = cancellation.clone();
    let pending = run_child(
        child("sleep")?,
        &cancellation,
        Duration::from_secs(20),
        false,
    );
    let cancel = async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(pending, cancel);
    assert_eq!(
        result.err().map(|error| error.code()),
        Some(ErrorCode::OperationCancelled)
    );
    cancellation.cancel();
    let marker = std::env::temp_dir().join(format!("panita-no-spawn-{}", Uuid::new_v4()));
    let mut prevented = child("marker")?;
    prevented.env("PANITA_PROCESSOR_TEST_MARKER", &marker);
    let result = run_child(prevented, &cancellation, Duration::from_secs(10), false).await;
    assert_eq!(
        result.err().map(|error| error.code()),
        Some(ErrorCode::OperationCancelled)
    );
    assert!(!marker.exists());
    Ok(())
}

#[test]
fn generated_outputs_require_content_and_exact_hash_and_reject_aliases()
-> Result<(), Box<dyn std::error::Error>> {
    let stage = workspace()?;
    let cancel = Cancellation::default();
    stage.write("libraries/output.jar", b"derived output")?;
    let hash = format!("{:x}", Sha1::digest(b"derived output"));
    let receipt = stage.generated("libraries/output.jar", Some(&hash), &cancel)?;
    assert_eq!(receipt.size, 14);
    assert_eq!(
        receipt.sha256,
        format!("{:x}", Sha256::digest(b"derived output"))
    );
    assert!(
        stage
            .generated("libraries/output.jar", Some(&"0".repeat(40)), &cancel)
            .is_err()
    );
    stage.write("libraries/empty.jar", b"")?;
    assert!(
        stage
            .generated("libraries/empty.jar", None, &cancel)
            .is_err()
    );
    assert!(
        stage
            .generated("libraries/missing.jar", None, &cancel)
            .is_err()
    );
    assert!(stage.generated("../outside.jar", None, &cancel).is_err());
    std::fs::hard_link(
        stage.root.join("libraries/output.jar"),
        stage.root.join("libraries/alias.jar"),
    )?;
    assert!(
        stage
            .generated("libraries/alias.jar", None, &cancel)
            .is_err()
    );
    assert!(
        stage
            .generated("libraries/output.jar", None, &cancel)
            .is_err()
    );
    Ok(())
}

#[test]
fn staging_copies_never_modify_cache_and_paths_with_spaces_remain_one_argument()
-> Result<(), Box<dyn std::error::Error>> {
    let stage = workspace()?;
    stage.write("cache/input.jar", b"verified source")?;
    let source = Artifact {
        url: String::new(),
        sha1: format!("{:x}", Sha1::digest(b"verified source")),
        size: 15,
        path: None,
    };
    stage.copy(
        &stage.root.join("cache/input.jar"),
        "libraries/path with spaces/input.jar",
        &source,
        &Cancellation::default(),
    )?;
    std::fs::write(
        stage.root.join("libraries/path with spaces/input.jar"),
        b"modified staging",
    )?;
    assert_eq!(
        std::fs::read(stage.root.join("cache/input.jar"))?,
        b"verified source"
    );
    let value = stage.path_text(
        "libraries/path with spaces/input.jar",
        if cfg!(windows) { ';' } else { ':' },
    )?;
    assert!(value.contains("path with spaces"));
    assert!(stage.path_text("../outside", ';').is_err());
    Ok(())
}

#[test]
fn manifests_accept_continuations_but_reject_ambiguous_or_missing_main_classes()
-> Result<(), CoreError> {
    assert_eq!(main_class(b"Manifest-Version: 1.0\r\nMain-Class: a.b.\r\n Main\r\n\r\nName: section\r\nMain-Class: evil.Main\r\n")?, "a.b.Main");
    for bad in [
        b"Main-Class: a.Main\nMain-Class: b.Main\n".as_slice(),
        b"Main-Class: a/b\n",
        b"Main-Class: \n",
        b"Name: other\n\nMain-Class: x.Main\n",
    ] {
        assert!(main_class(bad).is_err());
    }
    Ok(())
}

#[test]
fn rejects_redirected_output_directories_without_touching_outside_data()
-> Result<(), Box<dyn std::error::Error>> {
    let stage = workspace()?;
    let outside = workspace()?;
    outside.write("sentinel.jar", b"outside data")?;
    #[cfg(windows)]
    {
        // Directory junctions need no administrator/Developer Mode privilege.
        let result = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(
                stage
                    .root
                    .join("redirect")
                    .components()
                    .collect::<PathBuf>(),
            )
            .arg(outside.root.components().collect::<PathBuf>())
            .output()?;
        assert!(result.status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside.root, stage.root.join("redirect"))?;
    assert!(
        stage
            .generated("redirect/sentinel.jar", None, &Cancellation::default())
            .is_err()
    );
    assert_eq!(
        std::fs::read(outside.root.join("sentinel.jar"))?,
        b"outside data"
    );
    Ok(())
}
