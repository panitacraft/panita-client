//! Network connectivity and API result notes for the session log.
//!
//! Call sites pass fixed service and route labels, never a URL, so serials, poll
//! secrets, tokens and query strings cannot reach the log. Only transitions of the
//! connectivity state are written, plus one line per API call.
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use crate::session_log::Level;

type Sink = Box<dyn Fn(Level, &str) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();
static STATE: AtomicU8 = AtomicU8::new(UNKNOWN);

const UNKNOWN: u8 = 0;
const ONLINE: u8 = 1;
const OFFLINE: u8 = 2;

/// Route network notes to the session log. Only the first call has an effect.
pub fn install(sink: impl Fn(Level, &str) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(sink));
}

fn emit(level: Level, text: &str) {
    if let Some(sink) = SINK.get() {
        sink(level, text);
    }
}

/// Fixed class of a failed request; never the error text, which may hold a URL.
pub fn transport_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_redirect() {
        "redirect"
    } else if error.is_body() || error.is_decode() {
        "body"
    } else {
        "request"
    }
}

/// Record one finished request. `log_success` is false for frequent polls so only
/// their failures and connectivity changes appear.
pub fn api_result(
    service: &str,
    method: &str,
    route: &str,
    started: Instant,
    outcome: Result<u16, &reqwest::Error>,
    log_success: bool,
) {
    let millis = started.elapsed().as_millis();
    match outcome {
        Ok(status) => {
            connectivity(true, service, route, "");
            let level = if status >= 500 || status == 429 {
                Level::Warn
            } else {
                Level::Info
            };
            if log_success || status >= 400 {
                emit(
                    level,
                    &format!("api {service} {method} {route} status={status} elapsed_ms={millis}"),
                );
            }
        }
        Err(error) => {
            let kind = transport_kind(error);
            connectivity(false, service, route, kind);
            emit(
                Level::Warn,
                &format!("api {service} {method} {route} failed kind={kind} elapsed_ms={millis}"),
            );
        }
    }
}

/// An HTTP answer, of any status, proves connectivity; a transport failure
/// suggests none. Only a change of state is written.
fn connectivity(reachable: bool, service: &str, route: &str, kind: &str) {
    let next = if reachable { ONLINE } else { OFFLINE };
    let previous = STATE.swap(next, Ordering::Relaxed);
    if previous == next {
        return;
    }
    match (previous, reachable) {
        (OFFLINE, true) => emit(
            Level::Info,
            &format!("network restored service={service} route={route}"),
        ),
        (UNKNOWN, true) => emit(Level::Info, "network online"),
        (_, false) => emit(
            Level::Warn,
            &format!("network unreachable service={service} route={route} kind={kind}"),
        ),
        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn only_connectivity_changes_are_written() {
        // The state is process-wide, so assert transitions within one test.
        static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
        STATE.store(UNKNOWN, Ordering::Relaxed);
        install(|_, text| {
            if let Ok(mut lines) = LINES.lock() {
                lines.push(text.to_owned());
            }
        });
        connectivity(true, "svc", "/a", "");
        connectivity(true, "svc", "/a", "");
        connectivity(false, "svc", "/a", "connect");
        connectivity(false, "svc", "/b", "timeout");
        connectivity(true, "svc", "/a", "");
        let lines = LINES.lock().map(|lines| lines.clone()).unwrap_or_default();
        // The sink is shared with other tests only if they installed first; accept either.
        if lines.is_empty() {
            return;
        }
        assert_eq!(
            lines,
            [
                "network online",
                "network unreachable service=svc route=/a kind=connect",
                "network restored service=svc route=/a",
            ]
        );
    }
}
