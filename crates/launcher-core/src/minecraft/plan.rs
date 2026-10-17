//! Resolve an immutable, reproducible plan before mutating an instance.
use super::download::{Cancellation, Downloader};
use super::runtime::{RuntimeFile, RuntimeIndex, RuntimeManifest};
use super::{
    Artifact, Platform, RUNTIME_INDEX, ResolvedVersion, VERSION_MANIFEST, VersionMetadata, invalid,
    safe_path, valid_hash,
};
use crate::error::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashSet};

#[derive(Deserialize)]
struct VersionManifest {
    versions: Vec<VersionEntry>,
}
#[derive(Deserialize)]
struct VersionEntry {
    id: String,
    url: String,
    sha1: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlannedArtifact {
    pub destination: String,
    pub artifact: Artifact,
    pub executable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentIdentity {
    pub source: Artifact,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallPlan {
    pub schema_version: u32,
    pub version: ResolvedVersion,
    pub platform: String,
    pub architecture: String,
    pub runtime_root: String,
    pub java_executable: String,
    /// Includes the two TLS bootstrap index fingerprints and all resolved metadata.
    pub documents: Vec<DocumentIdentity>,
    pub artifacts: Vec<PlannedArtifact>,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}
#[derive(Deserialize)]
struct Assets {
    objects: BTreeMap<String, AssetObject>,
    #[serde(default)]
    r#virtual: bool,
    #[serde(default)]
    map_to_resources: bool,
}

fn identity(url: &str, bytes: &[u8]) -> Artifact {
    Artifact {
        url: url.into(),
        size: bytes.len() as u64,
        sha1: format!("{:x}", Sha1::digest(bytes)),
        path: None,
    }
}

pub async fn resolve(
    downloader: &Downloader,
    id: &str,
    platform: &Platform,
    cancel: &Cancellation,
) -> Result<InstallPlan, CoreError> {
    safe_path(id)?;
    if id.contains('/') {
        return Err(invalid());
    }
    platform.runtime_key()?;
    let index = downloader.document(VERSION_MANIFEST, cancel).await?;
    let mut documents = vec![DocumentIdentity {
        source: identity(VERSION_MANIFEST, &index),
    }];
    let index: VersionManifest = serde_json::from_slice(&index).map_err(|_| invalid())?;
    if index.versions.len() > 10_000 {
        return Err(invalid());
    }
    let mut chain = Vec::new();
    let mut requested = id.to_owned();
    let mut seen = HashSet::new();
    loop {
        if chain.len() >= 16 || !seen.insert(requested.clone()) {
            return Err(invalid());
        }
        let mut entries = index.versions.iter().filter(|entry| entry.id == requested);
        let entry = entries
            .next()
            .ok_or_else(|| CoreError::new(ErrorCode::UnsupportedInstance))?;
        if entries.next().is_some() || !valid_hash(&entry.sha1) {
            return Err(invalid());
        }
        let bytes = downloader.document(&entry.url, cancel).await?;
        let source = Artifact {
            url: entry.url.clone(),
            sha1: entry.sha1.clone(),
            size: bytes.len() as u64,
            path: None,
        };
        let metadata = VersionMetadata::decode(&bytes, &source, &requested)?;
        documents.push(DocumentIdentity { source });
        let parent = metadata.inherits_from.clone();
        chain.push(metadata);
        match parent {
            Some(parent) => requested = parent,
            None => break,
        }
    }
    let mut metadata = chain.pop().ok_or_else(invalid)?;
    while let Some(child) = chain.pop() {
        metadata = child.inherit(metadata)?;
    }
    let version = metadata.resolve(platform)?;
    let runtime_index = downloader.document(RUNTIME_INDEX, cancel).await?;
    documents.push(DocumentIdentity {
        source: identity(RUNTIME_INDEX, &runtime_index),
    });
    let runtime_index: RuntimeIndex =
        serde_json::from_slice(&runtime_index).map_err(|_| invalid())?;
    let runtime_source = super::runtime::select(&runtime_index, platform, &version.java)?;
    let bytes = downloader.document(&runtime_source.url, cancel).await?;
    runtime_source.verify(&bytes)?;
    let runtime: RuntimeManifest = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let runtime_files = runtime.plan()?;
    let runtime_root = format!(
        "runtime/{}/{}/{}",
        version.java.component,
        platform.runtime_key()?,
        runtime_source.sha1
    );
    documents.push(DocumentIdentity {
        source: runtime_source.clone(),
    });
    let java_name = if platform.os == "windows" {
        "bin/java.exe"
    } else {
        "bin/java"
    };
    if !runtime_files
        .iter()
        .any(|file| file.path == java_name && file.executable)
    {
        return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
    }
    let java_executable = format!("{runtime_root}/{java_name}");
    let mut artifacts = Vec::new();
    artifacts.push(PlannedArtifact {
        destination: format!("{runtime_root}/.runtime-manifest.json"),
        artifact: runtime_source,
        executable: false,
    });
    for RuntimeFile {
        path,
        artifact,
        executable,
    } in runtime_files
    {
        artifacts.push(PlannedArtifact {
            destination: format!("{runtime_root}/{path}"),
            artifact,
            executable,
        });
    }
    artifacts.push(PlannedArtifact {
        destination: format!("versions/{}/{}/client.jar", version.id, version.client.sha1),
        artifact: version.client.clone(),
        executable: false,
    });
    for artifact in version
        .classpath
        .iter()
        .chain(version.natives.iter().map(|native| &native.artifact))
    {
        artifacts.push(PlannedArtifact {
            destination: format!("libraries/{}", artifact.path.as_ref().ok_or_else(invalid)?),
            artifact: artifact.clone(),
            executable: false,
        });
    }
    let asset_bytes = downloader
        .document(&version.assets.artifact.url, cancel)
        .await?;
    version.assets.artifact.verify(&asset_bytes)?;
    documents.push(DocumentIdentity {
        source: version.assets.artifact.clone(),
    });
    let assets: Assets = serde_json::from_slice(&asset_bytes).map_err(|_| invalid())?;
    // These historical mutable asset layouts are outside the qualified releases.
    if assets.r#virtual || assets.map_to_resources {
        return Err(CoreError::new(ErrorCode::UnsupportedInstance));
    }
    if assets.objects.len() > 100_000 {
        return Err(invalid());
    }
    let mut object_size = 0u64;
    for (path, object) in assets.objects {
        safe_path(&path)?;
        if !valid_hash(&object.hash) || object.size == 0 || object.size > 512 * 1024 * 1024 {
            return Err(invalid());
        }
        object_size = object_size.checked_add(object.size).ok_or_else(invalid)?;
        let relative = format!("{}/{}", &object.hash[..2], object.hash);
        artifacts.push(PlannedArtifact {
            destination: format!("assets/objects/{relative}"),
            artifact: Artifact {
                url: format!("https://resources.download.minecraft.net/{relative}"),
                sha1: object.hash,
                size: object.size,
                path: None,
            },
            executable: false,
        });
    }
    if object_size > 16 * 1024 * 1024 * 1024 {
        return Err(invalid());
    }
    artifacts.push(PlannedArtifact {
        destination: format!("assets/indexes/{}.json", version.assets.id),
        artifact: version.assets.artifact.clone(),
        executable: false,
    });
    if let Some(logging) = &version.logging {
        artifacts.push(PlannedArtifact {
            destination: format!(
                "assets/log_configs/{}-{}",
                logging.file.artifact.sha1, logging.file.id
            ),
            artifact: logging.file.artifact.clone(),
            executable: false,
        });
    }
    let mut plan = InstallPlan {
        schema_version: 1,
        version,
        platform: platform.os.clone(),
        architecture: platform.arch.clone(),
        runtime_root,
        java_executable,
        documents,
        artifacts,
    };
    plan.deduplicate()?;
    plan.validate()?;
    Ok(plan)
}

impl InstallPlan {
    fn deduplicate(&mut self) -> Result<(), CoreError> {
        let mut paths: BTreeMap<String, (String, String, u64, bool)> = BTreeMap::new();
        let mut deduplicated = Vec::new();
        for file in self.artifacts.drain(..) {
            let identity = (
                file.destination.clone(),
                file.artifact.sha1.clone(),
                file.artifact.size,
                file.executable,
            );
            match paths.insert(file.destination.to_ascii_lowercase(), identity.clone()) {
                Some(previous) if previous != identity => return Err(invalid()),
                Some(_) => {}
                None => deduplicated.push(file),
            }
        }
        self.artifacts = deduplicated;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version != 1 || self.artifacts.len() > 100_000 || self.documents.len() > 20 {
            return Err(invalid());
        }
        safe_path(&self.java_executable)?;
        safe_path(&self.runtime_root)?;
        let platform = Platform {
            os: self.platform.clone(),
            arch: self.architecture.clone(),
            os_version: String::new(),
            features: BTreeMap::new(),
        };
        let parts: Vec<_> = self.runtime_root.split('/').collect();
        if parts.len() != 4
            || parts[0] != "runtime"
            || parts[1] != self.version.java.component
            || parts[2] != platform.runtime_key()?
            || !valid_hash(parts[3])
        {
            return Err(invalid());
        }
        let java_name = if self.platform == "windows" {
            "bin/java.exe"
        } else {
            "bin/java"
        };
        if self.java_executable != format!("{}/{java_name}", self.runtime_root) {
            return Err(invalid());
        }
        let mut paths = HashSet::new();
        for file in &self.artifacts {
            safe_path(&file.destination)?;
            if !file.destination.starts_with("libraries/")
                && !file.destination.starts_with("assets/")
                && !file.destination.starts_with("versions/")
                && !file.destination.starts_with("runtime/")
            {
                return Err(invalid());
            }
            if !paths.insert(file.destination.to_ascii_lowercase()) {
                return Err(invalid());
            }
            file.artifact.validate()?;
            if file.executable
                && !file
                    .destination
                    .starts_with(&format!("{}/", self.runtime_root))
            {
                return Err(invalid());
            }
        }
        if !self
            .artifacts
            .iter()
            .any(|file| file.destination == self.java_executable && file.executable)
        {
            return Err(invalid());
        }
        for document in &self.documents {
            document.source.validate()?;
        }
        let planned: BTreeMap<_, _> = self
            .artifacts
            .iter()
            .map(|file| (file.destination.as_str(), &file.artifact))
            .collect();
        require(
            &planned,
            &format!(
                "versions/{}/{}/client.jar",
                self.version.id, self.version.client.sha1
            ),
            &self.version.client,
        )?;
        require(
            &planned,
            &format!("assets/indexes/{}.json", self.version.assets.id),
            &self.version.assets.artifact,
        )?;
        for artifact in self
            .version
            .classpath
            .iter()
            .chain(self.version.natives.iter().map(|native| &native.artifact))
        {
            require(
                &planned,
                &format!("libraries/{}", artifact.path.as_ref().ok_or_else(invalid)?),
                artifact,
            )?;
        }
        if let Some(logging) = &self.version.logging {
            require(
                &planned,
                &format!(
                    "assets/log_configs/{}-{}",
                    logging.file.artifact.sha1, logging.file.id
                ),
                &logging.file.artifact,
            )?;
        }
        Ok(())
    }

    /// Verify completeness against the authenticated indexes, rather than trusting
    /// a local ownership list that may have lost one of its entries.
    pub fn verify_dependencies(&self, paths: &crate::paths::AppPaths) -> Result<(), CoreError> {
        let planned: BTreeMap<_, _> = self
            .artifacts
            .iter()
            .map(|file| (file.destination.as_str(), &file.artifact))
            .collect();
        let runtime_path = format!("{}/.runtime-manifest.json", self.runtime_root);
        let runtime_source = planned.get(runtime_path.as_str()).ok_or_else(invalid)?;
        if !self.runtime_root.ends_with(&runtime_source.sha1)
            || !self
                .documents
                .iter()
                .any(|document| &document.source == *runtime_source)
        {
            return Err(invalid());
        }
        let runtime: RuntimeManifest =
            serde_json::from_slice(&read_document(paths, &runtime_path, runtime_source)?)
                .map_err(|_| invalid())?;
        for file in runtime.plan()? {
            require(
                &planned,
                &format!("{}/{}", self.runtime_root, file.path),
                &file.artifact,
            )?;
        }
        let asset_path = format!("assets/indexes/{}.json", self.version.assets.id);
        let assets: Assets = serde_json::from_slice(&read_document(
            paths,
            &asset_path,
            &self.version.assets.artifact,
        )?)
        .map_err(|_| invalid())?;
        if assets.r#virtual || assets.map_to_resources || assets.objects.len() > 100_000 {
            return Err(invalid());
        }
        for object in assets.objects.values() {
            if !valid_hash(&object.hash) {
                return Err(invalid());
            }
            let relative = format!("{}/{}", &object.hash[..2], object.hash);
            require(
                &planned,
                &format!("assets/objects/{relative}"),
                &Artifact {
                    url: format!("https://resources.download.minecraft.net/{relative}"),
                    sha1: object.hash.clone(),
                    size: object.size,
                    path: None,
                },
            )?;
        }
        Ok(())
    }
}

fn require(
    planned: &BTreeMap<&str, &Artifact>,
    path: &str,
    expected: &Artifact,
) -> Result<(), CoreError> {
    expected.validate()?;
    if planned
        .get(path)
        .is_none_or(|artifact| *artifact != expected)
    {
        return Err(invalid());
    }
    Ok(())
}

fn read_document(
    paths: &crate::paths::AppPaths,
    path: &str,
    artifact: &Artifact,
) -> Result<Vec<u8>, CoreError> {
    use std::io::Read;
    let (directory, leaf) = super::download::ensure_parent(paths.root(), path)?;
    let mut file = directory
        .open_file(leaf)
        .map_err(crate::storage::map_private_error)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(super::download::io)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(invalid());
    }
    artifact.verify(&bytes)?;
    Ok(bytes)
}
