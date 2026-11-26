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
    /// The requested or installed variant is not offered by the instance any more.
    VariantUnavailable,
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

/// A fixed, token-free reason that refines an [`ErrorCode`], so one generic code such
/// as `invalid_data` can say which stage rejected the data. Never carries a message.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCause {
    /// The catalog's manifest for a pack version could not be turned into an install plan.
    ManifestInvalid,
    /// The recorded installation on disk is unreadable or inconsistent.
    InstalledRecordInvalid,
    /// A configuration or override archive of the pack failed validation.
    ArchiveInvalid,
    /// The state an operation was about to record failed its own validation.
    PublishedStateInvalid,
    /// A pack path was refused by the game-directory path rules.
    UnsafePath,
    /// A file's bytes did not match the expected size or hash.
    ChecksumMismatch,
    /// Another process holds the file, or the OS reports a sharing violation.
    FileInUse,
    /// The operating system refused access to the file.
    AccessDenied,
    /// The disk has no space left for the file.
    DiskFull,
    /// Any other file-system failure.
    FileIo,
}

impl ErrorCause {
    /// The cause a code implies when nothing more specific was recorded.
    fn implied_by(code: ErrorCode) -> Option<Self> {
        match code {
            ErrorCode::ArtifactMismatch => Some(Self::ChecksumMismatch),
            ErrorCode::StorageBusy => Some(Self::FileInUse),
            ErrorCode::PermissionDenied | ErrorCode::UnsafePermissions => Some(Self::AccessDenied),
            ErrorCode::InsufficientSpace => Some(Self::DiskFull),
            ErrorCode::StorageUnavailable
            | ErrorCode::StorageReadOnly
            | ErrorCode::StorageIoFailure => Some(Self::FileIo),
            _ => None,
        }
    }
}

/// What a failure can say about itself beyond its code. Every field is optional and
/// none can hold a message from the operating system or a service.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorDetail {
    pub cause: Option<ErrorCause>,
    /// Game-directory relative path (always `/`-separated) of the file concerned.
    pub path: Option<String>,
    /// Raw operating-system error number, for example 32 for a Windows sharing violation.
    pub os_error: Option<i32>,
}

impl ErrorDetail {
    pub fn is_empty(&self) -> bool {
        self.cause.is_none() && self.path.is_none() && self.os_error.is_none()
    }

    /// One log-safe fragment such as `cause=file_in_use path=mods/a.jar os_error=32`.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(cause) = self.cause
            && let Ok(serde_json::Value::String(name)) = serde_json::to_value(cause)
        {
            parts.push(format!("cause={name}"));
        }
        if let Some(path) = &self.path {
            parts.push(format!("path={path}"));
        }
        if let Some(number) = self.os_error {
            parts.push(format!("os_error={number}"));
        }
        parts.join(" ")
    }
}

const MAX_PATH_CHARS: usize = 240;

#[derive(Debug)]
pub struct CoreError {
    code: ErrorCode,
    cause: Option<ErrorCause>,
    path: Option<String>,
    os_error: Option<i32>,
}

impl CoreError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            cause: None,
            path: None,
            os_error: None,
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// Record the exact cause, replacing any earlier one.
    pub fn with_cause(mut self, cause: ErrorCause) -> Self {
        self.cause = Some(cause);
        self
    }

    /// Name the stage that rejected the data, but only for a bare `invalid_data`: a
    /// network, storage or checksum failure keeps the cause it already has.
    pub fn or_cause(mut self, cause: ErrorCause) -> Self {
        if self.code == ErrorCode::InvalidData && self.cause.is_none() {
            self.cause = Some(cause);
        }
        self
    }

    /// Record the game-relative file concerned, unless an inner step already did.
    pub fn with_path(mut self, path: &str) -> Self {
        if self.path.is_none() {
            let clean: String = path
                .chars()
                .filter(|ch| !ch.is_control())
                .take(MAX_PATH_CHARS)
                .collect();
            if !clean.is_empty() {
                self.path = Some(clean);
            }
        }
        self
    }

    pub fn with_os_error(mut self, number: Option<i32>) -> Self {
        self.os_error = number;
        self
    }

    /// Keep what an earlier failure knew when a later one replaces it, as when a
    /// rollback cannot finish and hides why the operation failed in the first place.
    pub fn with_detail_of(mut self, earlier: &CoreError) -> Self {
        self.cause = self.cause.or(earlier.cause());
        if self.path.is_none() {
            self.path.clone_from(&earlier.path);
        }
        self.os_error = self.os_error.or(earlier.os_error);
        self
    }

    pub fn cause(&self) -> Option<ErrorCause> {
        self.cause.or_else(|| ErrorCause::implied_by(self.code))
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn os_error(&self) -> Option<i32> {
        self.os_error
    }

    pub fn detail(&self) -> ErrorDetail {
        ErrorDetail {
            cause: self.cause(),
            path: self.path.clone(),
            os_error: self.os_error,
        }
    }
}

impl From<ErrorCode> for CoreError {
    fn from(code: ErrorCode) -> Self {
        Self::new(code)
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for CoreError {}

/// Attach detail to the error of a result without disturbing its code.
pub trait ResultDetail<T> {
    /// See [`CoreError::or_cause`].
    fn or_cause(self, cause: ErrorCause) -> Result<T, CoreError>;
    /// See [`CoreError::with_path`].
    fn at_path(self, path: &str) -> Result<T, CoreError>;
}

impl<T> ResultDetail<T> for Result<T, CoreError> {
    fn or_cause(self, cause: ErrorCause) -> Result<T, CoreError> {
        self.map_err(|error| error.or_cause(cause))
    }

    fn at_path(self, path: &str) -> Result<T, CoreError> {
        self.map_err(|error| error.with_path(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_refines_a_bare_invalid_data_but_keeps_other_causes() {
        let bare = CoreError::new(ErrorCode::InvalidData).or_cause(ErrorCause::ArchiveInvalid);
        assert_eq!(bare.cause(), Some(ErrorCause::ArchiveInvalid));
        let checksum =
            CoreError::new(ErrorCode::ArtifactMismatch).or_cause(ErrorCause::ArchiveInvalid);
        assert_eq!(checksum.cause(), Some(ErrorCause::ChecksumMismatch));
        let first = CoreError::new(ErrorCode::InvalidData)
            .or_cause(ErrorCause::ManifestInvalid)
            .or_cause(ErrorCause::ArchiveInvalid);
        assert_eq!(first.cause(), Some(ErrorCause::ManifestInvalid));
    }

    #[test]
    fn the_innermost_path_wins_and_control_characters_are_dropped() {
        let error = CoreError::new(ErrorCode::StorageBusy)
            .with_path("mods/a\u{7}.jar")
            .with_path("mods/outer.jar");
        assert_eq!(error.path(), Some("mods/a.jar"));
        assert_eq!(error.cause(), Some(ErrorCause::FileInUse));
        let long = CoreError::new(ErrorCode::InvalidData).with_path(&"x".repeat(1000));
        assert_eq!(long.path().map(str::len), Some(MAX_PATH_CHARS));
    }

    #[test]
    fn description_is_a_fixed_fragment_without_messages() {
        let error = CoreError::new(ErrorCode::StorageBusy)
            .with_path("mods/a.jar")
            .with_os_error(Some(32));
        assert_eq!(
            error.detail().describe(),
            "cause=file_in_use path=mods/a.jar os_error=32"
        );
        assert!(CoreError::new(ErrorCode::OperationBusy).detail().is_empty());
    }
}
