//! Bounded, nonredirecting service requests. Service bodies never become errors.
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use reqwest::{Client, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::accounts::{OAUTH_CLIENT_ID, Secret};
use crate::error::{CoreError, ErrorCode};
use crate::oauth::{CodeExchange, SCOPES};

const MAX_RESPONSE: usize = 128 * 1024;

#[derive(Clone)]
struct Endpoints {
    microsoft: String,
    xbox: String,
    xsts: String,
    minecraft: String,
    profile: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            microsoft: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token".to_owned(),
            xbox: "https://user.auth.xboxlive.com/user/authenticate".to_owned(),
            xsts: "https://xsts.auth.xboxlive.com/xsts/authorize".to_owned(),
            minecraft: "https://api.minecraftservices.com/authentication/login_with_xbox"
                .to_owned(),
            profile: "https://api.minecraftservices.com/minecraft/profile".to_owned(),
        }
    }
}

pub struct AuthClient {
    client: Client,
    endpoints: Endpoints,
}

pub struct MicrosoftToken {
    access: Secret,
    pub refresh: Option<Secret>,
    expires_at: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JavaProfile {
    pub id: Uuid,
    pub name: String,
}

pub struct MinecraftSession {
    pub profile: JavaProfile,
    access: Secret,
    expires_at: Instant,
    expires_wall: SystemTime,
}

impl MinecraftSession {
    pub fn needs_renewal(&self) -> bool {
        Instant::now() >= self.expires_at || SystemTime::now() >= self.expires_wall
    }
}

#[derive(Clone, Copy)]
enum Stage {
    Microsoft,
    Xbox,
    Xsts,
    Minecraft,
    Profile,
}

impl AuthClient {
    pub fn new() -> Result<Self, CoreError> {
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("PanitaClient/0.1.0")
            .build()
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?;
        Ok(Self {
            client,
            endpoints: Endpoints::default(),
        })
    }

    pub async fn exchange(&self, exchange: &CodeExchange) -> Result<MicrosoftToken, CoreError> {
        let dto: MicrosoftDto = self
            .request(
                self.client
                    .post(&self.endpoints.microsoft)
                    .form(&exchange.token_fields()),
                Stage::Microsoft,
            )
            .await?;
        dto.into_token()
    }

    pub async fn renew(&self, token: &Secret) -> Result<MicrosoftToken, CoreError> {
        let fields = [
            ("client_id", OAUTH_CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", token.expose()),
            ("scope", SCOPES),
        ];
        let dto: MicrosoftDto = self
            .request(
                self.client.post(&self.endpoints.microsoft).form(&fields),
                Stage::Microsoft,
            )
            .await?;
        dto.into_token()
    }

    pub async fn minecraft_session(
        &self,
        microsoft: &MicrosoftToken,
    ) -> Result<MinecraftSession, CoreError> {
        let xbox: XboxDto = self.request(self.client.post(&self.endpoints.xbox).json(&json!({
            "Properties": { "AuthMethod":"RPS", "SiteName":"user.auth.xboxlive.com", "RpsTicket": format!("d={}", microsoft.access.expose()) },
            "RelyingParty":"http://auth.xboxlive.com", "TokenType":"JWT"
        })), Stage::Xbox).await?;
        let xbox_deadline = parse_deadline(&xbox.not_after)?;
        let hash = xbox.user_hash()?;
        let xbox_token = Secret::new(xbox.token.clone())?;
        let xsts: XboxDto = self
            .request(
                self.client.post(&self.endpoints.xsts).json(&json!({
                    "Properties": { "SandboxId":"RETAIL", "UserTokens":[xbox_token.expose()] },
                    "RelyingParty":"rp://api.minecraftservices.com/", "TokenType":"JWT"
                })),
                Stage::Xsts,
            )
            .await?;
        let xsts_deadline = parse_deadline(&xsts.not_after)?;
        if xsts.user_hash()? != hash {
            return Err(CoreError::new(ErrorCode::AccountMismatch));
        }
        let minecraft: MinecraftDto = self
            .request(
                self.client.post(&self.endpoints.minecraft).json(&json!({
                    "identityToken":format!("XBL3.0 x={};{}", hash, xsts.token)
                })),
                Stage::Minecraft,
            )
            .await?;
        if !minecraft.token_type.eq_ignore_ascii_case("bearer") {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let access = Secret::new(minecraft.access_token.clone())?;
        let profile = self.fetch_profile(&access).await?;
        let expires_at = [
            microsoft.expires_at,
            xbox_deadline,
            xsts_deadline,
            deadline(minecraft.expires_in)?,
        ]
        .into_iter()
        .min()
        .ok_or_else(|| CoreError::new(ErrorCode::InvalidData))?;
        Ok(MinecraftSession {
            profile,
            access,
            expires_at,
            expires_wall: SystemTime::now()
                .checked_add(expires_at.saturating_duration_since(Instant::now()))
                .ok_or_else(|| CoreError::new(ErrorCode::InvalidData))?,
        })
    }

    /// Always performs a remote request. Local expiry is not a Play validation.
    pub async fn validate(&self, session: &MinecraftSession) -> Result<JavaProfile, CoreError> {
        let profile = self.fetch_profile(&session.access).await?;
        if profile.id != session.profile.id {
            return Err(CoreError::new(ErrorCode::AccountMismatch));
        }
        Ok(profile)
    }

    async fn fetch_profile(&self, access: &Secret) -> Result<JavaProfile, CoreError> {
        let dto: ProfileDto = self
            .request(
                self.client
                    .get(&self.endpoints.profile)
                    .bearer_auth(access.expose())
                    .header(reqwest::header::CACHE_CONTROL, "no-store"),
                Stage::Profile,
            )
            .await?;
        let id = Uuid::parse_str(&dto.id).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
        if id.is_nil()
            || !(3..=16).contains(&dto.name.len())
            || !dto
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        Ok(JavaProfile { id, name: dto.name })
    }

    async fn request<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        stage: Stage,
    ) -> Result<T, CoreError> {
        let mut response = request
            .send()
            .await
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE as u64)
        {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                bytes.zeroize();
                return Err(CoreError::new(ErrorCode::InvalidData));
            }
            bytes.extend_from_slice(&chunk);
        }
        let result = if status.is_success() {
            serde_json::from_slice(&bytes).map_err(|_| CoreError::new(ErrorCode::InvalidData))
        } else {
            Err(CoreError::new(classify_failure(status, stage, &bytes)))
        };
        bytes.zeroize();
        result
    }
}

