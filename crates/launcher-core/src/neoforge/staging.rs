//! Fresh private processor workspaces. No instance state or installed pointer is
//! changed here. Working-directory/path confinement is not an OS process sandbox.
use super::{
    plan::{InputSource, ProcessorArgument, ProcessorPlan},
    transport::{Transport, fingerprint},
    *,
};
use crate::{
    minecraft::{
        Artifact, Platform,
        download::{Cancellation, Downloader, ensure_parent, io, native_file, verified_file},
        plan::InstallPlan,
    },
    paths::AppPaths,
    storage::map_private_error,
};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use private_fs::PrivateDir;
use serde::Serialize;
use sha2::Sha256;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use uuid::Uuid;

const MAX_OUTPUT: u64 = 512 * 1024 * 1024;
const MAX_TOTAL_OUTPUT: u64 = 2 * 1024 * 1024 * 1024;
const MAX_MANIFEST: u64 = 64 * 1024;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Serialize)]
pub struct DerivedArtifact {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// A completed staging result, not an installed/reusable loader generation.
pub struct StagedLoader {
    pub(super) root: PathBuf,
    pub(super) operation: Uuid,
    pub(super) files: Vec<DerivedArtifact>,
}
impl StagedLoader {
    pub fn directory(&self) -> &Path {
        &self.root
    }
    pub fn operation(&self) -> Uuid {
        self.operation
    }
    pub fn derived_files(&self) -> &[DerivedArtifact] {
        &self.files
    }
}

#[derive(Clone, Copy, Debug)]
pub enum StagingProgress {
    Runtime { completed: usize, total: usize },
    Input { completed: usize, total: usize },
    Processor { completed: usize, total: usize },
}

/// Download/verify a reserved Mojang runtime and loader inputs, then execute the
/// supported local client processors. Caller must use an independently resolved
/// base plan; publishing to an instance still requires its idle operation lease.
/// Partial operations are preserved without a completion marker and never reused.
pub async fn prepare(
    paths: &AppPaths,
    plan: &ProcessorPlan,
    installer: &[u8],
    base: &InstallPlan,
    cancel: &Cancellation,
    progress: &(dyn Fn(StagingProgress) + Sync),
) -> Result<StagedLoader, CoreError> {
    cancel.check()?;
    InstallerMetadata::decode(installer, &plan.identity)?;
    base.validate()?;
    if base.version.id != plan.identity.minecraft
        || base.version.java.major_version != plan.java.major_version
        || base.version.java.component != plan.java.component
    {
        return Err(invalid());
    }
    let platform = Platform {
        os: base.platform.clone(),
        arch: base.architecture.clone(),
        os_version: String::new(),
        features: BTreeMap::new(),
    };
    let host = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return Err(invalid());
    };
    if platform.os != host {
        return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
    }
    let mut base = base.clone();
    base.allocate_runtime(paths)?;
    let downloader = Downloader::new()?;
    let runtime_prefix = format!("{}/", base.runtime_root);
    let runtime_files: Vec<_> = base
        .artifacts
        .iter()
        .filter(|file| file.destination.starts_with(&runtime_prefix))
        .collect();
    for (index, file) in runtime_files.iter().enumerate() {
        cancel.check()?;
        downloader
            .cache_planned(paths, file, cancel, &|_| {})
            .await?;
        progress(StagingProgress::Runtime {
            completed: index + 1,
            total: runtime_files.len(),
        });
    }
    // Check runtime completeness against its authenticated manifest without
    // requiring unrelated vanilla assets to have been installed.
    verify_runtime(paths, &base, cancel)?;
    let stage = Workspace::create(paths)?;
    let java = paths.root().join(&base.java_executable);
    let probe = run_child(
        java_command(
            &java,
            &stage.root,
            ["-XshowSettings:properties", "-version"],
        ),
        cancel,
        Duration::from_secs(20),
        true,
    )
    .await?;
    if !probe.success {
        return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
    }
    crate::minecraft::runtime::verify_properties(
        std::str::from_utf8(&probe.stderr).map_err(|_| invalid())?,
        &plan.java,
        &platform,
    )?;
    let transport = Transport::new()?;
    let mut archive = zip::ZipArchive::new(Cursor::new(installer)).map_err(|_| invalid())?;
    for (index, input) in plan.inputs.iter().enumerate() {
        cancel.check()?;
        match &input.source {
            InputSource::Library(source) => {
                let cached = transport.library(paths, source, cancel).await?;
                stage.copy(&cached, &input.destination, &fingerprint(source), cancel)?;
            }
            InputSource::Mojang(source) => {
                let cached = transport.mojang(paths, source, cancel).await?;
                stage.copy(&cached, &input.destination, source, cancel)?;
            }
            InputSource::Embedded { entry, sha1, size } => {
                let bytes = super::read_entry(&mut archive, entry)?;
                if bytes.len() as u64 != *size || format!("{:x}", Sha1::digest(&bytes)) != *sha1 {
                    return Err(CoreError::new(ErrorCode::ArtifactMismatch));
                }
                stage.write(&input.destination, &bytes)?;
            }
        }
        progress(StagingProgress::Input {
            completed: index + 1,
            total: plan.inputs.len(),
        });
    }
    let mut files = Vec::new();
    for (index, processor) in plan.processors.iter().enumerate() {
        cancel.check()?;
        stage.verify_inputs(plan, &files, cancel)?;
        verify_runtime(paths, &base, cancel)?;
        for output in processor.outputs.keys() {
            // Precreate protected files/parents before Java can write them.
            stage.write(output, &[])?;
        }
        let main = stage.main_class(&processor.jar)?;
        let separator = if cfg!(windows) { ';' } else { ':' };
        let classpath = processor
            .classpath
            .iter()
            .map(|path| stage.path_text(path, separator))
            .collect::<Result<Vec<_>, _>>()?
            .join(&separator.to_string());
        let mut args = vec!["-Xmx1024m".into(), "-cp".into(), classpath, main];
        for arg in &processor.args {
            args.push(match arg {
                ProcessorArgument::Literal(value) => value.clone(),
                ProcessorArgument::Path(path) => stage.path_text(path, separator)?,
            });
        }
        let result = run_child(
            java_command(&java, &stage.root, args),
            cancel,
            PROCESS_TIMEOUT,
            false,
        )
        .await?;
        if !result.success {
            return Err(CoreError::new(ErrorCode::LoaderProcessorFailed));
        }
        for (path, expected_sha1) in &processor.outputs {
            let file = stage.generated(path, expected_sha1.as_deref(), cancel)?;
            files.push(file);
        }
        if files.iter().map(|file| file.size).sum::<u64>() > MAX_TOTAL_OUTPUT {
            return Err(invalid());
        }
        progress(StagingProgress::Processor {
            completed: index + 1,
            total: plan.processors.len(),
        });
    }
    stage.verify_inputs(plan, &files, cancel)?;
    if files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>()
        != plan.derived.iter().map(String::as_str).collect()
    {
        return Err(invalid());
    }
    cancel.check()?;
    Ok(StagedLoader {
        root: stage.root,
        operation: stage.operation,
        files,
    })
}

