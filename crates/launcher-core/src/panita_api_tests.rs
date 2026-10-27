use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

pub(crate) const KEY: &str = "test-key";
pub(crate) const TOKEN: &str = "ps_fixture-token-value";
pub(crate) const SECRET: &str = "abcdefghijklmnopqrstuvwxyz-_0123456789abcde";
pub(crate) const REQUEST_ID: &str = "3b8f2c1d-6e4a-4f75-9a20-1c7d5e9b0a84";
// Far in the future so a fixture serial never expires while a test runs.
pub(crate) const EXPIRES: &str = "2099-01-01T00:03:00.000Z";

/// One scripted response of the loopback fixture.
pub(crate) struct Canned {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: String,
}

pub(crate) fn canned(status: u16, body: impl Into<String>) -> Canned {
    Canned {
        status,
        headers: Vec::new(),
        body: body.into(),
    }
}

impl Canned {
    pub(crate) fn header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.push((name, value));
        self
    }
}

/// Serves the scripted responses in order and returns every request it saw,
/// lowercased, head and body together.
pub(crate) fn server(
    replies: Vec<Canned>,
) -> (PanitaApiClient, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| panic!("loopback"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|_| panic!("nonblocking"));
    let base = format!(
        "http://{}",
        listener.local_addr().unwrap_or_else(|_| panic!("address"))
    );
    let task = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for reply in replies {
            let until = std::time::Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < until, "fixture accept timeout");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => panic!("accept"),
                }
            };
            stream
                .set_nonblocking(false)
                .unwrap_or_else(|_| panic!("blocking"));
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap_or_else(|_| panic!("timeout"));
            let mut bytes = Vec::new();
            let mut chunk = [0; 1024];
            let head_end = loop {
                if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break position + 4;
                }
                let count = stream.read(&mut chunk).unwrap_or_else(|_| panic!("read"));
                assert!(count > 0 && bytes.len() < 16 * 1024);
                bytes.extend_from_slice(&chunk[..count]);
            };
            let head = String::from_utf8_lossy(&bytes[..head_end]).to_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while bytes.len() < head_end + length {
                let count = stream.read(&mut chunk).unwrap_or_else(|_| panic!("body"));
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
            }
            seen.push(String::from_utf8_lossy(&bytes).to_lowercase());
            let mut response = format!("HTTP/1.1 {} Fixture\r\n", reply.status);
            for (name, value) in &reply.headers {
                response.push_str(&format!("{name}: {value}\r\n"));
            }
            response.push_str(&format!(
                "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.body.len(),
                reply.body
            ));
            stream
                .write_all(response.as_bytes())
                .unwrap_or_else(|_| panic!("write"));
        }
        seen
    });
    let client = PanitaApiClient::build(base, false).unwrap_or_else(|_| panic!("client"));
    (client, task)
}

fn requests(task: std::thread::JoinHandle<Vec<String>>) -> Vec<String> {
    task.join().unwrap_or_else(|_| panic!("fixture"))
}

pub(crate) fn user_json() -> String {
    r#"{"id":42,"ign":"javithor360","discord_name":"javithor360","avatar_url":null,
        "trusted_author":false,"has_password":true,"minecraft_status":"premium",
        "birthday":"1999-03-14","explore_visible":true,
        "capabilities":{"authenticated":true,"admin_or_mod":false,"trusted_author":false,
        "gallery_editor":false,"upload_authorized":false,"wiki_editor":false}}"#
        .to_owned()
}

pub(crate) fn created_json() -> String {
    created_with("5V3J-A5TD-YE4L", SECRET, REQUEST_ID, EXPIRES)
}

fn created_with(code: &str, secret: &str, id: &str, expires: &str) -> String {
    format!(
        r#"{{"data":{{"request_id":"{id}","code":"{code}","poll_secret":"{secret}","expires_at":"{expires}"}}}}"#
    )
}

