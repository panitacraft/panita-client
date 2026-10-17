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
    CoreError::new(if error.kind() == std::io::ErrorKind::PermissionDenied {
        ErrorCode::PermissionDenied
    } else {
        ErrorCode::StorageUnavailable
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
    inner: Arc<(AtomicBool, Notify)>,
}
impl Cancellation {
    pub fn cancel(&self) {
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

/// File handles are retained until drop, so another process cannot publish the same artifact.
pub(crate) async fn lock(
    root: &Path,
    relative: &str,
    cancel: &Cancellation,
) -> Result<std::fs::File, CoreError> {
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
    let file = native_file(&file)?;
    loop {
        cancel.check()?;
        match file.try_lock() {
            Ok(()) => {
                private_fs::verify(&file).map_err(map_private_error)?;
                return Ok(file);
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
    let mut file = match directory.open_file(name) {
        Ok(file) => file,
        Err(private_fs::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => return Err(map_private_error(error)),
    };
    if native_file(&file)?.metadata().map_err(io)?.len() != artifact.size {
        return Ok(false);
    }
    let mut buffer = [0; 64 * 1024];
    let mut hash = Sha1::new();
    loop {
        let count = file.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    file.verify().map_err(map_private_error)?;
    Ok(format!("{:x}", hash.finalize()) == artifact.sha1)
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
        artifact.validate()?;
        safe_path(relative)?;
        // The path lock also serializes different hashes requesting the same cache destination.
        let key = format!(
            "{:x}",
            Sha1::digest(relative.to_ascii_lowercase().as_bytes())
        );
        let _guard = lock(paths.root(), &format!(".artifact-locks/{key}.lock"), cancel).await?;
        let (directory, name) = ensure_parent(paths.root(), relative)?;
        cancel.check()?;
        if verified_file(&directory, &name, artifact)? {
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
                        let mut request = [0; 4096];
                        let _ = socket.read(&mut request);
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
        assert!(first.is_ok() && second.is_ok());
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
}
