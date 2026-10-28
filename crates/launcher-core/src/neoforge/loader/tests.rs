use super::*;
use crate::test_http::{Reply, Server};

const PATH: &str = "/net/neoforged/neoforge/21.1.264/neoforge-21.1.264-installer.jar";

fn sha1(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

fn code<T>(result: Result<T, CoreError>) -> Option<ErrorCode> {
    result.err().map(|error| error.code())
}

async fn installer(
    server: &Server,
    version: &str,
) -> Result<(InstallerIdentity, Vec<u8>), CoreError> {
    Maven::for_tests(server.base())?
        .installer("1.21.1", version, &Cancellation::default())
        .await
}

#[tokio::test]
async fn the_identity_is_the_published_sha1_of_the_exact_bytes_downloaded() {
    let bytes = b"official installer bytes".to_vec();
    let (served, hash) = (bytes.clone(), sha1(&bytes));
    let server = Server::start(move |request| {
        if request.path == format!("{PATH}.sha1") {
            Reply::ok(format!("{hash}\n"))
        } else if request.path == PATH {
            Reply::ok(served.clone())
        } else {
            Reply::status(404)
        }
    });
    let result = installer(&server, "21.1.264").await;
    let Ok((identity, downloaded)) = result else {
        panic!("installer");
    };
    assert_eq!(downloaded, bytes);
    assert_eq!(identity.sha1, sha1(&bytes));
    assert_eq!(identity.size, bytes.len() as u64);
    assert_eq!(
        (identity.minecraft.as_str(), identity.version.as_str()),
        ("1.21.1", "21.1.264")
    );
    assert!(
        identity
            .url()
            .is_ok_and(|url| url.ends_with("neoforge-21.1.264-installer.jar"))
    );
    // The sidecar is only trusted through the official HTTPS host, never alone:
    // the bytes must hash to it.
    for request in server.requests() {
        assert!(!request.headers.contains("authorization"));
    }
}

#[tokio::test]
async fn a_mismatched_hostile_or_missing_sidecar_never_yields_an_identity() {
    let bytes = b"official installer bytes".to_vec();
    let served = bytes.clone();
    let wrong = Server::start(move |request| {
        if request.path.ends_with(".sha1") {
            Reply::ok("0".repeat(40))
        } else {
            Reply::ok(served.clone())
        }
    });
    assert_eq!(
        code(installer(&wrong, "21.1.264").await),
        Some(ErrorCode::ArtifactMismatch)
    );

    for body in ["not a hash", "ABCDEF", "", "e".repeat(300).as_str()] {
        let served = body.to_owned();
        let bad = Server::start(move |_| Reply::ok(served.clone()));
        assert!(installer(&bad, "21.1.264").await.is_err(), "{body}");
    }

    let missing = Server::start(|_| Reply::status(404));
    assert_eq!(
        code(installer(&missing, "21.1.264").await),
        Some(ErrorCode::ServiceUnavailable)
    );

    let hash = sha1(&bytes);
    let oversized = Server::start(move |request| {
        if request.path.ends_with(".sha1") {
            Reply::ok(hash.clone())
        } else {
            Reply::ok("small").declared(MAX_INSTALLER_BYTES + 1)
        }
    });
    assert_eq!(
        code(installer(&oversized, "21.1.264").await),
        Some(ErrorCode::InvalidData)
    );

    let never = Server::start(|_| Reply::ok("unused"));
    for version in ["../escape", "21.1.264/../x", "", "bad version"] {
        assert!(installer(&never, version).await.is_err(), "{version}");
    }
    assert!(never.requests().is_empty());
}

#[tokio::test]
async fn transient_failures_retry_and_cancellation_stops() {
    let bytes = b"official installer bytes".to_vec();
    let (served, hash) = (bytes.clone(), sha1(&bytes));
    let flaky = Server::start(move |request| {
        if request.hit == 0 && request.path.ends_with(".sha1") {
            Reply::status(503)
        } else if request.path.ends_with(".sha1") {
            Reply::ok(hash.clone())
        } else {
            Reply::ok(served.clone())
        }
    });
    assert!(installer(&flaky, "21.1.264").await.is_ok());

    let slow = Server::start(|_| Reply::ok("late").delay(std::time::Duration::from_millis(300)));
    let cancel = Cancellation::default();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        trigger.cancel();
    });
    let result = Maven::for_tests(slow.base())
        .unwrap_or_else(|_| panic!("maven"))
        .installer("1.21.1", "21.1.264", &cancel)
        .await;
    assert_eq!(code(result), Some(ErrorCode::OperationCancelled));
}
