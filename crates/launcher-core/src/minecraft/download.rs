//! Bounded public-artifact transport and private, verified shared cache.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use private_fs::PrivateDir;
use sha1::{Digest, Sha1};
use tokio::sync::Notify;

use super::{Artifact, invalid, official_url, safe_path};
use crate::{
    error::{CoreError, ErrorCode},
    paths::AppPaths,
    storage::map_private_error,
};

pub(crate) fn io(error: std::io::Error) -> CoreError {
    CoreError::new(match error.kind() {
        std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
        std::io::ErrorKind::StorageFull => ErrorCode::InsufficientSpace,
        _ => ErrorCode::StorageUnavailable,
    })
}

pub(crate) fn native_file(file: &private_fs::PrivateFile) -> Result<std::fs::File, CoreError> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        Ok(std::fs::File::from(
            file.as_handle().try_clone_to_owned().map_err(io)?,
        ))
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        Ok(std::fs::File::from(
            file.as_fd().try_clone_to_owned().map_err(io)?,
        ))
    }
}

#[derive(Clone, Default)]
pub struct Cancellation {
    inner: Arc<(AtomicBool, Notify, std::sync::Mutex<()>)>,
}
impl Cancellation {
    pub fn cancel(&self) {
        let _commit = self
            .inner
            .2
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.inner.0.store(true, Ordering::SeqCst);
        self.inner.1.notify_waiters();
    }
    pub fn check(&self) -> Result<(), CoreError> {
        if self.inner.0.load(Ordering::SeqCst) {
            Err(CoreError::new(ErrorCode::OperationCancelled))
        } else {
            Ok(())
        }
    }
    /// Linearize cancellation against synchronous process creation. A cancel
    /// before commit prevents it; after commit the running process needs stop.
    pub(crate) fn commit<T>(
        &self,
        action: impl FnOnce() -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        let _commit = self
            .inner
            .2
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.check()?;
        action()
    }
    pub async fn cancelled(&self) {
        loop {
            let notified = self.inner.1.notified();
            if self.check().is_err() {
                return;
            }
            notified.await;
        }
    }
}

pub(crate) fn ensure_parent(
    root: &Path,
    relative: &str,
) -> Result<(PrivateDir, String), CoreError> {
    safe_path(relative)?;
    let mut directory = PrivateDir::ensure(root).map_err(map_private_error)?;
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return Ok((directory, part.into()));
        }
        directory = directory.ensure_dir(part).map_err(map_private_error)?;
    }
    Err(invalid())
}

/// Inspect an optional cache alias without creating its directory hierarchy.
pub(crate) fn existing_parent(
    root: &Path,
    relative: &str,
) -> Result<Option<(PrivateDir, String)>, CoreError> {
    safe_path(relative)?;
    let path = Path::new(relative);
    let leaf = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid)?;
    match PrivateDir::open(root.join(path.parent().ok_or_else(invalid)?)) {
        Ok(directory) => Ok(Some((directory, leaf.into()))),
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(map_private_error(error)),
    }
}

/// File handles are retained until drop, so another process cannot publish the same artifact.
pub(crate) struct ArtifactLock {
    // Retain both handles: closing a duplicate on Windows can release file locks.
    _file: std::fs::File,
    _source: private_fs::PrivateFile,
}

/// Nonblocking coordination for directory migration and local audits.
pub(crate) fn try_lock(root: &Path, relative: &str) -> Result<Option<ArtifactLock>, CoreError> {
    let (directory, name) = ensure_parent(root, relative)?;
    let source = match directory.open_file(&name) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            match directory.create(&name) {
                Ok(file) => file,
                Err(private_fs::Error::Io(error))
                    if error.kind() == std::io::ErrorKind::AlreadyExists =>
                {
                    directory.open_file(&name).map_err(map_private_error)?
                }
                Err(error) => return Err(map_private_error(error)),
            }
        }
        Err(error) => return Err(map_private_error(error)),
    };
    let file = native_file(&source)?;
    match file.try_lock() {
        Ok(()) => {
            private_fs::verify(&file).map_err(map_private_error)?;
            Ok(Some(ArtifactLock {
                _file: file,
                _source: source,
            }))
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(io(error)),
    }
}

