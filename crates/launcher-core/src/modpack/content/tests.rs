use super::*;
use crate::test_http::{Reply, Server};
use sha1::Sha1;
use sha2::Sha512;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use uuid::Uuid;

fn root() -> AppPaths {
    AppPaths::new(std::env::temp_dir().join(format!("panita-content-{}", Uuid::new_v4())))
        .unwrap_or_else(|_| panic!("root"))
}

fn expected(body: &[u8]) -> Expected {
    Expected {
        size: body.len() as u64,
        sha1: Some(format!("{:x}", Sha1::digest(body))),
        sha256: None,
        sha512: Some(format!("{:x}", Sha512::digest(body))),
    }
}

fn request(name: &str, body: &[u8]) -> Request {
    Request {
        url: format!("https://cdn.modrinth.com/data/project/versions/version/{name}"),
        origin: Origin::Modrinth,
        expected: expected(body),
        fallbacks: Vec::new(),
    }
}

fn entry(paths: &AppPaths, expected: &Expected) -> std::path::PathBuf {
    let key = ContentKey::for_expected(expected).unwrap_or_else(|_| panic!("key"));
    paths.root().join(key.relative())
}

fn partials(paths: &AppPaths, expected: &Expected) -> usize {
    let path = entry(paths, expected);
    std::fs::read_dir(path.parent().unwrap_or(paths.root()))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(".private-fs-"))
                .count()
        })
        .unwrap_or(0)
}

fn quiet() -> ProgressSink {
    Arc::new(|_| {})
}

#[test]
fn keys_use_the_strongest_hash_and_the_exact_size() {
    let both = expected(b"abc");
    let key = ContentKey::for_expected(&both).unwrap_or_else(|_| panic!("key"));
    assert_eq!(key.algorithm(), Algorithm::Sha512);
    let hash = both.sha512.clone().unwrap_or_default();
    assert_eq!(
        key.relative(),
        format!("content/sha512/{}/{hash}-3", &hash[..2])
    );
    let sha1_only = Expected {
        sha512: None,
        ..both.clone()
    };
    let key = ContentKey::for_expected(&sha1_only).unwrap_or_else(|_| panic!("key"));
    assert_eq!(key.algorithm(), Algorithm::Sha1);
    assert!(key.relative().starts_with("content/sha1/"));
    let archive = Expected::archive(&"c".repeat(64), 10);
    assert!(
        ContentKey::for_expected(&archive)
            .unwrap_or_else(|_| panic!("key"))
            .relative()
            .starts_with("content/sha256/cc/")
    );
    // Equal hashes with another size are another entry.
    let other = Expected { size: 4, ..both };
    assert_ne!(
        ContentKey::for_expected(&other)
            .map(|k| k.relative())
            .unwrap_or_default(),
        key.relative()
    );
    assert!(
        ContentKey::for_expected(&Expected {
            sha1: None,
            sha512: None,
            ..other
        })
        .is_err()
    );
}

