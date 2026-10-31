//! The exact loader embedded in an installed plan. Installer loaders (NeoForge)
//! run the official processors; profile loaders (Fabric, Quilt) only publish a
//! verified library set. Both are bound to a completion marker and audited the
//! same way before every launch.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{CoreError, ErrorCode},
    minecraft::{
        JavaRequirement, Platform,
        download::Cancellation,
        install::{InstalledLock, Progress, prepare_recorded},
        plan::InstallPlan,
    },
    neoforge::generation::{GenerationFile, GenerationRef},
    paths::AppPaths,
};

pub mod profile;

pub use profile::{Flavor, ProfileRef};

/// A loader the launcher can install, named as in an mrpack's dependencies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    NeoForge,
    Fabric,
    Quilt,
}

impl Kind {
    /// The loader name recorded in catalog entries and installed pack state.
    pub fn name(self) -> &'static str {
        match self {
            Self::NeoForge => "neoforge",
            Self::Fabric => "fabric",
            Self::Quilt => "quilt",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "neoforge" => Some(Self::NeoForge),
            "fabric" => Some(Self::Fabric),
            "quilt" => Some(Self::Quilt),
            _ => None,
        }
    }

    /// The mrpack dependency key that pins this loader's version.
    pub fn dependency_key(self) -> &'static str {
        match self {
            Self::NeoForge => "neoforge",
            Self::Fabric => "fabric-loader",
            Self::Quilt => "quilt-loader",
        }
    }

    pub fn from_dependency(key: &str) -> Option<Self> {
        [Self::NeoForge, Self::Fabric, Self::Quilt]
            .into_iter()
            .find(|kind| kind.dependency_key() == key)
    }
}

/// True when a catalog or pack loader name is one this launcher installs.
pub fn supported(name: &str) -> bool {
    Kind::from_name(name).is_some()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LoaderRef {
    Installer(GenerationRef),
    Profile(ProfileRef),
}

impl From<GenerationRef> for LoaderRef {
    fn from(value: GenerationRef) -> Self {
        Self::Installer(value)
    }
}

impl From<ProfileRef> for LoaderRef {
    fn from(value: ProfileRef) -> Self {
        Self::Profile(value)
    }
}

impl LoaderRef {
    pub fn kind(&self) -> &str {
        match self {
            Self::Installer(reference) => reference.kind(),
            Self::Profile(reference) => reference.kind(),
        }
    }

    pub fn minecraft(&self) -> &str {
        match self {
            Self::Installer(reference) => reference.minecraft(),
            Self::Profile(reference) => reference.minecraft(),
        }
    }

    pub fn version(&self) -> &str {
        match self {
            Self::Installer(reference) => reference.version(),
            Self::Profile(reference) => reference.version(),
        }
    }

    pub fn directory(&self) -> String {
        match self {
            Self::Installer(reference) => reference.directory(),
            Self::Profile(reference) => reference.directory(),
        }
    }

    pub fn java(&self) -> &JavaRequirement {
        match self {
            Self::Installer(reference) => reference.java(),
            Self::Profile(reference) => reference.java(),
        }
    }

    pub fn files(&self) -> &[GenerationFile] {
        match self {
            Self::Installer(reference) => reference.files(),
            Self::Profile(reference) => reference.files(),
        }
    }

    /// The identifier Minecraft reports as the running version.
    pub fn profile_id(&self) -> &str {
        match self {
            Self::Installer(reference) => &reference.profile().id,
            Self::Profile(reference) => reference.profile_id(),
        }
    }

    pub fn main_class(&self) -> &str {
        match self {
            Self::Installer(reference) => &reference.profile().main_class,
            Self::Profile(reference) => reference.main_class(),
        }
    }

    pub fn jvm_arguments(&self) -> &[String] {
        match self {
            Self::Installer(reference) => &reference.profile().arguments.jvm,
            Self::Profile(reference) => reference.jvm_arguments(),
        }
    }

    pub fn game_arguments(&self) -> &[String] {
        match self {
            Self::Installer(reference) => &reference.profile().arguments.game,
            Self::Profile(reference) => reference.game_arguments(),
        }
    }

    /// The directory `${library_directory}` names for this loader's launch.
    pub fn library_directory(&self, paths: &AppPaths) -> PathBuf {
        match self {
            Self::Installer(_) => paths.root().join(self.directory()).join("libraries"),
            Self::Profile(_) => paths.root().join("libraries"),
        }
    }

    pub fn validate(&self, minecraft: &str) -> Result<(), CoreError> {
        match self {
            Self::Installer(reference) => reference.validate(minecraft),
            Self::Profile(reference) => reference.validate(minecraft),
        }
    }

    pub fn verify_marker(&self, paths: &AppPaths) -> Result<(), CoreError> {
        match self {
            Self::Installer(reference) => reference.verify_marker(paths),
            Self::Profile(reference) => reference.verify_marker(paths),
        }
    }

    /// Full audit for install, repair and recovery. Never downloads or executes.
    pub fn verify(&self, paths: &AppPaths, cancel: &Cancellation) -> Result<(), CoreError> {
        match self {
            Self::Installer(reference) => reference.verify(paths, cancel),
            Self::Profile(reference) => reference.verify(paths, cancel),
        }
    }

    /// Explicit repair only: reproduce the exact accepted bytes in a fresh generation.
    pub async fn repair(
        &self,
        paths: &AppPaths,
        base: &InstallPlan,
        cancel: &Cancellation,
    ) -> Result<Self, CoreError> {
        match self {
            Self::Installer(reference) => reference
                .repair(paths, base, cancel)
                .await
                .map(Self::Installer),
            Self::Profile(reference) => reference.repair(paths, cancel).await.map(Self::Profile),
        }
    }

    pub(crate) fn classpath(
        &self,
        paths: &AppPaths,
        base: &InstallPlan,
    ) -> Result<Vec<PathBuf>, CoreError> {
        match self {
            Self::Installer(reference) => reference.classpath(paths, base),
            Self::Profile(reference) => reference.classpath(paths, base),
        }
    }
}

/// Prepare everything except the instance pointer: game, Java, loader and natives.
#[allow(clippy::too_many_arguments)]
pub async fn prepare_base(
    paths: &AppPaths,
    id: Uuid,
    operation: Uuid,
    platform: &Platform,
    kind: Kind,
    minecraft: &str,
    loader_version: &str,
    cancel: &Cancellation,
    progress: Progress,
) -> Result<InstalledLock, CoreError> {
    match kind {
        Kind::NeoForge => {
            crate::neoforge::loader::prepare_base(
                paths,
                id,
                operation,
                platform,
                minecraft,
                loader_version,
                cancel,
                progress,
            )
            .await
        }
        Kind::Fabric | Kind::Quilt => {
            let flavor = if kind == Kind::Fabric {
                Flavor::Fabric
            } else {
                Flavor::Quilt
            };
            let maven = crate::neoforge::loader::Maven::new()?;
            let plan = profile::prepare_plan(
                paths,
                platform,
                flavor,
                minecraft,
                loader_version,
                cancel,
                &maven,
            )
            .await?;
            prepare_recorded(paths, id, operation, platform, plan, cancel, progress).await
        }
    }
}

pub(crate) fn mismatch() -> CoreError {
    CoreError::new(ErrorCode::ArtifactMismatch)
}

#[cfg(test)]
mod tests;