fn classify_failure(status: StatusCode, stage: Stage, bytes: &[u8]) -> ErrorCode {
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return ErrorCode::ServiceUnavailable;
    }
    #[derive(Deserialize)]
    struct ErrorDto {
        error: Option<String>,
        #[serde(rename = "XErr")]
        xbox_error: Option<u64>,
    }
    let parsed = serde_json::from_slice::<ErrorDto>(bytes).ok();
    if matches!(stage, Stage::Xsts) && parsed.as_ref().and_then(|dto| dto.xbox_error).is_some() {
        return ErrorCode::AccountRestricted;
    }
    if matches!(stage, Stage::Microsoft)
        && let Some(dto) = &parsed
    {
        return match dto.error.as_deref() {
            Some("invalid_grant" | "interaction_required" | "login_required") => {
                ErrorCode::AuthenticationRequired
            }
            Some("access_denied") => ErrorCode::AuthenticationDenied,
            Some("invalid_client" | "unauthorized_client" | "invalid_scope") => {
                ErrorCode::AuthenticationConfiguration
            }
            Some("temporarily_unavailable" | "server_error") => ErrorCode::ServiceUnavailable,
            _ => ErrorCode::AuthenticationRequired,
        };
    }
    match status {
        StatusCode::UNAUTHORIZED => ErrorCode::AuthenticationRequired,
        StatusCode::FORBIDDEN => ErrorCode::AccountRestricted,
        StatusCode::NOT_FOUND if matches!(stage, Stage::Profile) => ErrorCode::MinecraftNotOwned,
        _ => ErrorCode::InvalidData,
    }
}

fn deadline(seconds: u64) -> Result<Instant, CoreError> {
    if seconds == 0 {
        return Err(CoreError::new(ErrorCode::InvalidData));
    }
    Instant::now()
        .checked_add(Duration::from_secs(seconds.saturating_sub(60)))
        .ok_or_else(|| CoreError::new(ErrorCode::InvalidData))
}

fn parse_deadline(not_after: &str) -> Result<Instant, CoreError> {
    let timestamp = DateTime::parse_from_rfc3339(not_after)
        .map_err(|_| CoreError::new(ErrorCode::InvalidData))?
        .timestamp();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CoreError::new(ErrorCode::InvalidData))?
        .as_secs();
    let timestamp =
        u64::try_from(timestamp).map_err(|_| CoreError::new(ErrorCode::AuthenticationRequired))?;
    deadline(
        timestamp
            .checked_sub(now)
            .ok_or_else(|| CoreError::new(ErrorCode::AuthenticationRequired))?,
    )
}