#[tokio::test]
async fn a_second_instance_reuses_the_verified_entry_without_downloading() {
    let body = b"shared mod".to_vec();
    let served = body.clone();
    let server = Server::start(move |_| Reply::ok(served.clone()));
    let downloader = PackDownloader::for_tests(server.base());
    let paths = root();
    let cancel = Cancellation::default();
    let request = request("mod.jar", &body);
    let first = ensure(&paths, &downloader, &request, &cancel, &|_| {}).await;
    assert_eq!(first.ok(), Some(Ensured::Ready { reused: false }));
    let second = ensure(&paths, &downloader, &request, &cancel, &|_| {}).await;
    assert_eq!(second.ok(), Some(Ensured::Ready { reused: true }));
    assert_eq!(server.requests().len(), 1);
    assert_eq!(
        std::fs::read(entry(&paths, &request.expected)).unwrap_or_default(),
        body
    );
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn a_damaged_entry_is_replaced_not_trusted() {
    let body = b"original bytes".to_vec();
    let served = body.clone();
    let server = Server::start(move |_| Reply::ok(served.clone()));
    let downloader = PackDownloader::for_tests(server.base());
    let paths = root();
    let cancel = Cancellation::default();
    let request = request("mod.jar", &body);
    assert!(
        ensure(&paths, &downloader, &request, &cancel, &|_| {})
            .await
            .is_ok()
    );
    let path = entry(&paths, &request.expected);
    // Same size, different bytes: the name alone must not be believed.
    assert!(std::fs::write(&path, b"tampered bytes").is_ok());
    let repaired = ensure(&paths, &downloader, &request, &cancel, &|_| {}).await;
    assert_eq!(repaired.ok(), Some(Ensured::Ready { reused: false }));
    assert_eq!(std::fs::read(&path).unwrap_or_default(), body);
    assert_eq!(server.requests().len(), 2);
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn failed_and_withdrawn_transfers_publish_nothing_and_leave_no_partials() {
    let body = b"never published".to_vec();
    let paths = root();
    let cancel = Cancellation::default();
    let bad = Server::start(|_| Reply::ok("not the expected bytes"));
    let downloader = PackDownloader::for_tests(bad.base());
    let request = request("mod.jar", &body);
    assert_eq!(
        ensure(&paths, &downloader, &request, &cancel, &|_| {})
            .await
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert!(!entry(&paths, &request.expected).exists());

    let gone = Server::start(|_| Reply::status(404));
    let downloader = PackDownloader::for_tests(gone.base());
    let withdrawn = Request {
        url: "https://edge.forgecdn.net/files/1/2/mod.jar".into(),
        origin: Origin::Curseforge,
        expected: Expected {
            sha512: None,
            ..expected(&body)
        },
        fallbacks: Vec::new(),
    };
    assert_eq!(
        ensure(&paths, &downloader, &withdrawn, &cancel, &|_| {})
            .await
            .ok(),
        Some(Ensured::Withdrawn)
    );
    assert!(!entry(&paths, &withdrawn.expected).exists());

    // A stale temporary from an interrupted process is removed by the next writer.
    let served = body.clone();
    let good = Server::start(move |_| Reply::ok(served.clone()));
    let downloader = PackDownloader::for_tests(good.base());
    let directory = entry(&paths, &request.expected)
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let stale = directory.join(format!(".private-fs-{}.tmp", Uuid::new_v4().simple()));
    assert!(private_fs::PrivateDir::ensure(&directory).is_ok());
    assert!(
        private_fs::PrivateDir::ensure(&directory)
            .and_then(|dir| dir.create(
                stale
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
            ))
            .is_ok()
    );
    assert_eq!(partials(&paths, &request.expected), 1);
    assert!(
        ensure(&paths, &downloader, &request, &cancel, &|_| {})
            .await
            .is_ok()
    );
    assert_eq!(partials(&paths, &request.expected), 0);
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn concurrent_writers_of_one_entry_download_it_once() {
    let body = vec![7u8; 64 * 1024];
    let served = body.clone();
    let server = Server::start(move |_| Reply::ok(served.clone()).delay(Duration::from_millis(60)));
    let downloader = PackDownloader::for_tests(server.base());
    let paths = root();
    let cancel = Cancellation::default();
    let request = request("mod.jar", &body);
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let (paths, downloader, cancel, request) = (
            paths.clone(),
            downloader.clone(),
            cancel.clone(),
            request.clone(),
        );
        tasks.push(tokio::spawn(async move {
            ensure(&paths, &downloader, &request, &cancel, &|_| {}).await
        }));
    }
    let mut reused = 0;
    for task in tasks {
        match task.await {
            Ok(Ok(Ensured::Ready { reused: true })) => reused += 1,
            Ok(Ok(Ensured::Ready { reused: false })) => {}
            _ => panic!("writer failed"),
        }
    }
    assert_eq!(reused, 3);
    assert_eq!(server.requests().len(), 1);
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn verified_streams_hash_again_and_reject_damage_or_absence() {
    let body = b"copy me into an instance".to_vec();
    let served = body.clone();
    let server = Server::start(move |_| Reply::ok(served.clone()));
    let downloader = PackDownloader::for_tests(server.base());
    let paths = root();
    let cancel = Cancellation::default();
    let request = request("mod.jar", &body);
    assert_eq!(
        stream_verified(&paths, &request.expected, &cancel, |_| Ok(()))
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert!(
        ensure(&paths, &downloader, &request, &cancel, &|_| {})
            .await
            .is_ok()
    );
    let mut copy = Vec::new();
    let result = stream_verified(&paths, &request.expected, &cancel, |chunk| {
        copy.extend_from_slice(chunk);
        Ok(())
    });
    assert!(result.is_ok());
    assert_eq!(copy, body);
    assert!(std::fs::write(entry(&paths, &request.expected), vec![0u8; body.len()]).is_ok());
    assert_eq!(
        stream_verified(&paths, &request.expected, &cancel, |_| Ok(()))
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    cancel.cancel();
    assert_eq!(
        stream_verified(&paths, &request.expected, &cancel, |_| Ok(()))
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::OperationCancelled)
    );
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn many_files_report_progress_and_report_withdrawn_files() {
    let mut bodies: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for index in 0..12 {
        bodies.insert(
            format!("/data/project/versions/version/f{index}.jar"),
            format!("file number {index}").into_bytes(),
        );
    }
    let served = bodies.clone();
    let server = Server::start(move |request| match served.get(&request.path) {
        Some(body) => Reply::ok(body.clone()),
        None => Reply::status(404),
    });
    let downloader = PackDownloader::for_tests(server.base());
    let paths = root();
    let cancel = Cancellation::default();
    let mut requests: Vec<Request> = bodies
        .iter()
        .map(|(path, body)| request(path.rsplit('/').next().unwrap_or_default(), body))
        .collect();
    let gone = b"withdrawn by its author".to_vec();
    requests.push(Request {
        url: "https://edge.forgecdn.net/files/1/2/gone.jar".into(),
        origin: Origin::Curseforge,
        expected: Expected {
            sha512: None,
            ..expected(&gone)
        },
        fallbacks: Vec::new(),
    });
    let last = Arc::new(Mutex::new(PackProgress::default()));
    let seen = last.clone();
    let progress: ProgressSink = Arc::new(move |value| {
        *seen.lock().unwrap_or_else(|poison| poison.into_inner()) = value;
    });
    let results = ensure_all(&paths, &downloader, &requests, &cancel, progress).await;
    let results = results.unwrap_or_default();
    assert_eq!(results.len(), 13);
    assert_eq!(
        results.iter().filter(|r| **r == Ensured::Withdrawn).count(),
        1
    );
    let final_state = *last.lock().unwrap_or_else(|poison| poison.into_inner());
    assert_eq!(final_state.files_done, 13);
    assert_eq!(final_state.files_total, 13);
    assert_eq!(final_state.bytes_done, final_state.bytes_total);
    assert_eq!(final_state.reused_files, 0);

    // A second instance reuses every published entry; the withdrawn one is asked again.
    let reused = Arc::new(AtomicUsize::new(0));
    let counted = reused.clone();
    let progress: ProgressSink = Arc::new(move |value| {
        counted.store(value.reused_files, Ordering::SeqCst);
    });
    assert!(
        ensure_all(&paths, &downloader, &requests, &cancel, progress)
            .await
            .is_ok()
    );
    assert_eq!(reused.load(Ordering::SeqCst), 12);
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn one_failure_aborts_the_batch_and_cancellation_is_honored() {
    let paths = root();
    let server = Server::start(|request| {
        if request.path.ends_with("bad.jar") {
            Reply::ok("wrong")
        } else {
            Reply::ok("fine").delay(Duration::from_millis(30))
        }
    });
    let downloader = PackDownloader::for_tests(server.base());
    let cancel = Cancellation::default();
    let requests = vec![
        request("a.jar", b"fine"),
        request("bad.jar", b"expected something else"),
        request("c.jar", b"fine"),
    ];
    assert_eq!(
        ensure_all(&paths, &downloader, &requests, &cancel, quiet())
            .await
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::ArtifactMismatch)
    );
    cancel.cancel();
    assert_eq!(
        ensure_all(&paths, &downloader, &requests[..1], &cancel, quiet())
            .await
            .err()
            .map(|e| e.code()),
        Some(ErrorCode::OperationCancelled)
    );
    let _ = std::fs::remove_dir_all(paths.root());
}

#[tokio::test]
async fn a_failing_mirror_falls_through_to_the_next_and_cancellation_does_not() {
    let paths = root();
    let cancel = Cancellation::default();
    let body = b"mirrored content".to_vec();
    let served = body.clone();
    let server = Server::start(move |request| match request.path.as_str() {
        // Verifiable but wrong bytes, then an overloaded host, then the real file.
        "/data/project/versions/version/wrong.jar" => Reply::ok(b"someone else's bytes".to_vec()),
        "/data/project/versions/version/busy.jar" => Reply::status(503),
        "/owner/repo/main/good.jar" => Reply::ok(served.clone()),
        _ => Reply::status(404),
    });
    let downloader = PackDownloader::for_tests(server.base());
    let mirrored = Request {
        url: "https://cdn.modrinth.com/data/project/versions/version/wrong.jar".into(),
        origin: Origin::Modrinth,
        expected: expected(&body),
        fallbacks: vec![
            (
                "https://cdn.modrinth.com/data/project/versions/version/busy.jar".into(),
                Origin::Modrinth,
            ),
            (
                "https://raw.githubusercontent.com/owner/repo/main/good.jar".into(),
                Origin::Mirror,
            ),
        ],
    };
    assert_eq!(
        ensure(&paths, &downloader, &mirrored, &cancel, &|_| {})
            .await
            .ok(),
        Some(Ensured::Ready { reused: false })
    );
    assert!(entry(&paths, &mirrored.expected).exists());
    assert_eq!(partials(&paths, &mirrored.expected), 0);
    let hits = |name: &str| {
        server
            .requests()
            .iter()
            .filter(|request| request.path.ends_with(name))
            .count()
    };
    assert_eq!(
        (hits("wrong.jar"), hits("busy.jar"), hits("good.jar")),
        (2, 3, 1)
    );

    // With every mirror failing, the last error is reported and nothing is published.
    let paths = root();
    let hopeless = Request {
        fallbacks: vec![(
            "https://cdn.modrinth.com/data/project/versions/version/busy.jar".into(),
            Origin::Modrinth,
        )],
        ..mirrored.clone()
    };
    let failed = ensure(&paths, &downloader, &hopeless, &cancel, &|_| {}).await;
    assert_eq!(
        failed.err().map(|error| error.code()),
        Some(ErrorCode::NetworkUnavailable)
    );
    assert!(!entry(&paths, &hopeless.expected).exists());
    assert_eq!(partials(&paths, &hopeless.expected), 0);

    // A cancelled operation stops at once instead of trying the next mirror.
    let paths = root();
    let before = server.requests().len();
    let stopped = Cancellation::default();
    stopped.cancel();
    assert_eq!(
        ensure(&paths, &downloader, &mirrored, &stopped, &|_| {})
            .await
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::OperationCancelled)
    );
    assert_eq!(server.requests().len(), before);
}
