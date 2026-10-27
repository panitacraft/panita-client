use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;
use uuid::Uuid;

use crate::error::ErrorCode;
use crate::operation::{OperationId, ProcessSessionId};

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

/// Events contain only fixed identifiers and error codes. There is no API for
/// arbitrary messages, request URLs, account data, or token-bearing errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogEvent {
    ApplicationStarted,
    OperationCancelled,
    StorageFailure(ErrorCode),
    CatalogFailure(ErrorCode),
    /// A Panita activation step failed or the session ended. Never the account,
    /// serial, token or service response, only the fixed error code.
    ActivationFailure(ErrorCode),
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
        let records = self
            .snapshot()
            .into_iter()
            .map(|record| {
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
            })
            .collect();
        DiagnosticSnapshot {
            capacity: MAX_EVENTS,
            records,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