fn verify_runtime(
    paths: &AppPaths,
    base: &InstallPlan,
    cancel: &Cancellation,
) -> Result<(), CoreError> {
    let prefix = format!("{}/", base.runtime_root);
    let manifest = base
        .artifacts
        .iter()
        .find(|file| file.destination == format!("{prefix}.runtime-manifest.json"))
        .ok_or_else(invalid)?;
    if !base
        .documents
        .iter()
        .any(|document| document.source == manifest.artifact)
    {
        return Err(invalid());
    }
    let directory =
        PrivateDir::open_data(paths.root().join(&base.runtime_root)).map_err(map_private_error)?;
    let mut file = directory
        .open_file(".runtime-manifest.json")
        .map_err(map_private_error)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_METADATA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    manifest.artifact.verify(&bytes)?;
    let runtime: crate::minecraft::runtime::RuntimeManifest =
        serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    for required in runtime.plan()? {
        if !base.artifacts.iter().any(|file| {
            file.destination == format!("{prefix}{}", required.path)
                && file.artifact == required.artifact
                && file.executable == required.executable
        }) {
            return Err(invalid());
        }
    }
    for planned in base
        .artifacts
        .iter()
        .filter(|file| file.destination.starts_with(&prefix))
    {
        cancel.check()?;
        let (parent, name) = ensure_parent(paths.root(), &planned.destination)?;
        if !crate::minecraft::download::verified_planned_file(
            &parent,
            &name,
            &planned.artifact,
            planned.executable,
        )? {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
    }
    Ok(())
}

struct Workspace {
    root: PathBuf,
    directory: PrivateDir,
    operation: Uuid,
}
impl Workspace {
    fn create(paths: &AppPaths) -> Result<Self, CoreError> {
        let root = PrivateDir::ensure_data(paths.root()).map_err(map_private_error)?;
        let libraries = root.ensure_dir("libraries").map_err(map_private_error)?;
        let container = libraries
            .ensure_dir("neoforge")
            .map_err(map_private_error)?;
        let operation = Uuid::new_v4();
        let directory = container
            .create_dir(operation.to_string())
            .map_err(map_private_error)?;
        for name in ["tmp", "home"] {
            directory.create_dir(name).map_err(map_private_error)?;
        }
        Ok(Self {
            root: paths
                .root()
                .join("libraries/neoforge")
                .join(operation.to_string()),
            directory,
            operation,
        })
    }
    fn write(&self, relative: &str, bytes: &[u8]) -> Result<(), CoreError> {
        let (parent, name) = ensure_parent(&self.root, relative)?;
        let mut file = parent.create(name).map_err(map_private_error)?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(map_private_error)
    }
    fn copy(
        &self,
        source: &Path,
        relative: &str,
        expected: &Artifact,
        cancel: &Cancellation,
    ) -> Result<(), CoreError> {
        let source_dir = PrivateDir::open_data(source.parent().ok_or_else(invalid)?)
            .map_err(map_private_error)?;
        let source_name = source.file_name().ok_or_else(invalid)?;
        if !verified_file(
            &source_dir,
            source_name.to_str().ok_or_else(invalid)?,
            expected,
        )? {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        let mut input = source_dir
            .open_file(source_name)
            .map_err(map_private_error)?;
        let (parent, name) = ensure_parent(&self.root, relative)?;
        let mut output = parent.create(&name).map_err(map_private_error)?;
        let mut buffer = [0; 64 * 1024];
        let mut count = 0u64;
        loop {
            cancel.check()?;
            let size = input.read(&mut buffer).map_err(io)?;
            if size == 0 {
                break;
            }
            count += size as u64;
            if count > expected.size {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
            output.write_all(&buffer[..size]).map_err(io)?;
        }
        output.sync_all().map_err(map_private_error)?;
        if !verified_file(&parent, &name, expected)? {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        Ok(())
    }
    fn path_text(&self, relative: &str, separator: char) -> Result<String, CoreError> {
        safe_path(relative)?;
        let path = self
            .root
            .join(relative)
            .to_str()
            .ok_or_else(invalid)?
            .to_owned();
        if path.contains(separator) || path.chars().any(char::is_control) {
            return Err(invalid());
        }
        Ok(path)
    }
    fn main_class(&self, relative: &str) -> Result<String, CoreError> {
        let file = self
            .directory
            .open_file(relative)
            .map_err(map_private_error)?;
        let mut zip = zip::ZipArchive::new(native_file(&file)?).map_err(|_| invalid())?;
        let manifest = zip.by_name("META-INF/MANIFEST.MF").map_err(|_| invalid())?;
        if manifest.size() > MAX_MANIFEST {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        manifest
            .take(MAX_MANIFEST + 1)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes.len() > MAX_MANIFEST as usize {
            return Err(invalid());
        }
        main_class(&bytes)
    }
    fn generated(
        &self,
        relative: &str,
        sha1: Option<&str>,
        cancel: &Cancellation,
    ) -> Result<DerivedArtifact, CoreError> {
        let file = self.open_generated(relative)?;
        let size = file.metadata().map_err(io)?.len();
        if size == 0 || size > MAX_OUTPUT {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        let mut reader = file;
        let mut hash = Sha256::new();
        let mut legacy = Sha1::new();
        let mut buffer = [0; 64 * 1024];
        let mut count = 0u64;
        loop {
            cancel.check()?;
            let size = reader.read(&mut buffer).map_err(io)?;
            if size == 0 {
                break;
            }
            count += size as u64;
            if count > MAX_OUTPUT {
                return Err(invalid());
            }
            hash.update(&buffer[..size]);
            legacy.update(&buffer[..size]);
        }
        private_fs::verify_data(&reader).map_err(map_private_error)?;
        if count != size
            || sha1.is_some_and(|expected| expected != format!("{:x}", legacy.finalize()))
        {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        Ok(DerivedArtifact {
            path: relative.into(),
            sha256: format!("{:x}", hash.finalize()),
            size,
        })
    }
    fn open_generated(&self, relative: &str) -> Result<std::fs::File, CoreError> {
        safe_path(relative)?;
        let mut directory = capability(&self.directory)?;
        let mut parts = relative.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_some() {
                directory = directory.open_dir_nofollow(part).map_err(io)?;
            } else {
                let metadata = directory.symlink_metadata(part).map_err(io)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() != 1
                {
                    return Err(invalid());
                }
                let mut options = cap_std::fs::OpenOptions::new();
                options.read(true).write(true).follow(FollowSymlinks::No);
                let file = directory.open_with(part, &options).map_err(io)?.into_std();
                // Windows outputs use ordinary access rights. Unix retains its
                // owner-only executable/data policy inside this fresh workspace.
                #[cfg(not(windows))]
                private_fs::harden(&file).map_err(map_private_error)?;
                #[cfg(windows)]
                private_fs::verify_data(&file).map_err(map_private_error)?;
                return Ok(file);
            }
        }
        Err(invalid())
    }
    fn verify_inputs(
        &self,
        plan: &ProcessorPlan,
        derived: &[DerivedArtifact],
        cancel: &Cancellation,
    ) -> Result<(), CoreError> {
        for input in &plan.inputs {
            cancel.check()?;
            let expected = match &input.source {
                InputSource::Library(source) => fingerprint(source),
                InputSource::Mojang(source) => source.clone(),
                InputSource::Embedded { sha1, size, .. } => Artifact {
                    url: String::new(),
                    sha1: sha1.clone(),
                    size: *size,
                    path: None,
                },
            };
            if !verified_file(&self.directory, &input.destination, &expected)? {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
        }
        for expected in derived {
            let actual = self.generated(&expected.path, None, cancel)?;
            if actual.size != expected.size || actual.sha256 != expected.sha256 {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
        }
        Ok(())
    }
}

fn capability(directory: &PrivateDir) -> Result<cap_std::fs::Dir, CoreError> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        Ok(cap_std::fs::Dir::from_std_file(std::fs::File::from(
            directory.as_handle().try_clone_to_owned().map_err(io)?,
        )))
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        Ok(cap_std::fs::Dir::from_std_file(std::fs::File::from(
            directory.as_fd().try_clone_to_owned().map_err(io)?,
        )))
    }
}

fn main_class(bytes: &[u8]) -> Result<String, CoreError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| invalid())?
        .replace("\r\n", "\n");
    let mut attributes: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            break;
        } // Main attributes only, not named sections.
        if let Some(continuation) = line.strip_prefix(' ') {
            attributes
                .last_mut()
                .ok_or_else(invalid)?
                .push_str(continuation);
        } else {
            attributes.push(line.into());
        }
    }
    let entries: Vec<_> = attributes
        .iter()
        .filter_map(|line| line.split_once(": "))
        .filter(|(key, _)| key.eq_ignore_ascii_case("Main-Class"))
        .collect();
    if entries.len() != 1 {
        return Err(invalid());
    }
    let main = entries[0].1;
    if main.len() > 512
        || !main.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_$".contains(&c))
        })
    {
        return Err(invalid());
    }
    Ok(main.into())
}

