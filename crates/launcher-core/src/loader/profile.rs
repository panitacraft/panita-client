//! Fabric and Quilt. Neither runs processors: the loader is a launch profile and a
//! small library set. The profile comes from the loader's public meta service and
//! is bound to the exact game and loader versions requested. Every library must come
//! from an approved Maven root and verify against a hash, either from the profile
//! itself or from the Maven SHA-1 sidecar, before the generation is published.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use private_fs::PrivateDir;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sha2::{Sha256, Sha512};
use uuid::Uuid;

use super::{LoaderRef, mismatch};
use crate::{
    error::{CoreError, ErrorCode},
    minecraft::{
        JavaRequirement, Platform,
        download::{Cancellation, Downloader, ensure_parent, io},
        plan::{self, InstallPlan},
        safe_path, valid_hash,
    },
    neoforge::{
        Coordinate, MAX_METADATA_BYTES, bounded_text, component,
        generation::{
            Completion, GenerationFile, MARKER, fingerprint, hash256, maven_key, read_error,
        },
        loader::Maven,
    },
    paths::AppPaths,
    storage::map_private_error,
};

const SCHEMA: u32 = 1;
const MAX_LIBRARIES: usize = 256;
const MAX_LIBRARY_BYTES: usize = 128 * 1024 * 1024;
const MAX_ARGUMENTS: usize = 64;
const MAX_SIDECAR: usize = 256;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
/// Libraries come only from these Maven repositories, over HTTPS, never redirected.
const ROOTS: [&str; 3] = [
    "https://maven.fabricmc.net/",
    "https://maven.quiltmc.org/repository/release/",
    "https://repo1.maven.org/maven2/",
];

fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidData)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Fabric,
    Quilt,
}

impl Flavor {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fabric => "fabric",
            Self::Quilt => "quilt",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "fabric" => Some(Self::Fabric),
            "quilt" => Some(Self::Quilt),
            _ => None,
        }
    }

    fn profile_url(self, minecraft: &str, version: &str) -> String {
        match self {
            Self::Fabric => format!(
                "https://meta.fabricmc.net/v2/versions/loader/{minecraft}/{version}/profile/json"
            ),
            Self::Quilt => format!(
                "https://meta.quiltmc.org/v3/versions/loader/{minecraft}/{version}/profile/json"
            ),
        }
    }

    fn profile_id(self, minecraft: &str, version: &str) -> String {
        format!("{}-loader-{version}-{minecraft}", self.name())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Launch {
    id: String,
    main_class: String,
    jvm: Vec<String>,
    game: Vec<String>,
}

/// Exact Fabric or Quilt state inside an installed plan.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRef {
    schema: u32,
    #[serde(rename = "type")]
    loader: String,
    id: Uuid,
    minecraft: String,
    version: String,
    java: JavaRequirement,
    profile: Launch,
    files: Vec<GenerationFile>,
    /// The exact URL each file came from, for explicit repair only.
    sources: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct RawProfile {
    id: String,
    #[serde(rename = "inheritsFrom")]
    inherits_from: String,
    #[serde(rename = "mainClass")]
    main_class: String,
    #[serde(default)]
    arguments: RawArguments,
    libraries: Vec<RawLibrary>,
}

#[derive(Default, Deserialize)]
struct RawArguments {
    #[serde(default)]
    game: Vec<String>,
    #[serde(default)]
    jvm: Vec<String>,
}

#[derive(Deserialize)]
struct RawLibrary {
    name: String,
    url: String,
    sha1: Option<String>,
    sha256: Option<String>,
    sha512: Option<String>,
    size: Option<u64>,
}

/// One library as the profile names it, before its bytes are downloaded.
struct Wanted {
    path: String,
    url: String,
    sha1: Option<String>,
    sha256: Option<String>,
    sha512: Option<String>,
    size: Option<u64>,
}

fn valid_class(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 512
        && name.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_$".contains(&c))
        })
}

fn valid_arguments(arguments: &[String]) -> bool {
    arguments.len() <= MAX_ARGUMENTS && arguments.iter().all(|argument| bounded_text(argument))
}

