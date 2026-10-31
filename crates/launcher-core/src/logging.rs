use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

use crate::error::ErrorCode;
use crate::operation::{OperationId, ProcessSessionId};
use crate::session_log::{self, Level, SessionFile};

const MAX_EVENTS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Workflow {
    Play,
    Install,
    Update,
    Repair,
    Uninstall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPhase {
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
    ExitedWithError,
    Failed,
    Cancelled,
}

/// Fixed browser launch stages; never include URLs or native error messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserLaunchEvent {
    DispatchFailed,
    ContextUnavailable,
    GioFailed,
    SystemOpenerFailed,
    SystemOpenerTimedOut,
    FallbackSucceeded,
}

impl BrowserLaunchEvent {
    fn name(self) -> &'static str {
        match self {
            Self::DispatchFailed => "browser_dispatch_failed",
            Self::ContextUnavailable => "browser_context_unavailable",
            Self::GioFailed => "browser_gio_failed",
            Self::SystemOpenerFailed => "browser_system_opener_failed",
            Self::SystemOpenerTimedOut => "browser_system_opener_timed_out",
            Self::FallbackSucceeded => "browser_fallback_succeeded",
        }
    }
}

/// Events contain only fixed identifiers and error codes. There is no API for
/// arbitrary messages, request URLs, account data, or token-bearing errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogEvent {
    ApplicationStarted,
    BrowserLaunch(BrowserLaunchEvent),
    OperationCancelled,
    StorageFailure(ErrorCode),
    CatalogFailure(ErrorCode),
    /// A Panita activation step failed or the session ended. Never the account,
    /// serial, token or service response, only the fixed error code.
    ActivationFailure(ErrorCode),
    LauncherUpdateFailure(ErrorCode),
    WorkflowChanged {
        request: Uuid,
        workflow: Workflow,
        phase: WorkflowPhase,
        error: Option<ErrorCode>,
    },
}

/// Explicit presentation allowlist. This never reads files or serializes backend
/// owners, account metadata, process arguments, callbacks or service responses.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRecord {
    process_session_id: Uuid,
    operation_id: Option<Uuid>,
    event: &'static str,
    workflow: Option<Workflow>,
    phase: Option<WorkflowPhase>,
    error: Option<ErrorCode>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSnapshot {
    capacity: usize,
    records: Vec<DiagnosticRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogRecord {
    pub process: ProcessSessionId,
    pub operation: Option<OperationId>,
    pub event: LogEvent,
}

#[derive(Default)]
pub struct BoundedLog {
    records: Mutex<VecDeque<LogRecord>>,
    file: Mutex<Option<SessionFile>>,
}

fn name_of(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        _ => String::from("unknown"),
    }
}

impl BoundedLog {
    pub fn record(&self, record: LogRecord) {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if records.len() == MAX_EVENTS {
            records.pop_front();
        }
        records.push_back(record);
        drop(records);
        let view = Self::view_of(record);
        self.write_file(Self::level_of(&view, &record), &Self::describe(&view));
    }

    /// Start this process's log file in `directory`, replaying events already held
    /// in memory. On failure in-memory diagnostics keep working.
    pub fn attach_file(
        &self,
        directory: &Path,
        process: ProcessSessionId,
        header: &[String],
    ) -> Option<PathBuf> {
        let mut file = SessionFile::create(directory, process).ok()?;
        file.write(
            Level::Info,
            &format!("session started process={}", process.value()),
        );
        for line in header {
            file.write(Level::Info, &session_log::redact(line));
        }
        for record in self.snapshot() {
            let view = Self::view_of(record);
            file.write(Self::level_of(&view, &record), &Self::describe(&view));
        }
        let path = file.path().to_path_buf();
        *self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(file);
        Some(path)
    }

    /// Record a free-text note in the session file only. The text is redacted and
    /// bounded; it never enters the in-memory view.
    pub fn note(&self, level: Level, message: &str) {
        self.write_file(level, &session_log::redact(message));
    }

