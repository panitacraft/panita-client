use super::*;
use crate::accounts::Secret;
use crate::catalog::{CatalogInstance, CatalogSource};
use crate::registry::{InstallationRecord, InstallationState};
use crate::settings::InstanceSettings;
use std::io::Write;

const TOKEN: &str = "fixture-access-token";

fn fixture() -> (AppPaths, Uuid) {
    let paths =
        AppPaths::new(std::env::temp_dir().join(format!("panita-process-{}", Uuid::new_v4())))
            .unwrap_or_else(|_| panic!("paths"));
    let id = Uuid::new_v4();
    let catalog = CatalogInstance {
        id,
        slug: "process-test".into(),
        name: "Prueba".into(),
        description: "Prueba".into(),
        minecraft_version: Some("1.21.1".into()),
        enabled: true,
        restricted: false,
        min_launcher_version: None,
        icon_url: None,
        banner_url: None,
        sort_order: 0,
        updated_at: "2026-10-17T00:00:00Z".into(),
        source: CatalogSource::Vanilla {
            minecraft_version: "1.21.1".into(),
        },
    };
    InstanceRegistry::new(paths.clone())
        .save(&InstallationRecord {
            schema_version: 1,
            id,
            directory_name: None,
            resolved_source: catalog.source.clone(),
            last_catalog: catalog,
            state: InstallationState::NotInstalled,
            managed_files: vec![],
            settings: InstanceSettings::default(),
        })
        .unwrap_or_else(|_| panic!("registry"));
    (paths, id)
}

fn command(paths: &AppPaths, id: Uuid, test: &str) -> LaunchCommand {
    LaunchCommand {
        executable: std::env::current_exe().unwrap_or_else(|_| panic!("test executable")),
        directory: paths
            .game_directory(id)
            .unwrap_or_else(|_| panic!("game directory")),
        args: vec![
            "--exact".into(),
            format!("minecraft::process::tests::{test}"),
            "--nocapture".into(),
        ],
        redactions: vec![Secret::new(TOKEN.into()).unwrap_or_else(|_| panic!("fixture secret"))],
    }
}

#[test]
fn child_output_fixture() {
    // Invoked only as an exact child test. Ordinary workspace runs do no work.
    if !std::env::args().any(|arg| arg == "--exact") {
        return;
    }
    print!("credential=fixture-");
    std::io::stdout()
        .flush()
        .unwrap_or_else(|_| panic!("flush"));
    std::thread::sleep(Duration::from_millis(20));
    println!("access-token");
    eprintln!("stderr {TOKEN}");
    println!("Authorization: Bearer unrecognized-sensitive-value");
    println!("{}{}", "x".repeat(MAX_LINE + 1), TOKEN);
    std::process::exit(7);
}

#[test]
fn child_sleep_fixture() {
    if !std::env::args().any(|arg| arg == "--exact") {
        return;
    }
    std::thread::sleep(Duration::from_secs(30));
}

