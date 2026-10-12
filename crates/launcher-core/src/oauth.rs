//! Backend-only OAuth state. Neither callback parameters nor browser URLs are DTOs.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::accounts::{OAUTH_CLIENT_ID, Secret};
use crate::error::{CoreError, ErrorCode};
use crate::operation::{OperationId, ProcessSessionId};

pub const REDIRECT_URI: &str = "panitacraft://oauth/callback";
pub const SCOPES: &str = "XboxLive.signin offline_access";
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
pub const AUTHORIZE_ENDPOINT: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";

pub struct CodeExchange {
    pub operation: OperationId,
    pub intended_account: Option<Uuid>,
    pub(crate) code: Secret,
    pub(crate) verifier: Secret,
}

impl CodeExchange {
    /// Sensitive fields for the backend token request; never log or send to IPC.
    pub fn token_fields(&self) -> [(&str, &str); 6] {
        [
            ("client_id", OAUTH_CLIENT_ID),
            ("grant_type", "authorization_code"),
            ("code", self.code.expose()),
            ("code_verifier", self.verifier.expose()),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPES),
        ]
    }
}

pub struct PkceLogin {
    pub operation: OperationId,
    pub process: ProcessSessionId,
    pub intended_account: Option<Uuid>,
    state: Secret,
    verifier: Secret,
    deadline: Instant,
}

impl PkceLogin {
    pub fn new(
        process: ProcessSessionId,
        intended_account: Option<Uuid>,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            operation: OperationId::new(),
            process,
            intended_account,
            state: random_secret()?,
            verifier: random_secret()?,
            deadline: Instant::now() + LOGIN_TIMEOUT,
        })
    }

    /// Only the desktop Rust opener should receive this URL.
    pub fn authorization_url(&self) -> Result<String, CoreError> {
        let mut url =
            Url::parse(AUTHORIZE_ENDPOINT).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", OAUTH_CLIENT_ID),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT_URI),
            ("response_mode", "query"),
            ("scope", SCOPES),
            ("state", self.state.expose()),
            ("code_challenge", &challenge(self.verifier.expose())),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ]);
        Ok(url.into())
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

#[derive(Default)]
pub struct PendingLogin {
    active: Option<PkceLogin>,
}

impl PendingLogin {
    pub fn begin(&mut self, login: PkceLogin) -> Result<(), CoreError> {
        if self.active.as_ref().is_some_and(|active| !active.expired()) {
            return Err(CoreError::new(ErrorCode::OperationBusy));
        }
        self.active = Some(login);
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.active = None;
    }

    pub fn expire(&mut self) -> bool {
        if self.active.as_ref().is_some_and(PkceLogin::expired) {
            self.cancel();
            return true;
        }
        false
    }