    fn write_file(&self, level: Level, text: &str) {
        if let Some(file) = self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_mut()
        {
            file.write(level, text);
        }
    }

    fn level_of(view: &DiagnosticRecord, record: &LogRecord) -> Level {
        if view.error.is_some()
            && !matches!(
                record.event,
                LogEvent::BrowserLaunch(_) | LogEvent::OperationCancelled
            )
            || matches!(
                view.phase,
                Some(WorkflowPhase::Failed | WorkflowPhase::ExitedWithError)
            )
        {
            Level::Error
        } else if matches!(
            view.phase,
            Some(WorkflowPhase::Warning | WorkflowPhase::Cancelled | WorkflowPhase::Cancelling)
        ) || matches!(
            record.event,
            LogEvent::OperationCancelled | LogEvent::BrowserLaunch(_)
        ) {
            Level::Warn
        } else {
            Level::Info
        }
    }

    fn describe(view: &DiagnosticRecord) -> String {
        let mut text = view.event.to_owned();
        if let Some(workflow) = &view.workflow {
            text.push_str(&format!(" workflow={}", name_of(workflow)));
        }
        if let Some(phase) = &view.phase {
            text.push_str(&format!(" phase={}", name_of(phase)));
        }
        if let Some(error) = &view.error {
            text.push_str(&format!(" error={}", name_of(error)));
        }
        if let Some(operation) = view.operation_id {
            text.push_str(&format!(" operation={operation}"));
        }
        text
    }

    pub fn snapshot(&self) -> Vec<LogRecord> {
        self.records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .copied()
            .collect()
    }

    pub fn view(&self) -> DiagnosticSnapshot {
        let records = self.snapshot().into_iter().map(Self::view_of).collect();
        DiagnosticSnapshot {
            capacity: MAX_EVENTS,
            records,
        }
    }

