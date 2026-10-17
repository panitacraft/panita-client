use serde::Serialize;

/// Stable, token-free error categories exposed to the desktop presentation layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidData,
    PermissionDenied,
    StorageUnavailable,
    StorageCommitUncertain,
    NetworkUnavailable,
    AuthenticationRequired,
    AccountNotFound,
    AccountMismatch,
    InvalidUsername,
    AuthenticationDenied,
    AuthenticationConfiguration,
    AuthenticationTimeout,
    ServiceUnavailable,
    AccountRestricted,
    MinecraftNotOwned,
    OperationBusy,
    CallbackRegistrationFailed,
    BrowserUnavailable,
    InstanceDisabled,
    InstanceMissing,
    UnsupportedInstance,
    OperationCancelled,
    ArtifactMismatch,
    RuntimeUnsupported,
    InsufficientSpace,
    ProcessStartFailed,
    ProcessControlFailed,
    CatalogConfirmationRequired,
    /// The persisted account file is damaged or unsupported; it is left untouched.
    AccountsFileInvalid,
    /// A browser callback arrived when no login was waiting for it.
    CallbackNotActive,
    /// The data directory or a file in it does not have owner-only permissions.
    UnsafePermissions,
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
