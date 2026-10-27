use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Server {
    endpoint: String,
    requests: Arc<AtomicUsize>,
    handle: std::thread::JoinHandle<()>,
}
fn server(response: Vec<u8>, count: usize) -> Result<Server, std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let endpoint = format!("http://{}/artifact", listener.local_addr()?);
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let handle = std::thread::spawn(move || {
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) && seen.load(Ordering::SeqCst) < count {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                    let mut buffer = [0; 8192];
                    let _ = stream.read(&mut buffer);
                    seen.fetch_add(1, Ordering::SeqCst);
                    let _ = stream.write_all(&response);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
    });
    Ok(Server {
        endpoint,
        requests,
        handle,
    })
}
fn source(bytes: &[u8]) -> LibraryArtifact {
    LibraryArtifact {
        path: "a/b/1/b-1.jar".into(),
        url: format!("{MAVEN_ROOT}a/b/1/b-1.jar"),
        sha1: format!("{:x}", Sha1::digest(bytes)),
        size: bytes.len() as u64,
    }
}
fn paths() -> Result<AppPaths, CoreError> {
    AppPaths::new(
        std::env::temp_dir().join(format!("panita-neoforge-transport-{}", Uuid::new_v4())),
    )
}
use uuid::Uuid;

#[tokio::test]
async fn concurrent_writers_download_once_and_reuse_only_verified_blobs()
-> Result<(), Box<dyn std::error::Error>> {
    let payload = b"official library";
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .into_bytes();
    response.extend_from_slice(payload);
    let server = server(response, 1)?;
    let mut transport = Transport::new()?;
    transport.endpoint = Some(server.endpoint.clone());
    let paths = paths()?;
    let source = source(payload);
    let cancel = Cancellation::default();
    let (one, two) = tokio::join!(
        transport.library(&paths, &source, &cancel),
        transport.library(&paths, &source, &cancel)
    );
    assert_eq!(one?, two?);
    let cached = transport.library(&paths, &source, &cancel).await?;
    assert_eq!(std::fs::read(cached)?, payload);
    server.handle.join().map_err(|_| "server")?;
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn bad_downloads_redirects_and_cancellation_never_replace_existing_blob()
-> Result<(), Box<dyn std::error::Error>> {
    for response in [b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad".as_slice(),
        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/blocked\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbad"] {
        let server = server(response.to_vec(), 1)?;
        let mut transport = Transport::new()?;
        transport.endpoint = Some(server.endpoint);
        let paths = paths()?;
        let source = source(b"new");
        let destination = blob_path(&source.sha1);
        let (directory, leaf) = ensure_parent(paths.root(), &destination)?;
        directory.write_atomic_durable(&leaf, b"preserved old blob")?;
        assert!(transport.library(&paths, &source, &Cancellation::default()).await.is_err());
        assert_eq!(std::fs::read(paths.root().join(destination))?, b"preserved old blob");
        let cancelled = Cancellation::default(); cancelled.cancel();
        assert_eq!(transport.library(&paths, &source, &cancelled).await.err().map(|error| error.code()), Some(ErrorCode::OperationCancelled));
        server.handle.join().map_err(|_| "server")?;
    }
    Ok(())
}