    pub fn accept(&mut self, callback: &str) -> Result<CodeExchange, CoreError> {
        if self.expire() {
            return Err(CoreError::new(ErrorCode::AuthenticationTimeout));
        }
        let active = self
            .active
            .as_ref()
            .ok_or_else(|| CoreError::new(ErrorCode::OperationCancelled))?;
        // Check spelling before URL normalization (which can erase dot segments).
        if callback.len() > 32_768 || !callback.starts_with(&format!("{REDIRECT_URI}?")) {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let url = Url::parse(callback).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
        if url.scheme() != "panitacraft"
            || url.host_str() != Some("oauth")
            || url.path() != "/callback"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
        {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let mut query = HashMap::new();
        for (key, value) in url.query_pairs() {
            if query.insert(key.into_owned(), value.into_owned()).is_some() {
                return Err(CoreError::new(ErrorCode::InvalidData));
            }
        }
        let state = query
            .get("state")
            .ok_or_else(|| CoreError::new(ErrorCode::InvalidData))?;
        if !equal_state(state.as_bytes(), active.state.expose().as_bytes()) {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        if query.contains_key("code") == query.contains_key("error") {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let active = self
            .active
            .take()
            .ok_or_else(|| CoreError::new(ErrorCode::OperationCancelled))?;
        if let Some(error) = query.get("error") {
            return Err(CoreError::new(match error.as_str() {
                "access_denied" => ErrorCode::AuthenticationDenied,
                "server_error" | "temporarily_unavailable" => ErrorCode::ServiceUnavailable,
                _ => ErrorCode::AuthenticationRequired,
            }));
        }
        let code = Secret::new(
            query
                .remove("code")
                .ok_or_else(|| CoreError::new(ErrorCode::InvalidData))?,
        )?;
        for value in query.values_mut() {
            value.zeroize();
        }
        Ok(CodeExchange {
            operation: active.operation,
            intended_account: active.intended_account,
            code,
            verifier: active.verifier,
        })
    }
}

fn random_secret() -> Result<Secret, CoreError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| CoreError::new(ErrorCode::StorageUnavailable))?;
    let value = URL_SAFE_NO_PAD.encode(bytes);
    bytes.zeroize();
    Secret::new(value)
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn equal_state(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (PendingLogin, String) {
        let login =
            PkceLogin::new(ProcessSessionId::new(), None).unwrap_or_else(|_| panic!("random"));
        let state = login.state.expose().to_owned();
        let mut pending = PendingLogin::default();
        assert!(pending.begin(login).is_ok());
        (pending, state)
    }

    #[test]
    fn s256_matches_rfc7636_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn callback_rejects_wrong_state_routes_duplicates_and_replay() {
        let (mut pending, state) = setup();
        for invalid in [
            format!("{REDIRECT_URI}?state=wrong&code=fixture"),
            format!("panitacraft://oauth/other/../callback?state={state}&code=fixture"),
            format!("panitacraft://attacker/callback?state={state}&code=fixture"),
            format!("{REDIRECT_URI}?state={state}&code=fixture&code=duplicate"),
            format!("{REDIRECT_URI}?code=fixture"),
            format!("{REDIRECT_URI}?state={state}&code=fixture#error"),
            format!("{REDIRECT_URI}?state={state}&code=fixture&error=access_denied"),
        ] {
            assert!(pending.accept(&invalid).is_err());
        }
        let valid = format!("{REDIRECT_URI}?state={state}&code=fixture");
        let exchange = pending
            .accept(&valid)
            .unwrap_or_else(|_| panic!("valid callback"));
        assert_eq!(exchange.code.expose(), "fixture");
        assert_eq!(exchange.verifier.expose().len(), 43);
        assert!(pending.accept(&valid).is_err());
    }

    #[test]
    fn cancelled_expired_and_denied_operations_cannot_resume() {
        let (mut pending, state) = setup();
        pending.cancel();
        assert!(
            pending
                .accept(&format!("{REDIRECT_URI}?state={state}&code=fixture"))
                .is_err()
        );
        let (mut pending, state) = setup();
        if let Some(active) = &mut pending.active {
            active.deadline = Instant::now();
        }
        assert!(
            matches!(pending.accept(&format!("{REDIRECT_URI}?state={state}&code=fixture")), Err(e) if e.code() == ErrorCode::AuthenticationTimeout)
        );
        let (mut pending, state) = setup();
        assert!(
            matches!(pending.accept(&format!("{REDIRECT_URI}?state={state}&error=access_denied")), Err(e) if e.code() == ErrorCode::AuthenticationDenied)
        );
        assert!(pending.active.is_none());
    }

    #[test]
    fn every_operation_has_independent_entropy_and_bound_scopes() {
        let a = PkceLogin::new(ProcessSessionId::new(), None).unwrap_or_else(|_| panic!("random"));
        let b = PkceLogin::new(ProcessSessionId::new(), None).unwrap_or_else(|_| panic!("random"));
        assert_ne!(a.state.expose(), b.state.expose());
        assert_ne!(a.verifier.expose(), b.verifier.expose());
        let url = Url::parse(&a.authorization_url().unwrap_or_else(|_| panic!("url")))
            .unwrap_or_else(|_| panic!("parse"));
        let query: HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(query.get("scope").map(|s| s.as_ref()), Some(SCOPES));
        assert_eq!(
            query.get("redirect_uri").map(|s| s.as_ref()),
            Some(REDIRECT_URI)
        );
        assert_eq!(
            query.get("code_challenge_method").map(|s| s.as_ref()),
            Some("S256")
        );
        assert!(!query.contains_key("client_secret"));
    }
}
