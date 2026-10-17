//! Pure command construction. Building a command does not authorize execution:
//! the launch coordinator must hold the instance lock and pass current preflight.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use zeroize::Zeroize;

use super::install::InstalledLock;
use super::{Platform, arguments, invalid, substitute};
use crate::accounts::{AccountInfo, AccountKind, Secret};
use crate::error::{CoreError, ErrorCode};
use crate::paths::AppPaths;
use crate::settings::{GlobalSettings, InstanceSettings};

/// Backend-only credentials. Microsoft credentials must come from current Play
/// validation; this builder alone cannot establish their validity.
pub struct LaunchIdentity<'a> {
    pub account: &'a AccountInfo,
    pub minecraft_token: Option<&'a Secret>,
    pub xuid: Option<&'a str>,
}

/// Intentionally excludes argument inspection and serialization from the public
/// API. Arguments may contain access tokens and must never reach IPC or logs.
pub struct LaunchCommand {
    pub(crate) executable: PathBuf,
    pub(crate) directory: PathBuf,
    pub(crate) args: Vec<String>,
}

impl std::fmt::Debug for LaunchCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LaunchCommand { <redacted> }")
    }
}

impl Drop for LaunchCommand {
    fn drop(&mut self) {
        self.args.zeroize();
    }
}

impl LaunchCommand {
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn working_directory(&self) -> &Path {
        &self.directory
    }
}

fn path_text(path: &Path) -> Result<String, CoreError> {
    let text = path.to_str().ok_or_else(invalid)?;
    if !path.is_absolute() || text.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(text.to_owned())
}

/// A limited JVM option grammar prevents user settings from replacing the
/// classpath, main class, managed heap/native paths, or injecting argument files.
fn validate_extra(arg: &str) -> Result<(), CoreError> {
    let option = arg.split('=').next().unwrap_or_default();
    if arg.chars().any(char::is_control)
        || !(arg.starts_with("-XX:") || arg.starts_with("-D"))
        || matches!(
            option,
            "-XX:Flags"
                | "-XX:VMOptionsFile"
                | "-XX:MaxHeapSize"
                | "-XX:InitialHeapSize"
                | "-Djava.class.path"
                | "-Djava.library.path"
                | "-Dorg.lwjgl.librarypath"
                | "-Djna.tmpdir"
                | "-Dlog4j.configurationFile"
                | "-Djava.system.class.loader"
                | "-Dorg.lwjgl.system.SharedLibraryExtractPath"
                | "-Dio.netty.native.workdir"
        )
    {
        return Err(invalid());
    }
    Ok(())
}

