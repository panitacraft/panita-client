use super::*;
use crate::test_http::{Reply, Server};

const MODRINTH: &str = "https://cdn.modrinth.com/data/project/versions/version/file.jar";
const CURSEFORGE: &str = "https://edge.forgecdn.net/files/1234/5678/file.jar";
const GITHUB: &str = "https://github.com/owner/repo/releases/download/tag/asset.jar";
const RAW: &str = "https://raw.githubusercontent.com/owner/repo/main/file.jar";
const UPLOAD: &str = "https://jqleaywgcoedgvglrvhc.supabase.co/storage/v1/object/public/modpacks/by-hash/sha256-abc/defaults.zip";

pub(super) fn expected(body: &[u8]) -> Expected {
    Expected {
        size: body.len() as u64,
        sha1: Some(format!("{:x}", Sha1::digest(body))),
        sha256: None,
        sha512: Some(format!("{:x}", Sha512::digest(body))),
    }
}

async fn fetch(
    downloader: &PackDownloader,
    url: &str,
    origin: Origin,
    expected: &Expected,
    cancel: &Cancellation,
) -> Result<Option<Vec<u8>>, CoreError> {
    match downloader
        .download(url, &origin, expected, cancel, &|_| {}, || Ok(Vec::new()))
        .await?
    {
        Fetch::Done(bytes) => Ok(Some(bytes)),
        Fetch::Withdrawn => Ok(None),
    }
}

fn code<T>(result: Result<T, CoreError>) -> Option<ErrorCode> {
    result.err().map(|error| error.code())
}

#[tokio::test]
async fn every_origin_downloads_verified_bytes_without_credentials() {
    let body = b"origin bytes".to_vec();
    let archive = Expected::archive(&format!("{:x}", Sha256::digest(&body)), body.len() as u64);
    let sha1_only = Expected {
        sha512: None,
        ..expected(&body)
    };
    let served = body.clone();
    let server = Server::start(move |_| Reply::ok(served.clone()));
    let downloader = PackDownloader::for_tests(server.base());
    let cancel = Cancellation::default();
    for (url, origin, expected) in [
        (MODRINTH, Origin::Modrinth, expected(&body)),
        (CURSEFORGE, Origin::Curseforge, sha1_only),
        (GITHUB, Origin::Github, expected(&body)),
        (RAW, Origin::Mirror, expected(&body)),
        (UPLOAD, Origin::Upload, archive),
    ] {
        let bytes = fetch(&downloader, url, origin, &expected, &cancel).await;
        assert_eq!(bytes.unwrap_or_default(), Some(body.clone()), "{url}");
    }
    for request in server.requests() {
        for forbidden in ["authorization", "x-api-key", "cookie"] {
            assert!(!request.headers.contains(forbidden), "{forbidden}");
        }
    }
}

