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
        variants: crate::catalog::standard_only(),
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
        ram_note: None,
    }
}

fn environment_changes(command: &tokio::process::Command) -> Vec<(String, Option<String>)> {
    command
        .as_std()
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect()
}

#[test]
fn game_environment_drops_desktop_identity_without_clearing_the_host_session() {
    let mut child = tokio::process::Command::new("java");
    child.env("DISPLAY", ":99");
    child.env("WAYLAND_DISPLAY", "wayland-test");
    child.env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/synthetic-bus");
    child.env("PULSE_PROP", "media.title=existing");
    child.env("PIPEWIRE_PROPS", "{ node.latency = 1024/48000 }");
    for key in JAVA_OPTIONS_ENVIRONMENT {
        child.env(key, "launcher-owned");
    }
    for key in DESKTOP_IDENTITY_ENVIRONMENT {
        child.env(key, "launcher-owned");
    }
    for key in AUDIO_IDENTITY_ENVIRONMENT {
        child.env(key, "launcher-owned");
    }

    sanitize_game_environment(&mut child);
    let changes = environment_changes(&child);
    for key in JAVA_OPTIONS_ENVIRONMENT {
        assert!(changes.contains(&(key.into(), None)), "{key}");
    }
    for key in DESKTOP_IDENTITY_ENVIRONMENT {
        assert!(changes.contains(&(key.into(), None)), "{key}");
    }
    for key in AUDIO_IDENTITY_ENVIRONMENT {
        assert!(changes.contains(&(key.into(), None)), "{key}");
    }
    for (key, value) in [
        ("DISPLAY", ":99"),
        ("WAYLAND_DISPLAY", "wayland-test"),
        ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/synthetic-bus"),
    ] {
        assert!(changes.contains(&(key.into(), Some(value.into()))), "{key}");
    }
    let pulse = changes
        .iter()
        .find(|(key, _)| key == "PULSE_PROP")
        .and_then(|(_, value)| value.as_deref())
        .unwrap_or_else(|| panic!("Pulse properties"));
    assert!(pulse.starts_with("media.title=existing "));
    assert!(pulse.ends_with(MINECRAFT_PULSE_PROPERTIES));
    let pipewire = changes
        .iter()
        .find(|(key, _)| key == "PIPEWIRE_PROPS")
        .and_then(|(_, value)| value.as_deref())
        .unwrap_or_else(|| panic!("PipeWire properties"));
    assert!(pipewire.contains("node.latency = 1024/48000"));
    assert!(pipewire.contains(MINECRAFT_PIPEWIRE_PROPERTY_BODY));
    assert!(!child.as_std().get_envs().any(|(key, _)| key == "HOME"));
}

#[test]
fn malformed_pipewire_properties_are_replaced_with_minecraft_identity() {
    assert_eq!(
        pipewire_properties(Some("not-a-property-object".into())),
        OsString::from(MINECRAFT_PIPEWIRE_PROPERTIES)
    );
    assert_eq!(
        pipewire_properties(Some("{ }".into())),
        OsString::from(MINECRAFT_PIPEWIRE_PROPERTIES)
    );
}

#[test]
fn appimage_cleanup_is_conditional_and_keeps_desktop_session_access() {
    let mut packaged = tokio::process::Command::new("java");
    for key in APPIMAGE_ENVIRONMENT {
        packaged.env(key, "/synthetic/appimage/value");
    }
    packaged.env("XDG_RUNTIME_DIR", "/run/user/1000");
    sanitize_appimage_environment(&mut packaged, true);
    let changes = environment_changes(&packaged);
    for key in APPIMAGE_ENVIRONMENT {
        assert!(changes.contains(&(key.into(), None)), "{key}");
    }
    assert!(changes.contains(&("XDG_RUNTIME_DIR".into(), Some("/run/user/1000".into()))));

    let mut installed = tokio::process::Command::new("java");
    installed.env("LD_LIBRARY_PATH", "/host/custom-libraries");
    sanitize_appimage_environment(&mut installed, false);
    assert!(environment_changes(&installed).contains(&(
        "LD_LIBRARY_PATH".into(),
        Some("/host/custom-libraries".into())
    )));
}

