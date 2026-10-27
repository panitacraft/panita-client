//! Low-level owned-child supervision. Callers must complete Play preflight before
//! spawning and keep the Tokio runtime alive until the supervised child exits.
//! This module does not authorize a launch or implement desktop close behavior.
use std::collections::VecDeque;
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
        let mut builder = tokio::process::Command::new(&command.executable);
        builder
            .args(&command.args)
            .current_dir(&command.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_remove("JAVA_TOOL_OPTIONS")
            .env_remove("_JAVA_OPTIONS")
            .env_remove("JDK_JAVA_OPTIONS")
            .kill_on_drop(false);
        #[cfg(windows)]
        builder.creation_flags(0x08000000);
        let mut child = builder
            .spawn()
            .map_err(|_| CoreError::new(ErrorCode::ProcessStartFailed))?;
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
            self,
            requests,
            complete,
            events.clone(),
            (output, errors),
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
    lease: LaunchLease,
    mut controls: mpsc::Receiver<Request>,
    complete: watch::Sender<Option<GameExit>>,
    events: Arc<Events>,
    pipes: (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>),
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
                    let result = stop_child(&mut child, request.force);
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
        drain(pipes.0, OutputStream::Stdout, &events),
        drain(pipes.1, OutputStream::Stderr, &events)
    );
    events.push(ProcessEventKind::Exited(status));
    drop(lease);
    let _ = complete.send(Some(status));
}

fn stop_child(child: &mut tokio::process::Child, force: bool) -> Result<StopOutcome, ErrorCode> {
    if force {
        child
            .start_kill()
            .map_err(|_| ErrorCode::ProcessControlFailed)?;
        return Ok(StopOutcome::Requested);
    }
    #[cfg(target_os = "linux")]
    {
        // The actor owns the unreaped Child; no other task waits or accepts a
        // caller-supplied PID. An exited unreaped child cannot have its PID reused.
        let pid = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(rustix::process::Pid::from_raw)
            .ok_or(ErrorCode::ProcessControlFailed)?;
        rustix::process::kill_process(pid, rustix::process::Signal::TERM)
            .map_err(|_| ErrorCode::ProcessControlFailed)?;
        Ok(StopOutcome::Requested)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(StopOutcome::Unsupported)
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
