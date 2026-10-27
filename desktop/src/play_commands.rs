//! Desktop ownership and token-free snapshots for the core Play workflow.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use launcher_core::error::ErrorCode;
use launcher_core::logging::{BoundedLog, LogEvent, LogRecord, Workflow, WorkflowPhase};
use launcher_core::minecraft::{
    Platform, download::Cancellation, play::PendingPlay, process::RunningGame,
};
use launcher_core::operation::ProcessSessionId;
use launcher_core::paths::AppPaths;
use serde_json::{Value, json};
use tauri::{Emitter, Manager};
use uuid::Uuid;

use super::{DesktopState, account_commands, activation_commands};

#[derive(Clone, Copy, PartialEq)]
enum OperationKind {
    Play,
    Install,
    Update,
    Repair,
    Uninstall,
}
impl OperationKind {
    fn diagnostic(self) -> Workflow {
        match self {
            Self::Play => Workflow::Play,
            Self::Install => Workflow::Install,
            Self::Update => Workflow::Update,
            Self::Repair => Workflow::Repair,
            Self::Uninstall => Workflow::Uninstall,
        }
    }
    fn text(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Install => "install",
            Self::Update => "update",
            Self::Repair => "repair",
            Self::Uninstall => "uninstall",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    Installing,
    Installed,
    Uninstalled,
    Warning,
    Starting,
    Running,
    Stopping,
    Cancelling,
    Exited,
    Failed,
    Cancelled,
}

impl Phase {
    fn diagnostic(self, exit_success: Option<bool>) -> WorkflowPhase {
        match self {
            Self::Preparing => WorkflowPhase::Preparing,
            Self::Installing => WorkflowPhase::Installing,
            Self::Installed => WorkflowPhase::Installed,
            Self::Uninstalled => WorkflowPhase::Uninstalled,
            Self::Warning => WorkflowPhase::Warning,
            Self::Starting => WorkflowPhase::Starting,
            Self::Running => WorkflowPhase::Running,
            Self::Stopping => WorkflowPhase::Stopping,
            Self::Cancelling => WorkflowPhase::Cancelling,
            Self::Exited if exit_success == Some(false) => WorkflowPhase::ExitedWithError,
            Self::Exited => WorkflowPhase::Exited,
            Self::Failed => WorkflowPhase::Failed,
            Self::Cancelled => WorkflowPhase::Cancelled,
        }
    }
    fn active(self) -> bool {
        !matches!(
            self,
            Self::Installed | Self::Uninstalled | Self::Exited | Self::Failed | Self::Cancelled
        )
    }
    fn text(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Installing => "installing",
            Self::Installed => "installed",
            Self::Uninstalled => "uninstalled",
            Self::Warning => "warning",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Cancelling => "cancelling",
            Self::Exited => "exited",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

struct Entry {
    request: Uuid,
    phase: Phase,
    cancel: Cancellation,
    pending: Option<PendingPlay>,
    game: Option<Arc<RunningGame>>,
    last_game: Option<Arc<RunningGame>>,
    exit_expected: bool,
    console_error: Option<ErrorCode>,
    error: Option<ErrorCode>,
    exit: Option<launcher_core::minecraft::process::GameExit>,
    kind: OperationKind,
    backup_requested: bool,
    backup_operation: Option<Uuid>,
    received_files: u64,
    reused_files: u64,
    retained_files: u32,
    native_cleanup: Option<launcher_core::minecraft::native_cache::CleanupReport>,
    last_progress: std::time::Instant,
    last_logged: Option<(WorkflowPhase, Option<ErrorCode>)>,
}

#[derive(Default)]
struct State {
    process: ProcessSessionId,
    logs: Arc<BoundedLog>,
    revision: u64,
    closing: bool,
    close_blocked: bool,
    background: bool,
    entries: HashMap<Uuid, Entry>,
}

impl State {
    fn cancel_pending(&mut self, include_installations: bool) {
        for entry in self.entries.values_mut() {
            if entry.kind != OperationKind::Play && !include_installations {
                continue;
            }
            if matches!(
                entry.phase,
                Phase::Preparing
                    | Phase::Installing
                    | Phase::Warning
                    | Phase::Starting
                    | Phase::Cancelling
            ) {
                entry.cancel.cancel();
                entry.phase = if entry.pending.take().is_some() {
                    Phase::Cancelled
                } else {
                    Phase::Cancelling
                };
            }
        }
        self.changed();
    }

    fn changed(&mut self) {
        for entry in self.entries.values_mut() {
            let current = (
                entry.phase.diagnostic(entry.exit.map(|exit| exit.success)),
                entry.error,
            );
            if entry.last_logged != Some(current) {
                self.logs.record(LogRecord {
                    process: self.process,
                    operation: None,
                    event: LogEvent::WorkflowChanged {
                        request: entry.request,
                        workflow: entry.kind.diagnostic(),
                        phase: current.0,
                        error: current.1,
                    },
                });
                entry.last_logged = Some(current);
            }
        }
        self.revision += 1;
        if !self.entries.values().any(|entry| entry.phase.active()) {
            self.close_blocked = false;
        }
    }
}

pub(super) struct PlayController {
    process: ProcessSessionId,
    state: Mutex<State>,
}

impl PlayController {
    fn complete_game(
        &self,
        instance: Uuid,
        request: Uuid,
        exit: launcher_core::minecraft::process::GameExit,
    ) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let Some(entry) = state
            .entries
            .get_mut(&instance)
            .filter(|entry| entry.request == request)
        else {
            return false;
        };
        let unexpected = !exit.success && !entry.exit_expected;
        entry.phase = Phase::Exited;
        entry.exit = Some(exit);
        entry.game = None;
        entry.error = None;
        if unexpected {
            // Make the crash visible before any queued idle-shutdown check can run.
            state.background = false;
        }
        state.changed();
        unexpected
    }

    fn game_output(&self, instance: Uuid, request: Uuid) -> Result<Value, ErrorCode> {
        self.game_output_page(instance, request, None)
    }

    fn game_output_page(
        &self,
        instance: Uuid,
        request: Uuid,
        offset: Option<usize>,
    ) -> Result<Value, ErrorCode> {
        use launcher_core::minecraft::process::{OUTPUT_PAGE_SIZE, OutputStream, ProcessEventKind};
        let state = self
            .state
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let entry = state
            .entries
            .get(&instance)
            .filter(|entry| entry.request == request)
            .ok_or(ErrorCode::OperationCancelled)?;
        let game = entry
            .last_game
            .as_ref()
            .ok_or(ErrorCode::OperationCancelled)?;
        let page = game.events_page(offset, OUTPUT_PAGE_SIZE);
        let next = page.offset + page.records.len();
        let lines: Vec<_> = page.records.into_iter().enumerate().filter_map(|(index, event)| match event.kind {
            ProcessEventKind::Output { stream, text } => Some(json!({"sequence":page.offset + index,
                "context": event.log_context.as_deref(), "cdata":event.cdata,
                "stream": if stream == OutputStream::Stdout { "stdout" } else { "stderr" }, "text": if text == "[sensitive output omitted]" { "[salida sensible omitida]" } else { &text }})),
            ProcessEventKind::OutputDiscarded { .. } => Some(json!({"sequence":page.offset + index,"stream": "notice", "text": "Se omitió una línea demasiado larga o una salida que no pudo leerse."})),
            _ => None,
        }).collect();
        Ok(
            json!({"lines":lines, "running":entry.game.is_some(), "exitCode":entry.exit.and_then(|exit|exit.code),
                "offset":page.offset,"nextOffset":next,"total":page.total,"pageSize":OUTPUT_PAGE_SIZE}),
        )
    }

    #[cfg(test)]
    pub fn new(process: ProcessSessionId) -> Self {
        Self::with_logs(process, Arc::new(BoundedLog::default()))
    }

    pub(super) fn with_logs(process: ProcessSessionId, logs: Arc<BoundedLog>) -> Self {
        Self {
            process,
            state: Mutex::new(State {
                process,
                logs,
                ..Default::default()
            }),
        }
    }

    fn reserve(&self, instance: Uuid, request: Uuid) -> Result<Cancellation, ErrorCode> {
        self.reserve_operation(instance, request, OperationKind::Play)
    }

    fn reserve_operation(
        &self,
        instance: Uuid,
        request: Uuid,
        kind: OperationKind,
    ) -> Result<Cancellation, ErrorCode> {
        if instance.is_nil() || request.is_nil() {
            return Err(ErrorCode::InvalidData);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if state.closing || state.background {
            return Err(ErrorCode::OperationCancelled);
        }
        if state
            .entries
            .get(&instance)
            .is_some_and(|entry| entry.phase.active())
        {
            return Err(ErrorCode::OperationBusy);
        }
        // Bound retained terminal snapshots without removing active process owners.
        if state.entries.len() >= 64 && !state.entries.contains_key(&instance) {
            state.entries.retain(|_, entry| entry.phase.active());
            if state.entries.len() >= 64 {
                return Err(ErrorCode::OperationBusy);
            }
        }
        let cancel = Cancellation::default();
        state.entries.insert(
            instance,
            Entry {
                request,
                phase: if kind != OperationKind::Play {
                    Phase::Installing
                } else {
                    Phase::Preparing
                },
                cancel: cancel.clone(),
                pending: None,
                game: None,
                last_game: None,
                exit_expected: false,
                console_error: None,
                error: None,
                exit: None,
                kind,
                backup_requested: false,
                backup_operation: None,
                received_files: 0,
                reused_files: 0,
                retained_files: 0,
                native_cleanup: None,
                last_progress: std::time::Instant::now(),
                last_logged: None,
            },
        );
        state.close_blocked = false;
        state.changed();
        Ok(cancel)
    }

    fn update(&self, instance: Uuid, request: Uuid, change: impl FnOnce(&mut Entry)) {
        if let Ok(mut state) = self.state.lock()
            && let Some(entry) = state.entries.get_mut(&instance)
            && entry.request == request
        {
            change(entry);
            state.changed();
        }
    }

    fn fail(&self, instance: Uuid, request: Uuid, error: ErrorCode) {
        self.update(instance, request, |entry| {
            if entry.game.is_some() {
                entry.error = Some(error);
                return;
            }
            entry.phase = if error == ErrorCode::OperationCancelled {
                Phase::Cancelled
            } else {
                Phase::Failed
            };
            entry.error = Some(error);
            entry.pending = None;
        });
    }

    fn installation_progress(
        &self,
        instance: Uuid,
        request: Uuid,
        progress: launcher_core::minecraft::download::DownloadProgress,
    ) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let Some(entry) = state
            .entries
            .get_mut(&instance)
            .filter(|entry| entry.request == request && entry.phase == Phase::Installing)
        else {
            return false;
        };
        if progress.bytes == progress.total {
            if progress.reused {
                entry.reused_files += 1;
            } else {
                entry.received_files += 1;
            }
        }
        // Progress is per artifact, with up to four concurrent downloads. Do not
        // sum partial byte counts or describe received files as verified yet.
        let publish = entry.last_progress.elapsed() >= std::time::Duration::from_millis(250);
        if publish {
            entry.last_progress = std::time::Instant::now();
        }
        state.changed();
        publish
    }

    pub fn snapshot(&self) -> Result<Value, ErrorCode> {
        let state = self
            .state
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let mut instances: Vec<_> = state
            .entries
            .iter()
            .map(|(id, entry)| {
                json!({
                    "instanceId": id, "requestId": entry.request, "phase": entry.phase.text(),
                    "confirmationId": entry.pending.as_ref().map(PendingPlay::operation_id),
                    "error": entry.error, "exitCode": entry.exit.and_then(|exit| exit.code),
                    "exitSuccess": entry.exit.map(|exit| exit.success),
                    "exitExpected": entry.exit_expected,
                    "consoleError": entry.console_error,
                    "operation": entry.kind.text(),
                    "backupRequested": entry.backup_requested, "backupOperation": entry.backup_operation,
                    "receivedFiles": entry.received_files, "reusedFiles": entry.reused_files,
                    "retainedFiles": entry.retained_files,
                    "nativeCleanup": entry.native_cleanup,
                })
            })
            .collect();
        instances.sort_by(|a, b| a["instanceId"].as_str().cmp(&b["instanceId"].as_str()));
        Ok(
            json!({"revision":state.revision,"processSessionId":self.process.value(),"closeBlocked":state.close_blocked,"gracefulStopSupported":cfg!(target_os="linux"),"instances":instances}),
        )
    }

    pub(super) fn publish(&self, app: &tauri::AppHandle) {
        if let Ok(snapshot) = self.snapshot() {
            let _ = app.emit("play-changed", snapshot);
        }
        // Window hiding, restoration and idle shutdown are ordered on the UI
        // thread. Recheck ownership there rather than using a stale snapshot.
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if handle.state::<DesktopState>().plays.finish_background() {
                handle.exit(0);
            }
        });
    }

