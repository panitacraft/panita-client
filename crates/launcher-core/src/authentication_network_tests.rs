//! Isolated network faults: no production endpoints, OS DNS changes or real credentials.
use super::*;
use reqwest::dns::{Name, Resolve, Resolving};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const CERT: &[u8] = include_bytes!("../../../tests/fixtures/network/localhost-cert.pem");
const KEY: &[u8] = include_bytes!("../../../tests/fixtures/network/localhost-key.pem");

fn at_endpoint(client: Client, base: &str) -> AuthClient {
    AuthClient {
        client,
        endpoints: Endpoints {
            microsoft: format!("{base}/token"),
            xbox: format!("{base}/xbox"),
            xsts: format!("{base}/xsts"),
            minecraft: format!("{base}/minecraft"),
            profile: format!("{base}/profile"),
        },
    }
}

struct FailedDns(Arc<AtomicUsize>);

impl Resolve for FailedDns {
    fn resolve(&self, name: Name) -> Resolving {
        assert_eq!(name.as_str(), "panita-fixture.invalid");
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "synthetic DNS failure").into())
        })
    }
}

pub(crate) fn dns_failure_client() -> (AuthClient, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .dns_resolver(Arc::new(FailedDns(calls.clone())))
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| panic!("isolated client"));
    (at_endpoint(client, "https://panita-fixture.invalid"), calls)
}

/// The server must see the certificate-rejection alert, not a successful HTTP request.
pub(crate) fn tls_fixture() -> (String, std::thread::JoinHandle<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| panic!("loopback"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|_| panic!("nonblocking"));
    let address = listener.local_addr().unwrap_or_else(|_| panic!("address"));
    let task = std::thread::spawn(move || {
        let end = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < end, "TLS fixture accept timeout");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => panic!("TLS fixture accept"),
            }
        };
        stream
            .set_nonblocking(false)
            .unwrap_or_else(|_| panic!("blocking connection"));
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap_or_else(|_| panic!("timeout"));
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap_or_else(|_| panic!("timeout"));
        let cert = CertificateDer::from_pem_slice(CERT).unwrap_or_else(|_| panic!("fixture cert"));
        let key = PrivateKeyDer::from_pem_slice(KEY).unwrap_or_else(|_| panic!("fixture key"));
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap_or_else(|_| panic!("TLS versions"))
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap_or_else(|_| panic!("TLS config"));
        let mut connection = rustls::ServerConnection::new(Arc::new(config))
            .unwrap_or_else(|_| panic!("TLS connection"));
        while connection.is_handshaking() {
            if let Err(error) = connection.complete_io(&mut stream) {
                let tls_error = error
                    .get_ref()
                    .and_then(|source| source.downcast_ref::<rustls::Error>());
                assert!(
                    matches!(
                        tls_error,
                        Some(rustls::Error::AlertReceived(
                            rustls::AlertDescription::UnknownCA
                                | rustls::AlertDescription::CertificateUnknown
                        ))
                    ),
                    "expected certificate rejection alert, got {error}"
                );
                return false;
            }
        }
        let mut tls = rustls::StreamOwned::new(connection, stream);
        let mut request = [0; 4096];
        let count = tls
            .read(&mut request)
            .unwrap_or_else(|_| panic!("HTTP request"));
        assert!(count > 0);
        tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .unwrap_or_else(|_| panic!("HTTP response"));
        tls.flush().unwrap_or_else(|_| panic!("flush"));
        true
    });
    (format!("https://{address}"), task)
}

pub(crate) fn tls_failure_client() -> (AuthClient, std::thread::JoinHandle<bool>) {
    let (base, task) = tls_fixture();
    let client = Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| panic!("isolated client"));
    (at_endpoint(client, &base), task)
}

pub(crate) fn callback_404_fixture() -> (String, std::thread::JoinHandle<()>) {
    let (client, task) = super::tests::mock_client(vec![(
        "/callback",
        404,
        "callback fixture not found".to_owned(),
    )]);
    (
        client.endpoints.microsoft.replace("/token", "/callback"),
        task,
    )
}

#[tokio::test]
async fn isolated_dns_failure_is_sanitized_without_transport_retry() {
    let (client, calls) = dns_failure_client();
    let secret = Secret::new("fixture-refresh".to_owned()).unwrap_or_else(|_| panic!("fixture"));
    let error = client
        .renew(&secret)
        .await
        .err()
        .unwrap_or_else(|| panic!("DNS must fail"));
    assert_eq!(error.code(), ErrorCode::NetworkUnavailable);
    let rendered = format!("{error:?}");
    assert!(!rendered.contains("fixture-refresh"));
    assert!(!rendered.contains("panita-fixture.invalid"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn isolated_tls_rejects_untrusted_certificate_before_sending_credentials() {
    let (client, task) = tls_failure_client();
    let secret = Secret::new("fixture-refresh".to_owned()).unwrap_or_else(|_| panic!("fixture"));
    let error = client
        .renew(&secret)
        .await
        .err()
        .unwrap_or_else(|| panic!("TLS must fail"));
    assert_eq!(error.code(), ErrorCode::NetworkUnavailable);
    assert!(!format!("{error:?}").contains("fixture-refresh"));
    assert!(!task.join().unwrap_or_else(|_| panic!("TLS server")));
}

#[tokio::test]
async fn isolated_tls_fixture_can_serve_http_when_explicitly_trusted_in_test_only() {
    // Positive control proves the rejection above is not a broken TLS server.
    let (base, task) = tls_fixture();
    let cert = reqwest::Certificate::from_pem(CERT).unwrap_or_else(|_| panic!("cert"));
    let client = Client::builder()
        .no_proxy()
        .add_root_certificate(cert)
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| panic!("client"));
    let response = client
        .get(base)
        .send()
        .await
        .unwrap_or_else(|error| panic!("trusted fixture: {error:?}"));
    assert_eq!(response.text().await.unwrap_or_default(), "OK");
    assert!(task.join().unwrap_or_else(|_| panic!("TLS server")));
}
