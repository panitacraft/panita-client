//! Launcher self-update: check the signed feed, remember a newer release durably,
//! download it and apply it only while the launcher is idle. The policy lives in
//! `launcher_core::launcher_update`; this module only connects it to the updater plugin.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use launcher_core::error::ErrorCode;
use launcher_core::launcher_update::{self, UpdateChannel};
use launcher_core::logging::{BoundedLog, LogEvent, LogRecord};
use launcher_core::operation::ProcessSessionId;
use launcher_core::paths::AppPaths;
use launcher_core::storage::PrivateStore;
use serde::Serialize;
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::DesktopState;
use crate::play_commands::PlayController;

const EVENT: &str = "launcher-update-changed";
const RECHECK: Duration = Duration::from_secs(600);
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Idle,
    Checking,
    Downloading,
    Ready,
    Applying,
    /// The package cannot update itself; the player installs the release manually.
    Manual,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Delivery {
    InPlace,
    Manual,
}

#[derive(Default)]
struct Runtime {
    phase: Option<Phase>,
    update: Option<Arc<Update>>,
    bytes: Option<Arc<Vec<u8>>>,
    release_page: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    last_percent: Option<u8>,
    apply_requested: bool,
    checked: Option<Instant>,
    error: Option<ErrorCode>,
}

impl Runtime {
    fn phase(&self) -> Phase {
        self.phase.unwrap_or(Phase::Idle)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateView {
    current_version: String,
    channel: UpdateChannel,
    phase: Phase,
    pending_version: Option<String>,
    delivery: Delivery,
    release_url: Option<String>,
    progress: Option<u8>,
    apply_requested: bool,
    error: Option<ErrorCode>,
}

pub(super) struct UpdateController {
    plays: Arc<PlayController>,
    logs: Arc<BoundedLog>,
    process: ProcessSessionId,
    runtime: Mutex<Runtime>,
    work: tokio::sync::Mutex<()>,
    started: AtomicBool,
}

impl UpdateController {
    pub(super) fn new(
        plays: Arc<PlayController>,
        logs: Arc<BoundedLog>,
        process: ProcessSessionId,
    ) -> Self {
        Self {
            plays,
            logs,
            process,
            runtime: Mutex::default(),
            work: tokio::sync::Mutex::new(()),
            started: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Runtime> {
        self.runtime.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, code: ErrorCode) {
        self.logs.record(LogRecord {
            process: self.process,
            operation: None,
            event: LogEvent::LauncherUpdateFailure(code),
        });
    }

    fn change(&self, app: &tauri::AppHandle, mutate: impl FnOnce(&mut Runtime)) {
        mutate(&mut self.lock());
        let _ = app.emit(EVENT, self.view(app));
    }

    fn fail(&self, app: &tauri::AppHandle, code: ErrorCode) {
        self.record(code);
        self.change(app, |runtime| {
            runtime.phase = Some(Phase::Failed);
            runtime.error = Some(code);
            runtime.apply_requested = false;
        });
    }

    fn view(&self, app: &tauri::AppHandle) -> UpdateView {
        let state = store(app)
            .and_then(|store| launcher_update::load(&store).map_err(|error| error.code()))
            .unwrap_or_default();
        let current = launcher_update::current_version().ok();
        let pending_version = current
            .as_ref()
            .and_then(|current| state.pending_version.filter(|pending| pending > current))
            .map(|pending| pending.to_string());
        let runtime = self.lock();
        UpdateView {
            current_version: current
                .map(|version| version.to_string())
                .unwrap_or_default(),
            channel: state.channel,
            phase: runtime.phase(),
            pending_version,
            delivery: delivery(),
            release_url: runtime.release_page.clone(),
            progress: runtime
                .total
                .filter(|total| *total > 0)
                .map(|total| (runtime.downloaded.saturating_mul(100) / total).min(100) as u8),
            apply_requested: runtime.apply_requested,
            error: runtime.error,
        }
    }

    fn advance(&self, app: &tauri::AppHandle, chunk: usize, total: Option<u64>) {
        let percent = {
            let mut runtime = self.lock();
            runtime.downloaded = runtime.downloaded.saturating_add(chunk as u64);
            runtime.total = total;
            let percent = total
                .filter(|total| *total > 0)
                .map(|total| (runtime.downloaded.saturating_mul(100) / total).min(100) as u8);
            let changed = percent != runtime.last_percent;
            runtime.last_percent = percent;
            changed
        };
        if percent {
            let _ = app.emit(EVENT, self.view(app));
        }
    }
}

fn store(app: &tauri::AppHandle) -> Result<PrivateStore, ErrorCode> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let paths = AppPaths::new(root).map_err(|error| error.code())?;
    PrivateStore::open(&paths).map_err(|error| error.code())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Map an updater failure to a fixed code. Updater error text is never recorded or shown.
fn failure(error: &tauri_plugin_updater::Error) -> ErrorCode {
    use tauri_plugin_updater::Error;
    match error {
        Error::Reqwest(_) | Error::Network(_) | Error::ReleaseNotFound => {
            ErrorCode::NetworkUnavailable
        }
        Error::Minisign(_)
        | Error::SignatureUtf8(_)
        | Error::Base64(_)
        | Error::SignedVersionMismatch { .. }
        | Error::MissingSignedVersion => ErrorCode::ArtifactMismatch,
        Error::TargetNotFound(_)
        | Error::TargetsNotFound(_)
        | Error::UnsupportedOs
        | Error::UnsupportedArch => ErrorCode::LauncherUpdateUnsupported,
        _ => ErrorCode::LauncherUpdateFailed,
    }
}

/// How this installation can be updated, from the package type the bundler stamped into it.
/// Anything the updater cannot own (such as the Arch package, which carries a deb stamp
/// but is installed by pacman) is manual: the player installs the release page's package.
fn delivery() -> Delivery {
    #[allow(unused_imports)]
    use tauri::utils::config::BundleType;
    match tauri::utils::platform::bundle_type() {
        #[cfg(windows)]
        Some(BundleType::Nsis) => Delivery::InPlace,
        #[cfg(target_os = "linux")]
        Some(BundleType::AppImage) => Delivery::InPlace,
        #[cfg(target_os = "linux")]
        Some(BundleType::Deb) if !arch_owned() => Delivery::InPlace,
        _ => Delivery::Manual,
    }
}

#[cfg(target_os = "linux")]
fn arch_owned() -> bool {
    std::path::Path::new("/etc/arch-release").exists()
        || std::fs::read_dir("/var/lib/pacman/local").is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("panita-client-")
            })
        })
}