pub(crate) async fn lock(
    root: &Path,
    relative: &str,
    cancel: &Cancellation,
) -> Result<ArtifactLock, CoreError> {
    let (directory, name) = ensure_parent(root, relative)?;
    let file = match directory.open_file(&name) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            match directory.create(&name) {
                Ok(file) => file,
                Err(private_fs::Error::Io(error))
                    if error.kind() == std::io::ErrorKind::AlreadyExists =>
                {
                    directory.open_file(&name).map_err(map_private_error)?
                }
                Err(error) => return Err(map_private_error(error)),
            }
        }
        Err(error) => return Err(map_private_error(error)),
    };
    let source = file;
    let file = native_file(&source)?;
    loop {
        cancel.check()?;
        match file.try_lock() {
            Ok(()) => {
                private_fs::verify(&file).map_err(map_private_error)?;
                return Ok(ArtifactLock {
                    _file: file,
                    _source: source,
                });
            }
            Err(std::fs::TryLockError::WouldBlock) => tokio::select! {
                _ = cancel.cancelled() => cancel.check()?,
                _ = tokio::time::sleep(Duration::from_millis(50)) => {},
            },
            Err(std::fs::TryLockError::Error(error)) => return Err(io(error)),
        }
    }
}

pub(crate) fn verified_file(
    directory: &PrivateDir,
    name: &str,
    artifact: &Artifact,
) -> Result<bool, CoreError> {
    let file = match directory.open_file(name) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => return Err(map_private_error(error)),
    };
    // These are public game artifacts, not credential files. Retain the private
    // handle and verify its identity/permissions at both boundaries; hash through
    // a duplicate of that same handle instead of repeating ACL queries per chunk.
    let mut input = native_file(&file)?;
    if input.metadata().map_err(io)?.len() != artifact.size {
        return Ok(false);
    }
    let mut buffer = [0; 64 * 1024];
    let mut hash = Sha1::new();
    loop {
        let count = input.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    file.verify().map_err(map_private_error)?;
    Ok(format!("{:x}", hash.finalize()) == artifact.sha1)
}

pub(crate) fn verified_planned_file(
    directory: &PrivateDir,
    name: &str,
    artifact: &Artifact,
    executable: bool,
) -> Result<bool, CoreError> {
    #[cfg(target_os = "linux")]
    if executable {
        return super::executable::matches(directory, name, artifact);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = executable;
    verified_file(directory, name, artifact)
}

fn cleanup_partials(directory: &PrivateDir, path: &Path) -> Result<(), CoreError> {
    for entry in std::fs::read_dir(path).map_err(io)? {
        let entry = entry.map_err(io)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(hex) = name
            .strip_prefix(".private-fs-")
            .and_then(|name| name.strip_suffix(".tmp"))
        else {
            continue;
        };
        if hex.len() != 32 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        directory.open_file(name).map_err(map_private_error)?;
        std::fs::remove_file(entry.path()).map_err(io)?;
    }
    Ok(())
}

fn legacy_alias(relative: &str, artifact: &Artifact) -> Result<Option<String>, CoreError> {
    if artifact.path.is_some() && relative == super::plan::library_cache_path(artifact)? {
        return Ok(Some(format!(
            "libraries/{}",
            artifact.path.as_ref().ok_or_else(invalid)?
        )));
    }
    if let Some(name) = relative.strip_prefix("assets/indexes/")
        && let Some(id) = name.strip_suffix(&format!("-{}.json", artifact.sha1))
        && !id.contains('/')
    {
        safe_path(id)?;
        return Ok(Some(format!("assets/indexes/{id}.json")));
    }
    Ok(None)
}

fn import_verified(
    paths: &AppPaths,
    alias: &str,
    directory: &PrivateDir,
    name: &str,
    artifact: &Artifact,
    cancel: &Cancellation,
) -> Result<bool, CoreError> {
    let Some((source, leaf)) = existing_parent(paths.root(), alias)? else {
        return Ok(false);
    };
    let mut input = match source.open_file(leaf) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => return Err(map_private_error(error)),
    };
    if native_file(&input)?.metadata().map_err(io)?.len() != artifact.size {
        return Ok(false);
    }
    let mut output = directory.atomic_durable(name).map_err(map_private_error)?;
    let mut hash = Sha1::new();
    let mut count = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        cancel.check()?;
        let size = input.read(&mut buffer).map_err(io)?;
        if size == 0 {
            break;
        }
        count += size as u64;
        if count > artifact.size {
            return Ok(false);
        }
        hash.update(&buffer[..size]);
        output.write_all(&buffer[..size]).map_err(io)?;
    }
    if count != artifact.size || format!("{:x}", hash.finalize()) != artifact.sha1 {
        return Ok(false);
    }
    cancel.check()?;
    output.commit().map_err(map_private_error)?;
    Ok(true)
}

