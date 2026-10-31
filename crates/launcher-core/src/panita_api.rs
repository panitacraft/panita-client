//! Bounded, nonredirecting requests to the Panita API for activation.
//! Service bodies never become errors: only fixed codes leave this module.
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use reqwest::{Client, RequestBuilder, StatusCode, header};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::accounts::Secret;
use crate::catalog::BASE;
use crate::error::{CoreError, ErrorCode};
use crate::panita_session::{MinecraftStatus, PanitaCapabilities, PanitaSession, PanitaUser};

const MAX_RESPONSE: usize = 128 * 1024;
const SERIAL_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const DEVICE_LIMIT: usize = 40;

/// The launcher key is a public client identifier: a binary can always be
/// searched for it, so nothing depends on hiding it. It tells the API which
/// client is calling; accounts are protected by the session token and by the
/// confirmation from Discord.
fn launcher_key() -> String {
    #[cfg(feature = "test-endpoints")]
    if let Ok(key) = std::env::var("PANITA_TEST_LAUNCHER_KEY") {
        return key;
    }
    option_env!("PANITA_LAUNCHER_API_KEY")
        .unwrap_or_default()
        .to_owned()
}

/// The launcher key and, once activated, the member's session token.
#[derive(Clone)]
pub struct PanitaCredentials {
    key: String,
    token: Option<Arc<Secret>>,
    /// The Panita account id of the session, used to bind cached catalogs.
    identity: Option<u64>,
}

impl PanitaCredentials {
    /// Key only. Fails when this build has no launcher key compiled in.
    pub fn launcher() -> Result<Self, CoreError> {
        Self::new(&launcher_key())
    }

    pub(crate) fn new(key: &str) -> Result<Self, CoreError> {
        if key.is_empty() || key.len() > 128 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(CoreError::new(ErrorCode::ActivationUnavailable));
        }
        Ok(Self {
            key: key.to_owned(),
            token: None,
            identity: None,
        })
    }

    pub fn with_session(mut self, session: &PanitaSession) -> Self {
        self.token = Some(Arc::clone(session.token()));
        self.identity = Some(session.user.id);
        self
    }

    pub(crate) fn identity(&self) -> Option<u64> {
        self.identity
    }

    pub fn has_session(&self) -> bool {
        self.token.is_some()
    }

    pub(crate) fn token(&self) -> Option<&Arc<Secret>> {
        self.token.as_ref()
    }

    /// Identify the client only. Activation requests never carry a session.
    fn apply_key(&self, request: RequestBuilder) -> RequestBuilder {
        request.header("x-api-key", &self.key)
    }

    /// Identify the client and, when present, the member. `bearer_auth` marks the
    /// header sensitive so it is excluded from debug output.
    pub(crate) fn apply(&self, request: RequestBuilder) -> RequestBuilder {
        let request = self.apply_key(request);
        match &self.token {
            Some(token) => request.bearer_auth(token.expose()),
            None => request,
        }
    }
}

/// Informational text that reaches Discord. Reduced to a conservative alphabet so
/// the API never rejects it and it needs no escaping.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceInfo {
    name: Option<String>,
    os: Option<String>,
}