/// Check at startup, once per process and never in a development build.
pub(super) fn start_check(app: tauri::AppHandle, controller: Arc<UpdateController>) {
    if cfg!(debug_assertions) || controller.started.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move { check(app, controller, false).await });
}

/// `force` is an explicit player retry; otherwise recent checks and offers are reused.
async fn check(app: tauri::AppHandle, controller: Arc<UpdateController>, force: bool) {
    let _work = controller.work.lock().await;
    {
        let runtime = controller.lock();
        let held = matches!(runtime.phase(), Phase::Ready | Phase::Manual) && !force;
        let recent = runtime.checked.is_some_and(|at| at.elapsed() < RECHECK) && !force;
        if matches!(runtime.phase(), Phase::Downloading | Phase::Applying) || held || recent {
            return;
        }
    }
    controller.change(&app, |runtime| {
        runtime.phase = Some(Phase::Checking);
        runtime.error = None;
        runtime.checked = Some(Instant::now());
    });
    match fetch(&app).await {
        Err(code) => controller.fail(&app, code),
        Ok(None) => controller.change(&app, |runtime| {
            *runtime = Runtime {
                checked: runtime.checked,
                ..Runtime::default()
            };
        }),
        Ok(Some((update, page))) => {
            let manual = delivery() == Delivery::Manual;
            let update = Arc::new(update);
            controller.change(&app, |runtime| {
                runtime.update = Some(update.clone());
                runtime.bytes = None;
                runtime.release_page = Some(page);
                runtime.downloaded = 0;
                runtime.total = None;
                runtime.last_percent = None;
                runtime.phase = Some(if manual {
                    Phase::Manual
                } else {
                    Phase::Downloading
                });
            });
            if !manual {
                download(&app, &controller, update).await;
            }
        }
    }
}