#[tokio::test]
async fn urls_are_validated_before_any_connection() {
    let server = Server::start(|_| Reply::ok("never"));
    let downloader = PackDownloader::for_tests(server.base());
    let cancel = Cancellation::default();
    let body = expected(b"never");
    for (url, origin) in [
        (
            "http://cdn.modrinth.com/data/p/versions/v/a.jar",
            Origin::Modrinth,
        ),
        (
            "https://cdn.modrinth.com.evil.test/data/p/versions/v/a.jar",
            Origin::Modrinth,
        ),
        (
            "https://objects.githubusercontent.com/a/b/c/d/e",
            Origin::Github,
        ),
        (MODRINTH, Origin::Curseforge),
        (
            "https://other.supabase.co/storage/v1/object/public/modpacks/a/b/c",
            Origin::Upload,
        ),
    ] {
        assert_eq!(
            code(fetch(&downloader, url, origin, &body, &cancel).await),
            Some(ErrorCode::InvalidData),
            "{url}"
        );
    }
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn github_redirects_stay_on_content_hosts_and_are_bounded() {
    let body = b"release asset".to_vec();
    let served = body.clone();
    let server = Server::start(move |request| match request.path.as_str() {
        "/owner/repo/releases/download/tag/asset.jar" => Reply::status(302).header(
            "Location",
            "https://objects.githubusercontent.com/blob/one?signature=secret",
        ),
        "/blob/one" => Reply::ok(served.clone()),
        "/chain/repo/releases/download/tag/asset.jar" => {
            Reply::status(302).header("Location", "https://a.githubusercontent.com/chain/1")
        }
        "/three/repo/releases/download/tag/asset.jar" => {
            Reply::status(302).header("Location", "https://a.githubusercontent.com/three/1")
        }
        "/three/1" => {
            Reply::status(302).header("Location", "https://b.githubusercontent.com/three/2")
        }
        "/three/2" => {
            Reply::status(302).header("Location", "https://c.githubusercontent.com/three/3")
        }
        "/three/3" => Reply::ok(served.clone()),
        "/chain/1" => {
            Reply::status(302).header("Location", "https://b.githubusercontent.com/chain/2")
        }
        "/chain/2" => {
            Reply::status(302).header("Location", "https://c.githubusercontent.com/chain/3")
        }
        "/chain/3" => {
            Reply::status(302).header("Location", "https://d.githubusercontent.com/chain/4")
        }
        "/chain/4" => Reply::ok(served.clone()),
        _ => Reply::status(302).header("Location", "https://evil.test/escape"),
    });
    let downloader = PackDownloader::for_tests(server.base());
    let cancel = Cancellation::default();
    assert_eq!(
        fetch(
            &downloader,
            GITHUB,
            Origin::Github,
            &expected(&body),
            &cancel
        )
        .await
        .unwrap_or_default(),
        Some(body.clone())
    );
    for location in [
        "https://evil.test/escape",
        "http://objects.githubusercontent.com/blob/one",
        "https://user@objects.githubusercontent.com/blob/one",
        "https://github.com/other",
        "/relative",
    ] {
        let escape = location.to_owned();
        let hostile = Server::start(move |_| Reply::status(302).header("Location", &escape));
        let downloader = PackDownloader::for_tests(hostile.base());
        assert_eq!(
            code(
                fetch(
                    &downloader,
                    GITHUB,
                    Origin::Github,
                    &expected(&body),
                    &cancel
                )
                .await
            ),
            Some(ErrorCode::InvalidData),
            "{location}"
        );
        assert_eq!(hostile.requests().len(), 1, "{location}");
    }
    // Three hops are the limit: one chain fits, the other needs four.
    let three = "https://github.com/three/repo/releases/download/tag/asset.jar";
    assert_eq!(
        fetch(
            &downloader,
            three,
            Origin::Github,
            &expected(&body),
            &cancel
        )
        .await
        .unwrap_or_default(),
        Some(body.clone())
    );
    let chain = "https://github.com/chain/repo/releases/download/tag/asset.jar";
    assert_eq!(
        code(
            fetch(
                &downloader,
                chain,
                Origin::Github,
                &expected(&body),
                &cancel
            )
            .await
        ),
        Some(ErrorCode::InvalidData)
    );
}

#[tokio::test]
async fn curseforge_redirects_stay_on_its_file_hosts_and_are_bounded() {
    let body = b"curseforge file".to_vec();
    let served = body.clone();
    let server = Server::start(move |request| match request.path.as_str() {
        "/files/1234/5678/file.jar" => Reply::status(302).header(
            "Location",
            "https://mediafilez.forgecdn.net/files/1234/5678/file.jar/final",
        ),
        "/files/1234/5678/file.jar/final" => Reply::ok(served.clone()),
        _ => Reply::status(302).header("Location", "https://mediafilez.forgecdn.net/files/a/b/c"),
    });
    let downloader = PackDownloader::for_tests(server.base());
    let cancel = Cancellation::default();
    let sha1_only = Expected {
        sha512: None,
        ..expected(&body)
    };
    assert_eq!(
        fetch(
            &downloader,
            CURSEFORGE,
            Origin::Curseforge,
            &sha1_only,
            &cancel
        )
        .await
        .unwrap_or_default(),
        Some(body.clone())
    );
    for location in [
        "https://evil.test/files/1/2/f.jar",
        "http://mediafilez.forgecdn.net/files/1/2/f.jar",
        "https://user@mediafilez.forgecdn.net/files/1/2/f.jar",
        "https://mediafilez.forgecdn.net/other/1/2/f.jar",
        "https://objects.githubusercontent.com/blob",
        "/relative",
    ] {
        let escape = location.to_owned();
        let hostile = Server::start(move |_| Reply::status(302).header("Location", &escape));
        let downloader = PackDownloader::for_tests(hostile.base());
        assert_eq!(
            code(
                fetch(
                    &downloader,
                    CURSEFORGE,
                    Origin::Curseforge,
                    &sha1_only,
                    &cancel
                )
                .await
            ),
            Some(ErrorCode::InvalidData),
            "{location}"
        );
        assert_eq!(hostile.requests().len(), 1, "{location}");
    }
    // An endless redirect loop on the allowed host stops at the hop limit.
    let looping = Server::start(|_| {
        Reply::status(302).header("Location", "https://mediafilez.forgecdn.net/files/1/2/loop")
    });
    let downloader = PackDownloader::for_tests(looping.base());
    assert_eq!(
        code(
            fetch(
                &downloader,
                CURSEFORGE,
                Origin::Curseforge,
                &sha1_only,
                &cancel
            )
            .await
        ),
        Some(ErrorCode::InvalidData)
    );
    assert_eq!(looping.requests().len(), MAX_REDIRECTS + 1);
}

#[tokio::test]
async fn only_github_and_curseforge_may_redirect() {
    let server = Server::start(|_| {
        Reply::status(302).header("Location", "https://objects.githubusercontent.com/blob")
    });
    let downloader = PackDownloader::for_tests(server.base());
    let cancel = Cancellation::default();
    for (url, origin) in [
        (MODRINTH, Origin::Modrinth),
        (CURSEFORGE, Origin::Curseforge),
        (UPLOAD, Origin::Upload),
        (RAW, Origin::Mirror),
    ] {
        assert_eq!(
            code(fetch(&downloader, url, origin, &expected(b"x"), &cancel).await),
            Some(ErrorCode::InvalidData),
            "{url}"
        );
    }
    assert_eq!(server.requests().len(), 4);
}

#[tokio::test]
async fn withdrawn_curseforge_files_are_reported_but_other_missing_files_fail() {
    let cancel = Cancellation::default();
    for status in [403, 404, 410] {
        let server = Server::start(move |_| Reply::status(status));
        let downloader = PackDownloader::for_tests(server.base());
        assert_eq!(
            fetch(
                &downloader,
                CURSEFORGE,
                Origin::Curseforge,
                &expected(b"x"),
                &cancel
            )
            .await
            .unwrap_or(Some(Vec::new())),
            None,
            "{status}"
        );
        assert_eq!(server.requests().len(), 1);
        assert_eq!(
            code(
                fetch(
                    &downloader,
                    MODRINTH,
                    Origin::Modrinth,
                    &expected(b"x"),
                    &cancel
                )
                .await
            ),
            Some(ErrorCode::ServiceUnavailable),
            "{status}"
        );
    }
}

#[tokio::test]
async fn transient_statuses_retry_a_bounded_number_of_times() {
    let body = b"eventually".to_vec();
    let served = body.clone();
    let server = Server::start(move |request| {
        if request.hit < 2 {
            Reply::status(503)
        } else {
            Reply::ok(served.clone())
        }
    });
    let downloader = PackDownloader::for_tests(server.base());
    let cancel = Cancellation::default();
    assert_eq!(
        fetch(
            &downloader,
            MODRINTH,
            Origin::Modrinth,
            &expected(&body),
            &cancel
        )
        .await
        .unwrap_or_default(),
        Some(body.clone())
    );
    assert_eq!(server.requests().len(), 3);

    let always = Server::start(|_| Reply::status(500));
    let downloader = PackDownloader::for_tests(always.base());
    assert_eq!(
        code(
            fetch(
                &downloader,
                MODRINTH,
                Origin::Modrinth,
                &expected(&body),
                &cancel
            )
            .await
        ),
        Some(ErrorCode::NetworkUnavailable)
    );
    assert_eq!(always.requests().len(), TRANSIENT_ATTEMPTS);

    let served = body.clone();
    let limited = Server::start(move |request| {
        if request.hit == 0 {
            Reply::status(429).header("Retry-After", "2")
        } else {
            Reply::ok(served.clone())
        }
    });
    let downloader = PackDownloader::for_tests(limited.base());
    assert_eq!(
        fetch(
            &downloader,
            MODRINTH,
            Origin::Modrinth,
            &expected(&body),
            &cancel
        )
        .await
        .unwrap_or_default(),
        Some(body)
    );
    assert_eq!(limited.requests().len(), 2);
}

#[test]
fn retry_after_hints_are_parsed_and_clamped() {
    assert_eq!(crate::panita_api::retry_after_seconds(Some("2")), Some(2));
    assert_eq!(
        crate::panita_api::retry_after_seconds(Some("999999")),
        Some(600)
    );
    assert_eq!(crate::panita_api::retry_after_seconds(Some("0")), Some(1));
    assert_eq!(
        crate::panita_api::retry_after_seconds(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
        None
    );
}

#[tokio::test]
async fn corrupt_truncated_and_oversized_transfers_never_succeed() {
    let body = b"the exact release".to_vec();
    let want = expected(&body);
    let cancel = Cancellation::default();

    let wrong = Server::start(|_| Reply::ok("the wrong release"));
    let downloader = PackDownloader::for_tests(wrong.base());
    assert_eq!(
        code(fetch(&downloader, MODRINTH, Origin::Modrinth, &want, &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
    assert_eq!(wrong.requests().len(), INTEGRITY_ATTEMPTS);

    let served = body.clone();
    let flaky = Server::start(move |request| {
        if request.hit == 0 {
            Reply::ok("the wrong release")
        } else {
            Reply::ok(served.clone())
        }
    });
    let downloader = PackDownloader::for_tests(flaky.base());
    assert_eq!(
        fetch(&downloader, MODRINTH, Origin::Modrinth, &want, &cancel)
            .await
            .unwrap_or_default(),
        Some(body.clone())
    );

    let served = body.clone();
    let truncated = Server::start(move |request| {
        if request.hit == 0 {
            Reply::ok(served[..4].to_vec()).declared(served.len())
        } else {
            Reply::ok(served.clone())
        }
    });
    let downloader = PackDownloader::for_tests(truncated.base());
    assert_eq!(
        fetch(&downloader, MODRINTH, Origin::Modrinth, &want, &cancel)
            .await
            .unwrap_or_default(),
        Some(body.clone())
    );

    let oversized = Server::start(|_| Reply::ok("the exact release plus more bytes"));
    let downloader = PackDownloader::for_tests(oversized.base());
    assert_eq!(
        code(fetch(&downloader, MODRINTH, Origin::Modrinth, &want, &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );

    // Every supplied hash must match, not only the strongest one.
    let served = body.clone();
    let one_bad_hash = Server::start(move |_| Reply::ok(served.clone()));
    let downloader = PackDownloader::for_tests(one_bad_hash.base());
    let mut partial = want.clone();
    partial.sha1 = Some("0".repeat(40));
    assert_eq!(
        code(fetch(&downloader, MODRINTH, Origin::Modrinth, &partial, &cancel).await),
        Some(ErrorCode::ArtifactMismatch)
    );
}

#[tokio::test]
async fn cancellation_stops_waiting_transfers() {
    let slow = Server::start(|_| Reply::ok("late").delay(Duration::from_millis(400)));
    let downloader = PackDownloader::for_tests(slow.base());
    let cancel = Cancellation::default();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        trigger.cancel();
    });
    assert_eq!(
        code(
            fetch(
                &downloader,
                MODRINTH,
                Origin::Modrinth,
                &expected(b"late"),
                &cancel
            )
            .await
        ),
        Some(ErrorCode::OperationCancelled)
    );
    assert_eq!(
        code(
            fetch(
                &downloader,
                MODRINTH,
                Origin::Modrinth,
                &expected(b"late"),
                &cancel
            )
            .await
        ),
        Some(ErrorCode::OperationCancelled)
    );
}

#[test]
fn expected_values_must_carry_a_valid_hash_and_bounded_size() {
    let good = expected(b"x");
    assert!(good.validate().is_ok());
    for bad in [
        Expected {
            sha1: None,
            sha512: None,
            ..good.clone()
        },
        Expected {
            sha1: Some("A".repeat(40)),
            ..good.clone()
        },
        Expected {
            sha512: Some("a".repeat(127)),
            ..good.clone()
        },
        Expected {
            size: MAX_FILE_BYTES + 1,
            ..good.clone()
        },
        Expected::archive("short", 1),
        Expected::archive(&"a".repeat(64), MAX_ARCHIVE_BYTES + 1),
    ] {
        assert!(bad.validate().is_err());
    }
    assert!(
        Expected::archive(&"a".repeat(64), MAX_ARCHIVE_BYTES)
            .validate()
            .is_ok()
    );
}

struct FullDisk;
impl Write for FullDisk {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_full_disk_fails_the_transfer_without_retrying() {
    let server = Server::start(|_| Reply::ok("some bytes"));
    let downloader = PackDownloader::for_tests(server.base());
    let result = downloader
        .download(
            MODRINTH,
            &Origin::Modrinth,
            &expected(b"some bytes"),
            &Cancellation::default(),
            &|_| {},
            || Ok(FullDisk),
        )
        .await;
    assert_eq!(code(result), Some(ErrorCode::InsufficientSpace));
    assert_eq!(server.requests().len(), 1);
}
