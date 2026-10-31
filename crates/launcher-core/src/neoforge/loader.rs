//! Production loader preparation. The official installer's identity comes from the
//! Maven SHA-1 sidecar, compared against the bytes actually downloaded; the exact
//! loader generation is then planned, staged and published without touching any
//! instance. This is the same pipeline the internal validation harness audits.
use std::time::Duration;

use sha1::{Digest, Sha1};
use uuid::Uuid;

use super::{
    Coordinate, InstallerIdentity, MAVEN_ROOT, MAX_INSTALLER_BYTES, component, invalid,
    plan::ProcessorPlan,
    staging::{self, StagingProgress},
};
use crate::{
    error::{CoreError, ErrorCode},
    minecraft::{
        Platform,
        download::{Cancellation, Downloader},
        install::{InstalledLock, Progress, prepare_recorded},
        plan,
        plan::InstallPlan,
        valid_hash,
    },
    paths::AppPaths,
};

const MAX_SIDECAR: usize = 256;

pub struct Maven {
    client: reqwest::Client,
    #[cfg(test)]
    base: Option<String>,
}

impl Maven {
    pub fn new() -> Result<Self, CoreError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("PanitaClient/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600));
        #[cfg(test)]
        let client = client.no_proxy();
        Ok(Self {
            client: client.build().map_err(|_| invalid())?,
            #[cfg(test)]
            base: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_tests(base: &str) -> Result<Self, CoreError> {
        let mut maven = Self::new()?;
        maven.base = Some(base.to_owned());
        Ok(maven)
    }

    #[cfg(test)]
    fn target(&self, url: &str) -> String {
        match &self.base {
            Some(base) => match url.strip_prefix(MAVEN_ROOT) {
                Some(path) => format!("{base}/{path}"),
                // Other hosts map onto the same loopback server by their path.
                None => match url::Url::parse(url) {
                    Ok(parsed) => format!("{base}{}", parsed.path()),
                    Err(_) => url.to_owned(),
                },
            },
            None => url.to_owned(),
        }
    }

    #[cfg(not(test))]
    fn target(&self, url: &str) -> String {
        url.to_owned()
    }

    pub(crate) async fn get(
        &self,
        url: &str,
        limit: usize,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, CoreError> {
        let mut last = CoreError::new(ErrorCode::NetworkUnavailable);
        for attempt in 0..3u32 {
            cancel.check()?;
            let sent = tokio::select! {
                _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
                response = self.client.get(self.target(url)).send() => response,
            };
            let Ok(mut response) = sent else {
                tokio::time::sleep(Duration::from_millis(250 << attempt)).await;
                continue;
            };
            let status = response.status();
            if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                last = CoreError::new(ErrorCode::ServiceUnavailable);
                tokio::time::sleep(Duration::from_millis(250 << attempt)).await;
                continue;
            }
            if status != reqwest::StatusCode::OK {
                return Err(CoreError::new(ErrorCode::ServiceUnavailable));
            }
            if response.content_length().is_some_and(|n| n > limit as u64) {
                return Err(invalid());
            }
            let mut bytes = Vec::new();
            loop {
                let chunk = tokio::select! {
                    _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
                    chunk = response.chunk() => chunk,
                };
                match chunk {
                    Ok(Some(chunk)) => {
                        if bytes.len() + chunk.len() > limit {
                            return Err(invalid());
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    Ok(None) => return Ok(bytes),
                    Err(_) => break,
                }
            }
        }
        Err(last)
    }

    /// Download the official installer and bind it to its published SHA-1.
    pub async fn installer(
        &self,
        minecraft: &str,
        version: &str,
        cancel: &Cancellation,
    ) -> Result<(InstallerIdentity, Vec<u8>), CoreError> {
        if !component(minecraft) || !component(version) {
            return Err(invalid());
        }
        let coordinate = Coordinate::parse(&format!("net.neoforged:neoforge:{version}:installer"))?;
        let url = format!("{MAVEN_ROOT}{}", coordinate.path());
        let sidecar = self
            .get(&format!("{url}.sha1"), MAX_SIDECAR, cancel)
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
        let bytes = self.get(&url, MAX_INSTALLER_BYTES, cancel).await?;
        if format!("{:x}", Sha1::digest(&bytes)) != published {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        let identity = InstallerIdentity {
            minecraft: minecraft.to_owned(),
            version: version.to_owned(),
            sha1: published,
            size: bytes.len() as u64,
        };
        identity.url()?;
        Ok((identity, bytes))
    }
}

/// Resolve the vanilla plan and publish the exact loader generation it needs.
pub async fn prepare_plan(
    paths: &AppPaths,
    platform: &Platform,
    minecraft: &str,
    loader_version: &str,
    cancel: &Cancellation,
    maven: &Maven,
) -> Result<InstallPlan, CoreError> {
    let (identity, installer) = maven.installer(minecraft, loader_version, cancel).await?;
    let downloader = Downloader::new()?;
    let mut base = plan::resolve(&downloader, minecraft, platform, cancel).await?;
    base.allocate_runtime(paths)?;
    let version = base
        .documents
        .iter()
        .find(|document| document.source.url.ends_with(&format!("/{minecraft}.json")))
        .ok_or_else(invalid)?;
    let game = downloader.document(&version.source.url, cancel).await?;
    let processors = ProcessorPlan::resolve(&installer, &identity, &game, &version.source)?;
    let staged = staging::prepare(
        paths,
        &processors,
        &installer,
        &base,
        cancel,
        &|event: StagingProgress| {
            let _ = event;
        },
    )
    .await?;
    let loader = super::generation::publish(paths, &processors, staged, cancel)?;
    base.schema_version = 2;
    base.loader = Some(loader.into());
    base.validate()?;
    Ok(base)
}

/// Prepare everything except the instance pointer: game, Java, loader and natives.
#[allow(clippy::too_many_arguments)]
pub async fn prepare_base(
    paths: &AppPaths,
    id: Uuid,
    operation: Uuid,
    platform: &Platform,
    minecraft: &str,
    loader_version: &str,
    cancel: &Cancellation,
    progress: Progress,
) -> Result<InstalledLock, CoreError> {
    let maven = Maven::new()?;
    let plan = prepare_plan(paths, platform, minecraft, loader_version, cancel, &maven).await?;
    prepare_recorded(paths, id, operation, platform, plan, cancel, progress).await
}

#[cfg(test)]
mod tests;