/// Construct direct process arguments from the installed manifest. Filesystem
/// integrity, runtime overrides, authentication, catalog status and locking are
/// separate preflight responsibilities. No files are changed or processes run.
/// `game_directory` must be resolved by AppPaths::game_directory for this UUID.
pub fn build_command(
    paths: &AppPaths,
    game_directory: &Path,
    installed: &InstalledLock,
    platform: &Platform,
    global: &GlobalSettings,
    instance: &InstanceSettings,
    identity: LaunchIdentity<'_>,
) -> Result<LaunchCommand, CoreError> {
    let plan = &installed.plan;
    plan.validate()?;
    let instance_root = game_directory.parent().ok_or_else(invalid)?;
    super::safe_path(
        instance_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(invalid)?,
    )?;
    if game_directory.file_name().and_then(|name| name.to_str()) != Some("minecraft")
        || instance_root.parent() != Some(paths.root().join("instances").as_path())
    {
        return Err(invalid());
    }
    if installed.schema_version != 1
        || installed.instance_id.is_nil()
        || installed.operation.is_nil()
        || plan.platform != platform.os
        || plan.architecture != platform.arch
    {
        return Err(invalid());
    }
    global.validate()?;
    instance.validate()?;
    let settings = instance.effective(global);
    settings.validate()?;
    // Custom runtimes need independent version/architecture/integrity checks.
    // Do not silently ignore an override or execute it before that exists.
    if settings.java_path_override.is_some() {
        return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
    }
    for arg in &settings.extra_jvm_args {
        validate_extra(arg)?;
    }
    let account = identity.account;
    if account.account_id.is_nil()
        || account.profile_id.is_nil()
        || !(3..=16).contains(&account.username.len())
        || !account
            .username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(invalid());
    }
    let (token, user_type, client_id) = match account.kind {
        AccountKind::Offline if identity.minecraft_token.is_none() && identity.xuid.is_none() => {
            ("0", "legacy", "")
        }
        AccountKind::Microsoft => (
            identity
                .minecraft_token
                .ok_or_else(|| CoreError::new(ErrorCode::AuthenticationRequired))?
                .expose(),
            "msa",
            account.oauth_client_id.as_deref().ok_or_else(invalid)?,
        ),
        _ => return Err(invalid()),
    };
    if account.kind == AccountKind::Microsoft && uuid::Uuid::parse_str(client_id).is_err() {
        return Err(invalid());
    }
    let xuid = identity.xuid.unwrap_or_default();
    if xuid.len() > 32 || !xuid.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let separator = match platform.os.as_str() {
        "windows" => ';',
        "linux" => ':',
        _ => return Err(CoreError::new(ErrorCode::RuntimeUnsupported)),
    };
    let version = &plan.version;
    let mut classpath = version
        .classpath
        .iter()
        .map(|artifact| path_text(&paths.root().join(plan.library_path(artifact)?)))
        .collect::<Result<Vec<_>, CoreError>>()?;
    classpath.push(path_text(&paths.root().join(plan.client_path()?))?);
    if classpath.iter().any(|path| path.contains(separator)) {
        return Err(invalid());
    }
    let native_directory = game_directory
        .parent()
        .ok_or_else(invalid)?
        .join(format!("natives-{}", installed.operation));
    let mut replacements = BTreeMap::from([
        ("auth_player_name".into(), account.username.clone()),
        ("auth_uuid".into(), account.profile_id.simple().to_string()),
        ("auth_access_token".into(), token.to_owned()),
        ("clientid".into(), client_id.to_owned()),
        ("auth_xuid".into(), xuid.to_owned()),
        ("user_type".into(), user_type.into()),
        ("user_properties".into(), "{}".into()),
        ("version_name".into(), version.id.clone()),
        ("version_type".into(), version.kind.clone()),
        ("game_directory".into(), path_text(game_directory)?),
        (
            "assets_root".into(),
            path_text(&paths.root().join("assets"))?,
        ),
        ("assets_index_name".into(), plan.asset_index_name()?),
        ("natives_directory".into(), path_text(&native_directory)?),
        (
            "library_directory".into(),
            path_text(&paths.root().join("libraries"))?,
        ),
        ("classpath".into(), classpath.join(&separator.to_string())),
        ("classpath_separator".into(), separator.to_string()),
        ("launcher_name".into(), "PanitaClient".into()),
        ("launcher_version".into(), env!("CARGO_PKG_VERSION").into()),
        (
            "resolution_width".into(),
            settings.window_size.width.to_string(),
        ),
        (
            "resolution_height".into(),
            settings.window_size.height.to_string(),
        ),
    ]);
    let mut launch_platform = platform.clone();
    launch_platform
        .features
        .insert("has_custom_resolution".into(), true);
    let result = (|| {
        let metadata = version
            .arguments
            .as_ref()
            .ok_or_else(|| CoreError::new(ErrorCode::UnsupportedInstance))?;
        if version.kind.is_empty()
            || version.main_class.is_empty()
            || !version
                .main_class
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._$".contains(&b))
        {
            return Err(invalid());
        }
        let mut command = LaunchCommand {
            executable: paths.root().join(&plan.java_executable),
            directory: game_directory.to_owned(),
            args: arguments(&metadata.jvm, &launch_platform, &replacements)?,
        };
        if let Some(logging) = &version.logging {
            let path = path_text(&paths.root().join(format!(
                "assets/log_configs/{}-{}",
                logging.file.artifact.sha1, logging.file.id
            )))?;
            command.args.push(substitute(
                &logging.argument,
                &BTreeMap::from([("path".into(), path)]),
            )?);
        }
        command.args.extend(settings.extra_jvm_args.clone());
        command.args.push(format!("-Xmx{}M", settings.ram_mib));
        command.args.push(version.main_class.clone());
        command
            .args
            .extend(arguments(&metadata.game, &launch_platform, &replacements)?);
        if command.args.len() > 4096
            || command
                .args
                .iter()
                .any(|arg| arg.len() > 1024 * 1024 || arg.contains('\0'))
        {
            return Err(invalid());
        }
        Ok(command)
    })();
    for value in replacements.values_mut() {
        value.zeroize();
    }
    result
}

#[cfg(test)]
mod tests;