/// Ask the feed, then validate and durably record any newer release it offers.
async fn fetch(app: &tauri::AppHandle) -> Result<Option<(Update, String)>, ErrorCode> {
    let store = store(app)?;
    let state = launcher_update::load(&store).map_err(|error| error.code())?;
    let current = launcher_update::current_version().map_err(|error| error.code())?;
    let feed =
        tauri::Url::parse(state.channel.feed_url()).map_err(|_| ErrorCode::LauncherUpdateFailed)?;
    let updater = app
        .updater_builder()
        .endpoints(vec![feed])
        .map_err(|error| failure(&error))?
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(|error| failure(&error))?;
    let offered = updater.check().await.map_err(|error| failure(&error))?;
    let Some(update) = offered else {
        launcher_update::record_check(&store, &current, None, now())
            .map_err(|error| error.code())?;
        return Ok(None);
    };
    let version = launcher_update::validate_offer(
        state.channel,
        &current,
        &update.version,
        update.download_url.as_str(),
    )
    .map_err(|error| error.code())?;
    launcher_update::record_check(&store, &current, Some(&version), now())
        .map_err(|error| error.code())?;
    Ok(Some((update, launcher_update::release_page(&version))))
}

async fn download(app: &tauri::AppHandle, controller: &Arc<UpdateController>, update: Arc<Update>) {
    let progress = controller.clone();
    let handle = app.clone();
    let result = update
        .download(
            move |chunk, total| progress.advance(&handle, chunk, total),
            || {},
        )
        .await;
    match result {
        // `download` verified the signature before returning the bytes.
        Ok(bytes) => {
            let waiting = {
                let mut runtime = controller.lock();
                runtime.bytes = Some(Arc::new(bytes));
                runtime.phase = Some(Phase::Ready);
                runtime.apply_requested
            };
            let _ = app.emit(EVENT, controller.view(app));
            if waiting {
                spawn_apply(app.clone(), controller.clone());
            }
        }
        Err(error) => controller.fail(app, failure(&error)),
    }
}

/// Wait until no game or operation is active, then reserve the launcher and apply.
fn spawn_apply(app: tauri::AppHandle, controller: Arc<UpdateController>) {
    tauri::async_runtime::spawn(async move {
        loop {
            {
                let runtime = controller.lock();
                if !runtime.apply_requested || runtime.phase() != Phase::Ready {
                    return;
                }
            }
            if controller.plays.begin_update_apply() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        apply(app, controller).await;
    });
}

async fn apply(app: tauri::AppHandle, controller: Arc<UpdateController>) {
    let (update, bytes) = {
        let mut runtime = controller.lock();
        match (runtime.update.clone(), runtime.bytes.clone()) {
            (Some(update), Some(bytes)) if runtime.phase() == Phase::Ready => {
                runtime.phase = Some(Phase::Applying);
                (update, bytes)
            }
            _ => {
                drop(runtime);
                controller.plays.cancel_update_apply();
                return;
            }
        }
    };
    let _ = app.emit(EVENT, controller.view(&app));
    // On Windows a successful install exits the process from inside the plugin.
    let outcome = tauri::async_runtime::spawn_blocking(move || install(&update, &bytes)).await;
    match outcome {
        Ok(Ok(())) => app.restart(),
        Ok(Err(code)) => {
            controller.plays.cancel_update_apply();
            controller.fail(&app, code);
        }
        Err(_) => {
            controller.plays.cancel_update_apply();
            controller.fail(&app, ErrorCode::LauncherUpdateFailed);
        }
    }
}

fn install(update: &Update, bytes: &[u8]) -> Result<(), ErrorCode> {
    #[cfg(target_os = "linux")]
    if matches!(
        tauri::utils::platform::bundle_type(),
        Some(tauri::utils::config::BundleType::Deb)
    ) {
        return install_deb(update, bytes);
    }
    update.install(bytes).map_err(|error| failure(&error))
}

/// The `.deb` goes through the system's `pkexec` prompt only. The launcher never reads
/// an administrator password and never falls back to `sudo` (AGENTS.md section 4.3).
#[cfg(target_os = "linux")]
fn install_deb(update: &Update, bytes: &[u8]) -> Result<(), ErrorCode> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::process::{Command, Stdio};

    update
        .verify(bytes)
        .map_err(|_| ErrorCode::ArtifactMismatch)?;
    let directory = std::env::temp_dir().join(format!("panita-update-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|_| ErrorCode::LauncherUpdateFailed)?;
    let package = directory.join("panita-client.deb");
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&package)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        let status = Command::new("pkexec")
            .arg("/usr/bin/dpkg")
            .arg("-i")
            .arg(&package)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(
                "package installation was not completed",
            ))
        }
    })();
    let _ = std::fs::remove_dir_all(&directory);
    result.map_err(|_| ErrorCode::LauncherUpdateFailed)
}

