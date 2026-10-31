use serde::Serialize;

/// Stable, token-free error categories exposed to the desktop presentation layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidData,
    PermissionDenied,
    StorageUnavailable,
    StorageReadOnly,
    StorageBusy,
    StorageIoFailure,
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
    NativeCleanupIncomplete,
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
    LoaderProcessorFailed,
    LoaderProcessorTimeout,
    CatalogConfirmationRequired,
    WorldBackupUnsafe,
    UpdateSelectionChanged,
    /// The persisted account file is damaged or unsupported; it is left untouched.
    AccountsFileInvalid,
    /// A browser callback arrived when no login was waiting for it.
    CallbackNotActive,
    /// The data directory or a file in it does not have owner-only permissions.
    UnsafePermissions,
    /// A Panita activation is required before this operation.
    ActivationRequired,
    /// This build cannot reach the activation service (no launcher key compiled in).
    ActivationUnavailable,
    /// Nobody has the ign or Discord name that was typed.
    ActivationUnknownAccount,
    /// The Panita account exists but is disabled.
    ActivationNotActivated,
    /// Too many open login requests; wait before asking again.
    ActivationRateLimited,
    /// The login request outlived its three minutes.
    ActivationExpired,
    /// The login request is unknown or its session was already handed over.
    ActivationConsumed,
    /// The persisted Panita session file is damaged or unsupported; it is left untouched.
    ActivationFileInvalid,
    /// The API closed or refused the Panita session token.
    SessionRejected,
    /// The API refused the launcher key (for example, after a key rotation).
    ApiKeyRejected,
    /// Required pack files must be downloaded by the player before this can finish.
    ManualFilesRequired,
    /// A pack file would replace or sit inside player-owned content; nothing was changed.
    PackConflict,
    /// The installed modpack is older than the published one and must be updated.
    UpdateRequired,
    /// A newer launcher release is required before a new game launch can start.
    LauncherUpdateRequired,
    /// This installation cannot update itself; the player must install the release manually.
    LauncherUpdateUnsupported,
    /// A launcher update was found but could not be downloaded, verified or applied.
    LauncherUpdateFailed,
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