#[derive(Deserialize)]
struct MicrosoftDto {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    token_type: String,
}
impl MicrosoftDto {
    fn into_token(mut self) -> Result<MicrosoftToken, CoreError> {
        if !self.token_type.eq_ignore_ascii_case("bearer") {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        Ok(MicrosoftToken {
            access: Secret::new(std::mem::take(&mut self.access_token))?,
            refresh: self.refresh_token.take().map(Secret::new).transpose()?,
            expires_at: deadline(self.expires_in)?,
        })
    }
}
impl Drop for MicrosoftDto {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(token) = &mut self.refresh_token {
            token.zeroize();
        }
    }
}

#[derive(Deserialize)]
struct XboxDto {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "NotAfter")]
    not_after: String,
    #[serde(rename = "DisplayClaims")]
    claims: XboxClaims,
}
impl XboxDto {
    fn user_hash(&self) -> Result<String, CoreError> {
        if self.claims.xui.len() != 1 {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let hash = &self.claims.xui[0].uhs;
        if hash.is_empty() || hash.len() > 64 || !hash.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        Ok(hash.clone())
    }
}
impl Drop for XboxDto {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}
#[derive(Deserialize)]
struct XboxClaims {
    xui: Vec<XboxUser>,
}
#[derive(Deserialize)]
struct XboxUser {
    uhs: String,
}
#[derive(Deserialize)]
struct MinecraftDto {
    access_token: String,
    expires_in: u64,
    token_type: String,
}
impl Drop for MinecraftDto {
    fn drop(&mut self) {
        self.access_token.zeroize();
    }
}
#[derive(Deserialize)]
struct ProfileDto {
    id: String,
    name: String,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    pub(crate) fn mock_client(
        responses: Vec<(&'static str, u16, String)>,
    ) -> (AuthClient, std::thread::JoinHandle<()>) {
        mock_client_inner(responses, None)
    }

    pub(crate) fn gated_client(
        responses: Vec<(&'static str, u16, String)>,
    ) -> (
        AuthClient,
        std::thread::JoinHandle<()>,
        tokio::sync::oneshot::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (seen, observed) = tokio::sync::oneshot::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let (client, task) = mock_client_inner(responses, Some((seen, wait)));
        (client, task, observed, resume)
    }

    type Gate = (
        tokio::sync::oneshot::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    );

    fn mock_client_inner(
        responses: Vec<(&'static str, u16, String)>,
        mut gate: Option<Gate>,
    ) -> (AuthClient, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| panic!("loopback"));
        let base = format!(
            "http://{}",
            listener.local_addr().unwrap_or_else(|_| panic!("address"))
        );
        let task = std::thread::spawn(move || {
            for (path, status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap_or_else(|_| panic!("request"));
                assert!(
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .is_ok()
                );
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    let count = stream.read(&mut chunk).unwrap_or_else(|_| panic!("read"));
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    assert!(bytes.len() < 64 * 1024);
                    if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..split]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if bytes.len() >= split + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8_lossy(&bytes);
                if path == "/token" && request.contains("grant_type=authorization_code") {
                    let body = request
                        .split_once("\r\n\r\n")
                        .map(|(_, body)| body)
                        .unwrap_or_else(|| panic!("request body"));
                    let fields: std::collections::HashMap<_, _> =
                        url::form_urlencoded::parse(body.as_bytes()).collect();
                    assert_eq!(
                        fields.get("redirect_uri").map(|value| value.as_ref()),
                        Some(crate::oauth::REDIRECT_URI)
                    );
                    assert!(fields.contains_key("code_verifier"));
                    assert!(!fields.contains_key("client_secret"));
                }
                assert!(
                    request
                        .lines()
                        .next()
                        .is_some_and(|line| line.contains(&format!(" {path} ")))
                );
                if path == "/profile" {
                    assert!(
                        request
                            .to_ascii_lowercase()
                            .contains("authorization: bearer fixture-minecraft")
                    );
                }
                if path == "/xbox" {
                    assert!(request.contains("d=fixture-microsoft"));
                }
                if path == "/xsts" {
                    assert!(request.contains("fixture-xbox"));
                }
                if path == "/minecraft" {
                    assert!(request.contains("XBL3.0 x=1234;fixture-xsts"));
                }
                let reply = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let gated = gate.is_some();
                if let Some((seen, wait)) = gate.take() {
                    assert!(seen.send(()).is_ok());
                    assert!(wait.recv_timeout(Duration::from_secs(10)).is_ok());
                }
                let written = stream.write_all(reply.as_bytes());
                if !gated {
                    assert!(written.is_ok());
                }
            }
        });
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| panic!("client"));
        let endpoint = |path| format!("{base}/{path}");
        (
            AuthClient {
                client,
                endpoints: Endpoints {
                    microsoft: endpoint("token"),
                    xbox: endpoint("xbox"),
                    xsts: endpoint("xsts"),
                    minecraft: endpoint("minecraft"),
                    profile: endpoint("profile"),
                },
            },
            task,
        )
    }

