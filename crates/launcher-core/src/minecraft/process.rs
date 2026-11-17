//! Low-level owned-child supervision. Callers must complete Play preflight before
//! spawning and keep the Tokio runtime alive until the supervised child exits.
//! This module does not authorize a launch or implement desktop close behavior.
#[cfg(target_os = "linux")]
use std::collections::BTreeMap;
use std::collections::VecDeque;
#[cfg(any(target_os = "linux", test))]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;
use zeroize::Zeroize;

use super::download::{ArtifactLock, try_lock};
use super::launch::LaunchCommand;
use crate::error::{CoreError, ErrorCode};
use crate::operation::{OperationId, ProcessSessionId};
use crate::paths::AppPaths;
use crate::registry::InstanceRegistry;

const MAX_LINE: usize = 8192;
pub const OUTPUT_PAGE_SIZE: usize = 500;

/// Desktop launch context belongs to the launcher process, not to a game it starts.
/// Some Linux shells use these values while matching a new window to a desktop entry;
/// allowing them to reach Java can therefore group Minecraft under Panita Client.
const DESKTOP_IDENTITY_ENVIRONMENT: [&str; 7] = [
    "GIO_LAUNCHED_DESKTOP_FILE",
    "GIO_LAUNCHED_DESKTOP_FILE_PID",
    "BAMF_DESKTOP_FILE_HINT",
    "CHROME_DESKTOP",
    "DESKTOP_STARTUP_ID",
    "XDG_ACTIVATION_TOKEN",
    "__CFBundleIdentifier",
];

const JAVA_OPTIONS_ENVIRONMENT: [&str; 3] =
    ["JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JDK_JAVA_OPTIONS"];

/// PulseAudio also accepts per-property variables whose names contain dots.
/// Remove inherited launcher values before publishing Minecraft's aggregate
/// property list, which also works when the Pulse client is backed by PipeWire.
#[cfg(any(target_os = "linux", test))]
const AUDIO_IDENTITY_ENVIRONMENT: [&str; 4] = [
    "PULSE_PROP_application.name",
    "PULSE_PROP_application.id",
    "PULSE_PROP_application.icon_name",
    "PULSE_PROP_media.role",
];

#[cfg(any(target_os = "linux", test))]
const MINECRAFT_PULSE_PROPERTIES: &str = "application.name=Minecraft application.id=com.panitacraft.minecraft application.icon_name=minecraft media.role=game";
#[cfg(any(target_os = "linux", test))]
const MINECRAFT_PIPEWIRE_PROPERTIES: &str = "{ application.name = \"Minecraft\" application.id = \"com.panitacraft.minecraft\" application.icon-name = \"minecraft\" media.role = \"Game\" }";
#[cfg(any(target_os = "linux", test))]
const MINECRAFT_PIPEWIRE_PROPERTY_BODY: &str = "application.name = \"Minecraft\" application.id = \"com.panitacraft.minecraft\" application.icon-name = \"minecraft\" media.role = \"Game\"";

/// AppImage runtime paths point into the launcher's mounted image. They can make
/// an independently installed Java load launcher libraries or desktop modules.
#[cfg(any(target_os = "linux", test))]
const APPIMAGE_ENVIRONMENT: [&str; 14] = [
    "APPIMAGE",
    "APPDIR",
    "ARGV0",
    "OWD",
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "GIO_EXTRA_MODULES",
    "GIO_MODULE_DIR",
    "GSETTINGS_SCHEMA_DIR",
    "GTK_PATH",
    "GTK_EXE_PREFIX",
    "GTK_DATA_PREFIX",
    "GDK_PIXBUF_MODULE_FILE",
    "GDK_PIXBUF_MODULEDIR",
];

fn sanitize_game_environment(command: &mut tokio::process::Command) {
    for key in JAVA_OPTIONS_ENVIRONMENT {
        command.env_remove(key);
    }
    for key in DESKTOP_IDENTITY_ENVIRONMENT {
        command.env_remove(key);
    }
    #[cfg(target_os = "linux")]
    sanitize_appimage_environment(command, std::env::var_os("APPIMAGE").is_some());
    #[cfg(any(target_os = "linux", test))]
    {
        for key in AUDIO_IDENTITY_ENVIRONMENT {
            command.env_remove(key);
        }
        publish_game_audio_identity(command);
    }
}

