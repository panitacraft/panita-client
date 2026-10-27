//! Disposable shared pack-content cache, `content/{algorithm}/{hash[0..2]}/{hash}-{size}`.
//! An entry is published only after its exact size and every supplied hash verify.
//! The name is a claim, never proof: readers hash again while copying, and instances
//! always receive copies, never links.
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use private_fs::PrivateDir;
use sha2::{Digest, Sha256};

use super::{
    MAX_DOWNLOADS, Origin, invalid,
    transfer::{Expected, Fetch, Hashers, PackDownloader},
};
use crate::{
    error::{CoreError, ErrorCode},
    minecraft::{
        download::{Cancellation, cleanup_partials, ensure_parent, existing_parent, io, lock},
        install::check_capacity,
    },
    paths::AppPaths,
    storage::map_private_error,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Sha512,
    Sha256,
    Sha1,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Self::Sha512 => "sha512",
            Self::Sha256 => "sha256",
            Self::Sha1 => "sha1",
        }
    }
}

/// The strongest supplied hash plus the exact size identifies an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentKey {
    algorithm: Algorithm,
    hash: String,
    size: u64,
}

impl ContentKey {
    pub fn for_expected(expected: &Expected) -> Result<Self, CoreError> {
        expected.validate()?;
        let (algorithm, hash) = if let Some(hash) = &expected.sha512 {
            (Algorithm::Sha512, hash)
        } else if let Some(hash) = &expected.sha256 {
            (Algorithm::Sha256, hash)
        } else if let Some(hash) = &expected.sha1 {
            (Algorithm::Sha1, hash)
        } else {
            return Err(invalid());
        };
        Ok(Self {
            algorithm,
            hash: hash.clone(),
            size: expected.size,
        })
    }

    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    fn directory(&self) -> String {
        format!("content/{}/{}", self.algorithm.name(), &self.hash[..2])
    }

    pub fn relative(&self) -> String {
        format!("{}/{}-{}", self.directory(), self.hash, self.size)
    }

