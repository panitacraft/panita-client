//! Panita activation: a community gate that is independent of Minecraft
//! authentication. Operation generations protect disk and memory from late
//! results, as in `sessions`. Only the API's `invalid_token` answer ends an
//! activation; a network failure never does.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;

use crate::accounts::Secret;
use crate::error::{CoreError, ErrorCode};
use crate::operation::OperationId;
use crate::panita_api::{
    DeviceInfo, LoginRequest, PanitaApiClient, PanitaApiError, PanitaCredentials, PollOutcome,
    now_seconds,
};
use crate::panita_session::{self, PanitaSession, PanitaUser};
use crate::storage::PrivateStore;

const POLL_INTERVAL: Duration = Duration::from_millis(2500);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationState {
    NoSession,
    /// The serial is being requested.
    Creating,
    /// The serial is shown and the member has to confirm it in Discord.
    Pending,
    Active,
    /// The session could not be verified right now; installed instances still launch.
    ActiveOffline,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActivationView {
    pub revision: u64,
    pub state: ActivationState,
    pub user: Option<PanitaUser>,
    /// Unix seconds of the last successful verification.
    pub verified_at: Option<u64>,
    pub serial: Option<String>,
    /// Unix seconds when the pending serial stops being valid.
    pub expires_at: Option<i64>,
    /// False while an activated session could not be written to disk.
    pub persisted: bool,
    pub error: Option<ErrorCode>,
    /// Seconds the API asked the member to wait, with a rate-limit error.
    pub retry_after: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshOutcome {
    NoSession,
    Verified,
    Offline,
    /// The API closed the session; it was removed locally.
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignOutOutcome {
    /// Whether the API confirmed the session is closed. Offline, it stays open
    /// on the server until the member closes it with the bot.
    pub server_closed: bool,
}

#[derive(Clone, Copy)]
struct Ticket {
    operation: OperationId,
    generation: u64,
}

#[derive(Clone)]
struct PendingLogin {
    ticket: Ticket,
    request: Arc<LoginRequest>,
}

enum Phase {
    NoSession,
    Creating(Ticket),
    Pending(PendingLogin),
    Active {
        session: PanitaSession,
        offline: bool,
        persisted: bool,
    },
}

struct State {
    generation: u64,
    cancellation: tokio::sync::watch::Sender<u64>,
    phase: Phase,
    error: Option<(ErrorCode, Option<u32>)>,
}

pub struct ActivationController {
    snapshot_counter: AtomicU64,
    store: PrivateStore,
    client: PanitaApiClient,
    /// The launcher key alone, or why this build has none.
    key: Result<PanitaCredentials, ErrorCode>,
    poll_interval: Duration,
    state: Mutex<State>,
}

impl ActivationController {
    pub fn new(store: PrivateStore) -> Result<Self, CoreError> {
        Self::with_parts(
            store,
            PanitaApiClient::new()?,
            PanitaCredentials::launcher().map_err(|error| error.code()),
            POLL_INTERVAL,
        )
    }

    /// A damaged or unsupported session file does not stop the launcher: it is
    /// preserved untouched, reported through the view, and blocks activation.
    pub(crate) fn with_parts(
        store: PrivateStore,
        client: PanitaApiClient,
        key: Result<PanitaCredentials, ErrorCode>,
        poll_interval: Duration,
    ) -> Result<Self, CoreError> {
        let (phase, error) = match panita_session::load(&store) {
            Ok(Some(session)) => (
                Phase::Active {
                    session,
                    offline: false,
                    persisted: true,
                },
                None,
            ),
            Ok(None) => (Phase::NoSession, None),
            Err(error) => (Phase::NoSession, Some((error.code(), None))),
        };
        Ok(Self {
            snapshot_counter: AtomicU64::new(0),
            store,
            client,
            key,
            poll_interval,
            state: Mutex::new(State {
                generation: 0,
                cancellation: tokio::sync::watch::channel(0).0,
                phase,
                error,
            }),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, CoreError> {
        self.state
            .lock()
            .map_err(|_| CoreError::new(ErrorCode::StorageUnavailable))
    }

    fn anonymous(&self) -> Result<PanitaCredentials, CoreError> {
        self.key.clone().map_err(CoreError::new)
    }

    pub fn view(&self) -> Result<ActivationView, CoreError> {
        let state = self.lock()?;
        let mut view = ActivationView {
            revision: self.snapshot_counter.fetch_add(1, Ordering::Relaxed),
            state: ActivationState::NoSession,
            user: None,
            verified_at: None,
            serial: None,
            expires_at: None,
            persisted: true,
            error: state.error.map(|(code, _)| code),
            retry_after: state.error.and_then(|(_, wait)| wait),
        };
        match &state.phase {
            Phase::NoSession => {}
            Phase::Creating(_) => view.state = ActivationState::Creating,
            Phase::Pending(pending) => {
                view.state = ActivationState::Pending;
                view.serial = Some(pending.request.serial.clone());
                view.expires_at = Some(pending.request.expires_at);
            }
            Phase::Active {
                session,
                offline,
                persisted,
            } => {
                view.state = if *offline {
                    ActivationState::ActiveOffline
                } else {
                    ActivationState::Active
                };
                view.user = Some(session.user.clone());
                view.verified_at = Some(session.verified_at);
                view.persisted = *persisted;
            }
        }
        Ok(view)
    }

    pub fn state(&self) -> Result<ActivationState, CoreError> {
        Ok(self.view()?.state)
    }

    /// The Panita account id of the active session, used to bind cached catalogs.
    pub fn identity(&self) -> Result<Option<u64>, CoreError> {
        Ok(match &self.lock()?.phase {
            Phase::Active { session, .. } => Some(session.user.id),
            _ => None,
        })
    }

    /// The gate for every authenticated call: key plus session of an activated launcher.
    pub fn session_credentials(&self) -> Result<PanitaCredentials, CoreError> {
        match &self.lock()?.phase {
            Phase::Active { session, .. } => Ok(self.anonymous()?.with_session(session)),
            _ => Err(CoreError::new(ErrorCode::ActivationRequired)),
        }
    }

    /// Install, update, repair and catalog refresh need a recently verified
    /// session, so they refuse while the session could not be checked.
    pub fn online_credentials(&self) -> Result<PanitaCredentials, CoreError> {
        if matches!(self.lock()?.phase, Phase::Active { offline: true, .. }) {
            return Err(CoreError::new(ErrorCode::NetworkUnavailable));
        }
        self.session_credentials()
    }

    /// Ask for a serial. The member then confirms it in Discord while
    /// `await_confirmation` polls.
    pub async fn begin(&self, identifier: &str, device: &DeviceInfo) -> Result<(), CoreError> {
        let credentials = self.anonymous()?;
        let (ticket, cancellation) = {
            let mut state = self.lock()?;
            if !matches!(state.phase, Phase::NoSession) {
                return Err(CoreError::new(ErrorCode::OperationBusy));
            }
            // Never ask a member to confirm into a file that cannot be written.
            match panita_session::load(&self.store) {
                Ok(None) => {}
                Ok(Some(session)) => {
                    // Another launcher process activated first.
                    state.phase = Phase::Active {
                        session,
                        offline: false,
                        persisted: true,
                    };
                    state.error = None;
                    return Err(CoreError::new(ErrorCode::OperationBusy));
                }
                Err(error) => {
                    state.error = Some((error.code(), None));
                    return Err(error);
                }
            }
            invalidate(&mut state);
            let ticket = Ticket {
                operation: OperationId::new(),
                generation: state.generation,
            };
            state.phase = Phase::Creating(ticket);
            (ticket, state.cancellation.subscribe())
        };
        let created = cancellable(
            cancellation,
            self.client
                .create_login_request(&credentials, identifier, device),
        )
        .await;
        let mut state = self.lock()?;
        let current = matches!(&state.phase, Phase::Creating(t) if same(*t, ticket))
            && state.generation == ticket.generation;
        match created {
            Ok(request) if current => {
                state.phase = Phase::Pending(PendingLogin {
                    ticket,
                    request: Arc::new(request),
                });
                Ok(())
            }
            Ok(_) => Err(CoreError::new(ErrorCode::OperationCancelled)),
            Err(error) => {
                if current && error.code != ErrorCode::OperationCancelled {
                    state.phase = Phase::NoSession;
                    state.error = Some((error.code, error.retry_after));
                }
                Err(error.into())
            }
        }
    }

    /// Poll until the member confirms, the serial expires or the login is cancelled.
    /// Transient network failures keep waiting: the serial's three minutes bound it.
    pub async fn await_confirmation(&self) -> Result<(), CoreError> {
        let credentials = self.anonymous()?;
        let (pending, cancellation) = {
            let state = self.lock()?;
            match &state.phase {
                Phase::Pending(pending) => (pending.clone(), state.cancellation.subscribe()),
                _ => return Err(CoreError::new(ErrorCode::OperationCancelled)),
            }
        };
        loop {
            if now_seconds() as i64 >= pending.request.expires_at {
                return self.fail_login(pending.ticket, ErrorCode::ActivationExpired.into());
            }
            let outcome = cancellable(
                cancellation.clone(),
                self.client.poll(&credentials, &pending.request),
            )
            .await;
            match outcome {
                Ok(PollOutcome::Confirmed(session)) => {
                    return self.complete_login(pending.ticket, session);
                }
                Ok(PollOutcome::Pending { .. }) => self.note_poll(pending.ticket, None)?,
                Err(error)
                    if matches!(
                        error.code,
                        ErrorCode::NetworkUnavailable | ErrorCode::ServiceUnavailable
                    ) =>
                {
                    self.note_poll(pending.ticket, Some(error.code))?;
                }
                Err(error) => return self.fail_login(pending.ticket, error),
            }
            let mut wake = cancellation.clone();
            tokio::select! {
                biased;
                _ = wake.changed() => return Err(CoreError::new(ErrorCode::OperationCancelled)),
                () = tokio::time::sleep(self.poll_interval) => {}
            }
        }
    }

    fn note_poll(&self, ticket: Ticket, warning: Option<ErrorCode>) -> Result<(), CoreError> {
        let mut state = self.lock()?;
        if pending_is(&state, ticket) {
            state.error = warning.map(|code| (code, None));
        }
        Ok(())
    }

    fn fail_login(&self, ticket: Ticket, error: PanitaApiError) -> Result<(), CoreError> {
        let mut state = self.lock()?;
        if error.code != ErrorCode::OperationCancelled && pending_is(&state, ticket) {
            state.phase = Phase::NoSession;
            state.error = Some((error.code, error.retry_after));
        }
        Err(error.into())
    }

    /// Persist before reporting the session as active. A failed write keeps a
    /// memory-only session and says so; it never claims the write succeeded.
    fn complete_login(&self, ticket: Ticket, session: PanitaSession) -> Result<(), CoreError> {
        let mut state = self.lock()?;
        if !pending_is(&state, ticket) {
            return Err(CoreError::new(ErrorCode::OperationCancelled));
        }
        let saved = panita_session::save(&self.store, &session);
        state.error = saved.as_ref().err().map(|error| (error.code(), None));
        state.phase = Phase::Active {
            session,
            offline: false,
            persisted: saved.is_ok(),
        };
        Ok(())
    }

    pub fn cancel(&self) -> Result<(), CoreError> {
        invalidate(&mut *self.lock()?);
        Ok(())
    }

    /// Retry writing a session that is active only in memory.
    pub fn retry_save(&self) -> Result<(), CoreError> {
        let mut state = self.lock()?;
        let Phase::Active {
            session, persisted, ..
        } = &mut state.phase
        else {
            return Err(CoreError::new(ErrorCode::ActivationRequired));
        };
        if *persisted {
            return Ok(());
        }
        let saved = panita_session::save(&self.store, session);
        *persisted = saved.is_ok();
        state.error = saved.as_ref().err().map(|error| (error.code(), None));
        saved
    }

    /// Check the session with the API. A network, timeout or service failure
    /// leaves the activation and the file untouched.
    pub async fn refresh(&self) -> Result<RefreshOutcome, CoreError> {
        let (session, generation, cancellation) = {
            let state = self.lock()?;
            let Phase::Active { session, .. } = &state.phase else {
                return Ok(RefreshOutcome::NoSession);
            };
            (
                session.clone(),
                state.generation,
                state.cancellation.subscribe(),
            )
        };
        let credentials = self.anonymous()?.with_session(&session);
        match cancellable(cancellation, self.client.me(&credentials)).await {
            Ok(user) => self.record_verified(&session, generation, user),
            Err(error) if error.code == ErrorCode::SessionRejected => {
                self.reject(session.token())?;
                Ok(RefreshOutcome::Rejected)
            }
            Err(error) if error.code == ErrorCode::OperationCancelled => Err(error.into()),
            Err(error) => {
                self.record_offline(&session, generation, error.code)?;
                Ok(RefreshOutcome::Offline)
            }
        }
    }

    fn record_verified(
        &self,
        checked: &PanitaSession,
        generation: u64,
        user: PanitaUser,
    ) -> Result<RefreshOutcome, CoreError> {
        let mut state = self.lock()?;
        let current_generation = state.generation;
        let Phase::Active {
            session,
            offline,
            persisted,
        } = &mut state.phase
        else {
            return Err(CoreError::new(ErrorCode::OperationCancelled));
        };
        if current_generation != generation || !Arc::ptr_eq(session.token(), checked.token()) {
            return Err(CoreError::new(ErrorCode::OperationCancelled));
        }
        session.user = user;
        session.verified_at = now_seconds();
        *offline = false;
        // The snapshot is a cache: a failed write is retried on the next check.
        // A session that was memory-only gets another chance to be saved.
        if *persisted {
            let _ = panita_session::refresh_snapshot(
                &self.store,
                session.token(),
                session.user.clone(),
                session.verified_at,
            );
        } else {
            *persisted = panita_session::save(&self.store, session).is_ok();
        }
        state.error = None;
        Ok(RefreshOutcome::Verified)
    }

    fn record_offline(
        &self,
        checked: &PanitaSession,
        generation: u64,
        code: ErrorCode,
    ) -> Result<(), CoreError> {
        let mut state = self.lock()?;
        let current_generation = state.generation;
        let Phase::Active {
            session, offline, ..
        } = &mut state.phase
        else {
            return Ok(());
        };
        if current_generation == generation && Arc::ptr_eq(session.token(), checked.token()) {
            *offline = true;
            state.error = Some((code, None));
        }
        Ok(())
    }

    /// The API refused this exact session token with `invalid_token`. Remove it
    /// from memory and disk unless a newer activation has replaced it.
    pub fn note_rejection(&self, credentials: &PanitaCredentials) -> Result<bool, CoreError> {
        match credentials.token() {
            Some(token) => self.reject(token),
            None => Ok(false),
        }
    }

    fn reject(&self, token: &Arc<Secret>) -> Result<bool, CoreError> {
        let mut state = self.lock()?;
        let current = matches!(
            &state.phase,
            Phase::Active { session, .. } if Arc::ptr_eq(session.token(), token)
        );
        if !current {
            return Ok(false);
        }
        invalidate(&mut state);
        state.phase = Phase::NoSession;
        state.error = Some((ErrorCode::SessionRejected, None));
        // A file that cannot be removed now is rejected again on the next start.
        let _ = panita_session::clear(&self.store, Some(token));
        let _ = crate::catalog::purge_cache(&self.store);
        Ok(true)
    }

    /// Close the session on the API as a courtesy, then always remove it locally.
    /// A pending login is simply cancelled.
    pub async fn sign_out(&self) -> Result<SignOutOutcome, CoreError> {
        let session = {
            let mut state = self.lock()?;
            invalidate(&mut state);
            let previous = std::mem::replace(&mut state.phase, Phase::NoSession);
            state.error = None;
            match previous {
                Phase::Active { session, .. } => session,
                _ => {
                    return Ok(SignOutOutcome {
                        server_closed: true,
                    });
                }
            }
        };
        let credentials = self.anonymous().map(|key| key.with_session(&session));
        let server_closed = match credentials {
            Ok(credentials) => match self.client.sign_out(&credentials).await {
                Ok(()) => true,
                // Already closed counts as closed.
                Err(error) => error.code == ErrorCode::SessionRejected,
            },
            Err(_) => false,
        };
        let cleared = panita_session::clear(&self.store, Some(session.token()));
        let purged = crate::catalog::purge_cache(&self.store);
        if let Err(error) = cleared.map(|_| ()).and(purged) {
            self.lock()?.error = Some((error.code(), None));
            return Err(error);
        }
        Ok(SignOutOutcome { server_closed })
    }
}

fn same(left: Ticket, right: Ticket) -> bool {
    left.operation == right.operation
}

fn pending_is(state: &State, ticket: Ticket) -> bool {
    state.generation == ticket.generation
        && matches!(&state.phase, Phase::Pending(pending) if same(pending.ticket, ticket))
}

fn invalidate(state: &mut State) {
    state.generation = state.generation.wrapping_add(1);
    state.cancellation.send_replace(state.generation);
    state.error = None;
    if matches!(state.phase, Phase::Creating(_) | Phase::Pending(_)) {
        state.phase = Phase::NoSession;
    }
}

async fn cancellable<T>(
    mut cancellation: tokio::sync::watch::Receiver<u64>,
    work: impl std::future::Future<Output = Result<T, PanitaApiError>>,
) -> Result<T, PanitaApiError> {
    tokio::select! {
        biased;
        _ = cancellation.changed() => Err(ErrorCode::OperationCancelled.into()),
        result = work => result,
    }
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod tests;