    /// Called on the UI thread. A failed hide must never enable hidden shutdown.
    fn close_window(&self, hide: impl FnOnce() -> bool) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return true;
        };
        if state
            .entries
            .values()
            .any(|entry| matches!(entry.phase, Phase::Running | Phase::Stopping))
        {
            if hide() {
                state.background = true;
                state.close_blocked = false;
                state.cancel_pending(true);
            } else {
                state.close_blocked = true;
                state.changed();
            }
            true
        } else if state.entries.values().any(|entry| entry.phase.active()) {
            state.close_blocked = true;
            state.changed();
            true
        } else {
            state.closing = true;
            false
        }
    }

    fn restore(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.closing {
            return false;
        }
        state.background = false;
        state.close_blocked = false;
        state.changed();
        true
    }

    fn finish_background(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.background
            && !state.closing
            && !state.entries.values().any(|entry| entry.phase.active())
        {
            state.closing = true;
            true
        } else {
            false
        }
    }

    /// Serialize normal exit against new requests. Fail closed on poisoned state.
    pub fn allow_close(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.entries.values().any(|entry| entry.phase.active()) {
            state.close_blocked = true;
            state.changed();
            false
        } else {
            state.closing = true;
            true
        }
    }

    pub fn cancel_pending(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.cancel_pending(false);
        }
    }

    fn cancel(&self, instance: Uuid, request: Uuid) -> Result<(), ErrorCode> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let entry = state
            .entries
            .get_mut(&instance)
            .filter(|entry| entry.request == request)
            .ok_or(ErrorCode::OperationCancelled)?;
        if !matches!(
            entry.phase,
            Phase::Preparing
                | Phase::Installing
                | Phase::Warning
                | Phase::Starting
                | Phase::Cancelling
        ) {
            return Err(ErrorCode::OperationBusy);
        }
        entry.cancel.cancel();
        entry.phase = if entry.pending.take().is_some() {
            Phase::Cancelled
        } else {
            Phase::Cancelling
        };
        state.changed();
        Ok(())
    }

    fn confirm(
        &self,
        instance: Uuid,
        request: Uuid,
        confirmation: Uuid,
    ) -> Result<PendingPlay, ErrorCode> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let entry = state
            .entries
            .get_mut(&instance)
            .filter(|entry| entry.request == request)
            .ok_or(ErrorCode::OperationCancelled)?;
        if entry.phase != Phase::Warning || entry.cancel.check().is_err() {
            return Err(ErrorCode::OperationCancelled);
        }
        if entry.pending.as_ref().map(PendingPlay::operation_id) != Some(confirmation) {
            return Err(ErrorCode::InvalidData);
        }
        let pending = entry.pending.take().ok_or(ErrorCode::OperationBusy)?;
        entry.phase = Phase::Starting;
        state.changed();
        Ok(pending)
    }
}

