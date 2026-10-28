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
use crate::modpack::state::{PackIdentity, PackState};
use crate::operation::{OperationId, ProcessSessionId};
use crate::panita_api::PanitaCredentials;
use crate::paths::AppPaths;
use crate::registry::{InstallationState, InstanceRegistry};
use crate::sessions::{AccountController, PlayAuthorization, PlaySelection};
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
    authorization: Option<AuthorizationTask>,
}

/// A validation older than this is discarded and repeated, so a launch confirmed
/// after a long warning still validates the account freshly.
const AUTHORIZATION_FRESH: std::time::Duration = std::time::Duration::from_secs(30);

/// Remote Play validation started once the catalog allows the launch, so it overlaps
/// the local audit instead of following it. It is single-use, aborted when its
/// attempt is dropped, and never started for an attempt that needs a warning.
struct AuthorizationTask {
    handle: Option<tokio::task::JoinHandle<Result<PlayAuthorization, CoreError>>>,
    started: std::time::Instant,
}

impl AuthorizationTask {
    fn start(accounts: Arc<AccountController>, selection: PlaySelection) -> Self {
        Self {
            handle: Some(tokio::spawn(async move {
                accounts.authorize_selected_play(&selection).await
            })),
            started: std::time::Instant::now(),
        }
    }

    fn fresh(&self) -> bool {
        self.started.elapsed() < AUTHORIZATION_FRESH
    }

    async fn finish(mut self) -> Result<PlayAuthorization, CoreError> {
        let handle = self.handle.take().ok_or_else(invalid)?;
        handle
            .await
            .map_err(|_| CoreError::new(ErrorCode::StorageUnavailable))?
    }
}

impl Drop for AuthorizationTask {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
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

/// Modpack updates are mandatory: a current response naming another published
/// version than the installed one blocks Play until the player updates. A failed
/// check never reaches here, so an outage keeps the warning-confirmed launch.
fn require_current_pack(
    installed: &CatalogSource,
    current: &CatalogSource,
) -> Result<(), CoreError> {
    match (installed, current) {
        (
            CatalogSource::Custom {
                current_version_id: installed,
                ..
            },
            CatalogSource::Custom {
                current_version_id: current,
                ..
            },
        ) if installed != current => Err(CoreError::new(ErrorCode::UpdateRequired)),
        (
            CatalogSource::Modrinth {
                project_id: installed_project,
                pack_version_id: installed,
                ..
            },
            CatalogSource::Modrinth {
                project_id: current_project,
                pack_version_id: current,
                ..
            },
        ) if installed_project != current_project || installed != current => {
            Err(CoreError::new(ErrorCode::UpdateRequired))
        }
        _ => Ok(()),
    }
}

/// The installed pack must be the one this catalog entry pins, in the same format.
fn pack_matches(source: &CatalogSource, pack: &PackState) -> bool {
    match (source, &pack.source) {
        (
            CatalogSource::Custom {
                minecraft_version,
                loader,
                ..
            },
            PackIdentity::Custom { .. },
        ) => {
            loader == "neoforge"
                && pack.loader == *loader
                && &pack.minecraft_version == minecraft_version
        }
        (
            CatalogSource::Modrinth {
                project_id,
                pack_version_id,
                loader,
            },
            PackIdentity::Modrinth {
                project_id: installed_project,
                version_id: installed_version,
                ..
            },
        ) => {
            pack.loader == "neoforge"
                && loader.as_deref().is_none_or(|loader| loader == "neoforge")
                && project_id == installed_project
                && pack_version_id == installed_version
        }
        _ => false,
    }
}

/// A complete modpack launches only with its manual files supplied. Its managed
/// content files were already checked, with receipts, by the installation audit; a
/// damaged one needs the explicit repair of Phase 12.
fn verify_pack(
    installed: &InstalledLock,
    record: &crate::registry::InstallationRecord,
    cancel: &Cancellation,
) -> Result<(), CoreError> {
    let (Some(pack), Some(_)) = (&installed.pack, &installed.plan.loader) else {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    };
    if !pack_matches(&record.resolved_source, pack)
        || pack.minecraft_version != installed.plan.version.id
    {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    if !pack.pending.is_empty() {
        return Err(CoreError::new(ErrorCode::ManualFilesRequired));
    }
    // Content files were checked by the installation audit, with Play receipts.
    cancel.check()
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
    auth: Option<PanitaCredentials>,
) -> Result<PendingPlay, CoreError> {
    let catalog = CatalogClient::new()?.with_auth(auth);
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
    // The audit does not depend on the catalog answer, so it runs while that is awaited.
    let root = paths.clone();
    let audit_cancel = cancel.clone();
    let audit = tokio::task::spawn_blocking(move || {
        audit_cancel.check()?;
        lease.recover_installation()?;
        let record = InstanceRegistry::new(root.clone())
            .get(instance)?
            .ok_or_else(invalid)?;
        if record.state == InstallationState::AwaitingFiles {
            return Err(CoreError::new(ErrorCode::ManualFilesRequired));
        }
        if !matches!(
            record.state,
            InstallationState::Installed | InstallationState::UpdateAvailable
        ) {
            return Err(invalid());
        }
        let installed = lease.verify_installation_with_cancel(&audit_cancel)?;
        match &record.resolved_source {
            CatalogSource::Vanilla { minecraft_version }
                if installed.plan.loader.is_none()
                    && installed.pack.is_none()
                    && minecraft_version == &installed.plan.version.id => {}
            CatalogSource::Custom { .. } | CatalogSource::Modrinth { .. } => {
                verify_pack(&installed, &record, &audit_cancel)?
            }
            _ => return Err(CoreError::new(ErrorCode::UnsupportedInstance)),
        }
        let global = GlobalSettings::load(&PrivateStore::open(&root)?)?;
        audit_cancel.check()?;
        Ok((lease, installed, global, record.settings))
    });
    let detail = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
        detail = catalog.detail(instance) => detail,
    };
    if let Ok(current) = &detail {
        // Preserve an explicit disabled result even though this attempt fails.
        registry.remember_catalog(std::slice::from_ref(current))?;
    }
    let current_source = detail.as_ref().ok().map(|current| current.source.clone());
    let decision = catalog_decision(&local.last_catalog, detail).and_then(|warning| {
        if let Some(current) = &current_source {
            require_current_pack(&local.resolved_source, current)?;
        }
        Ok(warning)
    });
    let warning_required = match decision {
        Ok(warning) => warning,
        Err(error) => {
            // Stop the audit and wait for it, so its launch reservation is released
            // before the refusal is reported.
            cancel.cancel();
            let _ = audit.await;
            return Err(error);
        }
    };
    // Validate the account now, overlapping the audit, but only for a launch that
    // needs no confirmation: a warned attempt validates when the player confirms.
    let authorization =
        (!warning_required).then(|| AuthorizationTask::start(accounts.clone(), selection.clone()));
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
        authorization,
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
    pub async fn start(
        mut self,
        confirmed_warning: Option<Uuid>,
    ) -> Result<RunningGame, CoreError> {
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
        let prepared = self.authorization.take();
        let authorization = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
            result = async {
                match prepared {
                    // Validated for this very Play request, recently enough to trust.
                    Some(task) if task.fresh() => task.finish().await,
                    _ => self.accounts.authorize_selected_play(&self.selection).await,
                }
            } => result?,
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
