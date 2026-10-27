//! Loader-only transport; shares hash-addressed blobs and parent locks with vanilla.
use super::*;
use crate::minecraft::{
    Artifact,
    download::{Cancellation, Downloader, ensure_parent, io, lock, verified_file},
};
use crate::{paths::AppPaths, storage::map_private_error};
use std::{io::Write, path::PathBuf, time::Duration};

pub(super) struct Transport {
    client: reqwest::Client,
    mojang: Downloader,
    #[cfg(test)]
    endpoint: Option<String>,
}

impl Transport {
    pub fn new() -> Result<Self, CoreError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent(concat!("PanitaClient/", env!("CARGO_PKG_VERSION")))
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .read_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(600))
                .build()
                .map_err(|_| invalid())?,
            mojang: Downloader::new()?,
            #[cfg(test)]
            endpoint: None,
        })
    }

    pub async fn mojang(
        &self,
        paths: &AppPaths,
        source: &Artifact,
        cancel: &Cancellation,
    ) -> Result<PathBuf, CoreError> {
        source.validate()?;
        let destination = blob_path(&source.sha1);
        self.mojang
            .cache(paths, &destination, source, cancel, &|_| {})
            .await
    }

    pub async fn library(
        &self,
        paths: &AppPaths,
        source: &LibraryArtifact,
        cancel: &Cancellation,
    ) -> Result<PathBuf, CoreError> {
        library_url(&source.url, &source.path)?;
        if !valid_hash(&source.sha1) || source.size == 0 || source.size > 2 * 1024 * 1024 * 1024 {
            return Err(invalid());
        }
        let destination = blob_path(&source.sha1);
        let parent = destination.rsplit_once('/').ok_or_else(invalid)?.0;
        // Match vanilla's parent lock exactly; concurrent source types cannot race.
        let key = format!("{:x}", Sha1::digest(parent.to_ascii_lowercase().as_bytes()));
        let _guard = lock(paths.root(), &format!(".artifact-locks/{key}.lock"), cancel).await?;
        let (directory, name) = ensure_parent(paths.root(), &destination)?;
        let fingerprint = fingerprint(source);
        cancel.check()?;
        if verified_file(&directory, &name, &fingerprint)? {
            return Ok(paths.root().join(destination));
        }
        let url = source.url.as_str();
        #[cfg(test)]
        let url = self.endpoint.as_deref().unwrap_or(url);
        let mut response = tokio::select! {
            _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
            response = self.client.get(url).send() => response.map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?,
        };
        if response.status() != reqwest::StatusCode::OK {
            return Err(CoreError::new(ErrorCode::ServiceUnavailable));
        }
        if response
            .content_length()
            .is_some_and(|size| size != source.size)
        {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        let mut output = directory.atomic_durable(&name).map_err(map_private_error)?;
        let mut hash = Sha1::new();
        let mut count = 0u64;
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
                chunk = response.chunk() => chunk.map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            count = count.checked_add(chunk.len() as u64).ok_or_else(invalid)?;
            if count > source.size {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
            hash.update(&chunk);
            output.write_all(&chunk).map_err(io)?;
        }
        if count != source.size || format!("{:x}", hash.finalize()) != source.sha1 {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        cancel.commit(|| output.commit().map_err(map_private_error))?;
        Ok(paths.root().join(destination))
    }
}

pub(super) fn blob_path(hash: &str) -> String {
    format!("libraries/{}/{}.jar", &hash[..2], hash)
}

pub(super) fn fingerprint(source: &LibraryArtifact) -> Artifact {
    // This is a hash/size adapter for the shared file verifier only. Transport
    // authorization remains loader-specific and does not invoke Artifact::validate.
    Artifact {
        url: source.url.clone(),
        sha1: source.sha1.clone(),
        size: source.size,
        path: Some(source.path.clone()),
    }
}

#[cfg(test)]
mod tests;