pub(crate) fn pending_json() -> String {
    format!(r#"{{"data":{{"status":"pending","expires_at":"{EXPIRES}"}}}}"#)
}

pub(crate) fn confirmed_json(token: &str) -> String {
    format!(
        r#"{{"data":{{"status":"confirmed","token":"{token}","expires_at":null,"user":{}}}}}"#,
        user_json()
    )
}

/// A client whose server is gone: every request is a transport failure.
pub(crate) fn closed_client() -> PanitaApiClient {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| panic!("loopback"));
    let base = format!(
        "http://{}",
        listener.local_addr().unwrap_or_else(|_| panic!("address"))
    );
    drop(listener);
    PanitaApiClient::build(base, false).unwrap_or_else(|_| panic!("client"))
}

pub(crate) fn failure_json(reason: &str) -> String {
    format!(
        r#"{{"error":{{"code":"unauthenticated","message":"fixture","details":{{"reason":"{reason}"}}}}}}"#
    )
}

pub(crate) fn credentials() -> PanitaCredentials {
    PanitaCredentials::new(KEY).unwrap_or_else(|_| panic!("credentials"))
}

pub(crate) fn session(token: &str) -> PanitaSession {
    PanitaSession::new(token.to_owned(), crate::panita_session::tests::user(), 1)
        .unwrap_or_else(|_| panic!("session"))
}

fn device() -> DeviceInfo {
    DeviceInfo::new(Some("DESKTOP-ABC"), Some("Windows 11"))
}

async fn create(client: &PanitaApiClient) -> Result<LoginRequest, PanitaApiError> {
    client
        .create_login_request(&credentials(), "  Javithor360 ", &device())
        .await
}

async fn created(client: &PanitaApiClient) -> LoginRequest {
    create(client).await.unwrap_or_else(|_| panic!("created"))
}