#[derive(Clone, Copy, Debug)]
pub struct DownloadProgress {
    pub bytes: u64,
    pub total: u64,
    pub reused: bool,
}

#[derive(Clone)]
pub struct Downloader {
    client: reqwest::Client,
    #[cfg(test)]
    test_base: Option<String>,
}

struct CacheRequest<'a> {
    relative: &'a str,
    artifact: &'a Artifact,
    executable: bool,
}

impl Downloader {
    pub fn new() -> Result<Self, CoreError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "PanitaClient/",
                env!("CARGO_PKG_VERSION"),
                " (+https://www.panitacraft.com/)"
            ))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 4 || official_url(attempt.url().as_str()).is_err() {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|_| invalid())?;
        Ok(Self {
            client,
            #[cfg(test)]
            test_base: None,
        })
    }

    async fn response(
        &self,
        url: &str,
        cancel: &Cancellation,
    ) -> Result<reqwest::Response, CoreError> {
        official_url(url)?;
        #[cfg(test)]
        let url = self.test_base.as_deref().unwrap_or(url);
        for attempt in 0..3 {
            cancel.check()?;
            let result = tokio::select! {
                _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
                response = self.client.get(url).send() => response,
            };
            match result {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response)
                    if response.status().is_server_error()
                        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {}
                Ok(_) => return Err(CoreError::new(ErrorCode::ServiceUnavailable)),
                Err(_) => {}
            }
            if attempt < 2 {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
                    _ = tokio::time::sleep(Duration::from_millis(250 << attempt)) => {},
                }
            }
        }
        Err(CoreError::new(ErrorCode::NetworkUnavailable))
    }

    /// The two bootstrap indexes publish no checksum. TLS/host allowlisting and a body
    /// limit authenticate acquisition; their exact bytes are fingerprinted in the install lock.
    pub async fn document(&self, url: &str, cancel: &Cancellation) -> Result<Vec<u8>, CoreError> {
        let mut response = self.response(url, cancel).await?;
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
                chunk = response.chunk() => chunk.map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(invalid());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    pub async fn cache(
        &self,
        paths: &AppPaths,
        relative: &str,
        artifact: &Artifact,
        cancel: &Cancellation,
        progress: &(dyn Fn(DownloadProgress) + Sync),
    ) -> Result<PathBuf, CoreError> {
        self.cache_request(
            paths,
            CacheRequest {
                relative,
                artifact,
                executable: false,
            },
            cancel,
            progress,
        )
        .await
    }

    pub(crate) async fn cache_planned(
        &self,
        paths: &AppPaths,
        file: &super::plan::PlannedArtifact,
        cancel: &Cancellation,
        progress: &(dyn Fn(DownloadProgress) + Sync),
    ) -> Result<PathBuf, CoreError> {
        self.cache_request(
            paths,
            CacheRequest {
                relative: &file.destination,
                artifact: &file.artifact,
                executable: file.executable,
            },
            cancel,
            progress,
        )
        .await
    }

    async fn cache_request(
        &self,
        paths: &AppPaths,
        request: CacheRequest<'_>,
        cancel: &Cancellation,
        progress: &(dyn Fn(DownloadProgress) + Sync),
    ) -> Result<PathBuf, CoreError> {
        let CacheRequest {
            relative,
            artifact,
            executable,
        } = request;
        artifact.validate()?;
        safe_path(relative)?;
        if executable && !relative.starts_with("runtime/") {
            return Err(invalid());
        }
        if !["libraries/", "assets/", "versions/", "runtime/"]
            .iter()
            .any(|prefix| relative.starts_with(prefix))
        {
            return Err(invalid());
        }
        // A parent-directory lock also protects stale atomic-temporary cleanup.
        let parent = relative.rsplit_once('/').map_or("", |(parent, _)| parent);
        let key = format!("{:x}", Sha1::digest(parent.to_ascii_lowercase().as_bytes()));
        let _guard = lock(paths.root(), &format!(".artifact-locks/{key}.lock"), cancel).await?;
        let (directory, name) = ensure_parent(paths.root(), relative)?;
        cleanup_partials(&directory, &paths.root().join(parent))?;
        cancel.check()?;
        if verified_planned_file(&directory, &name, artifact, executable)? {
            #[cfg(target_os = "linux")]
            if executable {
                super::executable::enable(&directory, &name)?;
            }
            progress(DownloadProgress {
                bytes: artifact.size,
                total: artifact.size,
                reused: true,
            });
            return Ok(paths.root().join(relative));
        }
        if let Some(alias) = legacy_alias(relative, artifact)?
            && import_verified(paths, &alias, &directory, &name, artifact, cancel)?
        {
            progress(DownloadProgress {
                bytes: artifact.size,
                total: artifact.size,
                reused: true,
            });
            return Ok(paths.root().join(relative));
        }
        let mut response = self.response(&artifact.url, cancel).await?;
        if response
            .content_length()
            .is_some_and(|length| length != artifact.size)
        {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        let staged = executable && cfg!(target_os = "linux");
        let target = if staged {
            format!(".private-fs-{}.tmp", uuid::Uuid::new_v4().simple())
        } else {
            name.clone()
        };
        let mut output = directory
            .atomic_durable(&target)
            .map_err(map_private_error)?;
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
            count += chunk.len() as u64;
            if count > artifact.size {
                return Err(CoreError::new(ErrorCode::ArtifactMismatch));
            }
            hash.update(&chunk);
            output.write_all(&chunk).map_err(io)?;
            progress(DownloadProgress {
                bytes: count,
                total: artifact.size,
                reused: false,
            });
        }
        if count != artifact.size || format!("{:x}", hash.finalize()) != artifact.sha1 {
            return Err(CoreError::new(ErrorCode::ArtifactMismatch));
        }
        cancel.check()?;
        output.commit().map_err(map_private_error)?;
        #[cfg(target_os = "linux")]
        if staged {
            super::executable::publish(&directory, &target, &name)?;
        }
        Ok(paths.root().join(relative))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;

    struct Server {
        base: String,
        requests: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        fn new(body: Vec<u8>, advertised: usize, delay: Duration) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| panic!("listener"));
            let base = format!(
                "http://{}/artifact",
                listener.local_addr().unwrap_or_else(|_| panic!("address"))
            );
            listener
                .set_nonblocking(true)
                .unwrap_or_else(|_| panic!("nonblocking"));
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(AtomicUsize::new(0));
            let flag = stop.clone();
            let count = requests.clone();
            let handle = std::thread::spawn(move || {
                while !flag.load(Ordering::SeqCst) {
                    if let Ok((mut socket, _)) = listener.accept() {
                        let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut request = Vec::new();
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 16 * 1024 {
                            let mut buffer = [0; 1024];
                            match socket.read(&mut buffer) {
                                Ok(0) | Err(_) => break,
                                Ok(count) => request.extend_from_slice(&buffer[..count]),
                            }
                        }
                        if !request.starts_with(b"GET ") || !request.ends_with(b"\r\n\r\n") {
                            continue;
                        }
                        count.fetch_add(1, Ordering::SeqCst);
                        let _ = write!(
                            socket,
                            "HTTP/1.1 200 OK\r\nContent-Length: {advertised}\r\nConnection: close\r\n\r\n"
                        );
                        let _ = socket.flush();
                        std::thread::sleep(delay);
                        let _ = socket.write_all(&body);
                    } else {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            });
            Self {
                base,
                requests,
                stop,
                handle: Some(handle),
            }
        }
        fn downloader(&self) -> Downloader {
            Downloader {
                client: reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap_or_else(|_| panic!("client")),
                test_base: Some(self.base.clone()),
            }
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }
    fn artifact(bytes: &[u8]) -> Artifact {
        Artifact {
            url: "https://libraries.minecraft.net/test.jar".into(),
            sha1: format!("{:x}", Sha1::digest(bytes)),
            size: bytes.len() as u64,
            path: None,
        }
    }
    fn paths() -> AppPaths {
        AppPaths::new(
            std::env::temp_dir().join(format!("panita-download-{}", uuid::Uuid::new_v4())),
        )
        .unwrap_or_else(|_| panic!("paths"))
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn executable_runtime_download_reuse_and_repair_preserve_protection() {
        use std::os::unix::fs::PermissionsExt;
        let bytes = b"verified runtime executable";
        let server = Server::new(bytes.to_vec(), bytes.len(), Duration::ZERO);
        let downloader = server.downloader();
        let paths = paths();
        let file = super::super::plan::PlannedArtifact {
            destination: "runtime/java-21-linux/bin/java".into(),
            artifact: artifact(bytes),
            executable: true,
        };
        let cancel = Cancellation::default();
        let local = paths.root().join(&file.destination);
        for _ in 0..2 {
            downloader
                .cache_planned(&paths, &file, &cancel, &|_| {})
                .await
                .unwrap_or_else(|error| panic!("executable cache: {error:?}"));
            assert_eq!(
                std::fs::metadata(&local)
                    .unwrap_or_else(|_| panic!("metadata"))
                    .permissions()
                    .mode()
                    & 0o7777,
                0o700
            );
            let (directory, leaf) =
                ensure_parent(paths.root(), &file.destination).unwrap_or_else(|_| panic!("parent"));
            assert!(
                verified_planned_file(&directory, &leaf, &file.artifact, true).unwrap_or(false)
            );
            assert!(
                directory.open_file(&leaf).is_err(),
                "metadata policy must remain 0600"
            );
        }
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
        std::fs::write(&local, vec![b'x'; bytes.len()])
            .unwrap_or_else(|_| panic!("corrupt fixture"));
        downloader
            .cache_planned(&paths, &file, &cancel, &|_| {})
            .await
            .unwrap_or_else(|error| panic!("repair executable: {error:?}"));
        assert_eq!(std::fs::read(&local).unwrap_or_default(), bytes);
        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
        // Failed download verification must preserve the existing file and mode.
        let bad_server = Server::new(vec![b'y'; bytes.len()], bytes.len(), Duration::ZERO);
        std::fs::write(&local, vec![b'x'; bytes.len()])
            .unwrap_or_else(|_| panic!("corrupt fixture"));
        assert!(
            bad_server
                .downloader()
                .cache_planned(&paths, &file, &cancel, &|_| {})
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(&local).unwrap_or_default(),
            vec![b'x'; bytes.len()]
        );
        assert_eq!(
            std::fs::metadata(&local)
                .unwrap_or_else(|_| panic!("metadata"))
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
        std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
    }

    #[tokio::test]
    async fn verified_reuse_and_corruption_replacement() {
        let bytes = b"verified content";
        let server = Server::new(bytes.to_vec(), bytes.len(), Duration::ZERO);
        let downloader = server.downloader();
        let paths = paths();
        let cancel = Cancellation::default();
        let artifact = artifact(bytes);
        let relative = "libraries/test/file.jar";
        let file = downloader
            .cache(&paths, relative, &artifact, &cancel, &|_| {})
            .await
            .unwrap_or_else(|error| panic!("download: {error}"));
        downloader
            .cache(&paths, relative, &artifact, &cancel, &|progress| {
                assert!(progress.reused)
            })
            .await
            .unwrap_or_else(|_| panic!("reuse"));
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
        let (directory, name) =
            ensure_parent(paths.root(), relative).unwrap_or_else(|_| panic!("parent"));
        directory
            .write_atomic_durable(name, b"corrupt")
            .unwrap_or_else(|_| panic!("corrupt"));
        downloader
            .cache(&paths, relative, &artifact, &cancel, &|_| {})
            .await
            .unwrap_or_else(|_| panic!("repair cache"));
        assert_eq!(std::fs::read(file).ok().as_deref(), Some(bytes.as_slice()));
        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn changed_coordinate_hash_keeps_both_blobs_and_imports_legacy_cache() {
        let paths = paths();
        let old_bytes = b"old library";
        let new_bytes = b"new library";
        let old_server = Server::new(old_bytes.to_vec(), old_bytes.len(), Duration::ZERO);
        let new_server = Server::new(new_bytes.to_vec(), new_bytes.len(), Duration::ZERO);
        let mut old = artifact(old_bytes);
        old.path = Some("group/library/1/library.jar".into());
        let mut new = artifact(new_bytes);
        new.path = old.path.clone();
        let legacy = "libraries/group/library/1/library.jar";
        let cancel = Cancellation::default();
        old_server
            .downloader()
            .cache(&paths, legacy, &old, &cancel, &|_| {})
            .await
            .unwrap_or_else(|_| panic!("legacy"));
        let old_path =
            super::super::plan::library_cache_path(&old).unwrap_or_else(|_| panic!("old path"));
        let new_path =
            super::super::plan::library_cache_path(&new).unwrap_or_else(|_| panic!("new path"));
        assert_ne!(old_path, new_path);
        old_server
            .downloader()
            .cache(&paths, &old_path, &old, &cancel, &|progress| {
                assert!(progress.reused)
            })
            .await
            .unwrap_or_else(|_| panic!("import"));
        assert_eq!(old_server.requests.load(Ordering::SeqCst), 1);
        new_server
            .downloader()
            .cache(&paths, &new_path, &new, &cancel, &|_| {})
            .await
            .unwrap_or_else(|_| panic!("new blob"));
        assert_eq!(
            std::fs::read(paths.root().join(old_path)).ok().as_deref(),
            Some(old_bytes.as_slice())
        );
        assert_eq!(
            std::fs::read(paths.root().join(legacy)).ok().as_deref(),
            Some(old_bytes.as_slice())
        );
        assert_eq!(
            std::fs::read(paths.root().join(new_path)).ok().as_deref(),
            Some(new_bytes.as_slice())
        );
    }

    #[test]
    fn missing_legacy_cache_lookup_does_not_create_coordinate_directories() {
        let paths = paths();
        let (directory, _) = ensure_parent(paths.root(), "libraries/ab/blob.jar")
            .unwrap_or_else(|_| panic!("cache"));
        assert!(
            !import_verified(
                &paths,
                "libraries/group/library/1/library.jar",
                &directory,
                "blob.jar",
                &artifact(b"jar"),
                &Cancellation::default()
            )
            .unwrap_or_else(|_| panic!("lookup"))
        );
        assert!(!paths.root().join("libraries/group").exists());
        assert!(existing_parent(paths.root(), "../escape.jar").is_err());
        drop(directory);
        std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
    }
    #[tokio::test]
    async fn hash_mismatch_truncation_and_cancel_leave_old_file_intact() {
        for (body, advertised, delay, cancelled) in [
            (b"wrong".to_vec(), 5, Duration::ZERO, false),
            (b"short".to_vec(), 10, Duration::ZERO, false),
            (b"valid data".to_vec(), 10, Duration::from_millis(200), true),
        ] {
            let server = Server::new(body, advertised, delay);
            let paths = paths();
            let relative = "assets/test";
            let (directory, name) =
                ensure_parent(paths.root(), relative).unwrap_or_else(|_| panic!("parent"));
            directory
                .write_atomic_durable(&name, b"old")
                .unwrap_or_else(|_| panic!("old"));
            let cancel = Cancellation::default();
            let bytes = if advertised == 5 {
                b"right".as_slice()
            } else {
                b"valid data".as_slice()
            };
            let downloader = server.downloader();
            let result = if cancelled {
                let expected = artifact(bytes);
                let request = downloader.cache(&paths, relative, &expected, &cancel, &|_| {});
                let trigger = async {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    cancel.cancel();
                };
                tokio::join!(request, trigger).0
            } else {
                downloader
                    .cache(&paths, relative, &artifact(bytes), &cancel, &|_| {})
                    .await
            };
            assert!(result.is_err());
            assert_eq!(
                std::fs::read(paths.root().join(relative)).ok().as_deref(),
                Some(b"old".as_slice())
            );
            assert!(
                std::fs::read_dir(paths.root().join("assets"))
                    .unwrap_or_else(|_| panic!("dir"))
                    .all(|entry| entry.is_ok_and(|entry| !entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".private-fs-")))
            );
        }
    }
    #[tokio::test]
    async fn racing_cache_writers_download_once_and_lock_wait_is_cancellable() {
        let bytes = b"verified";
        let server = Server::new(bytes.to_vec(), bytes.len(), Duration::from_millis(100));
        let downloader = server.downloader();
        let paths = paths();
        let cancel = Cancellation::default();
        let artifact = artifact(bytes);
        let (first, second) = tokio::join!(
            downloader.cache(&paths, "libraries/a.jar", &artifact, &cancel, &|_| {}),
            downloader.cache(&paths, "libraries/a.jar", &artifact, &cancel, &|_| {})
        );
        assert!(
            first.is_ok() && second.is_ok(),
            "cache writers: {first:?}, {second:?}"
        );
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
        let _guard = lock(paths.root(), "locks/test", &cancel)
            .await
            .unwrap_or_else(|_| panic!("lock"));
        let waiting = lock(paths.root(), "locks/test", &cancel);
        let trigger = async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            cancel.cancel();
        };
        assert!(tokio::join!(waiting, trigger).0.is_err());
    }

    #[tokio::test]
    async fn lock_child() {
        let Some(root) = std::env::var_os("PANITA_TEST_ARTIFACT_LOCK_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let _guard = lock(&root, "locks/process.lock", &Cancellation::default())
            .await
            .unwrap_or_else(|_| panic!("child lock"));
        let directory = PrivateDir::open(&root).unwrap_or_else(|_| panic!("root"));
        directory
            .write_atomic_durable("child-acquired", b"yes")
            .unwrap_or_else(|_| panic!("marker"));
    }

    #[tokio::test]
    async fn process_lock_excludes_an_independent_writer() {
        let paths = paths();
        let guard = lock(paths.root(), "locks/process.lock", &Cancellation::default())
            .await
            .unwrap_or_else(|_| panic!("parent lock"));
        let executable = std::env::current_exe().unwrap_or_else(|_| panic!("executable"));
        let mut child = std::process::Command::new(executable)
            .args([
                "--exact",
                "minecraft::download::tests::lock_child",
                "--nocapture",
            ])
            .env("PANITA_TEST_ARTIFACT_LOCK_ROOT", paths.root())
            .spawn()
            .unwrap_or_else(|_| panic!("child"));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!paths.root().join("child-acquired").exists());
        drop(guard);
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap_or_else(|_| panic!("wait")) {
                assert!(status.success());
                break;
            }
            if started.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                panic!("child lock timeout");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(paths.root().join("child-acquired").is_file());
    }

    #[test]
    fn existing_directory_redirect_is_rejected_before_writing() {
        let paths = paths();
        let outside = paths.root().with_extension("outside");
        PrivateDir::ensure(&outside).unwrap_or_else(|_| panic!("outside fixture"));
        ensure_parent(paths.root(), "libraries/placeholder")
            .unwrap_or_else(|_| panic!("libraries"));
        let link = paths.root().join("libraries").join("redirect");
        #[cfg(windows)]
        {
            let result = std::process::Command::new("cmd")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .output()
                .unwrap_or_else(|_| panic!("junction command"));
            assert!(
                result.status.success(),
                "junction creation failed: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap_or_else(|_| panic!("symlink"));
        assert!(ensure_parent(paths.root(), "libraries/redirect/payload.jar").is_err());
        assert!(!outside.join("payload.jar").exists());
    }
}