fn native_platform() -> Result<Platform, ErrorCode> {
    if std::env::consts::ARCH != "x86_64" {
        return Err(ErrorCode::RuntimeUnsupported);
    }
    #[cfg(windows)]
    let version = {
        let version = windows_version::OsVersion::current();
        format!("{}.{}.{}", version.major, version.minor, version.build)
    };
    #[cfg(target_os = "linux")]
    let version = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map_err(|_| ErrorCode::RuntimeUnsupported)?
        .trim()
        .to_owned();
    #[cfg(not(any(windows, target_os = "linux")))]
    let version = String::new();
    Platform::x64(std::env::consts::OS, &version).map_err(|error| error.code())
}

async fn start_and_monitor(
    app: tauri::AppHandle,
    controller: Arc<PlayController>,
    instance: Uuid,
    request: Uuid,
    pending: PendingPlay,
    confirmation: Option<Uuid>,
    accounts: Arc<launcher_core::sessions::AccountController>,
) {
    let show_console = pending.show_console_on_start();
    match pending.start(confirmation).await {
        Ok(game) => {
            // Always retain a committed child, even if cancellation arrived after
            // the atomic spawn boundary. Account removal must not kill that game.
            let game = Arc::new(game);
            controller.update(instance, request, |entry| {
                entry.phase = Phase::Running;
                entry.game = Some(game.clone());
                entry.last_game = Some(game.clone());
            });
            controller.publish(&app);
            account_commands::emit_accounts(&app, &accounts);
            if show_console && let Err(error) = show_game_log(&app, instance, request) {
                controller.update(instance, request, |entry| entry.console_error = Some(error));
                controller.publish(&app);
            }
            let mut changes = game.output_changes();
            let result = loop {
                tokio::select! {
                    exit = game.wait() => break exit,
                    changed = changes.changed() => {
                        if changed.is_err() { break game.wait().await; }
                        // Coalesce bursts without polling while the game is idle.
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        let total = *changes.borrow_and_update();
                        notify_game_log(&app, instance, request, total, true, None);
                    }
                }
            };
            let final_exit = result.as_ref().ok().copied();
            match result {
                Ok(exit) => {
                    if controller.complete_game(instance, request, exit) {
                        // A crash must remain visible even when the main window was hidden.
                        restore_window(&app, String::new());
                    }
                }
                Err(error) => controller.update(instance, request, |entry| {
                    entry.error = Some(error.code());
                }),
            }
            // Publish after controller state changes so the final page sees exit state.
            let total = *changes.borrow();
            notify_game_log(&app, instance, request, total, false, final_exit);
        }
        Err(error) => controller.fail(instance, request, error.code()),
    }
    account_commands::emit_accounts(&app, &accounts);
    controller.publish(&app);
}

