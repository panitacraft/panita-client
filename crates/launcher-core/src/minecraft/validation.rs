//! Development-only isolated loader qualification. These APIs deliberately create
//! a disabled, unsupported synthetic source; they never enable a real pack or
//! replace production catalog/account authorization. No account store is opened.
use super::{
    Platform,
    download::{Cancellation, io},
    install::{self, InstalledLock, Progress},
    invalid,
    plan::InstallPlan,
    process::{LaunchLease, RunningGame},
};
use crate::{
    accounts::{AccountInfo, AccountKind, offline_profile_id},
    catalog::{CatalogInstance, CatalogSource},
    error::{CoreError, ErrorCode},
    operation::{OperationId, ProcessSessionId},
    paths::AppPaths,
    registry::{InstallationState, InstanceRegistry},
    settings::{GlobalSettings, InstanceSettings},
    storage::{PrivateStore, map_private_error},
};
use serde::{Deserialize, Serialize};
use std::io::Read;
use uuid::Uuid;

const TARGET: &str = "loader-validation.json";
const PREPARED: &str = "prepared-plan.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationTarget {
    schema: u32,
    #[serde(rename = "type")]
    kind: String,
    id: Uuid,
    minecraft: String,
    neoforge: String,
}

impl ValidationTarget {
    pub fn id(&self) -> Uuid {
        self.id
    }
    fn validate(&self) -> Result<(), CoreError> {
        if self.schema != 1 || self.kind != "loader_validation" || self.id.is_nil() {
            return Err(invalid());
        }
        super::safe_path(&self.minecraft)?;
        super::safe_path(&self.neoforge)?;
        if self.minecraft.contains('/') || self.neoforge.contains('/') {
            return Err(invalid());
        }
        Ok(())
    }
    fn check(&self, plan: &InstallPlan) -> Result<(), CoreError> {
        self.validate()?;
        plan.validate()?;
        let loader = plan.loader.as_ref().ok_or_else(invalid)?;
        if plan.version.id != self.minecraft || loader.version() != self.neoforge {
            return Err(invalid());
        }
        Ok(())
    }
}

fn guard(paths: &AppPaths) -> Result<(), CoreError> {
    // Protect against accidentally pointing this tool at the user's real profile.
    if !paths
        .root()
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("phase-9-"))
    {
        return Err(invalid());
    }
    for name in ["accounts.json", "account-removals.json"] {
        match std::fs::symlink_metadata(paths.root().join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Err(invalid()),
            Err(error) => return Err(io(error)),
        }
    }
    private_fs::PrivateDir::open(paths.root()).map_err(map_private_error)?;
    Ok(())
}

fn target(paths: &AppPaths) -> Result<ValidationTarget, CoreError> {
    guard(paths)?;
    let bytes = PrivateStore::open(paths)?
        .read(TARGET)?
        .ok_or_else(invalid)?;
    let target: ValidationTarget = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    target.validate()?;
    Ok(target)
}

fn prepared(paths: &AppPaths) -> Result<(InstallPlan, Vec<u8>), CoreError> {
    let directory = private_fs::PrivateDir::open(paths.root()).map_err(map_private_error)?;
    let mut file = directory.open_file(PREPARED).map_err(map_private_error)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(invalid());
    }
    let plan: InstallPlan = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    plan.validate()?;
    if plan.loader.is_none() {
        return Err(invalid());
    }
    Ok((plan, bytes))
}

fn ensure_target(paths: &AppPaths, plan: &InstallPlan) -> Result<ValidationTarget, CoreError> {
    let loader = plan.loader.as_ref().ok_or_else(invalid)?;
    PrivateStore::open(paths)?.transaction(TARGET, |bytes| {
        let target = match bytes {
            Some(bytes) => {
                serde_json::from_slice::<ValidationTarget>(&bytes).map_err(|_| invalid())?
            }
            None => ValidationTarget {
                schema: 1,
                kind: "loader_validation".into(),
                id: Uuid::new_v4(),
                minecraft: plan.version.id.clone(),
                neoforge: loader.version().into(),
            },
        };
        target.check(plan)?;
        let bytes = crate::storage_json::pretty(&target).map_err(|_| invalid())?;
        Ok((bytes, target))
    })
}

fn catalog(target: &ValidationTarget) -> CatalogInstance {
    CatalogInstance {
        id: target.id,
        slug: "loader-validation".into(),
        name: "Validación de NeoForge".into(),
        description: "Instancia aislada de validación; no es un modpack instalado.".into(),
        minecraft_version: Some(target.minecraft.clone()),
        enabled: false,
        restricted: false,
        min_launcher_version: None,
        icon_url: None,
        banner_url: None,
        sort_order: 0,
        updated_at: "1970-01-01T00:00:00Z".into(),
        source: CatalogSource::Unsupported {
            source_type: "loader_validation".into(),
        },
    }
}

