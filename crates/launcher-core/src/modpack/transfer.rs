//! Verified public pack-file transport. It sends no credentials, validates every
//! URL by source before connecting, follows only bounded GitHub release redirects
//! and publishes nothing: the caller commits a sink only after every check passed.
use std::io::Write;
use std::time::Duration;

use reqwest::{StatusCode, header};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

use super::{
    Hashes, MAX_ARCHIVE_BYTES, MAX_FILE_BYTES, MAX_REDIRECTS, Origin, download_url,
    github_redirect, hex, invalid,
};
use crate::{
    error::{CoreError, ErrorCode},
    minecraft::download::Cancellation,
};

const TRANSIENT_ATTEMPTS: usize = 3;
const INTEGRITY_ATTEMPTS: usize = 2;

/// Exact size and every hash the source supplied; all of them must match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    pub size: u64,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub sha512: Option<String>,
}

impl Expected {
    pub fn file(hashes: &Hashes, size: u64) -> Self {
        Self {
            size,
            sha1: hashes.sha1.clone(),
            sha256: None,
            sha512: hashes.sha512.clone(),
        }
    }

    pub fn archive(sha256: &str, size: u64) -> Self {
        Self {
            size,
            sha1: None,
            sha256: Some(sha256.to_owned()),
            sha512: None,
        }
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        let limit = if self.sha256.is_some() {
            MAX_ARCHIVE_BYTES
        } else {
            MAX_FILE_BYTES
        };
        if (self.sha1.is_none() && self.sha256.is_none() && self.sha512.is_none())
            || self.size > limit
            || self.sha1.as_deref().is_some_and(|v| !hex(v, 40))
            || self.sha256.as_deref().is_some_and(|v| !hex(v, 64))
            || self.sha512.as_deref().is_some_and(|v| !hex(v, 128))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

/// Incremental hashing of exactly the algorithms an `Expected` supplies.
pub(crate) struct Hashers {
    sha1: Option<Sha1>,
    sha256: Option<Sha256>,
    sha512: Option<Sha512>,
    count: u64,
}

impl Hashers {
    pub(crate) fn new(expected: &Expected) -> Self {
        Self {
            sha1: expected.sha1.as_ref().map(|_| Sha1::new()),
            sha256: expected.sha256.as_ref().map(|_| Sha256::new()),
            sha512: expected.sha512.as_ref().map(|_| Sha512::new()),
            count: 0,
        }
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.count = self.count.saturating_add(bytes.len() as u64);
        if let Some(hash) = &mut self.sha1 {
            hash.update(bytes);
        }
        if let Some(hash) = &mut self.sha256 {
            hash.update(bytes);
        }
        if let Some(hash) = &mut self.sha512 {
            hash.update(bytes);
        }
    }

    pub(crate) fn count(&self) -> u64 {
        self.count
    }

    /// True only when the exact size and every supplied hash match.
    pub(crate) fn matches(self, expected: &Expected) -> bool {
        self.count == expected.size
            && self
                .sha1
                .is_none_or(|hash| Some(format!("{:x}", hash.finalize())) == expected.sha1)
            && self
                .sha256
                .is_none_or(|hash| Some(format!("{:x}", hash.finalize())) == expected.sha256)
            && self
                .sha512
                .is_none_or(|hash| Some(format!("{:x}", hash.finalize())) == expected.sha512)
    }
}

pub enum Fetch<W> {
    /// Every check passed; the caller decides whether to commit the sink.
    Done(W),
    /// A CurseForge CDN reports the author withdrew the file since publication.
    Withdrawn,
}

enum Failure {
    Transient(Option<u32>),
    Integrity,
    Withdrawn,
    Fatal(CoreError),
}

impl From<CoreError> for Failure {
    fn from(error: CoreError) -> Self {
        Self::Fatal(error)
    }
}

#[derive(Clone)]
pub struct PackDownloader {
    client: reqwest::Client,
    retry_unit: Duration,
    backoff: Duration,
    #[cfg(test)]
    test_base: Option<String>,
}

impl PackDownloader {
    pub fn new() -> Result<Self, CoreError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "PanitaClient/",
                env!("CARGO_PKG_VERSION"),
                " (+https://www.panitacraft.com/)"
            ))
            // Redirects are followed manually so each hop is authorized by source.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|_| invalid())?;
        Ok(Self {
            client,
            retry_unit: Duration::from_secs(1),
            backoff: Duration::from_millis(500),
            #[cfg(test)]
            test_base: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_tests(base: &str) -> Self {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| panic!("client"));
        Self {
            client,
            retry_unit: Duration::from_millis(1),
            backoff: Duration::from_millis(1),
            test_base: Some(base.to_owned()),
        }
    }

    #[cfg(test)]
    fn connect_url(&self, url: &url::Url) -> String {
        match &self.test_base {
            Some(base) => format!(
                "{base}{}{}",
                url.path(),
                url.query().map(|q| format!("?{q}")).unwrap_or_default()
            ),
            None => url.as_str().to_owned(),
        }
    }

    #[cfg(not(test))]
    fn connect_url(&self, url: &url::Url) -> String {
        url.as_str().to_owned()
    }

    /// Download one file into the sink that `open_sink` creates for each attempt.
    /// Only a sink whose size and every hash matched is returned; failed attempts
    /// drop theirs so the caller's atomic temporary is never published.
    pub async fn download<W: Write>(
        &self,
        url: &str,
        origin: &Origin,
        expected: &Expected,
        cancel: &Cancellation,
        progress: &(dyn Fn(u64) + Sync),
        mut open_sink: impl FnMut() -> Result<W, CoreError>,
    ) -> Result<Fetch<W>, CoreError> {
        expected.validate()?;
        let initial = download_url(url, origin)?;
        let (mut transient, mut integrity) = (0usize, 0usize);
        loop {
            cancel.check()?;
            progress(0);
            let mut sink = open_sink()?;
            let failure = match self
                .attempt(&initial, origin, expected, cancel, progress, &mut sink)
                .await
            {
                Ok(()) => return Ok(Fetch::Done(sink)),
                Err(failure) => failure,
            };
            drop(sink);
            let delay = match failure {
                Failure::Withdrawn => return Ok(Fetch::Withdrawn),
                Failure::Fatal(error) => return Err(error),
                Failure::Integrity => {
                    integrity += 1;
                    if integrity >= INTEGRITY_ATTEMPTS {
                        return Err(CoreError::new(ErrorCode::ArtifactMismatch));
                    }
                    self.backoff
                }
                Failure::Transient(retry_after) => {
                    transient += 1;
                    if transient >= TRANSIENT_ATTEMPTS {
                        return Err(CoreError::new(ErrorCode::NetworkUnavailable));
                    }
                    match retry_after {
                        Some(seconds) => self.retry_unit * seconds,
                        None => self.backoff * (1 << transient),
                    }
                }
            };
            tokio::select! {
                _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
                _ = tokio::time::sleep(delay) => {},
            }
        }
    }

    async fn open(
        &self,
        initial: &url::Url,
        origin: &Origin,
        cancel: &Cancellation,
    ) -> Result<reqwest::Response, Failure> {
        let mut current = initial.clone();
        let mut hops = 0usize;
        loop {
            let result = tokio::select! {
                _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled).into()),
                response = self.client.get(self.connect_url(&current)).send() => response,
            };
            let Ok(response) = result else {
                return Err(Failure::Transient(None));
            };
            let status = response.status();
            if status == StatusCode::OK {
                return Ok(response);
            }
            if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                // Only GitHub release downloads (to its content hosts) and the
                // CurseForge CDN (to its own file hosts) redirect.
                if !matches!(origin, Origin::Github | Origin::Curseforge) || hops >= MAX_REDIRECTS {
                    return Err(invalid().into());
                }
                hops += 1;
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(invalid)?;
                current = if *origin == Origin::Github {
                    github_redirect(location, hops)?
                } else {
                    download_url(location, origin)?
                };
                continue;
            }
            if *origin == Origin::Curseforge && matches!(status.as_u16(), 403 | 404 | 410) {
                return Err(Failure::Withdrawn);
            }
            if status == StatusCode::REQUEST_TIMEOUT
                || status == StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
            {
                let retry_after = crate::panita_api::retry_after_seconds(
                    response
                        .headers()
                        .get(header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok()),
                );
                return Err(Failure::Transient(retry_after));
            }
            return Err(CoreError::new(ErrorCode::ServiceUnavailable).into());
        }
    }

    async fn attempt<W: Write>(
        &self,
        initial: &url::Url,
        origin: &Origin,
        expected: &Expected,
        cancel: &Cancellation,
        progress: &(dyn Fn(u64) + Sync),
        sink: &mut W,
    ) -> Result<(), Failure> {
        let mut response = self.open(initial, origin, cancel).await?;
        if response
            .content_length()
            .is_some_and(|length| length != expected.size)
        {
            return Err(Failure::Integrity);
        }
        let mut hashers = Hashers::new(expected);
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled).into()),
                chunk = response.chunk() => chunk,
            };
            let Ok(chunk) = chunk else {
                return Err(Failure::Transient(None));
            };
            let Some(chunk) = chunk else {
                break;
            };
            if hashers.count().saturating_add(chunk.len() as u64) > expected.size {
                return Err(Failure::Integrity);
            }
            hashers.update(&chunk);
            sink.write_all(&chunk)
                .map_err(crate::minecraft::download::io)?;
            progress(hashers.count());
        }
        if !hashers.matches(expected) {
            return Err(Failure::Integrity);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