fn hex_of(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Decode the meta service's profile and bind it to the requested pair of versions.
fn decode(
    bytes: &[u8],
    flavor: Flavor,
    minecraft: &str,
    version: &str,
) -> Result<(Launch, Vec<Wanted>), CoreError> {
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(invalid());
    }
    let raw: RawProfile = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if raw.id != flavor.profile_id(minecraft, version)
        || raw.inherits_from != minecraft
        || !valid_class(&raw.main_class)
        || !valid_arguments(&raw.arguments.game)
        || !valid_arguments(&raw.arguments.jvm)
        || raw.libraries.is_empty()
        || raw.libraries.len() > MAX_LIBRARIES
    {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut wanted = Vec::new();
    for library in raw.libraries {
        let coordinate = Coordinate::parse(&library.name)?;
        let root = library.url.as_str();
        if !ROOTS.contains(&root)
            || library.sha1.as_deref().is_some_and(|v| !valid_hash(v))
            || library.sha256.as_deref().is_some_and(|v| !hash256(v))
            || library.sha512.as_deref().is_some_and(|v| !hex_of(v, 128))
            || library
                .size
                .is_some_and(|size| size == 0 || size > MAX_LIBRARY_BYTES as u64)
            || !seen.insert(coordinate.path().to_ascii_lowercase())
        {
            return Err(invalid());
        }
        wanted.push(Wanted {
            url: format!("{root}{}", coordinate.path()),
            path: coordinate.path().to_owned(),
            sha1: library.sha1,
            sha256: library.sha256,
            sha512: library.sha512,
            size: library.size,
        });
    }
    let launch = Launch {
        id: raw.id,
        main_class: raw.main_class,
        jvm: raw.arguments.jvm,
        game: raw.arguments.game,
    };
    Ok((launch, wanted))
}

/// Download one library and check every hash the profile supplied. Without a
/// hash in the profile, the Maven SHA-1 sidecar identifies the file.
async fn download(
    maven: &Maven,
    wanted: &Wanted,
    cancel: &Cancellation,
) -> Result<Vec<u8>, CoreError> {
    let sha1 = match &wanted.sha1 {
        Some(hash) => hash.clone(),
        None => {
            let sidecar = maven
                .get(&format!("{}.sha1", wanted.url), MAX_SIDECAR, cancel)
                .await?;
            let published = std::str::from_utf8(&sidecar)
                .map_err(|_| invalid())?
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !valid_hash(&published) {
                return Err(invalid());
            }
            published
        }
    };
    let bytes = maven.get(&wanted.url, MAX_LIBRARY_BYTES, cancel).await?;
    if bytes.is_empty()
        || wanted.size.is_some_and(|size| size != bytes.len() as u64)
        || format!("{:x}", Sha1::digest(&bytes)) != sha1
        || wanted
            .sha256
            .as_ref()
            .is_some_and(|hash| &format!("{:x}", Sha256::digest(&bytes)) != hash)
        || wanted
            .sha512
            .as_ref()
            .is_some_and(|hash| &format!("{:x}", Sha512::digest(&bytes)) != hash)
    {
        return Err(mismatch());
    }
    Ok(bytes)
}

struct Workspace {
    directory: PrivateDir,
    root: std::path::PathBuf,
    id: Uuid,
}

impl Workspace {
    fn create(paths: &AppPaths, flavor: Flavor) -> Result<Self, CoreError> {
        let root = PrivateDir::ensure(paths.root()).map_err(map_private_error)?;
        let libraries = root.ensure_dir("libraries").map_err(map_private_error)?;
        let container = libraries
            .ensure_dir(flavor.name())
            .map_err(map_private_error)?;
        let id = Uuid::new_v4();
        let directory = container
            .create_dir(id.to_string())
            .map_err(map_private_error)?;
        Ok(Self {
            directory,
            root: paths
                .root()
                .join("libraries")
                .join(flavor.name())
                .join(id.to_string()),
            id,
        })
    }

    fn write(&self, relative: &str, bytes: &[u8]) -> Result<(), CoreError> {
        let (parent, name) = ensure_parent(&self.root, relative)?;
        let mut file = parent.create(name).map_err(map_private_error)?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(map_private_error)
    }
}

impl ProfileRef {
    pub fn kind(&self) -> &str {
        &self.loader
    }

    pub fn minecraft(&self) -> &str {
        &self.minecraft
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn java(&self) -> &JavaRequirement {
        &self.java
    }

    pub fn files(&self) -> &[GenerationFile] {
        &self.files
    }

    pub fn profile_id(&self) -> &str {
        &self.profile.id
    }

    pub fn main_class(&self) -> &str {
        &self.profile.main_class
    }

    pub fn jvm_arguments(&self) -> &[String] {
        &self.profile.jvm
    }

    pub fn game_arguments(&self) -> &[String] {
        &self.profile.game
    }

    pub fn directory(&self) -> String {
        format!("libraries/{}/{}", self.loader, self.id)
    }

    fn flavor(&self) -> Result<Flavor, CoreError> {
        Flavor::from_name(&self.loader).ok_or_else(invalid)
    }

    pub fn validate(&self, minecraft: &str) -> Result<(), CoreError> {
        let flavor = self.flavor()?;
        if self.schema != SCHEMA
            || self.id.is_nil()
            || self.minecraft != minecraft
            || !component(&self.minecraft)
            || !component(&self.version)
            || self.java.major_version == 0
            || !component(&self.java.component)
            || self.profile.id != flavor.profile_id(&self.minecraft, &self.version)
            || !valid_class(&self.profile.main_class)
            || !valid_arguments(&self.profile.jvm)
            || !valid_arguments(&self.profile.game)
            || self.files.is_empty()
            || self.files.len() > MAX_LIBRARIES
            || self.sources.len() != self.files.len()
        {
            return Err(invalid());
        }
        let mut paths = BTreeSet::new();
        let mut total = 0u64;
        for file in &self.files {
            safe_path(&file.path)?;
            let library = file.path.strip_prefix("libraries/").ok_or_else(invalid)?;
            maven_key(library)?;
            let url = self.sources.get(&file.path).ok_or_else(invalid)?;
            if !valid_hash(&file.sha1)
                || !hash256(&file.sha256)
                || file.size == 0
                || file.size > MAX_LIBRARY_BYTES as u64
                || !paths.insert(file.path.to_ascii_lowercase())
                || !ROOTS.iter().any(|root| url == &format!("{root}{library}"))
            {
                return Err(invalid());
            }
            total = total.checked_add(file.size).ok_or_else(invalid)?;
        }
        if total > MAX_TOTAL {
            return Err(invalid());
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

    pub(crate) fn verify_marker(&self, paths: &AppPaths) -> Result<(), CoreError> {
        use std::io::Read;
        self.validate(&self.minecraft)?;
        let directory =
            PrivateDir::open(paths.root().join(self.directory())).map_err(read_error)?;
        let mut file = directory.open_file(MARKER).map_err(read_error)?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes.len() > 4096 {
            return Err(invalid());
        }
        let marker: Completion = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if marker.schema != 1 || marker.id != self.id || marker.plan_sha256 != self.digest()? {
            return Err(mismatch());
        }
        Ok(())
    }

    /// Full audit for install, repair and recovery. Never downloads or executes.
    pub fn verify(&self, paths: &AppPaths, cancel: &Cancellation) -> Result<(), CoreError> {
        self.verify_marker(paths)?;
        let directory =
            PrivateDir::open(paths.root().join(self.directory())).map_err(map_private_error)?;
        for expected in &self.files {
            cancel.check()?;
            let actual = fingerprint(&directory, &expected.path, cancel)?;
            if actual.sha256 != expected.sha256
                || actual.sha1 != expected.sha1
                || actual.size != expected.size
            {
                return Err(mismatch());
            }
        }
        Ok(())
    }

    /// Resolve library precedence while keeping the game's own classpath order. The
    /// loader's libraries replace same-named ones and follow the rest; the game jar
    /// comes last, where the loader's launcher expects to find it.
    pub(crate) fn classpath(
        &self,
        paths: &AppPaths,
        base: &InstallPlan,
    ) -> Result<Vec<std::path::PathBuf>, CoreError> {
        self.validate(&base.version.id)?;
        let root = paths.root().join(self.directory()).join("libraries");
        let mut children = BTreeMap::new();
        for file in &self.files {
            let library = file.path.strip_prefix("libraries/").ok_or_else(invalid)?;
            if children.insert(maven_key(library)?, library).is_some() {
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
        for file in &self.files {
            let library = file.path.strip_prefix("libraries/").ok_or_else(invalid)?;
            if used.insert(maven_key(library)?) {
                classpath.push(root.join(library));
            }
        }
        classpath.push(paths.root().join(base.client_path()?));
        Ok(classpath)
    }

    /// Explicit repair only: fetch the exact recorded files again into a fresh
    /// generation and require the same bytes.
    pub async fn repair(&self, paths: &AppPaths, cancel: &Cancellation) -> Result<Self, CoreError> {
        self.repair_with(paths, &Maven::new()?, cancel).await
    }

    async fn repair_with(
        &self,
        paths: &AppPaths,
        maven: &Maven,
        cancel: &Cancellation,
    ) -> Result<Self, CoreError> {
        self.validate(&self.minecraft)?;
        let flavor = self.flavor()?;
        let mut items = Vec::new();
        for file in &self.files {
            let url = self.sources.get(&file.path).ok_or_else(invalid)?;
            let wanted = Wanted {
                path: file.path.trim_start_matches("libraries/").to_owned(),
                url: url.clone(),
                sha1: Some(file.sha1.clone()),
                sha256: Some(file.sha256.clone()),
                sha512: None,
                size: Some(file.size),
            };
            items.push((
                wanted.path.clone(),
                url.clone(),
                download(maven, &wanted, cancel).await?,
            ));
        }
        let replacement = publish(
            paths,
            flavor,
            &self.minecraft,
            &self.version,
            self.java.clone(),
            self.profile.clone(),
            items,
            cancel,
        )?;
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
        if expected != actual {
            return Err(mismatch());
        }
        Ok(replacement)
    }
}

/// Stage the downloaded libraries, bind them with a completion marker and return
/// the reference. Nothing is published to an instance here.
#[allow(clippy::too_many_arguments)]
fn publish(
    paths: &AppPaths,
    flavor: Flavor,
    minecraft: &str,
    version: &str,
    java: JavaRequirement,
    launch: Launch,
    items: Vec<(String, String, Vec<u8>)>,
    cancel: &Cancellation,
) -> Result<ProfileRef, CoreError> {
    let workspace = Workspace::create(paths, flavor)?;
    let mut files = Vec::new();
    let mut sources = BTreeMap::new();
    for (library, url, bytes) in &items {
        cancel.check()?;
        let relative = format!("libraries/{library}");
        workspace.write(&relative, bytes)?;
        files.push(fingerprint(&workspace.directory, &relative, cancel)?);
        sources.insert(relative, url.clone());
    }
    let reference = ProfileRef {
        schema: SCHEMA,
        loader: flavor.name().to_owned(),
        id: workspace.id,
        minecraft: minecraft.to_owned(),
        version: version.to_owned(),
        java,
        profile: launch,
        files,
        sources,
    };
    reference.validate(minecraft)?;
    let marker = Completion {
        schema: 1,
        id: reference.id,
        plan_sha256: reference.digest()?,
    };
    cancel.commit(|| {
        workspace
            .directory
            .write_atomic_durable(
                MARKER,
                &crate::storage_json::pretty(&marker).map_err(|_| invalid())?,
            )
            .map_err(map_private_error)
    })?;
    reference.verify(paths, cancel)?;
    Ok(reference)
}

/// Resolve the game plan and publish the exact loader generation it needs.
pub async fn prepare_plan(
    paths: &AppPaths,
    platform: &Platform,
    flavor: Flavor,
    minecraft: &str,
    version: &str,
    cancel: &Cancellation,
    maven: &Maven,
) -> Result<InstallPlan, CoreError> {
    if !component(minecraft) || !component(version) {
        return Err(invalid());
    }
    let document = maven
        .get(
            &flavor.profile_url(minecraft, version),
            MAX_METADATA_BYTES,
            cancel,
        )
        .await?;
    let (launch, wanted) = decode(&document, flavor, minecraft, version)?;
    let mut items = Vec::new();
    for library in &wanted {
        items.push((
            library.path.clone(),
            library.url.clone(),
            download(maven, library, cancel).await?,
        ));
    }
    let downloader = Downloader::new()?;
    let mut base = plan::resolve(&downloader, minecraft, platform, cancel).await?;
    base.allocate_runtime(paths)?;
    let java = base.version.java.clone();
    let reference = publish(
        paths, flavor, minecraft, version, java, launch, items, cancel,
    )?;
    base.schema_version = 2;
    base.loader = Some(LoaderRef::Profile(reference));
    base.validate()?;
    Ok(base)
}

#[cfg(test)]
mod tests;
