//! Official vanilla metadata. Resolution is independent of desktop and account state.
use std::collections::{BTreeMap, HashSet};

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::error::{CoreError, ErrorCode};
pub mod download;
#[cfg(target_os = "linux")]
mod executable;
pub mod install;
pub mod launch;
mod log_context;
pub mod maintenance;
pub mod native_cache;
pub mod natives;
pub mod plan;
pub mod play;
pub mod process;
pub mod runtime;
pub mod support;
pub(crate) mod transaction;
pub mod update;
#[cfg(feature = "loader-validation")]
pub mod validation;
mod verification_cache;
mod world_backup;
pub(crate) use world_backup::capability as directory_capability;

pub const VERSION_MANIFEST: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const RUNTIME_INDEX: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

pub(crate) fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidData)
}

/// Portable relative paths also reject Windows aliases on Unix hosts.
pub fn safe_path(path: &str) -> Result<(), CoreError> {
    if path.is_empty()
        || !path.is_ascii()
        || path.len() > 2048
        || path.contains(['\\', ':'])
        || path.chars().any(|c| c.is_control())
    {
        return Err(invalid());
    }
    for part in path.split('/') {
        let stem = part
            .split('.')
            .next()
            .ok_or_else(invalid)?
            .to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.contains(['<', '>', '"', '|', '?', '*'])
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

pub fn official_url(raw: &str) -> Result<url::Url, CoreError> {
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    if raw.len() > 2048
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
        || !matches!(
            url.host_str(),
            Some(
                "piston-meta.mojang.com"
                    | "piston-data.mojang.com"
                    | "launchermeta.mojang.com"
                    | "launcher.mojang.com"
                    | "libraries.minecraft.net"
                    | "resources.download.minecraft.net"
            )
        )
    {
        return Err(invalid());
    }
    Ok(url)
}

pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 40
        && hash
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Artifact {
    pub url: String,
    pub sha1: String,
    pub size: u64,
    #[serde(default)]
    pub path: Option<String>,
}

impl Artifact {
    pub fn validate(&self) -> Result<(), CoreError> {
        official_url(&self.url)?;
        if !valid_hash(&self.sha1) || self.size == 0 || self.size > 2 * 1024 * 1024 * 1024 {
            return Err(invalid());
        }
        if let Some(path) = &self.path {
            safe_path(path)?;
        }
        Ok(())
    }

    pub fn verify(&self, bytes: &[u8]) -> Result<(), CoreError> {
        self.validate()?;
        if bytes.len() as u64 != self.size || format!("{:x}", Sha1::digest(bytes)) != self.sha1 {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Platform {
    pub os: String,
    /// Architecture reported by the selected JVM, independent of the host OS.
    pub arch: String,
    pub os_version: String,
    pub features: BTreeMap<String, bool>,
}

impl Platform {
    pub fn x64(os: &str, version: &str) -> Result<Self, CoreError> {
        if !matches!(os, "windows" | "linux") {
            return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
        }
        Ok(Self {
            os: os.into(),
            arch: "amd64".into(),
            os_version: version.into(),
            features: BTreeMap::new(),
        })
    }
    pub fn runtime_key(&self) -> Result<&'static str, CoreError> {
        match (self.os.as_str(), self.arch.as_str()) {
            ("windows", "amd64" | "x86_64") => Ok("windows-x64"),
            ("linux", "amd64" | "x86_64") => Ok("linux"),
            _ => Err(CoreError::new(ErrorCode::RuntimeUnsupported)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OsRule {
    pub name: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
    #[serde(rename = "versionRange")]
    pub version_range: Option<VersionRange>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VersionRange {
    pub min: Option<String>,
    pub max: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub action: Action,
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: BTreeMap<String, bool>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Disallow,
}

fn version_numbers(version: &str) -> Result<Vec<u32>, CoreError> {
    let mut numbers = version
        .split('.')
        .map(|part| part.parse().map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?;
    if numbers.is_empty() || numbers.len() > 8 {
        return Err(invalid());
    }
    numbers.resize(8, 0);
    Ok(numbers)
}

impl Rule {
    fn matches(&self, platform: &Platform) -> Result<bool, CoreError> {
        if self
            .features
            .iter()
            .any(|(name, value)| platform.features.get(name).copied().unwrap_or(false) != *value)
        {
            return Ok(false);
        }
        if let Some(os) = &self.os {
            if os.name.as_ref().is_some_and(|name| name != &platform.os) {
                return Ok(false);
            }
            for (pattern, value) in [
                (&os.arch, &platform.arch),
                (&os.version, &platform.os_version),
            ] {
                if let Some(pattern) = pattern {
                    if pattern.len() > 512 {
                        return Err(invalid());
                    }
                    let regex = Regex::new(&format!("^(?:{pattern})$")).map_err(|_| invalid())?;
                    if !regex.is_match(value) {
                        return Ok(false);
                    }
                }
            }
            if let Some(range) = &os.version_range {
                let current = version_numbers(&platform.os_version)?;
                if let Some(min) = &range.min
                    && current < version_numbers(min)?
                {
                    return Ok(false);
                }
                if let Some(max) = &range.max
                    && current >= version_numbers(max)?
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

/// Mojang rules are ordered: a nonempty list starts denied; the last matching action wins.
pub fn allowed(rules: &[Rule], platform: &Platform) -> Result<bool, CoreError> {
    let mut result = rules.is_empty();
    for rule in rules {
        if rule.matches(platform)? {
            result = matches!(rule.action, Action::Allow);
        }
    }
    Ok(result)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Values {
    One(String),
    Many(Vec<String>),
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Argument {
    Literal(String),
    Conditional {
        #[serde(default)]
        rules: Vec<Rule>,
        value: Values,
    },
}

pub fn arguments(
    args: &[Argument],
    platform: &Platform,
    replacements: &BTreeMap<String, String>,
) -> Result<Vec<String>, CoreError> {
    let mut output = Vec::new();
    for arg in args {
        let values = match arg {
            Argument::Literal(value) => vec![value],
            Argument::Conditional { rules, value } => {
                if !allowed(rules, platform)? {
                    continue;
                }
                match value {
                    Values::One(value) => vec![value],
                    Values::Many(values) => values.iter().collect(),
                }
            }
        };
        for value in values {
            output.push(substitute(value, replacements)?);
        }
    }
    Ok(output)
}

/// Substitute once; replacement values (including paths with spaces) remain single arguments.
pub fn substitute(
    value: &str,
    replacements: &BTreeMap<String, String>,
) -> Result<String, CoreError> {
    let mut output = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let end = rest.find('}').ok_or_else(invalid)?;
        output.push_str(replacements.get(&rest[..end]).ok_or_else(invalid)?);
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    if output.contains('\0') {
        return Err(invalid());
    }
    Ok(output)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LibraryDownloads {
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: BTreeMap<String, Artifact>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Extraction {
    #[serde(default)]
    pub exclude: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Library {
    pub name: String,
    pub downloads: LibraryDownloads,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub natives: BTreeMap<String, String>,
    pub extract: Option<Extraction>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaRequirement {
    pub component: String,
    pub major_version: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AssetIndex {
    pub id: String,
    #[serde(flatten)]
    pub artifact: Artifact,
    #[serde(rename = "totalSize")]
    pub total_size: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LoggingFile {
    pub id: String,
    #[serde(flatten)]
    pub artifact: Artifact,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Logging {
    pub argument: String,
    pub file: LoggingFile,
    #[serde(rename = "type")]
    pub kind: String,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionMetadata {
    pub id: String,
    pub inherits_from: Option<String>,
    pub main_class: Option<String>,
    pub java_version: Option<JavaRequirement>,
    pub asset_index: Option<AssetIndex>,
    #[serde(default)]
    pub downloads: BTreeMap<String, Artifact>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    pub arguments: Option<Arguments>,
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub logging: BTreeMap<String, Logging>,
    #[serde(rename = "type")]
    pub kind: String,
}

impl VersionMetadata {
    /// Decode only after authenticating the document against its manifest entry.
    pub fn decode(bytes: &[u8], source: &Artifact, expected_id: &str) -> Result<Self, CoreError> {
        source.verify(bytes)?;
        let metadata: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        if metadata.id != expected_id {
            return Err(invalid());
        }
        safe_path(expected_id)?;
        if expected_id.contains('/') {
            return Err(invalid());
        }
        Ok(metadata)
    }

    /// Merge parent first. The caller bounds depth and detects cycles before fetching parents.
    pub fn inherit(mut self, parent: Self) -> Result<Self, CoreError> {
        if self.inherits_from.as_deref() != Some(&parent.id) || parent.inherits_from.is_some() {
            return Err(invalid());
        }
        self.main_class = self.main_class.or(parent.main_class);
        self.java_version = self.java_version.or(parent.java_version);
        self.asset_index = self.asset_index.or(parent.asset_index);
        self.minecraft_arguments = self.minecraft_arguments.or(parent.minecraft_arguments);
        let mut downloads = parent.downloads;
        downloads.extend(self.downloads);
        self.downloads = downloads;
        let mut logging = parent.logging;
        logging.extend(self.logging);
        self.logging = logging;
        let mut libraries = parent.libraries;
        for library in self.libraries {
            let key = library_key(&library.name)?;
            if let Some(index) = libraries
                .iter()
                .position(|old| library_key(&old.name).is_ok_and(|old| old == key))
            {
                libraries[index] = library;
            } else {
                libraries.push(library);
            }
        }
        self.libraries = libraries;
        self.arguments = match (parent.arguments, self.arguments) {
            (Some(mut parent), Some(child)) => {
                parent.game.extend(child.game);
                parent.jvm.extend(child.jvm);
                Some(parent)
            }
            (parent, child) => child.or(parent),
        };
        self.inherits_from = None;
        Ok(self)
    }

    pub fn resolve(&self, platform: &Platform) -> Result<ResolvedVersion, CoreError> {
        platform.runtime_key()?;
        if self.inherits_from.is_some() || self.libraries.len() > 4096 {
            return Err(invalid());
        }
        safe_path(&self.id)?;
        let client = self.downloads.get("client").ok_or_else(invalid)?.clone();
        client.validate()?;
        let java = self
            .java_version
            .clone()
            .ok_or_else(|| CoreError::new(ErrorCode::RuntimeUnsupported))?;
        safe_path(&java.component)?;
        if java.component.contains('/') || java.major_version == 0 {
            return Err(invalid());
        }
        let assets = self.asset_index.clone().ok_or_else(invalid)?;
        safe_path(&assets.id)?;
        if assets.id.contains('/') {
            return Err(invalid());
        }
        assets.artifact.validate()?;
        let main_class = self.main_class.clone().ok_or_else(invalid)?;
        if main_class.is_empty()
            || !main_class
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || b"._$".contains(&ch))
        {
            return Err(invalid());
        }
        let mut classpath = Vec::new();
        let mut natives = Vec::new();
        let mut paths = HashSet::new();
        for library in &self.libraries {
            library_key(&library.name)?;
            if !allowed(&library.rules, platform)? {
                continue;
            }
            if let Some(artifact) = &library.downloads.artifact {
                // Modern native JARs stay on the classpath; LWJGL extracts them at runtime.
                let classifier = library.name.split(':').nth(3);
                if classifier.is_some_and(|name| {
                    name.starts_with("natives-") && !native_classifier(name, platform)
                }) {
                    continue;
                }
                artifact.validate()?;
                let path = artifact.path.as_ref().ok_or_else(invalid)?;
                if !paths.insert(path.to_ascii_lowercase()) {
                    return Err(invalid());
                }
                classpath.push(artifact.clone());
            }
            if let Some(classifier) = library.natives.get(&platform.os) {
                let bits = if matches!(platform.arch.as_str(), "amd64" | "x86_64" | "aarch64") {
                    "64"
                } else {
                    "32"
                };
                let key = classifier.replace("${arch}", bits);
                let artifact = library
                    .downloads
                    .classifiers
                    .get(&key)
                    .ok_or_else(invalid)?
                    .clone();
                artifact.validate()?;
                artifact.path.as_ref().ok_or_else(invalid)?;
                let excludes = library
                    .extract
                    .as_ref()
                    .map_or_else(Vec::new, |extract| extract.exclude.clone());
                for exclude in &excludes {
                    safe_path(exclude.trim_end_matches('/'))?;
                }
                natives.push(NativeArchive { artifact, excludes });
            }
        }
        let logging = self.logging.get("client").cloned();
        if let Some(logging) = &logging {
            logging.file.artifact.validate()?;
            safe_path(&logging.file.id)?;
            if logging.file.id.contains('/') || logging.kind != "log4j2-xml" {
                return Err(invalid());
            }
            substitute(
                &logging.argument,
                &BTreeMap::from([("path".into(), "logging.xml".into())]),
            )?;
        }
        Ok(ResolvedVersion {
            id: self.id.clone(),
            kind: self.kind.clone(),
            main_class,
            java,
            assets,
            client,
            classpath,
            natives,
            logging,
            arguments: self.arguments.clone(),
            legacy_arguments: self.minecraft_arguments.clone(),
        })
    }
}

fn library_key(name: &str) -> Result<String, CoreError> {
    let parts: Vec<_> = name.split(':').collect();
    if !(3..=4).contains(&parts.len())
        || parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || b"._-+".contains(&ch))
        })
    {
        return Err(invalid());
    }
    Ok(format!(
        "{}:{}:{}",
        parts[0],
        parts[1],
        parts.get(3).copied().unwrap_or_default()
    ))
}

fn native_classifier(name: &str, platform: &Platform) -> bool {
    let os = match platform.os.as_str() {
        "osx" => "macos",
        os => os,
    };
    let suffix = match platform.arch.as_str() {
        "amd64" | "x86_64" => "",
        "aarch64" => "-arm64",
        "x86" => "-x86",
        _ => return false,
    };
    name == format!("natives-{os}{suffix}")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeArchive {
    pub artifact: Artifact,
    pub excludes: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedVersion {
    pub id: String,
    /// Empty only in earlier development locks; a new resolution preserves Mojang's type.
    #[serde(default)]
    pub kind: String,
    pub main_class: String,
    pub java: JavaRequirement,
    pub assets: AssetIndex,
    pub client: Artifact,
    pub classpath: Vec<Artifact>,
    pub natives: Vec<NativeArchive>,
    pub logging: Option<Logging>,
    pub arguments: Option<Arguments>,
    pub legacy_arguments: Option<String>,
}

#[cfg(test)]
mod tests;