fn request_apply(
    app: &tauri::AppHandle,
    controller: &Arc<UpdateController>,
) -> Result<(), ErrorCode> {
    {
        let mut runtime = controller.lock();
        match runtime.phase() {
            Phase::Manual => return Err(ErrorCode::LauncherUpdateUnsupported),
            Phase::Applying => return Ok(()),
            Phase::Failed if runtime.bytes.is_some() => runtime.phase = Some(Phase::Ready),
            Phase::Ready => {}
            // Still downloading: apply as soon as the download completes.
            Phase::Downloading => {
                runtime.apply_requested = true;
                drop(runtime);
                let _ = app.emit(EVENT, controller.view(app));
                return Ok(());
            }
            _ => return Err(ErrorCode::LauncherUpdateFailed),
        }
        runtime.error = None;
        if runtime.apply_requested {
            return Ok(());
        }
        runtime.apply_requested = true;
    }
    let _ = app.emit(EVENT, controller.view(app));
    spawn_apply(app.clone(), controller.clone());
    Ok(())
}

#[tauri::command]
pub(super) fn launcher_update_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> UpdateView {
    state.launcher_update.view(&app)
}

/// `force` is an explicit retry; the window-focus recheck passes `false` and is throttled.
#[tauri::command]
pub(super) async fn check_launcher_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    force: bool,
) -> Result<UpdateView, ErrorCode> {
    let controller = state.launcher_update.clone();
    check(app.clone(), controller.clone(), force).await;
    Ok(controller.view(&app))
}

#[tauri::command]
pub(super) async fn set_update_channel(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    channel: UpdateChannel,
) -> Result<UpdateView, ErrorCode> {
    let controller = state.launcher_update.clone();
    if matches!(
        controller.lock().phase(),
        Phase::Downloading | Phase::Applying
    ) {
        return Err(ErrorCode::OperationBusy);
    }
    launcher_update::set_channel(&store(&app)?, channel).map_err(|error| error.code())?;
    controller.change(&app, |runtime| *runtime = Runtime::default());
    check(app.clone(), controller.clone(), true).await;
    Ok(controller.view(&app))
}

#[tauri::command]
pub(super) fn apply_launcher_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<UpdateView, ErrorCode> {
    request_apply(&app, &state.launcher_update)?;
    Ok(state.launcher_update.view(&app))
}

#[tauri::command]
pub(super) fn cancel_launcher_update_apply(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> UpdateView {
    let controller = state.launcher_update.clone();
    controller.change(&app, |runtime| {
        if runtime.phase() == Phase::Ready {
            runtime.apply_requested = false;
        }
    });
    controller.view(&app)
}

#[cfg(test)]
mod tests;