#[tokio::test]
async fn creating_a_request_sends_only_the_key_and_the_trimmed_identifier() {
    let (client, task) = server(vec![canned(201, created_json())]);
    // A session in the credentials must never reach an activation request.
    let credentials = credentials().with_session(&session(TOKEN));
    let request = client
        .create_login_request(&credentials, "  Javithor360 ", &device())
        .await
        .unwrap_or_else(|_| panic!("request"));
    assert_eq!(request.serial, "5V3J-A5TD-YE4L");
    assert_eq!(request.request_id.to_string(), REQUEST_ID);
    assert_eq!(request.expires_at, parse_time(EXPIRES).unwrap_or_default());
    assert_eq!(request.poll_secret().expose(), SECRET);
    let seen = requests(task);
    assert!(seen[0].starts_with("post /v1/auth/login-requests http/1.1"));
    assert!(seen[0].contains("x-api-key: test-key"));
    assert!(!seen[0].contains("authorization:") && !seen[0].contains(TOKEN));
    assert!(seen[0].contains(r#""identifier":"javithor360""#));
    assert!(seen[0].contains(r#""device_name":"desktop-abc""#));
    assert!(seen[0].contains(r#""device_os":"windows 11""#));
}

#[tokio::test]
async fn missing_device_text_is_omitted_from_the_body() {
    let (client, task) = server(vec![canned(201, created_json())]);
    let result = client
        .create_login_request(&credentials(), "javithor360", &DeviceInfo::default())
        .await;
    assert!(result.is_ok());
    let seen = requests(task);
    assert!(!seen[0].contains("device_name") && !seen[0].contains("device_os"));
}

#[tokio::test]
async fn invalid_identifiers_fail_before_any_request() {
    let (client, task) = server(Vec::new());
    for identifier in ["", "   ", "line\nbreak", &"a".repeat(101)] {
        let error = client
            .create_login_request(&credentials(), identifier, &device())
            .await
            .err();
        assert_eq!(error.map(|e| e.code), Some(ErrorCode::InvalidUsername));
    }
    assert!(requests(task).is_empty());
}

#[tokio::test]
async fn malformed_creation_responses_are_invalid_data() {
    let long_secret = format!("{SECRET}a");
    let bad = [
        created_with("5V3J-A5TD-YE40", SECRET, REQUEST_ID, EXPIRES),
        created_with("5V3JA5TDYE4L", SECRET, REQUEST_ID, EXPIRES),
        created_with("1234", SECRET, REQUEST_ID, EXPIRES),
        created_with("5V3J-A5TD-YE4L", "short", REQUEST_ID, EXPIRES),
        created_with("5V3J-A5TD-YE4L", &long_secret, REQUEST_ID, EXPIRES),
        created_with(
            "5V3J-A5TD-YE4L",
            SECRET,
            "00000000-0000-0000-0000-000000000000",
            EXPIRES,
        ),
        created_with("5V3J-A5TD-YE4L", SECRET, REQUEST_ID, "tomorrow"),
        "{}".to_owned(),
    ];
    for body in bad {
        let (client, task) = server(vec![canned(201, body)]);
        let error = create(&client).await.err();
        assert_eq!(error.map(|e| e.code), Some(ErrorCode::InvalidData));
        requests(task);
    }
}

#[tokio::test]
async fn polling_waits_and_then_delivers_the_session_once() {
    let (client, task) = server(vec![
        canned(201, created_json()),
        canned(200, pending_json()),
        canned(200, confirmed_json(TOKEN)),
    ]);
    let request = created(&client).await;
    let pending = client.poll(&credentials(), &request).await;
    assert!(matches!(
        pending,
        Ok(PollOutcome::Pending { expires_at }) if expires_at == request.expires_at
    ));
    let Ok(PollOutcome::Confirmed(session)) = client.poll(&credentials(), &request).await else {
        panic!("confirmed");
    };
    assert_eq!(session.token().expose(), TOKEN);
    assert_eq!(session.user.id, 42);
    assert_eq!(session.user.ign.as_deref(), Some("javithor360"));
    assert_eq!(
        session.user.minecraft_status,
        Some(MinecraftStatus::Premium)
    );
    assert!(session.verified_at > 0);
    let seen = requests(task);
    for poll in &seen[1..] {
        assert!(poll.starts_with(&format!("post /v1/auth/login-requests/{REQUEST_ID}/poll ")));
        assert!(poll.contains(&format!(r#""poll_secret":"{SECRET}""#)));
        assert!(poll.contains("x-api-key: test-key") && !poll.contains("authorization:"));
    }
}

#[tokio::test]
async fn confirmation_with_a_malformed_token_or_unknown_status_is_handled() {
    let (client, task) = server(vec![
        canned(201, created_json()),
        canned(200, confirmed_json("plain-token")),
        canned(
            200,
            confirmed_json(TOKEN).replace("premium", "somethingnew"),
        ),
    ]);
    let request = created(&client).await;
    let rejected = client.poll(&credentials(), &request).await.err();
    assert_eq!(rejected.map(|e| e.code), Some(ErrorCode::InvalidData));
    let Ok(PollOutcome::Confirmed(session)) = client.poll(&credentials(), &request).await else {
        panic!("confirmed");
    };
    assert_eq!(session.user.minecraft_status, None);
    requests(task);
}

#[tokio::test]
async fn identity_headers_and_error_codes_are_mapped_per_operation() {
    enum Call {
        Create,
        Poll,
        Me,
        SignOut,
    }
    let unauthenticated = |reason: &str| (401, failure_json(reason));
    let cases: Vec<(Call, (u16, String), ErrorCode)> = vec![
        (
            Call::Create,
            unauthenticated("unknown_account"),
            ErrorCode::ActivationUnknownAccount,
        ),
        (
            Call::Create,
            unauthenticated("not_activated"),
            ErrorCode::ActivationNotActivated,
        ),
        (
            Call::Create,
            unauthenticated("invalid_key"),
            ErrorCode::ApiKeyRejected,
        ),
        (
            Call::Create,
            unauthenticated("invalid_token"),
            ErrorCode::InvalidData,
        ),
        (
            Call::Create,
            (403, String::new()),
            ErrorCode::ApiKeyRejected,
        ),
        (
            Call::Create,
            (400, failure_json("invalid_request")),
            ErrorCode::InvalidData,
        ),
        (
            Call::Create,
            (500, String::new()),
            ErrorCode::ServiceUnavailable,
        ),
        (
            Call::Create,
            (429, "<html>blocked</html>".to_owned()),
            ErrorCode::ActivationRateLimited,
        ),
        (
            Call::Poll,
            (410, failure_json("expired")),
            ErrorCode::ActivationExpired,
        ),
        (
            Call::Poll,
            (410, failure_json("consumed")),
            ErrorCode::ActivationConsumed,
        ),
        (
            Call::Poll,
            (404, String::new()),
            ErrorCode::ActivationConsumed,
        ),
        (
            Call::Poll,
            unauthenticated("not_activated"),
            ErrorCode::ActivationNotActivated,
        ),
        (
            Call::Poll,
            unauthenticated("invalid_key"),
            ErrorCode::ApiKeyRejected,
        ),
        (
            Call::Poll,
            (503, String::new()),
            ErrorCode::ServiceUnavailable,
        ),
        (
            Call::Me,
            unauthenticated("invalid_token"),
            ErrorCode::SessionRejected,
        ),
        // A rotated key must never read as a closed session.
        (
            Call::Me,
            unauthenticated("invalid_key"),
            ErrorCode::ApiKeyRejected,
        ),
        (
            Call::Me,
            (401, "<html>proxy</html>".to_owned()),
            ErrorCode::InvalidData,
        ),
        (Call::Me, (401, "{}".to_owned()), ErrorCode::InvalidData),
        (
            Call::Me,
            unauthenticated("not_activated"),
            ErrorCode::InvalidData,
        ),
        (Call::Me, (403, String::new()), ErrorCode::ApiKeyRejected),
        (Call::Me, (404, String::new()), ErrorCode::InvalidData),
        (
            Call::Me,
            (502, String::new()),
            ErrorCode::ServiceUnavailable,
        ),
        (
            Call::Me,
            (200, "not json".to_owned()),
            ErrorCode::InvalidData,
        ),
        (
            Call::SignOut,
            unauthenticated("invalid_token"),
            ErrorCode::SessionRejected,
        ),
        (
            Call::SignOut,
            (500, String::new()),
            ErrorCode::ServiceUnavailable,
        ),
    ];
    for (call, (status, body), expected) in cases {
        let mut script = Vec::new();
        if matches!(call, Call::Poll) {
            script.push(canned(201, created_json()));
        }
        script.push(canned(status, body));
        let (client, task) = server(script);
        let signed_in = credentials().with_session(&session(TOKEN));
        let error = match call {
            Call::Create => create(&client).await.err(),
            Call::Poll => {
                let request = created(&client).await;
                client.poll(&credentials(), &request).await.err()
            }
            Call::Me => client.me(&signed_in).await.err(),
            Call::SignOut => client.sign_out(&signed_in).await.err(),
        };
        assert_eq!(error.map(|e| e.code), Some(expected), "status {status}");
        requests(task);
    }
}

#[tokio::test]
async fn retry_after_is_clamped_and_optional() {
    for (header, expected) in [
        (Some("42"), Some(42)),
        (Some("9999"), Some(600)),
        (Some("0"), Some(1)),
        (Some("soon"), None),
        (None, None),
    ] {
        let mut reply = canned(429, failure_json("rate_limited"));
        if let Some(value) = header {
            reply = reply.header("Retry-After", value);
        }
        let (client, task) = server(vec![reply]);
        let error = create(&client).await.err();
        assert_eq!(
            error.map(|e| e.code),
            Some(ErrorCode::ActivationRateLimited)
        );
        assert_eq!(error.and_then(|e| e.retry_after), expected);
        requests(task);
    }
}

#[tokio::test]
async fn me_sends_key_and_bearer_and_keeps_only_the_snapshot() {
    let (client, task) = server(vec![canned(200, format!(r#"{{"data":{}}}"#, user_json()))]);
    let signed_in = credentials().with_session(&session(TOKEN));
    let user = client
        .me(&signed_in)
        .await
        .unwrap_or_else(|_| panic!("user"));
    assert_eq!(user.discord_name, "javithor360");
    assert!(user.capabilities.authenticated);
    // The date of birth is personal data the launcher never keeps.
    assert!(!format!("{user:?}").contains("1999"));
    let seen = requests(task);
    assert!(seen[0].starts_with("get /v1/auth/me http/1.1"));
    assert!(seen[0].contains("x-api-key: test-key"));
    assert!(seen[0].contains(&format!("authorization: bearer {TOKEN}")));
}

#[tokio::test]
async fn sign_out_deletes_the_current_session() {
    let (client, task) = server(vec![canned(204, "")]);
    let signed_in = credentials().with_session(&session(TOKEN));
    assert!(client.sign_out(&signed_in).await.is_ok());
    let seen = requests(task);
    assert!(seen[0].starts_with("delete /v1/auth/session http/1.1"));
    assert!(seen[0].contains(&format!("authorization: bearer {TOKEN}")));
}

#[tokio::test]
async fn session_calls_without_a_session_make_no_request() {
    let (client, task) = server(Vec::new());
    let anonymous = credentials();
    assert_eq!(
        client.me(&anonymous).await.err().map(|e| e.code),
        Some(ErrorCode::ActivationRequired)
    );
    assert_eq!(
        client.sign_out(&anonymous).await.err().map(|e| e.code),
        Some(ErrorCode::ActivationRequired)
    );
    assert!(requests(task).is_empty());
}

#[tokio::test]
async fn transport_failure_and_oversized_bodies_are_not_rejections() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| panic!("loopback"));
    let base = format!(
        "http://{}",
        listener.local_addr().unwrap_or_else(|_| panic!("address"))
    );
    drop(listener);
    let closed = PanitaApiClient::build(base, false).unwrap_or_else(|_| panic!("client"));
    let signed_in = credentials().with_session(&session(TOKEN));
    assert_eq!(
        closed.me(&signed_in).await.err().map(|e| e.code),
        Some(ErrorCode::NetworkUnavailable)
    );
    let (client, task) = server(vec![canned(200, "x".repeat(MAX_RESPONSE + 1))]);
    assert_eq!(
        client.me(&signed_in).await.err().map(|e| e.code),
        Some(ErrorCode::InvalidData)
    );
    requests(task);
}

#[test]
fn credentials_require_a_plausible_public_key() {
    for key in ["", "has space", "line\nbreak", &"k".repeat(129)] {
        assert_eq!(
            PanitaCredentials::new(key).err().map(|e| e.code()),
            Some(ErrorCode::ActivationUnavailable)
        );
    }
    assert!(!credentials().has_session());
    assert!(credentials().with_session(&session(TOKEN)).has_session());
}

#[test]
fn device_text_is_reduced_to_a_safe_alphabet() {
    let cleaned = DeviceInfo::new(Some("  DESKTOP-ABC\u{202e}\n\t "), Some("Windows 11"));
    assert_eq!(cleaned.name(), Some("DESKTOP-ABC"));
    assert_eq!(cleaned.os(), Some("Windows 11"));
    let long = DeviceInfo::new(Some(&"a".repeat(100)), None);
    assert_eq!(long.name().map(str::len), Some(DEVICE_LIMIT));
    assert_eq!(long.os(), None);
    for empty in ["", "   ", "\u{200b}\u{2028}", "\u{1f600}"] {
        assert_eq!(
            DeviceInfo::new(Some(empty), Some(empty)),
            DeviceInfo::default()
        );
    }
    assert_eq!(
        DeviceInfo::new(None, Some("Debian GNU/Linux 12 (bookworm)")).os(),
        Some("Debian GNU/Linux 12 (bookworm)")
    );
    assert_eq!(
        DeviceInfo::new(Some("<@everyone> pc"), None).name(),
        Some("everyone pc")
    );
}