fn java_command(
    java: &Path,
    root: &Path,
    args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(java);
    command.env_clear();
    #[cfg(windows)]
    {
        if let Some(value) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", value);
        }
        command.creation_flags(0x08000000);
    }
    command
        .current_dir(root)
        .env("TEMP", root.join("tmp"))
        .env("TMP", root.join("tmp"))
        .arg(format!("-Djava.io.tmpdir={}", root.join("tmp").display()))
        .arg(format!("-Duser.home={}", root.join("home").display()))
        .args(args);
    command
}

struct ChildResult {
    success: bool,
    stderr: Vec<u8>,
}

async fn drain(mut input: impl AsyncRead + Unpin, capture: bool) -> Result<Vec<u8>, CoreError> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = input.read(&mut buffer).await.map_err(io)?;
        if count == 0 {
            break;
        }
        if capture {
            if bytes.len() + count > 16 * 1024 {
                return Err(invalid());
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        // Processor output is drained and discarded, never retained, logged or
        // returned. Only the bounded private Java property probe is captured.
    }
    Ok(bytes)
}

async fn run_child(
    mut command: tokio::process::Command,
    cancel: &Cancellation,
    timeout: Duration,
    probe: bool,
) -> Result<ChildResult, CoreError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cancel.commit(|| {
        command
            .spawn()
            .map_err(|_| CoreError::new(ErrorCode::ProcessStartFailed))
    })?;
    let stdout = child.stdout.take().ok_or_else(invalid)?;
    let stderr = child.stderr.take().ok_or_else(invalid)?;
    let work = async {
        let (stdout, stderr, status) =
            tokio::try_join!(drain(stdout, false), drain(stderr, probe), async {
                child.wait().await.map_err(io)
            })?;
        drop(stdout);
        Ok(ChildResult {
            success: status.success(),
            stderr,
        })
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = tokio::time::timeout(timeout, work) => result.unwrap_or_else(|_| Err(CoreError::new(ErrorCode::LoaderProcessorTimeout))),
    };
    if result.is_err() {
        // Reap the direct processor before releasing the workspace to its caller.
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result
}

#[cfg(test)]
mod tests;
