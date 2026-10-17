//! Mojang runtime planning. Links are materialized from verified manifest files,
//! avoiding platform symlink privileges and preventing filesystem traversal.
use super::{Artifact, JavaRequirement, Platform, invalid, safe_path};
use crate::error::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeIdentity {
    schema_version: u32,
    manifest: Artifact,
}

pub fn cache_root(java: &JavaRequirement, platform: &Platform) -> Result<String, CoreError> {
    Ok(format!(
        "runtime/java-{}-{}",
        java.major_version,
        platform.runtime_key()?
    ))
}

pub fn valid_cache_root(
    root: &str,
    java: &JavaRequirement,
    platform: &Platform,
) -> Result<bool, CoreError> {
    let base = cache_root(java, platform)?;
    Ok(root == base
        || root
            .strip_prefix(&format!("{base}-build-"))
            .is_some_and(|suffix| {
                suffix
                    .parse::<u32>()
                    .is_ok_and(|number| number >= 2 && number.to_string() == suffix)
            }))
}

/// Reserve the exact build before downloads begin, including interrupted builds.
/// Never infer ownership from a readable directory name or replace another build.
pub fn allocate_cache(
    paths: &crate::paths::AppPaths,
    java: &JavaRequirement,
    platform: &Platform,
    source: &Artifact,
) -> Result<String, CoreError> {
    use crate::storage::{PrivateStore, map_private_error};
    use std::io::Read;
    source.validate()?;
    let base = cache_root(java, platform)?;
    let store = PrivateStore::open(paths)?;
    store.with_lock(|_| {
        let parent = private_fs::PrivateDir::ensure(paths.root().join("runtime"))
            .map_err(map_private_error)?;
        let occupied: Vec<_> = std::fs::read_dir(paths.root().join("runtime"))
            .map_err(super::download::io)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()
            .map_err(super::download::io)?;
        let mut available = None;
        for build in 1..=10_000 {
            let root = if build == 1 {
                base.clone()
            } else {
                format!("{base}-build-{build}")
            };
            let name = root.strip_prefix("runtime/").ok_or_else(invalid)?;
            if occupied
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(name))
            {
                // Case variants, unrelated files, links, and unclaimed directories
                // remain occupied. Only a protected, matching reservation is reusable.
                if !occupied.iter().any(|existing| existing == name) {
                    continue;
                }
                if !std::fs::symlink_metadata(paths.root().join(&root))
                    .map_err(super::download::io)?
                    .is_dir()
                {
                    continue;
                }
                let directory = private_fs::PrivateDir::open(paths.root().join(&root))
                    .map_err(map_private_error)?;
                let mut file = match directory.open_file(".runtime-identity.json") {
                    Ok(file) => file,
                    Err(private_fs::Error::Io(error))
                        if error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        continue;
                    }
                    Err(error) => return Err(map_private_error(error)),
                };
                let mut bytes = Vec::new();
                file.by_ref()
                    .take(64 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(super::download::io)?;
                if bytes.len() <= 64 * 1024
                    && let Ok(identity) = serde_json::from_slice::<RuntimeIdentity>(&bytes)
                    && identity.schema_version == 1
                    && identity.manifest.sha1 == source.sha1
                    && identity.manifest.size == source.size
                {
                    return Ok(root);
                }
                continue;
            }
            if available.is_none() {
                available = Some(root);
            }
        }
        if let Some(root) = available {
            let name = root.strip_prefix("runtime/").ok_or_else(invalid)?;
            let directory = parent.create_dir(name).map_err(map_private_error)?;
            directory
                .write_atomic_durable(
                    ".runtime-identity.json",
                    &serde_json::to_vec(&RuntimeIdentity {
                        schema_version: 1,
                        manifest: source.clone(),
                    })
                    .map_err(|_| invalid())?,
                )
                .map_err(map_private_error)?;
            return Ok(root);
        }
        Err(CoreError::new(ErrorCode::OperationBusy))
    })
}