#[tauri::command]
pub(super) fn play_snapshot(state: tauri::State<'_, DesktopState>) -> Result<Value, ErrorCode> {
    state.plays.snapshot()
}

fn log_label(instance: Uuid, request: Uuid) -> String {
    format!("game-log-{instance}-{request}")
}

fn notify_game_log(
    app: &tauri::AppHandle,
    instance: Uuid,
    request: Uuid,
    total: usize,
    running: bool,
    exit: Option<launcher_core::minecraft::process::GameExit>,
) {
    let label = log_label(instance, request);
    if app.get_webview_window(&label).is_some() {
        let _ = app.emit_to(
            label.as_str(),
            "game-log-changed",
            json!({"instanceId":instance,"requestId":request,"total":total,
                "running":running,"exitCode":exit.and_then(|exit|exit.code)}),
        );
    }
}

#[tauri::command]
pub(super) async fn find_game_log(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
    query: String,
    start: usize,
    backwards: bool,
) -> Result<Value, ErrorCode> {
    if window.label() != "main" && window.label() != log_label(instance_id, request_id) {
        return Err(ErrorCode::InvalidData);
    }
    let game = {
        let state = state
            .plays
            .state
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        state
            .entries
            .get(&instance_id)
            .filter(|entry| entry.request == request_id)
            .and_then(|entry| entry.last_game.clone())
            .ok_or(ErrorCode::OperationCancelled)?
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        let result = game
            .find_output(&query, start, backwards)
            .map_err(|error| error.code())?;
        Ok::<_, ErrorCode>(json!({"found":result.found,"next":result.next,"total":result.total}))
    })
    .await
    .map_err(|_| ErrorCode::StorageUnavailable)??;
    let current = state
        .plays
        .state
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    if current
        .entries
        .get(&instance_id)
        .is_none_or(|entry| entry.request != request_id)
    {
        return Err(ErrorCode::OperationCancelled);
    }
    Ok(result)
}