    #[tokio::test]
    async fn service_chain_and_play_validation_use_expected_requests() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let not_after = DateTime::<chrono::Utc>::from_timestamp((now + 3600) as i64, 0)
            .unwrap_or_default()
            .to_rfc3339();
        let profile = r#"{"id":"5627dd98e6be3c21b8a8e92344183641","name":"PlayerOne"}"#.to_owned();
        let responses = vec![
            ("/token", 200, r#"{"access_token":"fixture-microsoft","refresh_token":"fixture-rotated","expires_in":3600,"token_type":"Bearer"}"#.to_owned()),
            ("/xbox", 200, json!({"Token":"fixture-xbox","NotAfter":not_after,"DisplayClaims":{"xui":[{"uhs":"1234"}]}}).to_string()),
            ("/xsts", 200, json!({"Token":"fixture-xsts","NotAfter":not_after,"DisplayClaims":{"xui":[{"uhs":"1234"}]}}).to_string()),
            ("/minecraft", 200, r#"{"access_token":"fixture-minecraft","expires_in":3600,"token_type":"Bearer"}"#.to_owned()),
            ("/profile", 200, profile.clone()), ("/profile", 200, profile),
        ];
        let (client, task) = mock_client(responses);
        let refresh =
            Secret::new("fixture-refresh".to_owned()).unwrap_or_else(|_| panic!("fixture"));
        let microsoft = client
            .renew(&refresh)
            .await
            .unwrap_or_else(|_| panic!("renew"));
        assert_eq!(
            microsoft.refresh.as_ref().map(Secret::expose),
            Some("fixture-rotated")
        );
        let session = client
            .minecraft_session(&microsoft)
            .await
            .unwrap_or_else(|_| panic!("chain"));
        assert!(!session.needs_renewal());
        assert_eq!(
            client
                .validate(&session)
                .await
                .unwrap_or_else(|_| panic!("validation")),
            session.profile
        );
        assert!(task.join().is_ok());
    }

    #[tokio::test]
    async fn remote_errors_do_not_include_service_bodies() {
        let (client, task) = mock_client(vec![(
            "/token",
            400,
            r#"{"error":"invalid_grant","error_description":"fixture-secret"}"#.to_owned(),
        )]);
        let secret =
            Secret::new("fixture-refresh".to_owned()).unwrap_or_else(|_| panic!("fixture"));
        let error = client
            .renew(&secret)
            .await
            .err()
            .unwrap_or_else(|| panic!("rejected"));
        assert_eq!(error.code(), ErrorCode::AuthenticationRequired);
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert!(task.join().is_ok());
    }

    #[test]
    fn failures_distinguish_rejection_restriction_and_service_outage() {
        assert_eq!(
            classify_failure(StatusCode::UNAUTHORIZED, Stage::Profile, b"{}"),
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(
            classify_failure(StatusCode::FORBIDDEN, Stage::Profile, b"{}"),
            ErrorCode::AccountRestricted
        );
        assert_eq!(
            classify_failure(StatusCode::NOT_FOUND, Stage::Profile, b"{}"),
            ErrorCode::MinecraftNotOwned
        );
        assert_eq!(
            classify_failure(
                StatusCode::BAD_REQUEST,
                Stage::Microsoft,
                br#"{"error":"invalid_grant","error_description":"sensitive fixture"}"#
            ),
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(
            classify_failure(
                StatusCode::UNAUTHORIZED,
                Stage::Xsts,
                br#"{"XErr":2148916233}"#
            ),
            ErrorCode::AccountRestricted
        );
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert_eq!(
                classify_failure(status, Stage::Profile, b"sensitive fixture"),
                ErrorCode::ServiceUnavailable
            );
        }
    }
}