#[test]
fn appimage_scrub_drops_entries_below_the_mount_and_keeps_host_entries_in_order() {
    let appdir = std::path::Path::new("/tmp/.mount_panitaXYZ");
    let environment = [
        (
            "XDG_DATA_DIRS",
            "/tmp/.mount_panitaXYZ/usr/share/:/tmp/.mount_panitaXYZ/usr/share:/usr/share/ubuntu:/usr/local/share/:/usr/share/",
        ),
        (
            "GTK_IM_MODULE_FILE",
            "/tmp/.mount_panitaXYZ//usr/lib/gtk-3.0/3.0.0/immodules.cache",
        ),
        ("PATH", "/tmp/.mount_panitaXYZ/usr/bin:/usr/bin"),
        ("GTK_THEME", "Adwaita:dark"),
        ("HOME", "/home/player"),
        // The mount name only appears inside a longer, unrelated entry.
        ("NOTE", "/tmp/.mount_panitaXYZ-backup/share"),
    ]
    .map(|(key, value)| (OsString::from(key), OsString::from(value)));
    let changes = scrub_appdir(environment, appdir);
    assert_eq!(
        changes,
        [
            (
                OsString::from("XDG_DATA_DIRS"),
                Some(OsString::from(
                    "/usr/share/ubuntu:/usr/local/share/:/usr/share/"
                ))
            ),
            (OsString::from("GTK_IM_MODULE_FILE"), None),
            (OsString::from("PATH"), Some(OsString::from("/usr/bin"))),
        ]
    );
    assert!(scrub_appdir([], std::path::Path::new("")).is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn appimage_unsets_skip_keys_the_launcher_provides_for_the_game() {
    let mut builder = tokio::process::Command::new("/usr/bin/systemd-run");
    builder.env("LD_LIBRARY_PATH", "/data/runtime-libs/libdecor");
    unset_appimage_properties(&mut builder, true);
    let args: Vec<_> = builder
        .as_std()
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    assert!(!args.contains(&"--property=UnsetEnvironment=LD_LIBRARY_PATH".to_owned()));
    assert!(args.contains(&"--property=UnsetEnvironment=LD_PRELOAD".to_owned()));
    assert!(args.contains(&"--property=UnsetEnvironment=GTK_PATH".to_owned()));

    let mut outside = tokio::process::Command::new("/usr/bin/systemd-run");
    unset_appimage_properties(&mut outside, false);
    assert_eq!(outside.as_std().get_args().count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn systemd_service_builder_preserves_io_contract_without_exposing_environment_values() {
    let launch = LaunchCommand {
        executable: "/opt/runtime/java$/bin/java".into(),
        directory: "/tmp/game directory".into(),
        args: vec!["-Dfixture=$HOME".into(), "main.Class".into()],
        redactions: vec![],
    };
    let operation = OperationId::new();
    let tools = SystemdTools {
        run: "/usr/bin/systemd-run".into(),
        control: "/usr/bin/systemctl".into(),
    };
    let (builder, control) = systemd_builder(&launch, operation, &tools, &[]);
    let args: Vec<_> = builder
        .as_std()
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    for expected in [
        "--user",
        "--wait",
        "--pipe",
        "--collect",
        "--same-dir",
        "--service-type=exec",
        "--description=Minecraft",
        "--property=UnsetEnvironment=GIO_LAUNCHED_DESKTOP_FILE",
        "--setenv=PULSE_PROP",
        "--setenv=PIPEWIRE_PROPS",
        "/opt/runtime/java$$/bin/java",
        "-Dfixture=$$HOME",
    ] {
        assert!(
            args.iter().any(|argument| argument == expected),
            "{expected}"
        );
    }
    assert!(!args.iter().any(|argument| {
        argument.contains(MINECRAFT_PULSE_PROPERTIES)
            || argument.contains(MINECRAFT_PIPEWIRE_PROPERTIES)
    }));
    let expected_unit = format!("minecraft-{}.service", operation.value().simple());
    assert!(
        args.iter()
            .any(|argument| argument == &format!("--unit={expected_unit}"))
    );
    assert!(matches!(
        control,
        ProcessControl::SystemdService { unit, .. } if unit == expected_unit
    ));
    assert_eq!(
        builder.as_std().get_current_dir(),
        Some(launch.directory.as_path())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn fallback_runner_uses_literal_arguments_and_an_owned_process_group() {
    let launch = LaunchCommand {
        executable: "/opt/runtime/java with spaces/bin/java".into(),
        directory: "/tmp/game directory".into(),
        args: vec!["literal;argument".into(), "$HOME".into()],
        redactions: vec![],
    };
    let (builder, control) = fallback_builder(&launch, &[]).unwrap_or_else(|_| panic!("fallback"));
    let args: Vec<_> = builder.as_std().get_args().collect();
    assert_eq!(args[0], "-c");
    assert_eq!(args[2], "panita-minecraft-runner");
    assert_eq!(args[3], launch.executable.as_os_str());
    assert_eq!(args[4], "literal;argument");
    assert_eq!(args[5], "$HOME");
    assert!(matches!(control, ProcessControl::ProcessGroup));
}

#[cfg(target_os = "linux")]
fn fallback_fixture_command(test: &str) -> LaunchCommand {
    LaunchCommand {
        executable: std::env::current_exe().unwrap_or_else(|_| panic!("test executable")),
        directory: std::env::temp_dir(),
        args: vec![
            "--exact".into(),
            format!("minecraft::process::tests::{test}"),
            "--nocapture".into(),
        ],
        redactions: vec![],
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn fallback_runner_preserves_output_and_exit_status() {
    let launch = fallback_fixture_command("child_output_fixture");
    let (mut builder, _) = fallback_builder(&launch, &[]).unwrap_or_else(|_| panic!("fallback"));
    let mut child = builder.spawn().unwrap_or_else(|_| panic!("spawn"));
    let stdin_guard = child.stdin.take().unwrap_or_else(|| panic!("stdin guard"));
    let mut stdout = child.stdout.take().unwrap_or_else(|| panic!("stdout"));
    let mut stderr = child.stderr.take().unwrap_or_else(|| panic!("stderr"));
    let output = async move {
        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .await
            .unwrap_or_else(|_| panic!("stdout read"));
        bytes
    };
    let errors = async move {
        let mut bytes = Vec::new();
        stderr
            .read_to_end(&mut bytes)
            .await
            .unwrap_or_else(|_| panic!("stderr read"));
        bytes
    };
    let (output, errors, status) = tokio::join!(output, errors, child.wait());
    drop(stdin_guard);
    let status = status.unwrap_or_else(|_| panic!("wait"));
    assert_eq!(status.code(), Some(7));
    assert!(String::from_utf8_lossy(&output).contains("credential=fixture-access-token"));
    assert!(String::from_utf8_lossy(&errors).contains("stderr fixture-access-token"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn fallback_watchdog_terminates_the_group_when_supervision_disappears() {
    let launch = fallback_fixture_command("child_sleep_fixture");
    let (mut builder, _) = fallback_builder(&launch, &[]).unwrap_or_else(|_| panic!("fallback"));
    let mut child = builder.spawn().unwrap_or_else(|_| panic!("spawn"));
    let stdin_guard = child.stdin.take().unwrap_or_else(|| panic!("stdin guard"));
    drop(stdin_guard);
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap_or_else(|_| panic!("watchdog timeout"))
        .unwrap_or_else(|_| panic!("wait"));
    assert!(!status.success());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn fallback_graceful_stop_targets_java_and_reaps_the_runner() {
    let launch = fallback_fixture_command("child_sleep_fixture");
    let (mut builder, control) =
        fallback_builder(&launch, &[]).unwrap_or_else(|_| panic!("fallback"));
    let mut child = builder.spawn().unwrap_or_else(|_| panic!("spawn"));
    let stdin_guard = child.stdin.take().unwrap_or_else(|| panic!("stdin guard"));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        stop_child(&mut child, &control, false).await,
        Ok(StopOutcome::Requested)
    );
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap_or_else(|_| panic!("graceful stop timeout"))
        .unwrap_or_else(|_| panic!("wait"));
    drop(stdin_guard);
    assert!(!status.success());
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
    println!("X-Api-Key: fixture-launcher-key");
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
    assert!(!format!("{events:?}").contains("fixture-launcher-key"));
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
    let stopped = tokio::time::timeout(Duration::from_secs(10), game.wait()).await;
    assert!(stopped.is_ok(), "events: {:?}", game.events());
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
async fn output_history_is_unlimited_paged_and_unterminated_secrets_are_redacted() {
    let (paths, id) = fixture();
    let command = Arc::new(command(&paths, id, "unused"));
    let events = Arc::new(Events {
        process: ProcessSessionId::new(),
        operation: OperationId::new(),
        instance: id,
        records: Mutex::new(VecDeque::new()),
        changed: watch::channel(0).0,
    });
    let mut changes = events.changed.subscribe();
    let input = format!("{}final {TOKEN}", "line\n".repeat(10_000));
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
    assert_eq!(records.len(), 10_001);
    assert!(matches!(&records[0].kind, ProcessEventKind::Output { text, .. } if text == "line"));
    assert!(
        matches!(&records.back().unwrap_or_else(|| panic!("last event")).kind, ProcessEventKind::Output { text, .. } if text == "final [redacted]")
    );
    assert!(!format!("{records:?}").contains(TOKEN));
    drop(records);
    let mut offset = 0;
    while offset < 10_001 {
        let page = events.page(Some(offset), usize::MAX);
        assert_eq!(page.offset, offset);
        assert_eq!(page.total, 10_001);
        assert!(page.records.len() <= OUTPUT_PAGE_SIZE);
        assert!(!page.records.is_empty());
        offset += page.records.len();
    }
    assert_eq!(offset, 10_001);
    let tail = events.page(None, OUTPUT_PAGE_SIZE);
    assert_eq!(tail.offset, 10_001 - OUTPUT_PAGE_SIZE);
    assert!(
        matches!(&tail.records.last().unwrap_or_else(|| panic!("last")).kind,
        ProcessEventKind::Output { text, .. } if text == "final [redacted]")
    );
    let empty = events.page(Some(usize::MAX), 0);
    assert_eq!(empty.offset, 10_001);
    assert!(empty.records.is_empty());
    assert!(changes.has_changed().unwrap_or(false));
    assert_eq!(*changes.borrow_and_update(), 10_001);
    assert!(!changes.has_changed().unwrap_or(true));
    let first = events
        .find("FINAL", 0, false)
        .unwrap_or_else(|_| panic!("search"));
    assert_eq!(first.found, None);
    assert_eq!(first.next, OUTPUT_PAGE_SIZE);
    let last = events
        .find("FINAL", 10_000, false)
        .unwrap_or_else(|_| panic!("search"));
    assert_eq!(last.found, Some(10_000));
    let reverse = events
        .find("final", usize::MAX, true)
        .unwrap_or_else(|_| panic!("search"));
    assert_eq!(reverse.found, Some(10_000));
    assert_eq!(reverse.next, 10_000);
    let previous = events
        .find("line", reverse.next, true)
        .unwrap_or_else(|_| panic!("search"));
    assert_eq!(previous.found, Some(9999));
    assert_eq!(
        events
            .find(TOKEN, 10_000, false)
            .ok()
            .and_then(|result| result.found),
        None
    );
    for query in ["", " \t", "line\n", &"x".repeat(1025)] {
        assert!(events.find(query, 0, false).is_err());
    }
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn log_context_is_redacted_before_retention_and_survives_page_boundaries() {
    let (paths, id) = fixture();
    let command = Arc::new(command(&paths, id, "unused"));
    let events = Arc::new(Events {
        process: ProcessSessionId::new(),
        operation: OperationId::new(),
        instance: id,
        records: Mutex::new(VecDeque::new()),
        changed: watch::channel(0).0,
    });
    let input = format!(
        "<log4j:Event logger=\"{TOKEN}\" level=\"INFO\">\n<log4j:Message><![CDATA[start\n{}end]]></log4j:Message></log4j:Event>\nplain",
        "payload\n".repeat(600)
    );
    read_output(
        Some(input.as_bytes()),
        OutputStream::Stdout,
        events.clone(),
        command,
    )
    .await;
    let page = events.page(Some(500), OUTPUT_PAGE_SIZE);
    assert!(page.records[0].cdata);
    assert!(
        page.records[0]
            .log_context
            .as_deref()
            .is_some_and(|header| header.contains("[redacted]"))
    );
    assert!(!format!("{:?}", page.records).contains(TOKEN));
    let last = page.records.last().unwrap_or_else(|| panic!("last"));
    assert_eq!(last.log_context, None);
    assert!(!last.cdata);
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
