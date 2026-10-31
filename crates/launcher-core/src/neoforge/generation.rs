//! Exact loader state embedded in an installed plan. A completion marker binds
//! its digest without persisting a duplicate staged plan.
use super::{
    plan::{InputSource, ProcessorPlan},
    staging::StagedLoader,
    *,
};
use crate::{
    minecraft::{
        Artifact,
        download::{Cancellation, io, native_file},
    },
    paths::AppPaths,
    storage::map_private_error,
};
use private_fs::PrivateDir;
use sha2::Sha256;
use std::io::Read;
use uuid::Uuid;

pub(crate) const MARKER: &str = "complete.json";
const MAX_FILES: usize = 8192;
const MAX_TOTAL: u64 = 16 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationRef {
    schema: u32,
    #[serde(rename = "type")]
    loader: String,
    id: Uuid,
    pub(crate) installer: InstallerIdentity,
    pub(crate) java: crate::minecraft::JavaRequirement,
    pub(crate) profile: LaunchProfile,
    pub(crate) files: Vec<GenerationFile>,
    derived: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationFile {
    pub path: String,
    pub sha1: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Completion {
    pub(crate) schema: u32,
    pub(crate) id: Uuid,
    pub(crate) plan_sha256: String,
}

impl GenerationRef {
    /// Explicit repair only: reproduce the exact accepted bytes in a fresh generation.
    pub async fn repair(
        &self,
        paths: &AppPaths,
        base: &crate::minecraft::plan::InstallPlan,
        cancel: &Cancellation,
    ) -> Result<Self, CoreError> {
        self.validate(&base.version.id)?;
        let transport = super::transport::Transport::new()?;
        let installer = transport.installer(paths, &self.installer, cancel).await?;
        let source = base
            .documents
            .iter()
            .find(|document| {
                document
                    .source
                    .url
                    .ends_with(&format!("/{}.json", self.minecraft()))
            })
            .ok_or_else(invalid)?;
        let downloader = crate::minecraft::download::Downloader::new()?;
        let game = downloader.document(&source.source.url, cancel).await?;
        let plan = ProcessorPlan::resolve(&installer, &self.installer, &game, &source.source)?;
        if serde_json::to_vec(&plan.launch).map_err(|_| invalid())?
            != serde_json::to_vec(&self.profile).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        let staged =
            super::staging::prepare(paths, &plan, &installer, base, cancel, &|_| {}).await?;
        let replacement = publish(paths, &plan, staged, cancel)?;
        self.verify_replacement(&replacement)?;
        Ok(replacement)
    }

    fn verify_replacement(&self, replacement: &Self) -> Result<(), CoreError> {
        let expected: BTreeMap<_, _> = self
            .files
            .iter()
            .map(|file| (&file.path, (&file.sha1, &file.sha256, file.size)))
            .collect();
        let actual: BTreeMap<_, _> = replacement
            .files
            .iter()
            .map(|file| (&file.path, (&file.sha1, &file.sha256, file.size)))
            .collect();
        if self.minecraft() != replacement.minecraft()
            || self.version() != replacement.version()
            || expected != actual
        {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        Ok(())
    }
    /// Resolve child library precedence while retaining parent classpath order.
    /// Minecraft itself is loaded from the verified renamed/extra/patch libraries.
    pub(crate) fn classpath(
        &self,
        paths: &AppPaths,
        base: &crate::minecraft::plan::InstallPlan,
    ) -> Result<Vec<std::path::PathBuf>, CoreError> {
        self.validate(&base.version.id)?;
        let root = paths.root().join(self.directory()).join("libraries");
        let mut children = BTreeMap::new();
        for library in &self.profile.libraries {
            let artifact = &library.downloads.artifact;
            if children
                .insert(maven_key(&artifact.path)?, artifact.path.as_str())
                .is_some()
            {
                return Err(invalid());
            }
        }
        let mut used = BTreeSet::new();
        let mut classpath = Vec::new();
        for artifact in &base.version.classpath {
            let key = maven_key(artifact.path.as_deref().ok_or_else(invalid)?)?;
            if let Some(child) = children.get(&key) {
                if used.insert(key) {
                    classpath.push(root.join(child));
                }
            } else {
                classpath.push(paths.root().join(base.library_path(artifact)?));
            }
        }
        for library in &self.profile.libraries {
            let key = maven_key(&library.downloads.artifact.path)?;
            if used.insert(key) {
                classpath.push(root.join(&library.downloads.artifact.path));
            }
        }
        Ok(classpath)
    }
    pub fn directory(&self) -> String {
        format!("libraries/neoforge/{}", self.id)
    }
    pub fn kind(&self) -> &str {
        &self.loader
    }
    pub fn java(&self) -> &crate::minecraft::JavaRequirement {
        &self.java
    }
    pub fn profile(&self) -> &LaunchProfile {
        &self.profile
    }
    pub fn files(&self) -> &[GenerationFile] {
        &self.files
    }
    pub fn minecraft(&self) -> &str {
        &self.installer.minecraft
    }
    pub fn version(&self) -> &str {
        &self.installer.version
    }

    pub fn validate(&self, minecraft: &str) -> Result<(), CoreError> {
        self.installer.url()?;
        if self.schema != 1
            || self.loader != "neoforge"
            || self.id.is_nil()
            || self.installer.minecraft != minecraft
            || self.profile.id != format!("neoforge-{}", self.installer.version)
            || self.profile.inherits_from != minecraft
            || self.java.major_version == 0
            || !component(&self.java.component)
            || self.files.is_empty()
            || self.files.len() > MAX_FILES
            || self.derived.is_empty()
            || self.derived.len() > MAX_PROCESSORS * 2
        {
            return Err(invalid());
        }
        let mut paths = BTreeSet::new();
        let mut total = 0u64;
        for file in &self.files {
            safe_path(&file.path)?;
            if !valid_hash(&file.sha1)
                || !hash256(&file.sha256)
                || file.size == 0
                || file.size > 2 * 1024 * 1024 * 1024
                || !paths.insert(file.path.to_ascii_lowercase())
            {
                return Err(invalid());
            }
            total = total.checked_add(file.size).ok_or_else(invalid)?;
        }
        if total > MAX_TOTAL {
            return Err(invalid());
        }
        for path in &paths {
            for (end, _) in path.match_indices('/') {
                if paths.contains(&path[..end]) {
                    return Err(invalid());
                }
            }
        }
        for path in &self.derived {
            safe_path(path)?;
            if !self.files.iter().any(|file| &file.path == path) {
                return Err(invalid());
            }
        }
        let patched = format!(
            "libraries/{}",
            Coordinate::parse(&format!(
                "net.neoforged:neoforge:{}:client",
                self.installer.version
            ))?
            .path()
        );
        if !self.derived.contains(&patched) {
            return Err(invalid());
        }
        let neoform = flag(&self.profile.arguments.game, "--fml.neoFormVersion")?;
        if !component(neoform) {
            return Err(invalid());
        }
        for classifier in ["srg", "extra"] {
            let path = format!(
                "libraries/{}",
                Coordinate::parse(&format!(
                    "net.minecraft:client:{minecraft}-{neoform}:{classifier}"
                ))?
                .path()
            );
            if !self.derived.contains(&path) {
                return Err(invalid());
            }
        }
        let universal = format!(
            "libraries/{}",
            Coordinate::parse(&format!(
                "net.neoforged:neoforge:{}:universal",
                self.version()
            ))?
            .path()
        );
        if !self.files.iter().any(|file| file.path == universal) {
            return Err(invalid());
        }
        for library in &self.profile.libraries {
            library.validate()?;
            let expected = &library.downloads.artifact;
            if !self.files.iter().any(|file| {
                file.path == format!("libraries/{}", expected.path)
                    && file.sha1 == expected.sha1
                    && file.size == expected.size
            }) {
                return Err(invalid());
            }
        }
        if self.profile.libraries.is_empty()
            || self.profile.libraries.len() > MAX_LIBRARIES
            || self.profile.main_class.is_empty()
            || self.profile.main_class.len() > 512
            || !self
                .profile
                .main_class
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._$".contains(&c))
            || self.profile.arguments.jvm.len() > MAX_ARGUMENTS
            || self.profile.arguments.game.len() > MAX_ARGUMENTS
            || self
                .profile
                .arguments
                .jvm
                .iter()
                .chain(&self.profile.arguments.game)
                .any(|arg| !bounded_text(arg))
        {
            return Err(invalid());
        }
        for (flag, value) in [
            ("--fml.neoForgeVersion", self.version()),
            ("--fml.mcVersion", self.minecraft()),
        ] {
            let matches: Vec<_> = self
                .profile
                .arguments
                .game
                .iter()
                .enumerate()
                .filter(|(_, arg)| arg.as_str() == flag)
                .collect();
            if matches.len() != 1
                || self
                    .profile
                    .arguments
                    .game
                    .get(matches[0].0 + 1)
                    .map(String::as_str)
                    != Some(value)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    pub(crate) fn verify_marker(&self, paths: &AppPaths) -> Result<(), CoreError> {
        self.validate(self.minecraft())?;
        let directory =
            PrivateDir::open_data(paths.root().join(self.directory())).map_err(read_error)?;
        let mut file = directory.open_file(MARKER).map_err(read_error)?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes.len() > 4096 {
            return Err(invalid());
        }
        let marker: Completion = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if marker.schema != 1 || marker.id != self.id || marker.plan_sha256 != self.digest()? {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        Ok(())
    }

    fn digest(&self) -> Result<String, CoreError> {
        let bytes = serde_json::to_vec(self).map_err(|_| invalid())?;
        if bytes.len() > MAX_METADATA_BYTES {
            return Err(invalid());
        }
        Ok(format!("{:x}", Sha256::digest(&bytes)))
    }

    /// Full audit for install, repair and recovery. Never downloads or executes.
    pub fn verify(&self, paths: &AppPaths, cancel: &Cancellation) -> Result<(), CoreError> {
        self.verify_marker(paths)?;
        let directory = PrivateDir::open_data(paths.root().join(self.directory()))
            .map_err(map_private_error)?;
        for expected in &self.files {
            cancel.check()?;
            let actual = fingerprint(&directory, &expected.path, cancel)?;
            if actual.sha256 != expected.sha256
                || actual.sha1 != expected.sha1
                || actual.size != expected.size
            {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
        }
        Ok(())
    }
}

/// Commit a new cache generation. Instance publication remains under its lease.
pub fn publish(
    paths: &AppPaths,
    plan: &ProcessorPlan,
    staged: StagedLoader,
    cancel: &Cancellation,
) -> Result<GenerationRef, CoreError> {
    cancel.check()?;
    if staged.operation.is_nil()
        || staged.root
            != paths
                .root()
                .join(format!("libraries/neoforge/{}", staged.operation))
    {
        return Err(invalid());
    }
    let directory = PrivateDir::open_data(&staged.root).map_err(map_private_error)?;
    match directory.open_file(MARKER) {
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(invalid()),
    }
    let mut files = Vec::new();
    for input in &plan.inputs {
        let file = fingerprint(&directory, &input.destination, cancel)?;
        let (hash, size) = match &input.source {
            InputSource::Library(source) => (&source.sha1, source.size),
            InputSource::Mojang(source) => (&source.sha1, source.size),
            InputSource::Embedded { sha1, size, .. } => (sha1, *size),
        };
        if &file.sha1 != hash || file.size != size {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        files.push(file);
    }
    for output in &staged.files {
        let file = fingerprint(&directory, &output.path, cancel)?;
        if file.sha256 != output.sha256 || file.size != output.size {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        files.push(file);
    }
    if staged
        .files
        .iter()
        .map(|file| &file.path)
        .collect::<BTreeSet<_>>()
        != plan.derived.iter().collect()
    {
        return Err(invalid());
    }
    let reference = GenerationRef {
        schema: 1,
        loader: "neoforge".into(),
        id: staged.operation,
        installer: plan.identity.clone(),
        java: plan.java.clone(),
        profile: plan.launch.clone(),
        files,
        derived: plan.derived.clone(),
    };
    reference.validate(reference.minecraft())?;
    let marker = Completion {
        schema: 1,
        id: reference.id,
        plan_sha256: reference.digest()?,
    };
    cancel.commit(|| {
        directory
            .write_atomic_durable(
                MARKER,
                &crate::storage_json::pretty(&marker).map_err(|_| invalid())?,
            )
            .map_err(map_private_error)
    })?;
    reference.verify(paths, cancel)?;
    Ok(reference)
}

pub(crate) fn hash256(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn flag<'a>(args: &'a [String], key: &str) -> Result<&'a str, CoreError> {
    let matches: Vec<_> = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| arg.as_str() == key)
        .collect();
    if matches.len() != 1 {
        return Err(invalid());
    }
    args.get(matches[0].0 + 1)
        .map(String::as_str)
        .ok_or_else(invalid)
}

pub(crate) fn maven_key(path: &str) -> Result<String, CoreError> {
    safe_path(path)?;
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() < 4 {
        return Err(invalid());
    }
    let version = parts[parts.len() - 2];
    let name = parts[parts.len() - 3];
    let prefix = format!("{name}-{version}");
    let suffix = parts[parts.len() - 1]
        .strip_suffix(".jar")
        .and_then(|file| file.strip_prefix(&prefix))
        .ok_or_else(invalid)?;
    if !suffix.is_empty() && !suffix.starts_with('-') {
        return Err(invalid());
    }
    Ok(format!("{}:{suffix}", parts[..parts.len() - 2].join("/")))
}

pub(crate) fn fingerprint(
    directory: &PrivateDir,
    path: &str,
    cancel: &Cancellation,
) -> Result<GenerationFile, CoreError> {
    safe_path(path)?;
    let file = directory.open_file(path).map_err(read_error)?;
    let mut input = native_file(&file)?;
    let size = input.metadata().map_err(io)?.len();
    if size == 0 || size > 2 * 1024 * 1024 * 1024 {
        return Err(CoreError::new(ErrorCode::ArtifactMismatch));
    }
    let mut sha1 = Sha1::new();
    let mut sha256 = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut count = 0u64;
    loop {
        cancel.check()?;
        let size = input.read(&mut buffer).map_err(io)?;
        if size == 0 {
            break;
        }
        count += size as u64;
        if count > 2 * 1024 * 1024 * 1024 {
            return Err(invalid());
        }
        sha1.update(&buffer[..size]);
        sha256.update(&buffer[..size]);
    }
    file.verify().map_err(map_private_error)?;
    if count != size {
        return Err(CoreError::new(ErrorCode::ArtifactMismatch));
    }
    Ok(GenerationFile {
        path: path.into(),
        sha1: format!("{:x}", sha1.finalize()),
        sha256: format!("{:x}", sha256.finalize()),
        size,
    })
}

impl GenerationFile {
    pub(crate) fn artifact(&self) -> Artifact {
        // Adapter for receipt verification only, never download policy.
        Artifact {
            url: String::new(),
            sha1: self.sha1.clone(),
            size: self.size,
            path: None,
        }
    }
}

pub(crate) fn read_error(error: private_fs::Error) -> CoreError {
    match &error {
        private_fs::Error::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            CoreError::new(ErrorCode::ArtifactMismatch)
        }
        _ => map_private_error(error),
    }
}

#[cfg(test)]
pub(crate) mod tests;