    fn view_of(record: LogRecord) -> DiagnosticRecord {
        {
            {
                let mut view = DiagnosticRecord {
                    process_session_id: record.process.value(),
                    operation_id: record.operation.map(OperationId::value),
                    event: "application_started",
                    workflow: None,
                    phase: None,
                    error: None,
                };
                match record.event {
                    LogEvent::ApplicationStarted => (),
                    LogEvent::BrowserLaunch(event) => {
                        view.event = event.name();
                        if !matches!(
                            event,
                            BrowserLaunchEvent::FallbackSucceeded
                                | BrowserLaunchEvent::ContextUnavailable
                        ) {
                            view.error = Some(ErrorCode::BrowserUnavailable);
                        }
                    }
                    LogEvent::OperationCancelled => view.event = "operation_cancelled",
                    LogEvent::StorageFailure(code) => {
                        view.event = "storage_failure";
                        view.error = Some(code);
                    }
                    LogEvent::CatalogFailure(code) => {
                        view.event = "catalog_failure";
                        view.error = Some(code);
                    }
                    LogEvent::ActivationFailure(code) => {
                        view.event = "activation_failure";
                        view.error = Some(code);
                    }
                    LogEvent::LauncherUpdateFailure(code) => {
                        view.event = "launcher_update_failure";
                        view.error = Some(code);
                    }
                    LogEvent::WorkflowChanged {
                        request,
                        workflow,
                        phase,
                        error,
                    } => {
                        view.event = "workflow_changed";
                        view.operation_id = Some(request);
                        view.workflow = Some(workflow);
                        view.phase = Some(phase);
                        view.error = error;
                    }
                }
                view
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_diagnostics_contain_only_fixed_stages_and_public_identifiers() {
        let log = BoundedLog::default();
        let events = [
            BrowserLaunchEvent::DispatchFailed,
            BrowserLaunchEvent::ContextUnavailable,
            BrowserLaunchEvent::GioFailed,
            BrowserLaunchEvent::SystemOpenerFailed,
            BrowserLaunchEvent::SystemOpenerTimedOut,
            BrowserLaunchEvent::FallbackSucceeded,
        ];
        for event in events {
            log.record(LogRecord {
                process: ProcessSessionId::new(),
                operation: None,
                event: LogEvent::BrowserLaunch(event),
            });
        }
        let value = serde_json::to_value(log.view()).unwrap_or_else(|_| panic!("serialize"));
        for (index, event) in events.into_iter().enumerate() {
            let record = &value["records"][index];
            assert_eq!(record["event"], event.name());
            assert_eq!(
                record["error"],
                if matches!(
                    event,
                    BrowserLaunchEvent::FallbackSucceeded | BrowserLaunchEvent::ContextUnavailable
                ) {
                    serde_json::Value::Null
                } else {
                    serde_json::json!("browser_unavailable")
                }
            );
            assert_eq!(record.as_object().map(|object| object.len()), Some(6));
        }
    }

    #[test]
    fn ring_keeps_only_the_last_256_structured_events() {
        let log = BoundedLog::default();
        let process = ProcessSessionId::new();
        let first = OperationId::new();
        for index in 0..300 {
            log.record(LogRecord {
                process,
                operation: Some(if index == 0 {
                    first
                } else {
                    OperationId::new()
                }),
                event: LogEvent::ApplicationStarted,
            });
        }
        let records = log.snapshot();
        assert_eq!(records.len(), MAX_EVENTS);
        assert_ne!(records[0].operation, Some(first));
    }

    #[test]
    fn activation_failures_expose_only_a_fixed_event_and_an_error_code() {
        let log = BoundedLog::default();
        log.record(LogRecord {
            process: ProcessSessionId::new(),
            operation: None,
            event: LogEvent::ActivationFailure(ErrorCode::SessionRejected),
        });
        let value = serde_json::to_value(log.view()).unwrap_or_else(|_| panic!("serialize"));
        let record = &value["records"][0];
        assert_eq!(record["event"], "activation_failure");
        assert_eq!(record["error"], "session_rejected");
        assert_eq!(record["operationId"], serde_json::Value::Null);
        assert_eq!(record["workflow"], serde_json::Value::Null);
    }

    #[test]
    fn diagnostic_payload_has_only_fixed_event_categories_and_public_identifiers() {
        let log = BoundedLog::default();
        let process = ProcessSessionId::new();
        let request = Uuid::new_v4();
        log.record(LogRecord {
            process,
            operation: None,
            event: LogEvent::WorkflowChanged {
                request,
                workflow: Workflow::Repair,
                phase: WorkflowPhase::Failed,
                error: Some(ErrorCode::ArtifactMismatch),
            },
        });
        let value = serde_json::to_value(log.view()).unwrap_or_else(|_| panic!("serialize"));
        assert_eq!(value["capacity"], 256);
        assert_eq!(value["records"][0]["operationId"], request.to_string());
        assert_eq!(value["records"][0]["workflow"], "repair");
        assert_eq!(value["records"][0]["error"], "artifact_mismatch");
        let keys: Vec<_> = value["records"][0]
            .as_object()
            .unwrap_or_else(|| panic!("record"))
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "error",
                "event",
                "operationId",
                "phase",
                "processSessionId",
                "workflow"
            ]
        );
        assert_eq!(
            log.snapshot().len(),
            1,
            "reading does not drain diagnostics"
        );
        for _ in 0..300 {
            log.record(LogRecord {
                process,
                operation: None,
                event: LogEvent::CatalogFailure(ErrorCode::NetworkUnavailable),
            });
        }
        let value = serde_json::to_value(log.view()).unwrap_or_else(|_| panic!("serialize"));
        assert_eq!(value["records"].as_array().map(Vec::len), Some(MAX_EVENTS));
        assert!(
            value["records"]
                .as_array()
                .unwrap_or_else(|| panic!("records"))
                .iter()
                .all(|entry| entry["event"] == "catalog_failure")
        );
    }
}
