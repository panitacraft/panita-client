use std::collections::VecDeque;
use std::sync::Mutex;

use crate::error::ErrorCode;
use crate::operation::{OperationId, ProcessSessionId};

const MAX_EVENTS: usize = 256;

/// Events contain only fixed identifiers and error codes. There is no API for
/// arbitrary messages, request URLs, account data, or token-bearing errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogEvent {
    ApplicationStarted,
    OperationCancelled,
    StorageFailure(ErrorCode),
    CatalogFailure(ErrorCode),
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
}