impl DeviceInfo {
    pub fn new(name: Option<&str>, os: Option<&str>) -> Self {
        Self {
            name: name.and_then(clean_device_text),
            os: os.and_then(clean_device_text),
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn os(&self) -> Option<&str> {
        self.os.as_deref()
    }
}

fn clean_device_text(value: &str) -> Option<String> {
    let kept: String = value
        .chars()
        .filter(|character| {
            character.is_alphanumeric()
                || matches!(
                    character,
                    ' ' | '-' | '_' | '.' | '/' | '(' | ')' | '+' | ':' | ','
                )
        })
        .take(DEVICE_LIMIT)
        .collect();
    let kept = kept.trim();
    (!kept.is_empty()).then(|| kept.to_owned())
}

/// Integer seconds of a `Retry-After` header, clamped to 1..=600; dates are ignored.
pub(crate) fn retry_after_seconds(value: Option<&str>) -> Option<u32> {
    value
        .and_then(|value| value.trim().parse::<u32>().ok())
        .map(|seconds| seconds.clamp(1, 600))
}

/// A code-only failure plus the wait the API asked for, when it asked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanitaApiError {
    pub code: ErrorCode,
    /// Seconds from `Retry-After`, clamped to 1..=600.
    pub retry_after: Option<u32>,
}

impl From<ErrorCode> for PanitaApiError {
    fn from(code: ErrorCode) -> Self {
        Self {
            code,
            retry_after: None,
        }
    }
}

impl From<CoreError> for PanitaApiError {
    fn from(error: CoreError) -> Self {
        error.code().into()
    }
}

impl From<PanitaApiError> for CoreError {
    fn from(error: PanitaApiError) -> Self {
        CoreError::new(error.code)
    }
}

/// A login request waiting for the member to confirm it in Discord.
pub struct LoginRequest {
    pub request_id: Uuid,
    /// `XXXX-YYYY-ZZZZ`: what the member sends to the bot.
    pub serial: String,
    /// Unix seconds when the request stops being valid.
    pub expires_at: i64,
    poll_secret: Secret,
}

impl LoginRequest {
    pub(crate) fn poll_secret(&self) -> &Secret {
        &self.poll_secret
    }
}

pub enum PollOutcome {
    Pending {
        expires_at: i64,
    },
    /// Delivered once: the API answers 410 to any later poll.
    Confirmed(PanitaSession),
}

pub struct PanitaApiClient {
    client: Client,
    base: String,
}

#[cfg(test)]
impl PanitaApiClient {
    /// Lets other clients' tests reuse this module's loopback fixture.
    pub(crate) fn test_transport(self) -> (Client, String) {
        (self.client, self.base)
    }
}

impl PanitaApiClient {
    pub fn new() -> Result<Self, CoreError> {
        #[cfg(feature = "test-endpoints")]
        if let Ok(base) = std::env::var("PANITA_TEST_PANITA_API_BASE") {
            return Self::build(base, false);
        }
        Self::build(BASE.to_owned(), true)
    }

    fn build(base: String, https_only: bool) -> Result<Self, CoreError> {
        let builder = Client::builder()
            .https_only(https_only)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("PanitaClient/", env!("CARGO_PKG_VERSION")));
        #[cfg(test)]
        let builder = builder.no_proxy();
        let client = builder
            .build()
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?;
        Ok(Self { client, base })
    }