#[tokio::test]
async fn captures_split_secret_both_streams_and_nonzero_exit_without_leaks() {
    let (paths, id) = fixture();
    let command = command(&paths, id, "child_output_fixture");
    let process = ProcessSessionId::new();
    let operation = OperationId::new();
    let game = LaunchLease::acquire(&paths, id)
        .unwrap_or_else(|_| panic!("lease"))
        .spawn(command, process, operation)
        .unwrap_or_else(|_| panic!("spawn"));
    let status = tokio::time::timeout(Duration::from_secs(10), game.wait())
        .await
        .unwrap_or_else(|_| panic!("child timeout"))
        .unwrap_or_else(|_| panic!("wait"));
    assert_eq!(
        status,
        GameExit {
            code: Some(7),
            success: false
        }
    );
    let events = game.events();
    assert!(events.iter().all(|event| event.instance == id
        && event.operation == operation
        && event.process == process));
    assert!(matches!(
        events.first().map(|event| &event.kind),
        Some(ProcessEventKind::Started)
    ));
    assert!(matches!(
        events.last().map(|event| &event.kind),
        Some(ProcessEventKind::Exited(_))
    ));
    for stream in [OutputStream::Stdout, OutputStream::Stderr] {
        assert!(events.iter().any(|event| matches!(&event.kind, ProcessEventKind::Output { stream: source, text } if *source == stream && text.contains("[redacted]"))));
    }
    assert!(events.iter().any(|event| matches!(
        event.kind,
        ProcessEventKind::OutputDiscarded {
            stream: OutputStream::Stdout
        }
    )));
    assert!(!format!("{events:?}").contains(TOKEN));
    assert!(!format!("{events:?}").contains("unrecognized-sensitive-value"));
    assert!(events.iter().any(|event| matches!(&event.kind, ProcessEventKind::Output { text, .. } if text == "[sensitive output omitted]")));
    assert_eq!(
        game.force_stop().await.ok(),
        Some(StopOutcome::AlreadyExited)
    );
    assert!(LaunchLease::acquire(&paths, id).is_ok());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn lease_excludes_duplicate_launch_and_mutation_until_force_stop() {
    let (paths, id) = fixture();
    let command = command(&paths, id, "child_sleep_fixture");
    let lease = LaunchLease::acquire(&paths, id).unwrap_or_else(|_| panic!("lease"));
    // Existing audit APIs must not try to acquire this lease a second time.
    assert_eq!(
        lease.verify_installation().err().map(|error| error.code()),
        Some(ErrorCode::InvalidData)
    );
    let game = lease
        .spawn(command, ProcessSessionId::new(), OperationId::new())
        .unwrap_or_else(|_| panic!("spawn"));
    let busy = LaunchLease::acquire(&paths, id)
        .err()
        .map(|error| error.code());
    let root = paths.instance(id).unwrap_or_else(|_| panic!("root"));
    let blocked_writer = try_lock(&root, ".installation.lock")
        .unwrap_or_else(|_| panic!("lock"))
        .is_none();
    #[cfg(windows)]
    assert_eq!(
        game.request_stop().await.ok(),
        Some(StopOutcome::Unsupported)
    );
    assert_eq!(game.force_stop().await.ok(), Some(StopOutcome::Requested));
    assert!(
        tokio::time::timeout(Duration::from_secs(10), game.wait())
            .await
            .is_ok()
    );
    assert_eq!(busy, Some(ErrorCode::OperationBusy));
    assert!(blocked_writer);
    assert!(LaunchLease::acquire(&paths, id).is_ok());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn dropping_ui_handle_keeps_child_lease_until_exit() {
    let (paths, id) = fixture();
    let command = command(&paths, id, "child_output_fixture");
    let game = LaunchLease::acquire(&paths, id)
        .unwrap_or_else(|_| panic!("lease"))
        .spawn(command, ProcessSessionId::new(), OperationId::new())
        .unwrap_or_else(|_| panic!("spawn"));
    let mut exit = game.exit.clone();
    drop(game);
    // The actor, not a UI subscription, retains ownership of the child and lock.
    tokio::time::timeout(Duration::from_secs(10), async {
        while exit.borrow_and_update().is_none() {
            exit.changed()
                .await
                .unwrap_or_else(|_| panic!("actor stopped"));
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timeout"));
    assert!(LaunchLease::acquire(&paths, id).is_ok());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn spawn_failure_releases_lease_and_returns_only_fixed_error() {
    let (paths, id) = fixture();
    let mut command = command(&paths, id, "child_sleep_fixture");
    command.executable = paths.root().join("missing-executable");
    let error = LaunchLease::acquire(&paths, id)
        .unwrap_or_else(|_| panic!("lease"))
        .spawn(command, ProcessSessionId::new(), OperationId::new())
        .err()
        .unwrap_or_else(|| panic!("expected spawn failure"));
    assert_eq!(error.code(), ErrorCode::ProcessStartFailed);
    assert!(!error.to_string().contains(TOKEN));
    assert!(LaunchLease::acquire(&paths, id).is_ok());
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn output_flood_is_bounded_and_unterminated_secret_is_redacted() {
    let (paths, id) = fixture();
    let command = Arc::new(command(&paths, id, "unused"));
    let events = Arc::new(Events {
        process: ProcessSessionId::new(),
        operation: OperationId::new(),
        instance: id,
        records: Mutex::new(VecDeque::new()),
    });
    let input = format!("{}final {TOKEN}", "line\n".repeat(400));
    read_output(
        Some(input.as_bytes()),
        OutputStream::Stdout,
        events.clone(),
        command,
    )
    .await;
    let records = events
        .records
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert_eq!(records.len(), MAX_EVENTS);
    assert!(
        matches!(&records.back().unwrap_or_else(|| panic!("last event")).kind, ProcessEventKind::Output { text, .. } if text == "final [redacted]")
    );
    assert!(!format!("{records:?}").contains(TOKEN));
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn graceful_stop_uses_the_owned_unreaped_child() {
    let (paths, id) = fixture();
    let command = command(&paths, id, "child_sleep_fixture");
    let game = LaunchLease::acquire(&paths, id)
        .unwrap_or_else(|_| panic!("lease"))
        .spawn(command, ProcessSessionId::new(), OperationId::new())
        .unwrap_or_else(|_| panic!("spawn"));
    assert_eq!(game.request_stop().await.ok(), Some(StopOutcome::Requested));
    assert!(
        tokio::time::timeout(Duration::from_secs(10), game.wait())
            .await
            .is_ok()
    );
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}
