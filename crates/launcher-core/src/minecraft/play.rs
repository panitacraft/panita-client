//! Launch preflight and single-use Play attempts. No credentials cross IPC.
use std::sync::Arc;
use uuid::Uuid;

use super::download::Cancellation;
use super::install::InstalledLock;
use super::launch::build_command;
use super::process::{LaunchLease, RunningGame};
use super::{Platform, invalid};
use crate::catalog::{CatalogClient, CatalogInstance, CatalogSource};
use crate::error::{CoreError, ErrorCode};
use crate::operation::{OperationId, ProcessSessionId};
use crate::paths::AppPaths;
use crate::registry::{InstallationState, InstanceRegistry};
use crate::sessions::{AccountController, PlaySelection};
use crate::settings::{GlobalSettings, InstanceSettings};
use crate::storage::PrivateStore;

/// Backend-only attempt. Dropping it cancels the attempt and releases its lease.
/// Confirmation belongs to this operation only and is never remembered globally.
pub struct PendingPlay {
    paths: AppPaths,
    accounts: Arc<AccountController>,
    selection: PlaySelection,
    lease: LaunchLease,
    installed: InstalledLock,
    global: GlobalSettings,
    settings: InstanceSettings,
    platform: Platform,
    operation: OperationId,
    process: ProcessSessionId,
    cancel: Cancellation,
    warning_required: bool,
}

fn client_version() -> Result<semver::Version, CoreError> {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())
}

fn catalog_decision(
    local: &CatalogInstance,
    detail: Result<CatalogInstance, CoreError>,
) -> Result<bool, CoreError> {
    match detail {
        Ok(current) => {
            current.installation_allowed(&client_version()?)?;
            Ok(false)
        }
        Err(error)
            if matches!(
                error.code(),
                ErrorCode::NetworkUnavailable | ErrorCode::ServiceUnavailable
            ) =>
        {
            local.installation_allowed(&client_version()?)?;
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

/// Reserve one instance, query its specific API status and verify a complete
/// existing installation. Never install, repair, download or execute Java here.
pub async fn prepare_play(
    paths: AppPaths,
    instance: Uuid,
    accounts: Arc<AccountController>,
    platform: Platform,
    process: ProcessSessionId,
    cancel: Cancellation,
) -> Result<PendingPlay, CoreError> {
    let catalog = CatalogClient::new()?;
    prepare_with_catalog(
        paths, instance, accounts, platform, process, cancel, catalog,
    )
    .await
}

async fn prepare_with_catalog(
    paths: AppPaths,
    instance: Uuid,
    accounts: Arc<AccountController>,
    platform: Platform,
    process: ProcessSessionId,
    cancel: Cancellation,
    catalog: CatalogClient,
) -> Result<PendingPlay, CoreError> {
    cancel.check()?;
    let selection = accounts.play_selection()?;
    let root = paths.clone();
    let lease = tokio::task::spawn_blocking(move || LaunchLease::acquire(&root, instance))
        .await
        .map_err(|_| CoreError::new(ErrorCode::StorageUnavailable))??;
    cancel.check()?;
    accounts.check_play_selection(&selection)?;
    let registry = InstanceRegistry::new(paths.clone());
    let local = registry
        .get(instance)?
        .ok_or_else(|| CoreError::new(ErrorCode::InstanceMissing))?;
    let detail = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        detail = catalog.detail(instance) => detail,
    };
    if let Ok(current) = &detail {
        // Preserve an explicit disabled result even though this attempt fails.
        registry.remember_catalog(std::slice::from_ref(current))?;
    }
    let warning_required = catalog_decision(&local.last_catalog, detail)?;
    let root = paths.clone();
    let audit_cancel = cancel.clone();
    let audit = tokio::task::spawn_blocking(move || {
        audit_cancel.check()?;
        lease.recover_installation()?;
        let record = InstanceRegistry::new(root.clone())
            .get(instance)?
            .ok_or_else(invalid)?;
        if !matches!(
            record.state,
            InstallationState::Installed | InstallationState::UpdateAvailable
        ) {
            return Err(invalid());
        }
        let installed = lease.verify_installation_with_cancel(&audit_cancel)?;
        if !matches!(&record.resolved_source, CatalogSource::Vanilla { minecraft_version } if minecraft_version == &installed.plan.version.id)
        {
            return Err(CoreError::new(ErrorCode::UnsupportedInstance));
        }
        let global = GlobalSettings::load(&PrivateStore::open(&root)?)?;
        audit_cancel.check()?;
        Ok((lease, installed, global, record.settings))
    });
    let (lease, installed, global, settings) = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = audit => result.map_err(|_| CoreError::new(ErrorCode::StorageUnavailable))??,
    };
    accounts.check_play_selection(&selection)?;
    Ok(PendingPlay {
        paths,
        accounts,
        selection,
        lease,
        installed,
        global,
        settings,
        platform,
        operation: OperationId::new(),
        process,
        cancel,
        warning_required,
    })
}

impl PendingPlay {
    /// Presentation preference captured with the launch settings for this attempt.
    pub fn show_console_on_start(&self) -> bool {
        self.settings
            .show_console_on_start
            .unwrap_or(self.global.show_console_on_start)
    }

    pub fn operation_id(&self) -> Uuid {
        self.operation.value()
    }

    pub fn catalog_warning_required(&self) -> bool {
        self.warning_required
    }

    /// The desktop must show its Spanish outage warning and pass this attempt's
    /// operation UUID only after the player confirms. Each attempt is single-use.
    pub async fn start(self, confirmed_warning: Option<Uuid>) -> Result<RunningGame, CoreError> {
        self.cancel.check()?;
        if self.warning_required && confirmed_warning != Some(self.operation.value()) {
            return Err(CoreError::new(ErrorCode::CatalogConfirmationRequired));
        }
        if confirmed_warning.is_some_and(|id| id != self.operation.value()) {
            return Err(invalid());
        }
        self.accounts.check_play_selection(&self.selection)?;
        let record = InstanceRegistry::new(self.paths.clone())
            .get(self.installed.instance_id)?
            .ok_or_else(invalid)?;
        record
            .last_catalog
            .installation_allowed(&client_version()?)?;
        let authorization = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
            result = self.accounts.authorize_selected_play(&self.selection) => result?,
        };
        self.cancel.commit(|| {
            self.accounts
                .with_play_authorization(authorization, |identity| {
                    let command = build_command(
                        &self.paths,
                        self.lease.game_directory(),
                        &self.installed,
                        &self.platform,
                        &self.global,
                        &self.settings,
                        identity,
                    )?;
                    self.lease.spawn(command, self.process, self.operation)
                })
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
