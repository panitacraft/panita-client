//! Desktop ownership and token-free snapshots for the core Play workflow.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use launcher_core::error::ErrorCode;
use launcher_core::minecraft::{
    Platform, download::Cancellation, play::PendingPlay, process::RunningGame,
};
use launcher_core::operation::ProcessSessionId;
use launcher_core::paths::AppPaths;
use serde_json::{Value, json};
use tauri::{Emitter, Manager};
use uuid::Uuid;

use super::{DesktopState, account_commands};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    Installing,
    Installed,
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
    fn active(self) -> bool {
        !matches!(
            self,
            Self::Installed | Self::Exited | Self::Failed | Self::Cancelled
        )
    }
    fn text(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Installing => "installing",
            Self::Installed => "installed",
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
    error: Option<ErrorCode>,
    exit: Option<launcher_core::minecraft::process::GameExit>,
    installation: bool,
    received_files: u64,
    reused_files: u64,
    last_progress: std::time::Instant,
}

#[derive(Default)]
struct State {
    revision: u64,
    closing: bool,
    close_blocked: bool,
    background: bool,
    entries: HashMap<Uuid, Entry>,
}

impl State {
    fn cancel_pending(&mut self, include_installations: bool) {
        for entry in self.entries.values_mut() {
            if entry.installation && !include_installations {
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
    pub fn new(process: ProcessSessionId) -> Self {
        Self {
            process,
            state: Mutex::new(State::default()),
        }
    }

    fn reserve(&self, instance: Uuid, request: Uuid) -> Result<Cancellation, ErrorCode> {
        self.reserve_operation(instance, request, false)
    }

    fn reserve_operation(
        &self,
        instance: Uuid,
        request: Uuid,
        installation: bool,
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
                phase: if installation {
                    Phase::Installing
                } else {
                    Phase::Preparing
                },
                cancel: cancel.clone(),
                pending: None,
                game: None,
                error: None,
                exit: None,
                installation,
                received_files: 0,
                reused_files: 0,
                last_progress: std::time::Instant::now(),
            },
        );
        state.close_blocked = false;
        state.revision += 1;
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
                    "operation": if entry.installation { "install" } else { "play" },
                    "receivedFiles": entry.received_files, "reusedFiles": entry.reused_files,
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
        state.revision += 1;
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
    match pending.start(confirmation).await {
        Ok(game) => {
            // Always retain a committed child, even if cancellation arrived after
            // the atomic spawn boundary. Account removal must not kill that game.
            let game = Arc::new(game);
            controller.update(instance, request, |entry| {
                entry.phase = Phase::Running;
                entry.game = Some(game.clone());
            });
            controller.publish(&app);
            account_commands::emit_accounts(&app, &accounts);
            match game.wait().await {
                Ok(exit) => controller.update(instance, request, |entry| {
                    entry.phase = Phase::Exited;
                    entry.exit = Some(exit);
                    entry.game = None;
                    entry.error = None;
                }),
                Err(error) => controller.update(instance, request, |entry| {
                    entry.error = Some(error.code());
                }),
            }
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

#[tauri::command]
pub(super) fn begin_install(
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
    // Use the same initialized private root as the other desktop workflows.
    account_commands::controller(&state)?;
    let controller = state.plays.clone();
    let cancel = controller.reserve_operation(instance_id, request_id, true)?;
    controller.publish(&app);
    tauri::async_runtime::spawn(async move {
        let result = async {
            use launcher_core::registry::{InstallationState, InstanceRegistry};
            let record = InstanceRegistry::new(paths.clone())
                .get(instance_id)
                .map_err(|error| error.code())?;
            if record.is_some_and(|record| {
                matches!(
                    record.state,
                    InstallationState::Installed
                        | InstallationState::UpdateAvailable
                        | InstallationState::Running
                )
            }) {
                return Err(ErrorCode::OperationBusy);
            }
            cancel.check().map_err(|error| error.code())?;
            let progress_controller = controller.clone();
            let progress_app = app.clone();
            let progress = Arc::new(move |progress| {
                if progress_controller.installation_progress(instance_id, request_id, progress) {
                    progress_controller.publish(&progress_app);
                }
            });
            launcher_core::minecraft::install::install(
                &paths,
                instance_id,
                &platform,
                &cancel,
                progress,
            )
            .await
            .map_err(|error| error.code())?;
            Ok(())
        }
        .await;
        match result {
            // The published manifest is authoritative if cancellation races
            // successful completion. Never report committed data as cancelled.
            Ok(()) => controller.update(instance_id, request_id, |entry| {
                entry.phase = Phase::Installed
            }),
            Err(error) => controller.fail(instance_id, request_id, error),
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
    let controller = state.plays.clone();
    let cancel = controller.reserve(instance_id, request_id)?;
    controller.publish(&app);
    tauri::async_runtime::spawn(async move {
        match launcher_core::minecraft::play::prepare_play(
            paths,
            instance_id,
            accounts.clone(),
            platform,
            controller.process,
            cancel.clone(),
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
            Err(error) => controller.fail(instance_id, request_id, error.code()),
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
        let state = controller
            .state
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        state
            .entries
            .get(&instance_id)
            .filter(|entry| entry.request == request_id)
            .and_then(|entry| entry.game.clone())
            .ok_or(ErrorCode::OperationCancelled)?
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
                    entry.error = None;
                }
            })
        }
        Ok(launcher_core::minecraft::process::StopOutcome::Unsupported) => {
            return Err(ErrorCode::RuntimeUnsupported);
        }
        Ok(_) => {}
        Err(error) => {
            controller.update(instance_id, request_id, |entry| {
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