#[cfg(any(target_os = "linux", test))]
fn sanitize_appimage_environment(command: &mut tokio::process::Command, appimage: bool) {
    if appimage {
        for key in APPIMAGE_ENVIRONMENT {
            command.env_remove(key);
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn command_environment_value(command: &tokio::process::Command, key: &str) -> Option<OsString> {
    match command.as_std().get_envs().find(|(name, _)| *name == key) {
        Some((_, value)) => value.map(OsStr::to_os_string),
        None => std::env::var_os(key),
    }
}

#[cfg(any(target_os = "linux", test))]
fn append_environment_properties(existing: Option<OsString>, properties: &str) -> OsString {
    let mut value = existing.unwrap_or_default();
    if !value.is_empty() {
        value.push(" ");
    }
    value.push(properties);
    value
}

#[cfg(any(target_os = "linux", test))]
fn pipewire_properties(existing: Option<OsString>) -> OsString {
    let Some(existing) = existing.and_then(|value| value.into_string().ok()) else {
        return MINECRAFT_PIPEWIRE_PROPERTIES.into();
    };
    let trimmed = existing.trim();
    let Some(body) = trimmed
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    else {
        return MINECRAFT_PIPEWIRE_PROPERTIES.into();
    };
    let body = body.trim();
    if body.is_empty() {
        MINECRAFT_PIPEWIRE_PROPERTIES.into()
    } else {
        format!("{{ {body} {MINECRAFT_PIPEWIRE_PROPERTY_BODY} }}").into()
    }
}

#[cfg(any(target_os = "linux", test))]
fn publish_game_audio_identity(command: &mut tokio::process::Command) {
    let pulse = append_environment_properties(
        command_environment_value(command, "PULSE_PROP"),
        MINECRAFT_PULSE_PROPERTIES,
    );
    let pipewire = pipewire_properties(command_environment_value(command, "PIPEWIRE_PROPS"));
    command
        .env("PULSE_PROP", pulse)
        .env("PIPEWIRE_PROPS", pipewire);
}

#[derive(Debug)]
enum ProcessControl {
    #[cfg(not(target_os = "linux"))]
    Direct,
    #[cfg(target_os = "linux")]
    ProcessGroup,
    #[cfg(target_os = "linux")]
    SystemdService { systemctl: PathBuf, unit: String },
}

struct SpawnedGame {
    child: tokio::process::Child,
    control: ProcessControl,
    stdin_guard: Option<tokio::process::ChildStdin>,
}

struct SupervisedIo {
    _stdin_guard: Option<tokio::process::ChildStdin>,
    output: tokio::task::JoinHandle<()>,
    errors: tokio::task::JoinHandle<()>,
}

/// Variables the launcher sets for the game after sanitizing, such as the window identity.
type GameEnvironment = [(OsString, OsString)];

fn configure_process_builder(
    builder: &mut tokio::process::Command,
    command: &LaunchCommand,
    environment: &GameEnvironment,
) {
    builder
        .current_dir(&command.directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    sanitize_game_environment(builder);
    for (key, value) in environment {
        builder.env(key, value);
    }
    #[cfg(windows)]
    builder.creation_flags(0x08000000);
}

#[cfg(not(target_os = "linux"))]
fn spawn_game_process(
    command: &LaunchCommand,
    _operation: OperationId,
    environment: &GameEnvironment,
) -> Result<SpawnedGame, CoreError> {
    let mut builder = tokio::process::Command::new(&command.executable);
    builder.args(&command.args);
    configure_process_builder(&mut builder, command, environment);
    let child = builder
        .spawn()
        .map_err(|_| CoreError::new(ErrorCode::ProcessStartFailed))?;
    Ok(SpawnedGame {
        child,
        control: ProcessControl::Direct,
        stdin_guard: None,
    })
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct SystemdTools {
    run: PathBuf,
    control: PathBuf,
}

#[cfg(target_os = "linux")]
fn fixed_executable(candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
}

#[cfg(target_os = "linux")]
fn systemd_version(run: &std::path::Path) -> Option<u32> {
    let mut command = std::process::Command::new(run);
    command.arg("--version");
    if std::env::var_os("APPIMAGE").is_some() {
        for key in APPIMAGE_ENVIRONMENT {
            command.env_remove(key);
        }
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
fn systemd_tools() -> Option<SystemdTools> {
    use std::os::unix::fs::FileTypeExt;

    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    if !runtime.is_absolute()
        || !std::fs::symlink_metadata(runtime.join("systemd/private"))
            .ok()
            .is_some_and(|metadata| metadata.file_type().is_socket())
    {
        return None;
    }
    let run = fixed_executable(&["/usr/bin/systemd-run", "/bin/systemd-run"])?;
    let control = fixed_executable(&["/usr/bin/systemctl", "/bin/systemctl"])?;
    // --pipe, --wait, --same-dir and service type exec are all available here.
    if systemd_version(&run)? < 240 {
        return None;
    }
    let mut probe = std::process::Command::new(&control);
    probe
        .args(["--user", "show-environment"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if std::env::var_os("APPIMAGE").is_some() {
        for key in APPIMAGE_ENVIRONMENT {
            probe.env_remove(key);
        }
    }
    if !probe.status().ok()?.success() {
        return None;
    }
    Some(SystemdTools { run, control })
}

#[cfg(target_os = "linux")]
fn valid_systemd_environment_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

#[cfg(target_os = "linux")]
fn explicitly_set(command: &tokio::process::Command, key: &str) -> bool {
    command
        .as_std()
        .get_envs()
        .any(|(name, value)| name == key && value.is_some())
}

#[cfg(target_os = "linux")]
fn effective_environment_keys(command: &tokio::process::Command) -> Vec<OsString> {
    let mut environment: BTreeMap<OsString, OsString> = std::env::vars_os()
        .filter(|(key, value)| {
            valid_systemd_environment_name(key)
                && value
                    .to_str()
                    .is_some_and(|value| value.chars().all(|character| !character.is_control()))
        })
        .collect();
    for (key, value) in command.as_std().get_envs() {
        if let Some(value) = value.filter(|value| {
            value
                .to_str()
                .is_some_and(|value| value.chars().all(|character| !character.is_control()))
        }) && valid_systemd_environment_name(key)
        {
            environment.insert(key.to_os_string(), value.to_os_string());
        } else {
            environment.remove(key);
        }
    }
    environment.into_keys().collect()
}

#[cfg(target_os = "linux")]
fn systemd_escape_argument(value: &OsStr) -> OsString {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let bytes = value.as_bytes();
    let mut escaped = Vec::with_capacity(bytes.len());
    for byte in bytes {
        escaped.push(*byte);
        if *byte == b'$' {
            escaped.push(b'$');
        }
    }
    OsString::from_vec(escaped)
}

/// UnsetEnvironment also removes a value passed with --setenv, so keep the keys
/// the launcher deliberately provides for the game.
#[cfg(target_os = "linux")]
fn unset_appimage_properties(builder: &mut tokio::process::Command, appimage: bool) {
    if !appimage {
        return;
    }
    for key in APPIMAGE_ENVIRONMENT {
        if !explicitly_set(builder, key) {
            builder.arg(format!("--property=UnsetEnvironment={key}"));
        }
    }
}

#[cfg(target_os = "linux")]
fn systemd_builder(
    command: &LaunchCommand,
    operation: OperationId,
    tools: &SystemdTools,
    environment: &GameEnvironment,
) -> (tokio::process::Command, ProcessControl) {
    let unit = format!("minecraft-{}.service", operation.value().simple());
    let mut builder = tokio::process::Command::new(&tools.run);
    configure_process_builder(&mut builder, command, environment);
    builder.args([
        "--user",
        "--quiet",
        "--wait",
        "--pipe",
        "--collect",
        "--same-dir",
        "--service-type=exec",
        "--description=Minecraft",
    ]);
    builder.arg(format!("--unit={unit}"));
    for key in JAVA_OPTIONS_ENVIRONMENT
        .iter()
        .chain(DESKTOP_IDENTITY_ENVIRONMENT.iter())
    {
        builder.arg(format!("--property=UnsetEnvironment={key}"));
    }
    unset_appimage_properties(&mut builder, std::env::var_os("APPIMAGE").is_some());
    for key in effective_environment_keys(&builder) {
        builder.arg(format!("--setenv={}", key.to_string_lossy()));
    }
    builder.arg("--");
    builder.arg(systemd_escape_argument(command.executable.as_os_str()));
    builder.args(
        command
            .args
            .iter()
            .map(|argument| systemd_escape_argument(OsStr::new(argument))),
    );
    (
        builder,
        ProcessControl::SystemdService {
            systemctl: tools.control.clone(),
            unit,
        },
    )
}

#[cfg(target_os = "linux")]
fn fallback_builder(
    command: &LaunchCommand,
    environment: &GameEnvironment,
) -> Result<(tokio::process::Command, ProcessControl), CoreError> {
    const SCRIPT: &str = "exec 3<&0; trap '' TERM INT HUP; (trap '' TERM INT HUP; while IFS= read -r line <&3; do :; done; kill -KILL 0) & watchdog=$!; (trap - TERM INT HUP; exec 3<&-; exec \"$@\" </dev/null) & child=$!; wait \"$child\"; status=$?; kill -KILL \"$watchdog\" 2>/dev/null; wait \"$watchdog\" 2>/dev/null; exit \"$status\"";

    let shell = fixed_executable(&["/bin/sh", "/usr/bin/sh"])
        .ok_or_else(|| CoreError::new(ErrorCode::ProcessStartFailed))?;
    let mut builder = tokio::process::Command::new(shell);
    builder
        .arg("-c")
        .arg(SCRIPT)
        .arg("panita-minecraft-runner")
        .arg(&command.executable)
        .args(&command.args);
    configure_process_builder(&mut builder, command, environment);
    builder.stdin(Stdio::piped()).process_group(0);
    Ok((builder, ProcessControl::ProcessGroup))
}

#[cfg(target_os = "linux")]
fn spawn_game_process(
    command: &LaunchCommand,
    operation: OperationId,
    environment: &GameEnvironment,
) -> Result<SpawnedGame, CoreError> {
    if !command.executable.is_file() {
        return Err(CoreError::new(ErrorCode::ProcessStartFailed));
    }
    if let Some(tools) = systemd_tools() {
        let (mut builder, control) = systemd_builder(command, operation, &tools, environment);
        if let Ok(child) = builder.spawn() {
            return Ok(SpawnedGame {
                child,
                control,
                stdin_guard: None,
            });
        }
    }
    let (mut builder, control) = fallback_builder(command, environment)?;
    let mut child = builder
        .spawn()
        .map_err(|_| CoreError::new(ErrorCode::ProcessStartFailed))?;
    let stdin_guard = child.stdin.take();
    if stdin_guard.is_none() {
        return Err(CoreError::new(ErrorCode::ProcessStartFailed));
    }
    Ok(SpawnedGame {
        child,
        control,
        stdin_guard,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameExit {
    pub code: Option<i32>,
    pub success: bool,
}

impl From<ExitStatus> for GameExit {
    fn from(status: ExitStatus) -> Self {
        Self {
            code: status.code(),
            success: status.success(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessEventKind {
    /// Process creation, not proof of reaching the Minecraft menu.
    Started,
    Output {
        stream: OutputStream,
        text: String,
    },
    OutputDiscarded {
        stream: OutputStream,
    },
    GracefulStopRequested,
    ForceStopRequested,
    Exited(GameExit),
    Failed(ErrorCode),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessEvent {
    pub process: ProcessSessionId,
    pub operation: OperationId,
    pub instance: Uuid,
    pub kind: ProcessEventKind,
    pub log_context: Option<Arc<str>>,
    pub cdata: bool,
}

/// A bounded read of an unlimited in-memory execution history.
pub struct ProcessEventPage {
    pub offset: usize,
    pub total: usize,
    pub records: Vec<ProcessEvent>,
}

pub struct ProcessEventSearch {
    pub found: Option<usize>,
    pub next: usize,
    pub total: usize,
}

struct Events {
    process: ProcessSessionId,
    operation: OperationId,
    instance: Uuid,
    records: Mutex<VecDeque<ProcessEvent>>,
    changed: watch::Sender<usize>,
}

impl Events {
    fn push(&self, kind: ProcessEventKind) {
        self.push_context(kind, None, false);
    }

    fn push_context(&self, kind: ProcessEventKind, log_context: Option<Arc<str>>, cdata: bool) {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        records.push_back(ProcessEvent {
            process: self.process,
            operation: self.operation,
            instance: self.instance,
            kind,
            log_context,
            cdata,
        });
        self.changed.send_replace(records.len());
    }

    fn page(&self, offset: Option<usize>, limit: usize) -> ProcessEventPage {
        let records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let total = records.len();
        let limit = limit.clamp(1, OUTPUT_PAGE_SIZE);
        let offset = offset
            .unwrap_or_else(|| total.saturating_sub(limit))
            .min(total);
        ProcessEventPage {
            offset,
            total,
            records: records
                .range(offset..offset.saturating_add(limit).min(total))
                .cloned()
                .collect(),
        }
    }

    fn find(
        &self,
        query: &str,
        start: usize,
        backwards: bool,
    ) -> Result<ProcessEventSearch, CoreError> {
        if query.trim().is_empty() || query.len() > 1024 || query.chars().any(char::is_control) {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let query = query.to_lowercase();
        let records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let total = records.len();
        let start = start.min(total);
        let (from, to) = if backwards {
            (start.saturating_sub(OUTPUT_PAGE_SIZE), start)
        } else {
            (start, start.saturating_add(OUTPUT_PAGE_SIZE).min(total))
        };
        let matches = |index: &usize| {
            matches!(&records[*index].kind,
            ProcessEventKind::Output { text, .. } if text.to_lowercase().contains(&query))
        };
        let found = if backwards {
            (from..to).rev().find(matches)
        } else {
            (from..to).find(matches)
        };
        Ok(ProcessEventSearch {
            found,
            total,
            next: found.map_or(if backwards { from } else { to }, |index| {
                if backwards { index } else { index + 1 }
            }),
        })
    }
}

/// Reserve before preflight and retain through child exit. This excludes other
/// launchers, installers and migrations that use the same interprocess lock.
pub struct LaunchLease {
    paths: AppPaths,
    instance: Uuid,
    directory: PathBuf,
    _lock: ArtifactLock,
}

impl LaunchLease {
    pub fn acquire(paths: &AppPaths, instance: Uuid) -> Result<Self, CoreError> {
        let root = InstanceRegistry::new(paths.clone()).prepare_instance(instance)?;
        let lock = try_lock(&root, ".installation.lock")?
            .ok_or_else(|| CoreError::new(ErrorCode::OperationBusy))?;
        Ok(Self {
            paths: paths.clone(),
            instance,
            directory: root.join("minecraft"),
            _lock: lock,
        })
    }

    pub fn verify_installation(&self) -> Result<super::install::InstalledLock, CoreError> {
        self.verify_installation_with_cancel(&super::download::Cancellation::default())
    }

    pub fn verify_installation_with_cancel(
        &self,
        cancel: &super::download::Cancellation,
    ) -> Result<super::install::InstalledLock, CoreError> {
        super::install::verify_play_under_lock(
            &self.paths,
            self.instance,
            self.directory.parent().ok_or_else(super::invalid)?,
            cancel,
        )
    }

    pub(crate) fn recover_installation(&self) -> Result<(), CoreError> {
        super::install::recover_locked(
            &self.paths,
            self.instance,
            self.directory.parent().ok_or_else(super::invalid)?,
        )
    }

    pub fn game_directory(&self) -> &std::path::Path {
        &self.directory
    }

    /// Low-level primitive: the caller must have passed current catalog,
    /// authentication, installation and runtime preflight under this lease.
    /// Dropping RunningGame does not stop the child or release its lease.
    pub fn spawn(
        self,
        command: LaunchCommand,
        process: ProcessSessionId,
        operation: OperationId,
    ) -> Result<RunningGame, CoreError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::new(ErrorCode::ProcessStartFailed))?;
        if command.directory != self.directory
            || command
                .redactions
                .iter()
                .any(|secret| secret.expose().chars().any(char::is_control))
        {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        #[cfg(target_os = "linux")]
        let environment = super::linux_window::game_window_environment(&self.paths);
        #[cfg(not(target_os = "linux"))]
        let environment: Vec<(OsString, OsString)> = Vec::new();
        let SpawnedGame {
            mut child,
            control,
            stdin_guard,
        } = spawn_game_process(&command, operation, &environment)?;
        let events = Arc::new(Events {
            process,
            operation,
            instance: self.instance,
            records: Mutex::new(VecDeque::new()),
            changed: watch::channel(0).0,
        });
        events.push(ProcessEventKind::Started);
        let redactions = Arc::new(command);
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let output = runtime.spawn(read_output(
            stdout,
            OutputStream::Stdout,
            events.clone(),
            redactions.clone(),
        ));
        let errors = runtime.spawn(read_output(
            stderr,
            OutputStream::Stderr,
            events.clone(),
            redactions,
        ));
        let (controls, requests) = mpsc::channel(8);
        let (complete, exit) = watch::channel(None);
        runtime.spawn(supervise(
            child,
            control,
            self,
            requests,
            complete,
            events.clone(),
            SupervisedIo {
                _stdin_guard: stdin_guard,
                output,
                errors,
            },
        ));
        Ok(RunningGame {
            controls,
            exit,
            events,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOutcome {
    Requested,
    Unsupported,
    AlreadyExited,
}

struct Request {
    force: bool,
    reply: oneshot::Sender<Result<StopOutcome, ErrorCode>>,
}

pub struct RunningGame {
    controls: mpsc::Sender<Request>,
    exit: watch::Receiver<Option<GameExit>>,
    events: Arc<Events>,
}

impl RunningGame {
    /// Notifications carry only a cursor. Readers recover every retained event by page.
    pub fn output_changes(&self) -> watch::Receiver<usize> {
        self.events.changed.subscribe()
    }

    pub fn find_output(
        &self,
        query: &str,
        start: usize,
        backwards: bool,
    ) -> Result<ProcessEventSearch, CoreError> {
        self.events.find(query, start, backwards)
    }
    pub fn events_page(&self, offset: Option<usize>, limit: usize) -> ProcessEventPage {
        self.events.page(offset, limit)
    }

    pub fn events(&self) -> Vec<ProcessEvent> {
        self.events
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub async fn wait(&self) -> Result<GameExit, CoreError> {
        let mut exit = self.exit.clone();
        loop {
            if let Some(result) = *exit.borrow_and_update() {
                return Ok(result);
            }
            exit.changed()
                .await
                .map_err(|_| CoreError::new(ErrorCode::ProcessControlFailed))?;
        }
    }

    /// Linux uses SIGTERM; Windows has no supported graceful protocol here.
    /// A graceful request never escalates automatically to force-stop.
    pub async fn request_stop(&self) -> Result<StopOutcome, CoreError> {
        self.control(false).await
    }

    pub async fn force_stop(&self) -> Result<StopOutcome, CoreError> {
        self.control(true).await
    }

    async fn control(&self, force: bool) -> Result<StopOutcome, CoreError> {
        if self.exit.borrow().is_some() {
            return Ok(StopOutcome::AlreadyExited);
        }
        let (reply, result) = oneshot::channel();
        if self.controls.send(Request { force, reply }).await.is_err() {
            return if self.exit.borrow().is_some() {
                Ok(StopOutcome::AlreadyExited)
            } else {
                Err(CoreError::new(ErrorCode::ProcessControlFailed))
            };
        }
        match result.await {
            Ok(result) => result.map_err(CoreError::new),
            Err(_) if self.exit.borrow().is_some() => Ok(StopOutcome::AlreadyExited),
            Err(_) => Err(CoreError::new(ErrorCode::ProcessControlFailed)),
        }
    }
}

async fn supervise(
    mut child: tokio::process::Child,
    control: ProcessControl,
    lease: LaunchLease,
    mut controls: mpsc::Receiver<Request>,
    complete: watch::Sender<Option<GameExit>>,
    events: Arc<Events>,
    pipes: SupervisedIo,
) {
    let mut controls_open = true;
    let status = loop {
        tokio::select! {
            biased;
            result = child.wait() => match result {
                Ok(status) => break GameExit::from(status),
                Err(_) => {
                    events.push(ProcessEventKind::Failed(ErrorCode::ProcessControlFailed));
                    // Keep ownership and the lock if exit cannot be established.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            },
            request = controls.recv(), if controls_open => {
                if let Some(request) = request {
                    let result = stop_child(&mut child, &control, request.force).await;
                    if result == Ok(StopOutcome::Requested) {
                        events.push(if request.force { ProcessEventKind::ForceStopRequested } else { ProcessEventKind::GracefulStopRequested });
                    }
                    let _ = request.reply.send(result);
                } else { controls_open = false; }
            }
        }
    };
    // Descendants may retain a pipe after the owned child exits. Bound draining
    // without ever keeping the instance busy indefinitely for inherited pipes.
    tokio::join!(
        drain(pipes.output, OutputStream::Stdout, &events),
        drain(pipes.errors, OutputStream::Stderr, &events)
    );
    events.push(ProcessEventKind::Exited(status));
    drop(lease);
    let _ = complete.send(Some(status));
}

async fn stop_child(
    child: &mut tokio::process::Child,
    control: &ProcessControl,
    force: bool,
) -> Result<StopOutcome, ErrorCode> {
    if child
        .try_wait()
        .map_err(|_| ErrorCode::ProcessControlFailed)?
        .is_some()
    {
        return Ok(StopOutcome::AlreadyExited);
    }
    #[cfg(target_os = "linux")]
    {
        match control {
            ProcessControl::ProcessGroup => {
                // The fallback runner is a process-group leader.
                // The actor owns its unreaped Child, so this PID cannot be reused.
                let group = child
                    .id()
                    .and_then(|pid| i32::try_from(pid).ok())
                    .and_then(rustix::process::Pid::from_raw)
                    .ok_or(ErrorCode::ProcessControlFailed)?;
                let signal = if force {
                    rustix::process::Signal::KILL
                } else {
                    rustix::process::Signal::TERM
                };
                rustix::process::kill_process_group(group, signal)
                    .map_err(|_| ErrorCode::ProcessControlFailed)?;
                Ok(StopOutcome::Requested)
            }
            ProcessControl::SystemdService { systemctl, unit } => {
                let signal = async {
                    // systemd-run is already spawned, but the transient unit can
                    // take a short moment to become addressable over the user bus.
                    for attempt in 0..20 {
                        if child
                            .try_wait()
                            .map_err(|_| ErrorCode::ProcessControlFailed)?
                            .is_some()
                        {
                            return Ok(StopOutcome::AlreadyExited);
                        }
                        let mut inspect = tokio::process::Command::new(systemctl);
                        inspect
                            .args(["--user", "show", "--property=MainPID", "--value"])
                            .arg(unit)
                            .stdin(Stdio::null())
                            .stderr(Stdio::null());
                        if std::env::var_os("APPIMAGE").is_some() {
                            sanitize_appimage_environment(&mut inspect, true);
                        }
                        let main_ready = inspect
                            .output()
                            .await
                            .ok()
                            .filter(|output| output.status.success())
                            .and_then(|output| {
                                String::from_utf8(output.stdout)
                                    .ok()?
                                    .trim()
                                    .parse::<u32>()
                                    .ok()
                            })
                            .is_some_and(|pid| pid != 0);
                        if !main_ready {
                            if attempt != 19 {
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                            continue;
                        }
                        let mut command = tokio::process::Command::new(systemctl);
                        command
                            .args(["--user", "kill"])
                            .arg(if force {
                                "--kill-whom=all"
                            } else {
                                "--kill-whom=main"
                            })
                            .arg(if force {
                                "--signal=KILL"
                            } else {
                                "--signal=TERM"
                            })
                            .arg(unit)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null());
                        if std::env::var_os("APPIMAGE").is_some() {
                            sanitize_appimage_environment(&mut command, true);
                        }
                        if command
                            .status()
                            .await
                            .map_err(|_| ErrorCode::ProcessControlFailed)?
                            .success()
                        {
                            return Ok(StopOutcome::Requested);
                        }
                        if attempt != 19 {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                    Err(ErrorCode::ProcessControlFailed)
                };
                tokio::time::timeout(Duration::from_secs(3), signal)
                    .await
                    .map_err(|_| ErrorCode::ProcessControlFailed)?
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = control;
        if force {
            child
                .start_kill()
                .map_err(|_| ErrorCode::ProcessControlFailed)?;
            Ok(StopOutcome::Requested)
        } else {
            Ok(StopOutcome::Unsupported)
        }
    }
}

async fn drain(mut task: tokio::task::JoinHandle<()>, stream: OutputStream, events: &Events) {
    if tokio::time::timeout(Duration::from_secs(2), &mut task)
        .await
        .is_err()
    {
        task.abort();
        let _ = task.await;
        events.push(ProcessEventKind::OutputDiscarded { stream });
    }
}

struct LineBuffer {
    bytes: Vec<u8>,
    discarded: bool,
    context: super::log_context::LogContext,
}
impl Drop for LineBuffer {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl LineBuffer {
    fn finish(&mut self, stream: OutputStream, events: &Events, command: &LaunchCommand) {
        if self.discarded {
            self.context = super::log_context::LogContext::default();
            events.push(ProcessEventKind::OutputDiscarded { stream });
        } else if !self.bytes.is_empty() {
            let mut text = String::from_utf8_lossy(&self.bytes).into_owned();
            for secret in &command.redactions {
                let clean = text.replace(secret.expose(), "[redacted]");
                text.zeroize();
                text = clean;
            }
            let lower = text.to_ascii_lowercase();
            if [
                "accesstoken",
                "access_token",
                "refresh_token",
                "authorization",
                "x-api-key",
                "bearer ",
                "panitacraft://",
                "oauth/callback",
                "client_secret",
                "password",
            ]
            .iter()
            .any(|marker| lower.contains(marker))
            {
                text.zeroize();
                text = "[sensitive output omitted]".into();
            }
            let text: String = text
                .chars()
                .filter(|ch| !ch.is_control() || *ch == '\t')
                .collect();
            let (context, cdata) = self.context.consume(&text);
            events.push_context(ProcessEventKind::Output { stream, text }, context, cdata);
        }
        self.bytes.zeroize();
        self.bytes.clear();
        self.discarded = false;
    }
}

async fn read_output<R: AsyncRead + Unpin>(
    reader: Option<R>,
    stream: OutputStream,
    events: Arc<Events>,
    command: Arc<LaunchCommand>,
) {
    let Some(mut reader) = reader else {
        return;
    };
    let mut line = LineBuffer {
        bytes: Vec::new(),
        discarded: false,
        context: super::log_context::LogContext::default(),
    };
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => {
                line.finish(stream, &events, &command);
                break;
            }
            Ok(count) => {
                for byte in &chunk[..count] {
                    if *byte == b'\n' {
                        line.finish(stream, &events, &command);
                    } else if !line.discarded {
                        if line.bytes.len() == MAX_LINE {
                            line.bytes.zeroize();
                            line.bytes.clear();
                            line.discarded = true;
                        } else {
                            line.bytes.push(*byte);
                        }
                    }
                }
                chunk.zeroize();
            }
            Err(_) => {
                chunk.zeroize();
                events.push(ProcessEventKind::OutputDiscarded { stream });
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;