#[tauri::command]
pub(super) fn game_log_snapshot(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
    offset: Option<usize>,
) -> Result<Value, ErrorCode> {
    if window.label() != "main" && window.label() != log_label(instance_id, request_id) {
        return Err(ErrorCode::InvalidData);
    }
    state
        .plays
        .game_output_page(instance_id, request_id, offset)
}

#[tauri::command]
pub(super) async fn open_game_log(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
) -> Result<(), ErrorCode> {
    if window.label() != "main" {
        return Err(ErrorCode::InvalidData);
    }
    state.plays.game_output(instance_id, request_id)?;
    show_game_log(&app, instance_id, request_id)
}

fn show_game_log(
    app: &tauri::AppHandle,
    instance_id: Uuid,
    request_id: Uuid,
) -> Result<(), ErrorCode> {
    let label = log_label(instance_id, request_id);
    if let Some(window) = app.get_webview_window(&label) {
        window
            .unminimize()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        window
            .set_focus()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
    } else {
        tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
            .title("Registro de Minecraft — Panita Client")
            .inner_size(900.0, 600.0)
            .min_inner_size(520.0, 360.0)
            .build()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn open_crash_reports(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
) -> Result<Value, ErrorCode> {
    use tauri_plugin_opener::OpenerExt;
    if window.label() != "main" {
        return Err(ErrorCode::InvalidData);
    }
    // Require the current owned execution; never accept a frontend-provided path.
    state.plays.game_output(instance_id, request_id)?;
    let paths = AppPaths::new(
        app.path()
            .app_data_dir()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    )
    .map_err(|error| error.code())?;
    let directory = tokio::task::spawn_blocking(move || {
        launcher_core::minecraft::support::crash_report_directory(&paths, instance_id)
    })
    .await
    .map_err(|_| ErrorCode::StorageUnavailable)?
    .map_err(|error| error.code())?;
    let Some(directory) = directory else {
        return Ok(json!(false));
    };
    // Windows shell integration may keep waiting even after Explorer appears.
    // Keep the runtime and dialog usable, and report an indeterminate result honestly.
    let opening = tokio::task::spawn_blocking(move || {
        app.opener()
            .open_path(directory.to_string_lossy(), None::<&str>)
            .map_err(|_| ErrorCode::StorageUnavailable)
    });
    match tokio::time::timeout(std::time::Duration::from_secs(5), opening).await {
        Ok(result) => {
            result.map_err(|_| ErrorCode::StorageUnavailable)??;
            Ok(json!(true))
        }
        Err(_) => Ok(json!("pending")),
    }
}

#[tauri::command]
pub(super) fn begin_install(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
) -> Result<(), ErrorCode> {
    begin_maintenance(
        app,
        state,
        instance_id,
        request_id,
        OperationKind::Install,
        None,
    )
}

#[tauri::command]
pub(super) fn begin_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
    update: launcher_core::minecraft::update::UpdateRequest,
) -> Result<(), ErrorCode> {
    begin_maintenance(
        app,
        state,
        instance_id,
        request_id,
        OperationKind::Update,
        Some(update),
    )
}

#[tauri::command]
pub(super) fn begin_repair(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
) -> Result<(), ErrorCode> {
    begin_maintenance(
        app,
        state,
        instance_id,
        request_id,
        OperationKind::Repair,
        None,
    )
}

#[tauri::command]
pub(super) fn begin_uninstall(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
) -> Result<(), ErrorCode> {
    begin_maintenance(
        app,
        state,
        instance_id,
        request_id,
        OperationKind::Uninstall,
        None,
    )
}

#[tauri::command]
pub(super) async fn clean_native_cache(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    instance_id: Uuid,
) -> Result<launcher_core::minecraft::native_cache::CleanupReport, ErrorCode> {
    if window.label() != "main" {
        return Err(ErrorCode::InvalidData);
    }
    let paths = AppPaths::new(
        app.path()
            .app_data_dir()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    )
    .map_err(|error| error.code())?;
    tauri::async_runtime::spawn_blocking(move || {
        launcher_core::minecraft::native_cache::cleanup(
            &paths,
            instance_id,
            &Cancellation::default(),
        )
        .map_err(|error| error.code())
    })
    .await
    .map_err(|_| ErrorCode::StorageUnavailable)?
}

fn begin_maintenance(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
    kind: OperationKind,
    update: Option<launcher_core::minecraft::update::UpdateRequest>,
) -> Result<(), ErrorCode> {
    let platform = if kind == OperationKind::Uninstall {
        None
    } else {
        Some(native_platform()?)
    };
    let paths = AppPaths::new(
        app.path()
            .app_data_dir()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    )
    .map_err(|error| error.code())?;
    // Use the same initialized private root as the other desktop workflows.
    account_commands::controller(&state)?;
    let activation = activation_commands::controller(&state)?;
    // Installing, updating and repairing need a recently verified session; removing
    // local files only needs the files.
    let credentials = if kind == OperationKind::Uninstall {
        None
    } else {
        Some(
            activation
                .online_credentials()
                .map_err(|error| error.code())?,
        )
    };
    let controller = state.plays.clone();
    let cancel = controller.reserve_operation(instance_id, request_id, kind)?;
    controller.update(instance_id, request_id, |entry| {
        entry.backup_requested = update.as_ref().is_some_and(|request| request.backup_worlds)
    });
    controller.publish(&app);
    tauri::async_runtime::spawn(async move {
        let result = async {
            use launcher_core::registry::{InstallationState, InstanceRegistry};
            let record = InstanceRegistry::new(paths.clone())
                .get(instance_id)
                .map_err(|error| error.code())?;
            if kind == OperationKind::Install
                && record.is_some_and(|record| {
                    matches!(
                        record.state,
                        InstallationState::Installed
                            | InstallationState::UpdateAvailable
                            | InstallationState::Running
                    )
                })
            {
                return Err(ErrorCode::OperationBusy);
            }
            cancel.check().map_err(|error| error.code())?;
            if kind == OperationKind::Uninstall {
                let outcome = tauri::async_runtime::spawn_blocking(move || {
                    launcher_core::minecraft::maintenance::uninstall(&paths, instance_id, &cancel)
                })
                .await
                .map_err(|_| ErrorCode::StorageCommitUncertain)?
                .map_err(|error| error.code())?;
                return Ok((None, outcome.retained_files, outcome.native_cleanup));
            }
            let platform = platform.as_ref().ok_or(ErrorCode::RuntimeUnsupported)?;
            let progress_controller = controller.clone();
            let progress_app = app.clone();
            let progress = Arc::new(move |progress| {
                if progress_controller.installation_progress(instance_id, request_id, progress) {
                    progress_controller.publish(&progress_app);
                }
            });
            if kind == OperationKind::Repair {
                let outcome = launcher_core::minecraft::maintenance::repair(
                    &paths,
                    instance_id,
                    platform,
                    &cancel,
                    progress,
                    credentials.clone(),
                )
                .await
                .map_err(|error| error.code())?;
                Ok((None, outcome.retained_files, outcome.native_cleanup))
            } else if let Some(update) = update {
                let outcome = launcher_core::minecraft::update::update(
                    &paths,
                    instance_id,
                    platform,
                    update,
                    &cancel,
                    progress,
                    credentials.clone(),
                )
                .await
                .map_err(|error| error.code())?;
                Ok((
                    outcome.backup.map(|backup| backup.operation),
                    0,
                    Some(outcome.native_cleanup),
                ))
            } else {
                launcher_core::minecraft::install::install(
                    &paths,
                    instance_id,
                    platform,
                    &cancel,
                    progress,
                    credentials.clone(),
                )
                .await
                .map_err(|error| error.code())?;
                Ok((None, 0, None))
            }
        }
        .await;
        match result {
            // The published manifest is authoritative if cancellation races
            // successful completion. Never report committed data as cancelled.
            Ok((backup, retained, native_cleanup)) => {
                controller.update(instance_id, request_id, |entry| {
                    entry.phase = if kind == OperationKind::Uninstall {
                        Phase::Uninstalled
                    } else {
                        Phase::Installed
                    };
                    entry.backup_operation = backup;
                    entry.retained_files = retained;
                    entry.native_cleanup = native_cleanup;
                })
            }
            Err(error) => {
                if let Some(credentials) = &credentials {
                    activation_commands::observe(
                        &app,
                        &controller,
                        &activation,
                        credentials,
                        error,
                    );
                }
                controller.fail(instance_id, request_id, error)
            }
        }
        controller.publish(&app);
    });
    Ok(())
}

#[tauri::command]
pub(super) fn begin_play(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
) -> Result<(), ErrorCode> {
    let platform = native_platform()?;
    let paths = AppPaths::new(
        app.path()
            .app_data_dir()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    )
    .map_err(|error| error.code())?;
    let accounts = account_commands::controller(&state)?;
    // An unverified session still launches installed instances; none at all does not.
    let activation = activation_commands::controller(&state)?;
    let credentials = activation
        .session_credentials()
        .map_err(|error| error.code())?;
    let controller = state.plays.clone();
    let cancel = controller.reserve(instance_id, request_id)?;
    controller.publish(&app);
    tauri::async_runtime::spawn(async move {
        // A session that could not be verified gets one check before Play, so a
        // reconnected launcher recovers without a restart. If the API closed the
        // session meanwhile, this launch ends here.
        let credentials = if activation
            .state()
            .is_ok_and(|state| state == launcher_core::activation::ActivationState::ActiveOffline)
        {
            // Bounded: a network that swallows requests must not stall every launch.
            if let Ok(Ok(outcome)) =
                tokio::time::timeout(std::time::Duration::from_secs(5), activation.refresh()).await
            {
                activation_commands::after_refresh(&app, &controller, &activation, outcome);
            }
            match activation.session_credentials() {
                Ok(fresh) => fresh,
                Err(error) => {
                    controller.fail(instance_id, request_id, error.code());
                    controller.publish(&app);
                    return;
                }
            }
        } else {
            credentials
        };
        match launcher_core::minecraft::play::prepare_play(
            paths,
            instance_id,
            accounts.clone(),
            platform,
            controller.process,
            cancel.clone(),
            Some(credentials.clone()),
        )
        .await
        {
            Ok(pending) => {
                if cancel.check().is_err() {
                    controller.fail(instance_id, request_id, ErrorCode::OperationCancelled);
                } else if pending.catalog_warning_required() {
                    controller.update(instance_id, request_id, |entry| {
                        if entry.cancel.check().is_ok() {
                            entry.phase = Phase::Warning;
                            entry.pending = Some(pending);
                        } else {
                            entry.phase = Phase::Cancelled;
                        }
                    });
                } else {
                    controller.update(instance_id, request_id, |entry| {
                        entry.phase = Phase::Starting
                    });
                    controller.publish(&app);
                    start_and_monitor(
                        app,
                        controller,
                        instance_id,
                        request_id,
                        pending,
                        None,
                        accounts,
                    )
                    .await;
                    return;
                }
            }
            Err(error) => {
                activation_commands::observe(
                    &app,
                    &controller,
                    &activation,
                    &credentials,
                    error.code(),
                );
                controller.fail(instance_id, request_id, error.code())
            }
        }
        controller.publish(&app);
    });
    Ok(())
}

#[tauri::command]
pub(super) fn confirm_play(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
    confirmation_id: Uuid,
) -> Result<(), ErrorCode> {
    let accounts = account_commands::controller(&state)?;
    let pending = state
        .plays
        .confirm(instance_id, request_id, confirmation_id)?;
    let controller = state.plays.clone();
    controller.publish(&app);
    tauri::async_runtime::spawn(start_and_monitor(
        app,
        controller,
        instance_id,
        request_id,
        pending,
        Some(confirmation_id),
        accounts,
    ));
    Ok(())
}

#[tauri::command]
pub(super) fn cancel_play(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
) -> Result<(), ErrorCode> {
    state.plays.cancel(instance_id, request_id)?;
    state.plays.publish(&app);
    Ok(())
}

#[tauri::command]
pub(super) async fn stop_game(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    instance_id: Uuid,
    request_id: Uuid,
    force: bool,
) -> Result<(), ErrorCode> {
    let controller = state.plays.clone();
    let game = {
        let mut state = controller
            .state
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let entry = state
            .entries
            .get_mut(&instance_id)
            .filter(|entry| entry.request == request_id)
            .ok_or(ErrorCode::OperationCancelled)?;
        let game = entry.game.clone().ok_or(ErrorCode::OperationCancelled)?;
        entry.exit_expected = true;
        game
    };
    let result = if force {
        game.force_stop().await
    } else {
        game.request_stop().await
    };
    match result {
        Ok(launcher_core::minecraft::process::StopOutcome::Requested) => {
            controller.update(instance_id, request_id, |entry| {
                if entry.game.is_some() {
                    entry.phase = Phase::Stopping;
                    entry.exit_expected = true;
                    entry.error = None;
                }
            })
        }
        Ok(launcher_core::minecraft::process::StopOutcome::Unsupported) => {
            controller.update(instance_id, request_id, |entry| entry.exit_expected = false);
            return Err(ErrorCode::RuntimeUnsupported);
        }
        Ok(_) => {}
        Err(error) => {
            controller.update(instance_id, request_id, |entry| {
                entry.exit_expected = false;
                entry.error = Some(error.code())
            });
            controller.publish(&app);
            return Err(error.code());
        }
    }
    controller.publish(&app);
    Ok(())
}

pub(super) fn prevent_close(app: &tauri::AppHandle) -> bool {
    let controller = &app.state::<DesktopState>().plays;
    let prevent = !controller.allow_close();
    if prevent {
        controller.publish(app);
    }
    prevent
}

pub(super) fn close_window(window: &tauri::Window) -> bool {
    let controller = &window.state::<DesktopState>().plays;
    let prevent = controller.close_window(|| window.hide().is_ok());
    if !prevent {
        // Auxiliary viewers must not keep an idle launcher without its main window.
        for (label, viewer) in window.app_handle().webview_windows() {
            if label.starts_with("game-log-") {
                let _ = viewer.close();
            }
        }
    }
    controller.publish(window.app_handle());
    prevent
}

pub(super) fn restore_window(app: &tauri::AppHandle, context: String) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("main")
            && handle.state::<DesktopState>().plays.restore()
        {
            #[cfg(target_os = "linux")]
            super::linux_activation::present(window, &context);
            #[cfg(not(target_os = "linux"))]
            {
                let _ = context;
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            handle.state::<DesktopState>().plays.publish(&handle);
        }
    });
}

#[cfg(test)]
mod tests;