    pub async fn create_login_request(
        &self,
        credentials: &PanitaCredentials,
        identifier: &str,
        device: &DeviceInfo,
    ) -> Result<LoginRequest, PanitaApiError> {
        let identifier = identifier.trim();
        if identifier.is_empty()
            || identifier.chars().count() > 100
            || identifier.chars().any(char::is_control)
        {
            return Err(ErrorCode::InvalidUsername.into());
        }
        #[derive(Serialize)]
        struct Body<'a> {
            identifier: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            device_name: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            device_os: Option<&'a str>,
        }
        let request = credentials
            .apply_key(
                self.client
                    .post(format!("{}/v1/auth/login-requests", self.base)),
            )
            .json(&Body {
                identifier,
                device_name: device.name(),
                device_os: device.os(),
            });
        let reply = self
            .send(request, "POST", "/v1/auth/login-requests", true)
            .await?;
        if reply.status != StatusCode::CREATED {
            return Err(reply.failure(Operation::CreateRequest));
        }
        let created: Envelope<CreatedDto> = reply.parse()?;
        created.data.into_request()
    }

    pub async fn poll(
        &self,
        credentials: &PanitaCredentials,
        request: &LoginRequest,
    ) -> Result<PollOutcome, PanitaApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            poll_secret: &'a str,
        }
        let http = credentials
            .apply_key(self.client.post(format!(
                "{}/v1/auth/login-requests/{}/poll",
                self.base, request.request_id
            )))
            .json(&Body {
                poll_secret: request.poll_secret().expose(),
            });
        let reply = self
            .send(http, "POST", "/v1/auth/login-requests/{id}/poll", false)
            .await?;
        if reply.status != StatusCode::OK {
            return Err(reply.failure(Operation::Poll));
        }
        let status: Envelope<StatusDto> = reply.parse()?;
        match status.data {
            StatusDto::Pending { expires_at } => Ok(PollOutcome::Pending {
                expires_at: parse_time(&expires_at)?,
            }),
            StatusDto::Confirmed { token, user } => {
                let session = PanitaSession::new(token, user.into_user()?, now_seconds())
                    .map_err(|_| PanitaApiError::from(ErrorCode::InvalidData))?;
                Ok(PollOutcome::Confirmed(session))
            }
        }
    }

    /// The session and the account are re-read by the API on every call.
    pub async fn me(&self, credentials: &PanitaCredentials) -> Result<PanitaUser, PanitaApiError> {
        if !credentials.has_session() {
            return Err(ErrorCode::ActivationRequired.into());
        }
        let request = credentials.apply(self.client.get(format!("{}/v1/auth/me", self.base)));
        let reply = self.send(request, "GET", "/v1/auth/me", true).await?;
        if reply.status != StatusCode::OK {
            return Err(reply.failure(Operation::Me));
        }
        let me: Envelope<MeDto> = reply.parse()?;
        me.data.into_user()
    }

    pub async fn sign_out(&self, credentials: &PanitaCredentials) -> Result<(), PanitaApiError> {
        if !credentials.has_session() {
            return Err(ErrorCode::ActivationRequired.into());
        }
        let request =
            credentials.apply(self.client.delete(format!("{}/v1/auth/session", self.base)));
        let reply = self
            .send(request, "DELETE", "/v1/auth/session", true)
            .await?;
        if reply.status == StatusCode::NO_CONTENT {
            return Ok(());
        }
        Err(reply.failure(Operation::SignOut))
    }

    /// `route` is a fixed label for the session log, never the request URL, which
    /// carries a login request id. Frequent polls pass `log_success = false`.
    async fn send(
        &self,
        request: RequestBuilder,
        method: &'static str,
        route: &'static str,
        log_success: bool,
    ) -> Result<Reply, PanitaApiError> {
        let network = |_| PanitaApiError::from(ErrorCode::NetworkUnavailable);
        let started = std::time::Instant::now();
        let sent = request
            .header(header::ACCEPT, "application/json")
            .send()
            .await;
        crate::network_log::api_result(
            "panita-api",
            method,
            route,
            started,
            sent.as_ref().map(|reply| reply.status().as_u16()),
            log_success,
        );
        let mut response = sent.map_err(network)?;
        let status = response.status();
        let retry_after = retry_after_seconds(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
        );
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE as u64)
        {
            return Err(ErrorCode::InvalidData.into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            if body.len() + chunk.len() > MAX_RESPONSE {
                return Err(ErrorCode::InvalidData.into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Reply {
            status,
            retry_after,
            body,
        })
    }
}

#[derive(Clone, Copy)]
enum Operation {
    CreateRequest,
    Poll,
    Me,
    SignOut,
}

struct Reply {
    status: StatusCode,
    retry_after: Option<u32>,
    body: Vec<u8>,
}

/// A confirmed login carries the session token in this body.
impl Drop for Reply {
    fn drop(&mut self) {
        self.body.zeroize();
    }
}

impl Reply {
    fn parse<T: serde::de::DeserializeOwned>(&self) -> Result<T, PanitaApiError> {
        serde_json::from_slice(&self.body).map_err(|_| ErrorCode::InvalidData.into())
    }

    /// Only `invalid_token` may end a session: `invalid_key` means this launcher's
    /// key was refused while every session is still valid. Anything unrecognized,
    /// including a proxy page, is invalid data and never a rejection.
    fn failure(&self, operation: Operation) -> PanitaApiError {
        let reason = error_reason(&self.body);
        let reason = reason.as_deref();
        let code = match (self.status.as_u16(), operation, reason) {
            (401 | 403, _, Some("invalid_key")) | (403, _, _) => ErrorCode::ApiKeyRejected,
            (401, Operation::Me | Operation::SignOut, Some("invalid_token")) => {
                ErrorCode::SessionRejected
            }
            (401, Operation::CreateRequest, Some("unknown_account")) => {
                ErrorCode::ActivationUnknownAccount
            }
            (401, Operation::CreateRequest | Operation::Poll, Some("not_activated")) => {
                ErrorCode::ActivationNotActivated
            }
            (404, Operation::Poll, _) => ErrorCode::ActivationConsumed,
            (410, Operation::Poll, Some("expired")) => ErrorCode::ActivationExpired,
            (410, Operation::Poll, _) => ErrorCode::ActivationConsumed,
            // The firewall's 429 has no envelope; honor it all the same.
            (429, _, _) => {
                return PanitaApiError {
                    code: ErrorCode::ActivationRateLimited,
                    retry_after: self.retry_after,
                };
            }
            (500.., _, _) => ErrorCode::ServiceUnavailable,
            _ => ErrorCode::InvalidData,
        };
        code.into()
    }
}

/// `error.details.reason` of the API's error envelope. A proxy page or any other
/// body has none, and an unknown reason is never treated as a rejection.
pub(crate) fn error_reason(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Failure {
        error: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        details: Option<Details>,
    }
    #[derive(Deserialize)]
    struct Details {
        reason: Option<String>,
    }
    serde_json::from_slice::<Failure>(body)
        .ok()
        .and_then(|failure| failure.error.details)
        .and_then(|details| details.reason)
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct CreatedDto {
    request_id: Uuid,
    code: String,
    poll_secret: String,
    expires_at: String,
}

impl CreatedDto {
    fn into_request(self) -> Result<LoginRequest, PanitaApiError> {
        let invalid = || PanitaApiError::from(ErrorCode::InvalidData);
        let groups: Vec<&str> = self.code.split('-').collect();
        let serial_ok = groups.len() == 3
            && groups.iter().all(|group| {
                group.len() == 4 && group.bytes().all(|byte| SERIAL_ALPHABET.contains(&byte))
            });
        let secret_ok = self.poll_secret.len() == 43
            && self
                .poll_secret
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if self.request_id.is_nil() || !serial_ok || !secret_ok {
            return Err(invalid());
        }
        Ok(LoginRequest {
            request_id: self.request_id,
            serial: self.code,
            expires_at: parse_time(&self.expires_at)?,
            poll_secret: Secret::new(self.poll_secret).map_err(|_| invalid())?,
        })
    }
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
enum StatusDto {
    Pending { expires_at: String },
    Confirmed { token: String, user: MeDto },
}

#[derive(Deserialize)]
struct MeDto {
    id: u64,
    ign: Option<String>,
    discord_name: String,
    avatar_url: Option<String>,
    minecraft_status: Option<String>,
    capabilities: CapabilitiesDto,
}

#[derive(Deserialize)]
struct CapabilitiesDto {
    authenticated: bool,
    admin_or_mod: bool,
    trusted_author: bool,
    gallery_editor: bool,
    upload_authorized: bool,
    wiki_editor: bool,
}

impl MeDto {
    fn into_user(self) -> Result<PanitaUser, PanitaApiError> {
        let user = PanitaUser {
            id: self.id,
            ign: self.ign,
            discord_name: self.discord_name,
            avatar_url: self.avatar_url,
            // A value this launcher does not know is only a missing hint.
            minecraft_status: match self.minecraft_status.as_deref() {
                Some("premium") => Some(MinecraftStatus::Premium),
                Some("cracked") => Some(MinecraftStatus::Cracked),
                _ => None,
            },
            capabilities: PanitaCapabilities {
                authenticated: self.capabilities.authenticated,
                admin_or_mod: self.capabilities.admin_or_mod,
                trusted_author: self.capabilities.trusted_author,
                gallery_editor: self.capabilities.gallery_editor,
                upload_authorized: self.capabilities.upload_authorized,
                wiki_editor: self.capabilities.wiki_editor,
            },
        };
        user.validate()?;
        Ok(user)
    }
}

fn parse_time(value: &str) -> Result<i64, PanitaApiError> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.timestamp())
        .map_err(|_| ErrorCode::InvalidData.into())
}

pub(crate) fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
#[path = "panita_api_tests.rs"]
pub(crate) mod tests;
