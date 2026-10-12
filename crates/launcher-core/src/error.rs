use serde::Serialize;

/// Stable, token-free error categories exposed to the desktop presentation layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidData,
    PermissionDenied,
    StorageUnavailable,
    NetworkUnavailable,
    AuthenticationRequired,
    InstanceDisabled,
    UnsupportedInstance,
    OperationCancelled,
}

#[derive(Debug)]
pub struct CoreError {
    code: ErrorCode,
}

impl CoreError {
    pub fn new(code: ErrorCode) -> Self {
        Self { code }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for CoreError {}