/// Install the recorded loader-only candidate through production preparation,
/// verification, journal, UUID registry and instance lease code. No catalog entry
/// is fetched or represented as enabled; this is never a pack acceptance result.
pub async fn install(
    paths: &AppPaths,
    cancel: &Cancellation,
    progress: Progress,
) -> Result<ValidationTarget, CoreError> {
    guard(paths)?;
    cancel.check()?;
    // A completed validation installation is independently reusable on restart.
    if PrivateStore::open(paths)?.read(TARGET)?.is_some() {
        let existing = target(paths)?;
        let registry = InstanceRegistry::new(paths.clone());
        if let Some(record) = registry.get(existing.id)? {
            let root = registry.prepare_instance(existing.id)?;
            let lease = LaunchLease::acquire(paths, existing.id)?;
            lease.recover_installation()?;
            if install::read_json::<InstalledLock>(&root, "installed.json")?.is_some() {
                let installed = install::verify_under_lock(paths, existing.id, &root, cancel)?;
                existing.check(&installed.plan)?;
                if record.resolved_source != catalog(&existing).source {
                    return Err(invalid());
                }
                return Ok(existing);
            }
        }
    }
    let (plan, staged_bytes) = prepared(paths)?;
    let target = ensure_target(paths, &plan)?;
    let platform = Platform::x64(std::env::consts::OS, "")?;
    if platform.os != plan.platform || platform.arch != plan.architecture {
        return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
    }
    let registry = InstanceRegistry::new(paths.clone());
    let root = registry.ensure_catalog_instance(&catalog(&target))?;
    let lease = LaunchLease::acquire(paths, target.id)?;
    lease.recover_installation()?;
    private_fs::PrivateDir::open(&root)
        .map_err(map_private_error)?
        .ensure_dir("minecraft")
        .map_err(map_private_error)?;
    let mut record = registry.get(target.id)?.ok_or_else(invalid)?;
    record.state = InstallationState::Installing;
    registry.save_installation(&record)?;
    let operation = Uuid::new_v4();
    let journal =
        |active| serde_json::json!({"schema_version":1,"operation":operation,"active":active});
    install::write_json(&root, "transaction.json", &journal(true))?;
    let installed = install::prepare_recorded(
        paths, target.id, operation, &platform, plan, cancel, progress,
    )
    .await?;
    cancel.commit(|| install::write_json(&root, "installed.json", &installed))?;
    record.state = InstallationState::Installed;
    registry.save_installation(&record)?;
    install::write_json(&root, "transaction.json", &journal(false))?;
    super::transaction::remove(&root)?;
    // Retire the sole staging plan only after the durable installed pointer.
    PrivateStore::open(paths)?.with_lock(|transaction| {
        let (_, current) = prepared(paths)?;
        if current != staged_bytes {
            return Err(invalid());
        }
        transaction.remove_verified_file(PREPARED)
    })?;
    install::verify_under_lock(paths, target.id, &root, cancel)?;
    drop(lease);
    Ok(target)
}

pub fn verify(paths: &AppPaths, cancel: &Cancellation) -> Result<InstalledLock, CoreError> {
    let target = target(paths)?;
    let lease = LaunchLease::acquire(paths, target.id)?;
    lease.recover_installation()?;
    let root = lease.game_directory().parent().ok_or_else(invalid)?;
    let installed = install::verify_under_lock(paths, target.id, root, cancel)?;
    target.check(&installed.plan)?;
    Ok(installed)
}

/// Ephemeral explicit offline identity for the isolated disabled validation
/// source only. Never reads saved accounts or bypasses Microsoft validation for
/// a real account. This API is absent from normal builds and all desktop IPC.
pub fn launch(paths: &AppPaths, cancel: &Cancellation) -> Result<RunningGame, CoreError> {
    let target = target(paths)?;
    let registry = InstanceRegistry::new(paths.clone());
    let record = registry.get(target.id)?.ok_or_else(invalid)?;
    if record.resolved_source != catalog(&target).source {
        return Err(invalid());
    }
    let lease = LaunchLease::acquire(paths, target.id)?;
    lease.recover_installation()?;
    let installed = lease.verify_installation_with_cancel(cancel)?;
    target.check(&installed.plan)?;
    let platform = Platform::x64(std::env::consts::OS, "")?;
    let account = AccountInfo {
        account_id: Uuid::new_v4(),
        kind: AccountKind::Offline,
        username: "Loader_Check".into(),
        profile_id: offline_profile_id("Loader_Check"),
        oauth_client_id: None,
    };
    let command = super::launch::build_command(
        paths,
        lease.game_directory(),
        &installed,
        &platform,
        &GlobalSettings::default(),
        &InstanceSettings::default(),
        super::launch::LaunchIdentity {
            account: &account,
            minecraft_token: None,
            xuid: None,
        },
    )?;
    cancel.commit(|| lease.spawn(command, ProcessSessionId::new(), OperationId::new()))
}

#[cfg(test)]
mod tests;
