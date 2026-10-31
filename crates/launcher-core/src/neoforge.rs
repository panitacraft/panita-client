//! Pinned official NeoForge installer contracts. Decoding does not install files,
//! execute processors, or authorize a game launch.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor},
};
use sha1::{Digest, Sha1};

use crate::error::{CoreError, ErrorCode};
use crate::minecraft::{safe_path, valid_hash};

pub mod generation;
pub mod loader;
pub mod plan;
pub mod staging;
mod transport;

pub const MAVEN_ROOT: &str = "https://maven.neoforged.net/releases/";
pub const MAX_INSTALLER_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_LIBRARIES: usize = 2048;
const MAX_PROCESSORS: usize = 128;
const MAX_ARGUMENTS: usize = 512;

fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidData)
}

pub(crate) fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-+".contains(&c))
        && safe_path(value).is_ok()
}

/// Maven coordinates are parsed rather than interpolated as filesystem paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coordinate {
    path: String,
}

impl Coordinate {
    pub fn parse(value: &str) -> Result<Self, CoreError> {
        if value.len() > 512 {
            return Err(invalid());
        }
        let mut extension = value.split('@');
        let coordinate = extension.next().ok_or_else(invalid)?;
        let extension_name = extension.next().unwrap_or("jar");
        if extension.next().is_some() || !component(extension_name) {
            return Err(invalid());
        }
        let parts: Vec<_> = coordinate.split(':').collect();
        if !(3..=4).contains(&parts.len())
            || !parts.iter().all(|part| component(part))
            || !parts[0].split('.').all(component)
        {
            return Err(invalid());
        }
        let classifier = parts
            .get(3)
            .map(|part| format!("-{part}"))
            .unwrap_or_default();
        let path = format!(
            "{}/{}/{}/{}-{}{}.{}",
            parts[0].replace('.', "/"),
            parts[1],
            parts[2],
            parts[1],
            parts[2],
            classifier,
            extension_name
        );
        safe_path(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

/// A separate policy leaves the existing Mojang-only vanilla transport intact.
pub fn library_url(raw: &str, path: &str) -> Result<url::Url, CoreError> {
    safe_path(path)?;
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    if raw.len() > 2048
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || ![
            format!("{MAVEN_ROOT}{path}"),
            format!("https://libraries.minecraft.net/{path}"),
        ]
        .contains(&raw.to_owned())
    {
        return Err(invalid());
    }
    Ok(url)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallerIdentity {
    pub minecraft: String,
    pub version: String,
    pub sha1: String,
    pub size: u64,
}

impl InstallerIdentity {
    pub fn url(&self) -> Result<String, CoreError> {
        if !component(&self.minecraft)
            || !component(&self.version)
            || !valid_hash(&self.sha1)
            || self.size == 0
            || self.size > MAX_INSTALLER_BYTES as u64
        {
            return Err(invalid());
        }
        let coordinate = Coordinate::parse(&format!(
            "net.neoforged:neoforge:{}:installer",
            self.version
        ))?;
        Ok(format!("{MAVEN_ROOT}{}", coordinate.path()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryArtifact {
    pub path: String,
    pub url: String,
    pub sha1: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryDownloads {
    pub artifact: LibraryArtifact,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub name: String,
    pub downloads: LibraryDownloads,
}

impl Library {
    fn validate(&self) -> Result<(), CoreError> {
        let coordinate = Coordinate::parse(&self.name)?;
        let artifact = &self.downloads.artifact;
        if coordinate.path() != artifact.path
            || !valid_hash(&artifact.sha1)
            || artifact.size == 0
            || artifact.size > 2 * 1024 * 1024 * 1024
        {
            return Err(invalid());
        }
        library_url(&artifact.url, coordinate.path())?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SideData {
    pub client: String,
    pub server: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Processor {
    #[serde(default)]
    pub sides: Option<Vec<String>>,
    pub jar: String,
    pub classpath: Vec<String>,
    pub args: Vec<String>,
    #[serde(default, deserialize_with = "unique_map")]
    pub outputs: BTreeMap<String, String>,
}

impl Processor {
    pub fn runs_on_client(&self) -> bool {
        self.sides
            .as_ref()
            .is_none_or(|sides| sides.iter().any(|side| side == "client"))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct InstallProfile {
    pub spec: u32,
    pub version: String,
    pub minecraft: String,
    pub json: String,
    #[serde(deserialize_with = "unique_map")]
    pub data: BTreeMap<String, SideData>,
    pub processors: Vec<Processor>,
    pub libraries: Vec<Library>,
    #[serde(flatten)]
    decorations: BTreeMap<String, serde_json::Value>,
}

// Map decoding must not silently replace duplicate processor variables or outputs.
fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Unique<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Unique<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an object with unique keys")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
            let mut map = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<String, T>()? {
                if map.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate metadata key"));
                }
            }
            Ok(map)
        }
    }
    deserializer.deserialize_map(Unique(std::marker::PhantomData))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchArguments {
    pub game: Vec<String>,
    pub jvm: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchProfile {
    pub id: String,
    pub time: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    pub r#type: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(rename = "inheritsFrom")]
    pub inherits_from: String,
    pub arguments: LaunchArguments,
    pub libraries: Vec<Library>,
}

#[derive(Clone, Debug)]
pub struct InstallerMetadata {
    pub install: InstallProfile,
    pub launch: LaunchProfile,
}

pub(crate) fn bounded_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 16 * 1024 && !value.chars().any(char::is_control)
}

fn token_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
}

fn validate_tokens(value: &str, known: &BTreeSet<&str>) -> Result<(), CoreError> {
    if !bounded_text(value) {
        return Err(invalid());
    }
    if value.starts_with('[') && value.ends_with(']') {
        Coordinate::parse(&value[1..value.len() - 1])?;
        return Ok(());
    }
    let mut rest = value;
    while let Some((prefix, suffix)) = rest.split_once('{') {
        if prefix.contains('}') {
            return Err(invalid());
        }
        let (key, tail) = suffix.split_once('}').ok_or_else(invalid)?;
        if !known.contains(key) {
            return Err(invalid());
        }
        rest = tail;
    }
    if rest.contains('}') {
        return Err(invalid());
    }
    Ok(())
}

impl InstallerMetadata {
    /// Verify the entire pinned archive before interpreting any embedded metadata.
    /// Reads bounded entries in memory and never extracts to a filesystem.
    pub fn decode(bytes: &[u8], identity: &InstallerIdentity) -> Result<Self, CoreError> {
        identity.url()?;
        if bytes.len() as u64 != identity.size
            || format!("{:x}", Sha1::digest(bytes)) != identity.sha1
        {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid())?;
        if archive.len() > MAX_ENTRIES {
            return Err(invalid());
        }
        let mut names = BTreeSet::new();
        for index in 0..archive.len() {
            let entry = archive.by_index(index).map_err(|_| invalid())?;
            let name = entry.name().strip_suffix('/').unwrap_or(entry.name());
            safe_path(name)?;
            if !names.insert(name.to_ascii_lowercase())
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(invalid());
            }
        }
        let profile_bytes = read_entry(&mut archive, "install_profile.json")?;
        let install: InstallProfile =
            serde_json::from_slice(&profile_bytes).map_err(|_| invalid())?;
        if install.json != "/version.json" {
            return Err(invalid());
        }
        let launch_bytes = read_entry(&mut archive, "version.json")?;
        let launch: LaunchProfile = serde_json::from_slice(&launch_bytes).map_err(|_| invalid())?;
        let metadata = Self { install, launch };
        metadata.validate(identity)?;
        Ok(metadata)
    }

    pub fn validate(&self, identity: &InstallerIdentity) -> Result<(), CoreError> {
        identity.url()?;
        let install = &self.install;
        let launch = &self.launch;
        let id = format!("neoforge-{}", identity.version);
        if install.decorations.keys().any(|key| {
            !matches!(
                key.as_str(),
                "profile"
                    | "icon"
                    | "logo"
                    | "welcome"
                    | "mirrorList"
                    | "hideExtract"
                    | "serverJarPath"
            )
        }) {
            return Err(invalid());
        }
        if install.spec != 1
            || install.version != id
            || install.minecraft != identity.minecraft
            || install.json != "/version.json"
            || launch.id != id
            || launch.inherits_from != identity.minecraft
            || !launch.main_class.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_$".contains(&c))
            })
            || launch.main_class.len() > 512
            || launch.r#type != "release"
            || install.data.len() > 128
            || install.processors.len() > MAX_PROCESSORS
            || install.processors.is_empty()
        {
            return Err(invalid());
        }
        validate_libraries(&install.libraries)?;
        validate_libraries(&launch.libraries)?;
        // Reject aliases with conflicting content between processor and launch libraries.
        let mut paths = BTreeMap::new();
        for library in install.libraries.iter().chain(&launch.libraries) {
            let artifact = &library.downloads.artifact;
            let signature = (&artifact.sha1, artifact.size);
            if paths
                .insert(artifact.path.to_ascii_lowercase(), signature)
                .is_some_and(|previous| previous != signature)
            {
                return Err(invalid());
            }
        }
        for args in [&launch.arguments.game, &launch.arguments.jvm] {
            if args.len() > MAX_ARGUMENTS || !args.iter().all(|arg| bounded_text(arg)) {
                return Err(invalid());
            }
        }
        for (flag, expected) in [
            ("--fml.neoForgeVersion", &identity.version),
            ("--fml.mcVersion", &identity.minecraft),
        ] {
            let positions: Vec<_> = launch
                .arguments
                .game
                .iter()
                .enumerate()
                .filter(|(_, value)| value.as_str() == flag)
                .map(|(index, _)| index)
                .collect();
            if positions.len() != 1 || launch.arguments.game.get(positions[0] + 1) != Some(expected)
            {
                return Err(invalid());
            }
        }
        let reserved = [
            "SIDE",
            "MINECRAFT_JAR",
            "MINECRAFT_VERSION",
            "ROOT",
            "INSTALLER",
            "LIBRARY_DIR",
        ];
        let mut known: BTreeSet<_> = reserved.into_iter().collect();
        for (key, data) in &install.data {
            if !token_name(key) || !known.insert(key.as_str()) {
                return Err(invalid());
            }
            validate_data(&data.client)?;
            validate_data(&data.server)?;
        }
        let libraries: BTreeSet<_> = install
            .libraries
            .iter()
            .map(|lib| lib.name.as_str())
            .collect();
        let mut client_count = 0;
        for processor in &install.processors {
            if processor.sides.as_ref().is_some_and(|sides| {
                sides.is_empty()
                    || sides.len() > 2
                    || sides
                        .iter()
                        .any(|side| !matches!(side.as_str(), "client" | "server"))
                    || sides.iter().collect::<BTreeSet<_>>().len() != sides.len()
            }) || processor.classpath.len() > MAX_LIBRARIES
                || processor.args.len() > MAX_ARGUMENTS
                || processor.outputs.len() > 128
            {
                return Err(invalid());
            }
            if !processor.runs_on_client() {
                continue;
            }
            client_count += 1;
            if !libraries.contains(processor.jar.as_str())
                || processor
                    .classpath
                    .iter()
                    .any(|name| !libraries.contains(name.as_str()))
                || processor.args.is_empty()
            {
                return Err(invalid());
            }
            for argument in &processor.args {
                validate_tokens(argument, &known)?;
            }
            for (output, hash) in &processor.outputs {
                validate_tokens(output, &known)?;
                validate_tokens(hash, &known)?;
                if !(valid_hash(hash) || hash.starts_with('{') && hash.ends_with('}')) {
                    return Err(invalid());
                }
            }
        }
        if client_count == 0 {
            return Err(invalid());
        }
        Ok(())
    }
}

fn validate_libraries(libraries: &[Library]) -> Result<(), CoreError> {
    if libraries.is_empty() || libraries.len() > MAX_LIBRARIES {
        return Err(invalid());
    }
    let mut paths = BTreeSet::new();
    for library in libraries {
        library.validate()?;
        if !paths.insert(library.downloads.artifact.path.to_ascii_lowercase()) {
            return Err(invalid());
        }
    }
    Ok(())
}

fn validate_data(value: &str) -> Result<(), CoreError> {
    if !bounded_text(value) {
        return Err(invalid());
    }
    if value.starts_with('[') && value.ends_with(']') {
        Coordinate::parse(&value[1..value.len() - 1])?;
    } else if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        // Literals are data, never shell commands or recursively expanded tokens.
    } else {
        let entry = value.strip_prefix('/').ok_or_else(invalid)?;
        safe_path(entry)?;
        if !entry.starts_with("data/") {
            return Err(invalid());
        }
    }
    Ok(())
}

fn read_entry(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, CoreError> {
    let entry = archive.by_name(name).map_err(|_| invalid())?;
    if entry.is_dir() || entry.size() > MAX_METADATA_BYTES as u64 {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    entry
        .take(MAX_METADATA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
