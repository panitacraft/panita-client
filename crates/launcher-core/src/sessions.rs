//! Operation generations protect disk and memory from late authentication results.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use uuid::Uuid;

use crate::accounts::{
    AccountInfo, AccountKind, AccountRepository, OAUTH_CLIENT_ID, RemovalOutcome, Secret,
};
use crate::authentication::{AuthClient, JavaProfile, MinecraftSession};
use crate::error::{CoreError, ErrorCode};
use crate::oauth::{LoginRedirect, PendingLogin, PkceLogin};
use crate::operation::{OperationId, ProcessSessionId};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Offline,
    WaitingForBrowser,
    Authenticating,
    Ready,
    SignInRequired,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccountView {
    pub metadata: AccountInfo,
    pub status: SessionStatus,
    pub error: Option<ErrorCode>,
    pub persisted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccountsView {
    pub revision: u64,
    pub accounts: Vec<AccountView>,
    pub selected_account_id: Option<Uuid>,
    pub login_pending: bool,
    pub login_error: Option<ErrorCode>,
}

pub struct LoginLaunch {
    pub operation_id: Uuid,
    pub browser_url: String,
}

#[derive(Clone, Copy)]
struct Ticket {
    operation: OperationId,
    generation: u64,
}

struct State {
    generation: u64,
    cancellation: tokio::sync::watch::Sender<u64>,
    pending: PendingLogin,
    interactive: Option<Ticket>,
    login_error: Option<ErrorCode>,
    accounts: HashMap<Uuid, AccountView>,
    saved_ids: HashSet<Uuid>,
    selected: Option<Uuid>,
    sessions: HashMap<Uuid, Arc<MinecraftSession>>,
    unsaved_refresh: HashMap<Uuid, Secret>,
    flights: HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>,
}

pub struct AccountController {
    snapshot_counter: AtomicU64,
    repository: AccountRepository,
    client: AuthClient,
    process: ProcessSessionId,
    state: Mutex<State>,
}

impl AccountController {
    pub fn new(
        repository: AccountRepository,
        process: ProcessSessionId,
    ) -> Result<Self, CoreError> {
        Self::with_client(repository, process, AuthClient::new()?)
    }

    fn with_client(
        repository: AccountRepository,
        process: ProcessSessionId,
        client: AuthClient,
    ) -> Result<Self, CoreError> {
        let snapshot = repository.snapshot()?;
        let accounts: HashMap<Uuid, AccountView> = snapshot
            .accounts
            .into_iter()
            .map(|metadata| {
                let status = if metadata.kind == AccountKind::Offline {
                    SessionStatus::Offline
                } else {
                    SessionStatus::SignInRequired
                };
                (
                    metadata.account_id,
                    AccountView {
                        metadata,
                        status,
                        error: None,
                        persisted: true,
                    },
                )
            })
            .collect();
        let saved_ids = accounts.keys().copied().collect();
        Ok(Self {
            snapshot_counter: AtomicU64::new(0),
            repository,
            client,
            process,
            state: Mutex::new(State {
                generation: 0,
                cancellation: tokio::sync::watch::channel(0).0,
                pending: PendingLogin::default(),
                interactive: None,
                login_error: None,
                accounts,
                saved_ids,
                selected: snapshot.selected_account_id,
                sessions: HashMap::new(),
                unsaved_refresh: HashMap::new(),
                flights: HashMap::new(),
            }),
        })
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, CoreError> {
        self.state
            .lock()
            .map_err(|_| CoreError::new(ErrorCode::StorageUnavailable))
    }

    pub fn view(&self) -> Result<AccountsView, CoreError> {
        let state = self.state()?;
        let mut accounts: Vec<_> = state.accounts.values().cloned().collect();
        accounts.sort_by(|a, b| {
            a.metadata
                .username
                .cmp(&b.metadata.username)
                .then(a.metadata.account_id.cmp(&b.metadata.account_id))
        });
        Ok(AccountsView {
            revision: self.snapshot_counter.fetch_add(1, Ordering::Relaxed),
            accounts,
            selected_account_id: state.selected,
            login_pending: state.interactive.is_some(),
            login_error: state.login_error,
        })
    }

    pub fn report_orphaned_callback(&self) -> Result<(), CoreError> {
        self.state()?.login_error = Some(ErrorCode::OperationCancelled);
        Ok(())
    }

    pub fn add_offline(&self, name: &str) -> Result<AccountInfo, CoreError> {
        let mut state = self.state()?;
        invalidate(&mut state);
        let metadata = self.repository.add_offline(name)?;
        self.repository.select(metadata.account_id)?;
        state.selected = Some(metadata.account_id);
        state.saved_ids.insert(metadata.account_id);
        state.accounts.insert(
            metadata.account_id,
            AccountView {
                metadata: metadata.clone(),
                status: SessionStatus::Offline,
                error: None,
                persisted: true,
            },
        );
        Ok(metadata)
    }

    pub fn select(&self, id: Uuid) -> Result<(), CoreError> {
        let mut state = self.state()?;
        let persisted = state
            .accounts
            .get(&id)
            .ok_or_else(|| CoreError::new(ErrorCode::AccountNotFound))?
            .persisted;
        invalidate(&mut state);
        if persisted {
            self.repository.select(id)?;
        }
        state.selected = Some(id);
        Ok(())
    }

    pub fn begin_login(&self, intended_account: Option<Uuid>) -> Result<LoginLaunch, CoreError> {
        self.begin_login_with_redirect(intended_account, LoginRedirect::default())
    }

    pub fn begin_login_with_redirect(
        &self,
        intended_account: Option<Uuid>,
        redirect: LoginRedirect,
    ) -> Result<LoginLaunch, CoreError> {
        let mut state = self.state()?;
        if state.interactive.is_some() {
            return Err(CoreError::new(ErrorCode::OperationBusy));
        }
        if let Some(id) = intended_account {
            let account = state
                .accounts
                .get(&id)
                .ok_or_else(|| CoreError::new(ErrorCode::AccountNotFound))?;
            if account.metadata.kind != AccountKind::Microsoft {
                return Err(CoreError::new(ErrorCode::AccountMismatch));
            }
        }
        invalidate(&mut state);
        let login = PkceLogin::with_redirect(self.process, intended_account, redirect)?;
        let launch = LoginLaunch {
            operation_id: login.operation.value(),
            browser_url: login.authorization_url()?,
        };
        state.interactive = Some(Ticket {
            operation: login.operation,
            generation: state.generation,
        });
        state.pending.begin(login)?;
        Ok(launch)
    }

    pub fn cancel_login(&self) -> Result<(), CoreError> {
        invalidate(&mut *self.state()?);
        Ok(())
    }

    pub fn expire_login(&self) -> Result<bool, CoreError> {
        let mut state = self.state()?;
        if state.pending.expire() {
            invalidate(&mut state);
            state.login_error = Some(ErrorCode::AuthenticationTimeout);
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn callback(&self, callback: &str) -> Result<(), CoreError> {
        let (exchange, ticket, cancellation) = {
            let mut state = self.state()?;
            let ticket = state
                .interactive
                .ok_or_else(|| CoreError::new(ErrorCode::OperationCancelled))?;
            if !state.pending.is_active() {
                return Err(CoreError::new(ErrorCode::OperationCancelled));
            }
            match state.pending.accept(callback) {
                Ok(exchange) => (exchange, ticket, state.cancellation.subscribe()),
                Err(error) => {
                    if !state.pending.is_active() {
                        state.interactive = None;
                        state.login_error = Some(error.code());
                    }
                    return Err(error);
                }
            }
        };
        let result = async {
            let microsoft =
                cancellable(cancellation.clone(), self.client.exchange(&exchange)).await?;
            self.check_ticket(ticket)?;
            let session =
                cancellable(cancellation, self.client.minecraft_session(&microsoft)).await?;
            let mut state = self.state()?;
            verify_ticket(&state, ticket)?;
            if let Some(id) = exchange.intended_account {
                let intended = state
                    .accounts
                    .get(&id)
                    .ok_or_else(|| CoreError::new(ErrorCode::OperationCancelled))?;
                if intended.metadata.profile_id != session.profile.id {
                    return Err(CoreError::new(ErrorCode::AccountMismatch));
                }
            }
            let token = microsoft
                .refresh
                .as_ref()
                .ok_or_else(|| CoreError::new(ErrorCode::AuthenticationRequired))?;
            let save = if let Some(id) = exchange
                .intended_account
                .filter(|id| !state.saved_ids.contains(id))
            {
                let mut metadata = state
                    .accounts
                    .get(&id)
                    .ok_or_else(|| CoreError::new(ErrorCode::OperationCancelled))?
                    .metadata
                    .clone();
                metadata.username = session.profile.name.clone();
                self.repository.persist_unsaved(&metadata, token)
            } else {
                self.repository.save_microsoft(
                    session.profile.id,
                    &session.profile.name,
                    token,
                    exchange.intended_account,
                )
            };
            let (metadata, persisted, save_error) = match save {
                Ok(metadata) => (metadata, true, None),
                Err(error) if is_storage_error(error.code()) => {
                    let existing_id = exchange.intended_account.or_else(|| {
                        state
                            .accounts
                            .values()
                            .find(|a| {
                                a.metadata.kind == AccountKind::Microsoft
                                    && a.metadata.profile_id == session.profile.id
                            })
                            .map(|a| a.metadata.account_id)
                    });
                    (
                        AccountInfo {
                            account_id: existing_id.unwrap_or_else(Uuid::new_v4),
                            kind: AccountKind::Microsoft,
                            profile_id: session.profile.id,
                            username: session.profile.name.clone(),
                            oauth_client_id: Some(OAUTH_CLIENT_ID.to_owned()),
                        },
                        false,
                        Some(error.code()),
                    )
                }
                Err(error) => return Err(error),
            };
            if !persisted {
                state
                    .unsaved_refresh
                    .insert(metadata.account_id, Secret::new(token.expose().to_owned())?);
            }
            if persisted {
                state.saved_ids.insert(metadata.account_id);
                state.unsaved_refresh.remove(&metadata.account_id);
            }
            state.selected = Some(metadata.account_id);
            state
                .sessions
                .insert(metadata.account_id, Arc::new(session));
            state.accounts.insert(
                metadata.account_id,
                AccountView {
                    metadata,
                    status: SessionStatus::Ready,
                    error: save_error,
                    persisted,
                },
            );
            Ok(())
        }
        .await;
        let mut state = self.state()?;
        if verify_ticket(&state, ticket).is_ok() {
            state.interactive = None;
            state.login_error = result.as_ref().err().map(CoreError::code);
        }
        result
    }

    fn check_ticket(&self, ticket: Ticket) -> Result<(), CoreError> {
        verify_ticket(&*self.state()?, ticket)
    }

    pub async fn renew_selected(&self) -> Result<(), CoreError> {
        let selected = self.state()?.selected;
        if let Some(id) = selected {
            let kind = self.state()?.accounts.get(&id).map(|a| a.metadata.kind);
            if kind == Some(AccountKind::Microsoft) {
                self.acquire(id, false).await?;
            }
        }
        Ok(())
    }

    pub async fn maintain_selected(&self) -> Result<(), CoreError> {
        let should_renew = {
            let state = self.state()?;
            state.interactive.is_none()
                && state.selected.is_some_and(|id| {
                    state
                        .accounts
                        .get(&id)
                        .is_some_and(|account| matches!(account.status, SessionStatus::Ready))
                })
        };
        if should_renew {
            self.renew_selected().await?;
        }
        Ok(())
    }

    pub async fn retry_selected(&self) -> Result<(), CoreError> {
        let selected = {
            let state = self.state()?;
            state.selected.filter(|id| {
                state
                    .accounts
                    .get(id)
                    .is_some_and(|account| account.metadata.kind == AccountKind::Microsoft)
            })
        };
        if let Some(id) = selected {
            self.acquire(id, true).await?;
        }
        Ok(())
    }

    async fn acquire(&self, id: Uuid, force: bool) -> Result<Arc<MinecraftSession>, CoreError> {
        let flight = self.state()?.flights.entry(id).or_default().clone();
        let _flight = flight.lock().await;
        let (generation, token, retry_save, cancellation) = {
            let mut state = self.state()?;
            if state.selected != Some(id) || !state.accounts.contains_key(&id) {
                return Err(CoreError::new(ErrorCode::OperationCancelled));
            }
            if state.interactive.is_some() {
                return Err(CoreError::new(ErrorCode::OperationBusy));
            }
            if !force
                && let Some(session) = state
                    .sessions
                    .get(&id)
                    .filter(|session| !session.needs_renewal())
            {
                return Ok(session.clone());
            }
            let token = if let Some(token) = state.unsaved_refresh.get(&id) {
                Secret::new(token.expose().to_owned())?
            } else {
                self.repository.refresh_token(id)?
            };
            if let Some(account) = state.accounts.get_mut(&id) {
                account.status = SessionStatus::Authenticating;
                account.error = None;
            }
            (
                state.generation,
                token,
                state.unsaved_refresh.contains_key(&id),
                state.cancellation.subscribe(),
            )
        };
        let result = async {
            let microsoft = cancellable(cancellation.clone(), self.client.renew(&token)).await?;
            {
                let mut state = self.state()?;
                verify_account_generation(&state, id, generation)?;
                if let Some(replacement) =
                    microsoft
                        .refresh
                        .as_ref()
                        .or(if retry_save { Some(&token) } else { None })
                {
                    let save = if state.saved_ids.contains(&id) {
                        self.repository.replace_refresh_token(id, replacement)
                    } else {
                        let metadata = &state
                            .accounts
                            .get(&id)
                            .ok_or_else(|| CoreError::new(ErrorCode::OperationCancelled))?
                            .metadata;
                        self.repository
                            .persist_unsaved(metadata, replacement)
                            .map(|_| ())
                    };
                    if let Err(error) = save {
                        if !is_storage_error(error.code()) {
                            return Err(error);
                        }
                        state
                            .unsaved_refresh
                            .insert(id, Secret::new(replacement.expose().to_owned())?);
                        if let Some(account) = state.accounts.get_mut(&id) {
                            account.persisted = false;
                            account.error = Some(error.code());
                        }
                    } else {
                        state.unsaved_refresh.remove(&id);
                        state.saved_ids.insert(id);
                        if let Some(account) = state.accounts.get_mut(&id) {
                            account.persisted = true;
                            account.error = None;
                        }
                    }
                }
            }
            let session =
                cancellable(cancellation, self.client.minecraft_session(&microsoft)).await?;
            let mut state = self.state()?;
            verify_account_generation(&state, id, generation)?;
            let account = state
                .accounts
                .get_mut(&id)
                .ok_or_else(|| CoreError::new(ErrorCode::OperationCancelled))?;
            if account.metadata.profile_id != session.profile.id {
                return Err(CoreError::new(ErrorCode::AccountMismatch));
            }
            account.metadata.username = session.profile.name.clone();
            account.status = SessionStatus::Ready;
            let session = Arc::new(session);
            state.sessions.insert(id, session.clone());
            Ok(session)
        }
        .await;
        if let Err(error) = &result {
            let mut state = self.state()?;
            if verify_account_generation(&state, id, generation).is_ok() {
                if let Some(account) = state.accounts.get_mut(&id) {
                    account.status = if error.code() == ErrorCode::AuthenticationRequired {
                        SessionStatus::SignInRequired
                    } else {
                        SessionStatus::Error
                    };
                    account.error = Some(error.code());
                }
                if error.code() == ErrorCode::AuthenticationRequired {
                    state.sessions.remove(&id);
                }
            }
        }
        result
    }

    /// Future game execution consumes this validated metadata, never a UI token.
    pub async fn validate_play(&self) -> Result<JavaProfile, CoreError> {
        let (id, kind, profile, generation, already_renewed, cancellation) = {
            let state = self.state()?;
            let id = state
                .selected
                .ok_or_else(|| CoreError::new(ErrorCode::AccountNotFound))?;
            let account = state
                .accounts
                .get(&id)
                .ok_or_else(|| CoreError::new(ErrorCode::AccountNotFound))?;
            (
                id,
                account.metadata.kind,
                JavaProfile {
                    id: account.metadata.profile_id,
                    name: account.metadata.username.clone(),
                },
                state.generation,
                state
                    .sessions
                    .get(&id)
                    .is_none_or(|session| session.needs_renewal()),
                state.cancellation.subscribe(),
            )
        };
        if kind == AccountKind::Offline {
            return Ok(profile);
        }
        let session = self.acquire(id, false).await?;
        let first = cancellable(cancellation.clone(), self.client.validate(&session)).await;
        let result = match first {
            Err(error) if error.code() == ErrorCode::AuthenticationRequired && !already_renewed => {
                let renewed = self.acquire(id, true).await?;
                cancellable(cancellation, self.client.validate(&renewed)).await
            }
            other => other,
        };
        let mut state = self.state()?;
        verify_account_generation(&state, id, generation)?;
        if let Some(account) = state.accounts.get_mut(&id) {
            account.error = result.as_ref().err().map(CoreError::code);
            account.status = if result.is_ok() {
                SessionStatus::Ready
            } else {
                SessionStatus::Error
            };
        }
        if matches!(&result, Err(error) if error.code() == ErrorCode::AuthenticationRequired) {
            state.sessions.remove(&id);
        }
        result
    }

    pub fn remove(&self, id: Uuid) -> Result<RemovalOutcome, CoreError> {
        let mut state = self.state()?;
        invalidate(&mut state);
        state.accounts.remove(&id);
        state.sessions.remove(&id);
        state.unsaved_refresh.remove(&id);
        state.saved_ids.remove(&id);
        if state.selected == Some(id) {
            state.selected = None;
        }
        Ok(self.repository.remove(id))
    }

    pub fn retry_save(&self, id: Uuid) -> Result<(), CoreError> {
        let mut state = self.state()?;
        let token = state
            .unsaved_refresh
            .get(&id)
            .ok_or_else(|| CoreError::new(ErrorCode::AccountNotFound))?;
        if state.saved_ids.contains(&id) {
            self.repository.replace_refresh_token(id, token)?;
        } else {
            let metadata = &state
                .accounts
                .get(&id)
                .ok_or_else(|| CoreError::new(ErrorCode::AccountNotFound))?
                .metadata;
            self.repository.persist_unsaved(metadata, token)?;
        }
        state.unsaved_refresh.remove(&id);
        state.saved_ids.insert(id);
        if let Some(account) = state.accounts.get_mut(&id) {
            account.persisted = true;
            account.error = None;
        }
        Ok(())
    }
}

fn invalidate(state: &mut State) {
    state.generation = state.generation.wrapping_add(1);
    state.cancellation.send_replace(state.generation);
    state.pending.cancel();
    state.interactive = None;
    state.login_error = None;
    for account in state.accounts.values_mut() {
        if matches!(
            account.status,
            SessionStatus::Authenticating | SessionStatus::WaitingForBrowser
        ) {
            account.status = SessionStatus::SignInRequired;
        }
    }
}

async fn cancellable<T>(
    mut cancellation: tokio::sync::watch::Receiver<u64>,
    work: impl std::future::Future<Output = Result<T, CoreError>>,
) -> Result<T, CoreError> {
    tokio::select! {
        biased;
        _ = cancellation.changed() => Err(CoreError::new(ErrorCode::OperationCancelled)),
        result = work => result,
    }
}

fn verify_ticket(state: &State, ticket: Ticket) -> Result<(), CoreError> {
    if state.generation != ticket.generation
        || !state
            .interactive
            .is_some_and(|current| current.operation == ticket.operation)
    {
        return Err(CoreError::new(ErrorCode::OperationCancelled));
    }
    Ok(())
}

fn verify_account_generation(state: &State, id: Uuid, generation: u64) -> Result<(), CoreError> {
    if state.generation != generation
        || state.selected != Some(id)
        || !state.accounts.contains_key(&id)
    {
        return Err(CoreError::new(ErrorCode::OperationCancelled));
    }
    Ok(())
}

fn is_storage_error(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::StorageUnavailable
            | ErrorCode::StorageCommitUncertain
            | ErrorCode::PermissionDenied
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authentication::tests::{gated_client, mock_client};
    use crate::paths::AppPaths;
    use crate::storage::PrivateStore;

    fn saved_microsoft(client: AuthClient) -> (AppPaths, AccountController, Uuid) {
        let paths =
            AppPaths::new(std::env::temp_dir().join(format!("panita-auth-{}", Uuid::new_v4())))
                .unwrap_or_else(|_| panic!("path"));
        let repo =
            AccountRepository::new(PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store")));
        let refresh =
            Secret::new("fixture-refresh".to_owned()).unwrap_or_else(|_| panic!("fixture"));
        let profile = Uuid::parse_str("5627dd98-e6be-3c21-b8a8-e92344183641")
            .unwrap_or_else(|_| panic!("profile"));
        let account = repo
            .save_microsoft(profile, "PlayerOne", &refresh, None)
            .unwrap_or_else(|_| panic!("save"));
        let controller = AccountController::with_client(repo, ProcessSessionId::new(), client)
            .unwrap_or_else(|_| panic!("controller"));
        (paths, controller, account.account_id)
    }

    fn chain() -> Vec<(&'static str, u16, String)> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expiry = chrono::DateTime::<chrono::Utc>::from_timestamp((now + 3600) as i64, 0)
            .unwrap_or_default()
            .to_rfc3339();
        vec![
            ("/token", 200, r#"{"access_token":"fixture-microsoft","refresh_token":"fixture-rotated","expires_in":3600,"token_type":"Bearer"}"#.to_owned()),
            ("/xbox", 200, serde_json::json!({"Token":"fixture-xbox","NotAfter":expiry,"DisplayClaims":{"xui":[{"uhs":"1234"}]}}).to_string()),
            ("/xsts", 200, serde_json::json!({"Token":"fixture-xsts","NotAfter":expiry,"DisplayClaims":{"xui":[{"uhs":"1234"}]}}).to_string()),
            ("/minecraft", 200, r#"{"access_token":"fixture-minecraft","expires_in":3600,"token_type":"Bearer"}"#.to_owned()),
            ("/profile", 200, r#"{"id":"5627dd98e6be3c21b8a8e92344183641","name":"PlayerOne"}"#.to_owned()),
        ]
    }

    #[tokio::test]
    async fn sign_out_during_remote_renewal_cannot_restore_credentials() {
        let (client, task, observed, resume) = gated_client(chain().into_iter().take(1).collect());
        let (paths, controller, id) = saved_microsoft(client);
        let controller = Arc::new(controller);
        let worker = controller.clone();
        let renewal = tokio::spawn(async move { worker.renew_selected().await });
        assert!(observed.await.is_ok());
        assert!(matches!(controller.remove(id), Ok(RemovalOutcome::Removed)));
        assert!(resume.send(()).is_ok());
        assert!(
            matches!(renewal.await, Ok(Err(error)) if error.code() == ErrorCode::OperationCancelled)
        );
        assert!(
            controller
                .repository
                .snapshot()
                .unwrap_or_else(|_| panic!("snapshot"))
                .accounts
                .is_empty()
        );
        assert!(controller.repository.refresh_token(id).is_err());
        assert!(task.join().is_ok());
        drop(controller);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[tokio::test]
    async fn duplicate_callback_during_exchange_does_not_cancel_valid_login() {
        let (client, task, observed, resume) = gated_client(chain());
        let (paths, controller, id) = saved_microsoft(client);
        let controller = Arc::new(controller);
        let launch = controller
            .begin_login(Some(id))
            .unwrap_or_else(|_| panic!("login"));
        let url = url::Url::parse(&launch.browser_url).unwrap_or_else(|_| panic!("url"));
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_else(|| panic!("state"));
        let callback = format!(
            "{}?state={state}&code=fixture",
            crate::oauth::DEEP_LINK_CALLBACK
        );
        let worker = controller.clone();
        let first_callback = callback.clone();
        let login = tokio::spawn(async move { worker.callback(&first_callback).await });
        assert!(observed.await.is_ok());
        assert!(controller.callback(&callback).await.is_err());
        assert!(
            controller
                .view()
                .unwrap_or_else(|_| panic!("view"))
                .login_pending
        );
        assert!(resume.send(()).is_ok());
        assert!(matches!(login.await, Ok(Ok(()))));
        assert_eq!(
            controller
                .view()
                .unwrap_or_else(|_| panic!("view"))
                .accounts
                .len(),
            1
        );
        assert!(task.join().is_ok());
        drop(controller);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[tokio::test]
    async fn login_save_failure_keeps_memory_session_and_retries_same_identity() {
        let (client, task) = mock_client(chain());
        let paths =
            AppPaths::new(std::env::temp_dir().join(format!("panita-memory-{}", Uuid::new_v4())))
                .unwrap_or_else(|_| panic!("path"));
        let repo =
            AccountRepository::new(PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store")));
        let controller = AccountController::with_client(repo, ProcessSessionId::new(), client)
            .unwrap_or_else(|_| panic!("controller"));
        assert!(std::fs::create_dir(paths.accounts()).is_ok());
        let launch = controller
            .begin_login(None)
            .unwrap_or_else(|_| panic!("login"));
        let url = url::Url::parse(&launch.browser_url).unwrap_or_else(|_| panic!("url"));
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_else(|| panic!("state"));
        assert!(
            controller
                .callback(&format!(
                    "{}?state={state}&code=fixture",
                    crate::oauth::DEEP_LINK_CALLBACK
                ))
                .await
                .is_ok()
        );
        let view = controller.view().unwrap_or_else(|_| panic!("view"));
        assert!(!view.accounts[0].persisted);
        assert!(matches!(view.accounts[0].status, SessionStatus::Ready));
        let id = view.accounts[0].metadata.account_id;
        assert!(std::fs::remove_dir(paths.accounts()).is_ok());
        assert!(controller.retry_save(id).is_ok());
        assert_eq!(
            controller
                .repository
                .snapshot()
                .unwrap_or_else(|_| panic!("saved"))
                .selected_account_id,
            Some(id)
        );
        assert!(
            controller
                .view()
                .unwrap_or_else(|_| panic!("view"))
                .accounts[0]
                .persisted
        );
        assert!(task.join().is_ok());
        drop(controller);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[tokio::test]
    async fn rotation_is_saved_before_downstream_failure_without_deleting_profile() {
        let mut responses = chain();
        responses.truncate(2);
        responses[1] = ("/xbox", 503, "{}".to_owned());
        let (client, task) = mock_client(responses);
        let (paths, controller, id) = saved_microsoft(client);
        assert!(
            matches!(controller.renew_selected().await, Err(error) if error.code() == ErrorCode::ServiceUnavailable)
        );
        assert_eq!(
            controller
                .repository
                .refresh_token(id)
                .unwrap_or_else(|_| panic!("rotation"))
                .expose(),
            "fixture-rotated"
        );
        assert_eq!(
            controller
                .view()
                .unwrap_or_else(|_| panic!("view"))
                .accounts
                .len(),
            1
        );
        assert!(task.join().is_ok());
        drop(controller);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[tokio::test]
    async fn play_rejection_renews_once_and_persistent_rejection_blocks() {
        let mut responses = chain();
        responses.push(("/profile", 401, "{}".to_owned()));
        responses.extend(chain());
        responses.push(("/profile", 401, "{}".to_owned()));
        let (client, task) = mock_client(responses);
        let (paths, controller, _) = saved_microsoft(client);
        assert!(controller.renew_selected().await.is_ok());
        assert!(
            matches!(controller.validate_play().await, Err(error) if error.code() == ErrorCode::AuthenticationRequired)
        );
        assert!(
            controller
                .state()
                .unwrap_or_else(|_| panic!("state"))
                .sessions
                .is_empty()
        );
        assert!(task.join().is_ok());
        drop(controller);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[tokio::test]
    async fn concurrent_renewal_shares_the_result_and_permission_errors_do_not_retry() {
        let mut responses = chain();
        responses.push(("/profile", 403, "{}".to_owned()));
        let (client, task) = mock_client(responses);
        let (paths, controller, _) = saved_microsoft(client);
        let (first, second) =
            tokio::join!(controller.renew_selected(), controller.renew_selected());
        assert!(first.is_ok() && second.is_ok());
        assert!(
            matches!(controller.validate_play().await, Err(error) if error.code() == ErrorCode::AccountRestricted)
        );
        assert!(task.join().is_ok());
        drop(controller);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }

    #[test]
    fn switching_and_removal_invalidate_inflight_authentication() {
        let paths =
            AppPaths::new(std::env::temp_dir().join(format!("panita-sessions-{}", Uuid::new_v4())))
                .unwrap_or_else(|_| panic!("path"));
        let repo =
            AccountRepository::new(PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store")));
        let controller = AccountController::new(repo, ProcessSessionId::new())
            .unwrap_or_else(|_| panic!("controller"));
        let account = controller
            .add_offline("PlayerOne")
            .unwrap_or_else(|_| panic!("account"));
        let launch = controller
            .begin_login(None)
            .unwrap_or_else(|_| panic!("login"));
        let ticket = controller
            .state()
            .unwrap_or_else(|_| panic!("state"))
            .interactive
            .unwrap_or_else(|| panic!("ticket"));
        assert_eq!(launch.operation_id, ticket.operation.value());
        assert!(controller.select(account.account_id).is_ok());
        assert!(controller.check_ticket(ticket).is_err());
        assert!(matches!(
            controller.remove(account.account_id),
            Ok(RemovalOutcome::Removed)
        ));
        assert!(
            controller
                .view()
                .unwrap_or_else(|_| panic!("view"))
                .accounts
                .is_empty()
        );
        drop(controller);
        assert!(std::fs::remove_dir_all(paths.root()).is_ok());
    }
}