#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeCandidate {
    pub manifest: Artifact,
    pub version: RuntimeVersion,
}
#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeVersion {
    pub name: String,
}
pub type RuntimeIndex = BTreeMap<String, BTreeMap<String, Vec<RuntimeCandidate>>>;

pub fn select(
    index: &RuntimeIndex,
    platform: &Platform,
    java: &JavaRequirement,
) -> Result<Artifact, CoreError> {
    let candidates = index
        .get(platform.runtime_key()?)
        .and_then(|components| components.get(&java.component))
        .ok_or_else(|| CoreError::new(ErrorCode::RuntimeUnsupported))?;
    for candidate in candidates {
        if candidate
            .version
            .name
            .split('.')
            .next()
            .and_then(|major| major.parse::<u32>().ok())
            == Some(java.major_version)
        {
            candidate.manifest.validate()?;
            return Ok(candidate.manifest.clone());
        }
    }
    Err(CoreError::new(ErrorCode::RuntimeUnsupported))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RuntimeEntry {
    Directory,
    File {
        downloads: BTreeMap<String, Artifact>,
        executable: bool,
    },
    Link {
        target: String,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RuntimeManifest {
    pub files: BTreeMap<String, RuntimeEntry>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RuntimeFile {
    pub path: String,
    pub artifact: Artifact,
    pub executable: bool,
}

impl RuntimeManifest {
    pub fn plan(&self) -> Result<Vec<RuntimeFile>, CoreError> {
        if self.files.is_empty() || self.files.len() > 10_000 {
            return Err(invalid());
        }
        let mut seen = HashSet::new();
        for path in self.files.keys() {
            safe_path(path)?;
            if !seen.insert(path.to_ascii_lowercase()) {
                return Err(invalid());
            }
            let mut parent = path.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                if let Some(entry) = self.files.get(prefix)
                    && !matches!(entry, RuntimeEntry::Directory)
                {
                    return Err(invalid());
                }
                parent = prefix;
            }
        }
        let mut files = Vec::new();
        for (path, entry) in &self.files {
            if matches!(entry, RuntimeEntry::Directory) {
                continue;
            }
            let (artifact, executable) = self.resolve(path, &mut HashSet::new())?;
            artifact.validate()?;
            files.push(RuntimeFile {
                path: path.clone(),
                artifact: artifact.clone(),
                executable,
            });
        }
        Ok(files)
    }

    fn resolve<'a>(
        &'a self,
        path: &str,
        visited: &mut HashSet<String>,
    ) -> Result<(&'a Artifact, bool), CoreError> {
        if visited.len() >= 32 || !visited.insert(path.into()) {
            return Err(invalid());
        }
        match self.files.get(path).ok_or_else(invalid)? {
            RuntimeEntry::File {
                downloads,
                executable,
            } => Ok((downloads.get("raw").ok_or_else(invalid)?, *executable)),
            RuntimeEntry::Directory => Err(invalid()),
            RuntimeEntry::Link { target } => {
                if target.is_empty() || target.starts_with('/') || target.contains(['\\', ':']) {
                    return Err(invalid());
                }
                let mut parts: Vec<_> = path.split('/').collect();
                parts.pop();
                for part in target.split('/') {
                    match part {
                        ".." => {
                            parts.pop().ok_or_else(invalid)?;
                        }
                        "." => {}
                        "" => return Err(invalid()),
                        part => parts.push(part),
                    }
                }
                let target = parts.join("/");
                safe_path(&target)?;
                self.resolve(&target, visited)
            }
        }
    }
}

/// Parse the stable Java property output without persisting or exposing it to the UI.
pub fn verify_properties(
    output: &str,
    requirement: &JavaRequirement,
    platform: &Platform,
) -> Result<(), CoreError> {
    let properties: BTreeMap<_, _> = output
        .lines()
        .filter_map(|line| line.trim().split_once(" = "))
        .collect();
    let major = properties
        .get("java.specification.version")
        .and_then(|value| value.parse::<u32>().ok());
    let arch = properties.get("os.arch").copied();
    let architecture_matches = match (platform.arch.as_str(), arch) {
        ("amd64" | "x86_64", Some("amd64" | "x86_64")) => true,
        (expected, Some(actual)) => expected == actual,
        _ => false,
    };
    if major != Some(requirement.major_version) || !architecture_matches {
        return Err(CoreError::new(ErrorCode::RuntimeUnsupported));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readable_runtime_allocates_conflicts_and_reuses_interrupted_reservations() {
        let paths = crate::paths::AppPaths::new(
            std::env::temp_dir().join(format!("panita-runtime-cache-{}", uuid::Uuid::new_v4())),
        )
        .unwrap_or_else(|_| panic!("paths"));
        let java = JavaRequirement {
            component: "java-runtime-delta".into(),
            major_version: 21,
        };
        let platform = Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform"));
        let source = Artifact {
            url: "https://piston-meta.mojang.com/runtime.json".into(),
            sha1: "a".repeat(40),
            size: 10,
            path: None,
        };
        let first = allocate_cache(&paths, &java, &platform, &source)
            .unwrap_or_else(|error| panic!("first: {error}"));
        assert_eq!(first, "runtime/java-21-windows-x64");
        assert_eq!(
            allocate_cache(&paths, &java, &platform, &source).ok(),
            Some(first.clone())
        );
        let other = Artifact {
            sha1: "b".repeat(40),
            ..source.clone()
        };
        let second =
            allocate_cache(&paths, &java, &platform, &other).unwrap_or_else(|_| panic!("second"));
        assert_eq!(second, "runtime/java-21-windows-x64-build-2");
        let original = std::fs::read(paths.root().join(&first).join(".runtime-identity.json"))
            .unwrap_or_else(|_| panic!("identity"));
        let paths = std::sync::Arc::new(paths);
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let (paths, java, platform, source) =
                    (paths.clone(), java.clone(), platform.clone(), other.clone());
                std::thread::spawn(move || allocate_cache(&paths, &java, &platform, &source))
            })
            .collect();
        for worker in workers {
            assert_eq!(
                worker.join().ok().and_then(Result::ok),
                Some(second.clone())
            );
        }
        assert_eq!(
            std::fs::read(paths.root().join(first).join(".runtime-identity.json")).ok(),
            Some(original)
        );
        for invalid in [
            "runtime/java-21-linux",
            "runtime/java-25-windows-x64",
            "runtime/java-21-windows-x64-build-01",
            "runtime/java-21-windows-x64-build-2/escape",
        ] {
            assert!(!valid_cache_root(invalid, &java, &platform).unwrap_or(true));
        }
    }

    #[test]
    fn runtime_cache_preserves_unowned_and_malformed_directories() {
        let paths = crate::paths::AppPaths::new(
            std::env::temp_dir().join(format!("panita-runtime-cache-{}", uuid::Uuid::new_v4())),
        )
        .unwrap_or_else(|_| panic!("paths"));
        let java = JavaRequirement {
            component: "java-runtime-delta".into(),
            major_version: 21,
        };
        let platform = Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform"));
        crate::storage::PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store"));
        private_fs::PrivateDir::ensure(paths.root().join("runtime"))
            .unwrap_or_else(|_| panic!("runtime"));
        let directory = private_fs::PrivateDir::ensure(
            paths
                .root()
                .join(cache_root(&java, &platform).unwrap_or_default()),
        )
        .unwrap_or_else(|_| panic!("directory"));
        directory
            .write_atomic_durable("keep.txt", b"unowned")
            .unwrap_or_else(|_| panic!("sentinel"));
        let source = Artifact {
            url: "https://piston-meta.mojang.com/runtime.json".into(),
            sha1: "a".repeat(40),
            size: 10,
            path: None,
        };
        assert_eq!(
            allocate_cache(&paths, &java, &platform, &source)
                .ok()
                .as_deref(),
            Some("runtime/java-21-windows-x64-build-2")
        );
        directory
            .write_atomic_durable(".runtime-identity.json", b"{broken")
            .unwrap_or_else(|_| panic!("corrupt"));
        assert_eq!(
            allocate_cache(&paths, &java, &platform, &source)
                .ok()
                .as_deref(),
            Some("runtime/java-21-windows-x64-build-2")
        );
        assert_eq!(
            std::fs::read(paths.root().join("runtime/java-21-windows-x64/keep.txt"))
                .ok()
                .as_deref(),
            Some(b"unowned".as_slice())
        );
    }
    #[test]
    fn all_four_official_runtime_manifests_resolve_links_inside_the_manifest() {
        for (bytes, hash) in [
            (
                include_bytes!("fixtures/windows-x64-java-runtime-epsilon.json").as_slice(),
                "b374544c680d965fb5535977d7cb04c6befe1930",
            ),
            (
                include_bytes!("fixtures/windows-x64-java-runtime-delta.json").as_slice(),
                "cb4394a27089d19f65d5baa6cf0482c27c3c7865",
            ),
            (
                include_bytes!("fixtures/linux-java-runtime-epsilon.json").as_slice(),
                "ccf325ef8f5be596cf8229fd349298d8a74d2388",
            ),
            (
                include_bytes!("fixtures/linux-java-runtime-delta.json").as_slice(),
                "06a3884df3d9d4072b9b6df541495d81878ad2de",
            ),
        ] {
            Artifact {
                url: format!("https://piston-meta.mojang.com/v1/packages/{hash}/manifest.json"),
                size: bytes.len() as u64,
                sha1: hash.into(),
                path: None,
            }
            .verify(bytes)
            .unwrap_or_else(|error| panic!("hash: {error}"));
            let manifest: RuntimeManifest =
                serde_json::from_slice(bytes).unwrap_or_else(|_| panic!("manifest"));
            let files = manifest
                .plan()
                .unwrap_or_else(|error| panic!("plan: {error}"));
            assert!(files.len() > 200);
            assert!(
                files
                    .iter()
                    .any(|file| file.path.starts_with("bin/java") && file.executable)
            );
        }
    }
    #[test]
    fn runtime_links_reject_escapes_cycles_directories_and_case_aliases() {
        for value in [
            r#"{"files":{"a":{"type":"link","target":"../outside"}}}"#,
            r#"{"files":{"a":{"type":"link","target":"b"},"b":{"type":"link","target":"a"}}}"#,
            r#"{"files":{"a":{"type":"link","target":"b"},"b":{"type":"directory"}}}"#,
            r#"{"files":{"a":{"type":"directory"},"A":{"type":"directory"}}}"#,
        ] {
            let manifest: RuntimeManifest =
                serde_json::from_str(value).unwrap_or_else(|_| panic!("manifest"));
            assert!(manifest.plan().is_err());
        }
    }
    #[test]
    fn java_properties_require_both_major_and_architecture() {
        let java = JavaRequirement {
            component: "component".into(),
            major_version: 21,
        };
        let platform = Platform::x64("windows", "10").unwrap_or_else(|_| panic!("platform"));
        assert!(
            verify_properties(
                "java.specification.version = 21\nos.arch = amd64",
                &java,
                &platform
            )
            .is_ok()
        );
        assert!(
            verify_properties(
                "java.specification.version = 25\nos.arch = amd64",
                &java,
                &platform
            )
            .is_err()
        );
        assert!(
            verify_properties(
                "java.specification.version = 21\nos.arch = aarch64",
                &java,
                &platform
            )
            .is_err()
        );
    }
}