    /// One lock per shard directory also protects stale temporary cleanup there.
    fn lock_path(&self) -> String {
        let digest = Sha256::digest(self.directory().as_bytes());
        format!(".artifact-locks/content-{digest:x}.lock")
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub url: String,
    pub origin: Origin,
    pub expected: Expected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ensured {
    Ready {
        reused: bool,
    },
    /// The CDN withdrew the file; the caller turns it into a manual download.
    Withdrawn,
}

fn entry_present(paths: &AppPaths, expected: &Expected) -> Result<bool, CoreError> {
    let key = ContentKey::for_expected(expected)?;
    let Some((directory, name)) = existing_parent(paths.root(), &key.relative())? else {
        return Ok(false);
    };
    match directory.open_file(name) {
        Ok(file) => Ok(crate::minecraft::download::native_file(&file)?
            .metadata()
            .map_err(io)?
            .len()
            == expected.size),
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(map_private_error(error)),
    }
}

fn verified_entry(
    directory: &PrivateDir,
    name: &str,
    expected: &Expected,
    cancel: &Cancellation,
) -> Result<bool, CoreError> {
    let file = match directory.open_file(name) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => return Err(map_private_error(error)),
    };
    let mut input = crate::minecraft::download::native_file(&file)?;
    if input.metadata().map_err(io)?.len() != expected.size {
        return Ok(false);
    }
    let mut hashers = Hashers::new(expected);
    let mut buffer = [0; 64 * 1024];
    loop {
        cancel.check()?;
        let count = input.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        hashers.update(&buffer[..count]);
    }
    file.verify().map_err(map_private_error)?;
    Ok(hashers.matches(expected))
}

/// Make the verified entry exist: reuse a verified one or download and publish it.
pub async fn ensure(
    paths: &AppPaths,
    downloader: &PackDownloader,
    request: &Request,
    cancel: &Cancellation,
    progress: &(dyn Fn(u64) + Sync),
) -> Result<Ensured, CoreError> {
    let key = ContentKey::for_expected(&request.expected)?;
    let _guard = lock(paths.root(), &key.lock_path(), cancel).await?;
    let (directory, name) = ensure_parent(paths.root(), &key.relative())?;
    cleanup_partials(&directory, &paths.root().join(key.directory()))?;
    cancel.check()?;
    if verified_entry(&directory, &name, &request.expected, cancel)? {
        progress(request.expected.size);
        return Ok(Ensured::Ready { reused: true });
    }
    let fetched = downloader
        .download(
            &request.url,
            &request.origin,
            &request.expected,
            cancel,
            progress,
            || directory.atomic_durable(&name).map_err(map_private_error),
        )
        .await?;
    match fetched {
        Fetch::Withdrawn => Ok(Ensured::Withdrawn),
        Fetch::Done(file) => {
            cancel.check()?;
            file.commit().map_err(map_private_error)?;
            Ok(Ensured::Ready { reused: false })
        }
    }
}

/// Verify a file the player supplied and publish it as a verified entry. The input
/// is always read and checked in full, so a wrong file is rejected even when a
/// matching entry already exists.
pub async fn import(
    paths: &AppPaths,
    expected: &Expected,
    input: &mut dyn Read,
    cancel: &Cancellation,
) -> Result<(), CoreError> {
    let key = ContentKey::for_expected(expected)?;
    let mismatch = || CoreError::new(ErrorCode::ArtifactMismatch);
    let _guard = lock(paths.root(), &key.lock_path(), cancel).await?;
    let (directory, name) = ensure_parent(paths.root(), &key.relative())?;
    cleanup_partials(&directory, &paths.root().join(key.directory()))?;
    let mut output = directory.atomic_durable(&name).map_err(map_private_error)?;
    let mut hashers = Hashers::new(expected);
    let mut buffer = [0; 64 * 1024];
    loop {
        cancel.check()?;
        let count = input.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        if hashers.count().saturating_add(count as u64) > expected.size {
            return Err(mismatch());
        }
        hashers.update(&buffer[..count]);
        output.write_all(&buffer[..count]).map_err(io)?;
    }
    if !hashers.matches(expected) {
        return Err(mismatch());
    }
    cancel.check()?;
    output.commit().map_err(map_private_error)
}

/// Read an entry, hashing it again, and hand its bytes to `sink`. Only an `Ok`
/// result proves the bytes the sink saw were exactly the expected content, so the
/// sink must publish nothing until this returns.
pub fn stream_verified(
    paths: &AppPaths,
    expected: &Expected,
    cancel: &Cancellation,
    mut sink: impl FnMut(&[u8]) -> Result<(), CoreError>,
) -> Result<(), CoreError> {
    let key = ContentKey::for_expected(expected)?;
    let mismatch = || CoreError::new(ErrorCode::ArtifactMismatch);
    let (directory, name) = existing_parent(paths.root(), &key.relative())?.ok_or_else(mismatch)?;
    let file = match directory.open_file(name) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(mismatch());
        }
        Err(error) => return Err(map_private_error(error)),
    };
    let mut input = crate::minecraft::download::native_file(&file)?;
    if input.metadata().map_err(io)?.len() != expected.size {
        return Err(mismatch());
    }
    let mut hashers = Hashers::new(expected);
    let mut buffer = [0; 64 * 1024];
    loop {
        cancel.check()?;
        let count = input.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        hashers.update(&buffer[..count]);
        sink(&buffer[..count])?;
    }
    file.verify().map_err(map_private_error)?;
    if hashers.matches(expected) {
        Ok(())
    } else {
        Err(mismatch())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackProgress {
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub reused_files: usize,
}

pub type ProgressSink = Arc<dyn Fn(PackProgress) + Send + Sync>;

struct Tracker {
    state: Mutex<(PackProgress, Vec<u64>)>,
    sink: ProgressSink,
}

impl Tracker {
    fn new(requests: &[Request], sink: ProgressSink) -> Self {
        let progress = PackProgress {
            files_total: requests.len(),
            bytes_total: requests.iter().map(|r| r.expected.size).sum(),
            ..PackProgress::default()
        };
        Self {
            state: Mutex::new((progress, vec![0; requests.len()])),
            sink,
        }
    }

    fn set(&self, index: usize, bytes: u64, finished: Option<bool>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (progress, current) = &mut *state;
        let Some(slot) = current.get_mut(index) else {
            return;
        };
        progress.bytes_done = progress
            .bytes_done
            .saturating_sub(*slot)
            .saturating_add(bytes);
        *slot = bytes;
        if let Some(reused) = finished {
            progress.files_done += 1;
            progress.reused_files += usize::from(reused);
        }
        (self.sink)(*progress);
    }
}

/// Ensure every request with at most `MAX_DOWNLOADS` concurrent workers. The first
/// failure aborts the rest; a withdrawn file is reported, not treated as a failure.
pub async fn ensure_all(
    paths: &AppPaths,
    downloader: &PackDownloader,
    requests: &[Request],
    cancel: &Cancellation,
    progress: ProgressSink,
) -> Result<Vec<Ensured>, CoreError> {
    let mut missing = 0u64;
    for request in requests {
        cancel.check()?;
        if !entry_present(paths, &request.expected)? {
            missing = missing
                .checked_add(request.expected.size + 4096)
                .ok_or_else(invalid)?;
        }
    }
    check_capacity(missing, fs2::available_space(paths.root()).map_err(io)?)?;
    let tracker = Arc::new(Tracker::new(requests, progress));
    let mut results = vec![None; requests.len()];
    let mut workers = tokio::task::JoinSet::new();
    let mut queue = requests.iter().cloned().enumerate();
    loop {
        while workers.len() < MAX_DOWNLOADS {
            let Some((index, request)) = queue.next() else {
                break;
            };
            let (paths, downloader, cancel) = (paths.clone(), downloader.clone(), cancel.clone());
            let tracker = tracker.clone();
            workers.spawn(async move {
                let report = |bytes: u64| tracker.set(index, bytes, None);
                let ensured = ensure(&paths, &downloader, &request, &cancel, &report).await?;
                let reused = matches!(ensured, Ensured::Ready { reused: true });
                tracker.set(index, request.expected.size, Some(reused));
                Ok::<_, CoreError>((index, ensured))
            });
        }
        let Some(result) = workers.join_next().await else {
            break;
        };
        let result = result
            .map_err(|_| CoreError::new(ErrorCode::StorageUnavailable))
            .and_then(|result| result);
        match result {
            Ok((index, ensured)) => results[index] = Some(ensured),
            Err(error) => {
                workers.abort_all();
                while workers.join_next().await.is_some() {}
                return Err(error);
            }
        }
    }
    results
        .into_iter()
        .map(|result| result.ok_or_else(invalid))
        .collect()
}

#[cfg(test)]
mod tests;
