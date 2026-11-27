//! Weekly health check of the email provider's key (hourly while it fails).
//!
//! Only the configured transport is checked (`MAIL_TRANSPORT`): the other
//! providers are off. Every provider with a key is checked the same way:
//! Brevo, Resend, Postmark, Mailgun, Scaleway, and SMTP with a login.
//!
//! - Brevo deactivates API keys that go unused for 90 days: if every app keeps
//!   pinging, no alert goes out and the key expires silently. The check is an
//!   authenticated read, which keeps the key in use.
//! - The others do not expire unused keys, but a key can still be revoked or
//!   deleted: the weekly health check finds out before the next alert does.
//!
//! A refused key cannot be reported by email through the same key. Instead,
//! while the key is refused the entanglement pings are held back, so the other
//! instances see this one go down and alert with their own email. The outcome
//! is also logged and shown to the master (`/api/me` → `mail_key`).

use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;

use crate::{AppState, clock, mail::KeyCheck};

/// Weekly health check, the same for every provider. Far within Brevo's 90 days.
const CHECK_EVERY: Duration = Duration::from_secs(7 * 24 * 3600);
/// After a network error, an unexpected answer or a refusal: a refusal by
/// mistake (an incident at the provider) must not hold the pings for a week.
const RETRY_AFTER: Duration = Duration::from_secs(3600);

pub const REJECTED: &str = "rejected";

/// Last outcome of the check, shown to the master.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct KeyStatus {
    /// Provider that was checked (`Mailer::describe`).
    pub provider: &'static str,
    /// `ok`, `rejected` or `unreachable`.
    pub state: &'static str,
    /// Unix milliseconds.
    pub checked_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Shared outcome of the latest check: `None` before the first one, or without a key.
pub struct KeyMonitor {
    /// `false` without a key (SMTP without login): no check.
    enabled: bool,
    status: watch::Sender<Option<KeyStatus>>,
}

impl KeyMonitor {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, status: watch::Sender::new(None) }
    }

    pub fn get(&self) -> Option<KeyStatus> {
        self.status.borrow().clone()
    }

    pub(crate) fn set(&self, status: KeyStatus) {
        self.status.send_replace(Some(status));
    }

    /// The key was refused: alerts cannot be sent, entanglement pings are held.
    pub fn rejected(&self) -> bool {
        self.status.borrow().as_ref().is_some_and(|s| s.state == REJECTED)
    }

    /// Waits for the first check (immediately without a key), so that at startup
    /// no entanglement ping goes out before we know whether the key works.
    pub async fn first_check(&self) {
        if self.enabled {
            let _ = self.status.subscribe().wait_for(Option::is_some).await;
        }
    }
}

/// Checks the key at startup and then every week. Nothing without a key.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        while let Some(check) = state.mailer.check_key().await {
            let wait = record(&state, check);
            tokio::time::sleep(wait).await;
        }
    });
}

/// Stores the outcome and returns when to check again.
fn record(state: &AppState, check: KeyCheck) -> Duration {
    let checked_at = clock::now_ms();
    let provider = state.mailer.describe();
    let status = |state, error| KeyStatus { provider, state, checked_at, error };
    match check {
        KeyCheck::Valid => {
            if state.mail_key.rejected() {
                tracing::info!(provider, "email key valid again: entanglement pings resume");
            } else {
                tracing::info!(provider, "email key valid");
            }
            state.mail_key.set(status("ok", None));
            CHECK_EVERY
        }
        KeyCheck::Rejected(detail) => {
            tracing::error!(
                provider,
                %detail,
                "the email provider refused the key: alert emails cannot be sent, entanglement pings are held"
            );
            state.mail_key.set(status(REJECTED, Some(detail)));
            RETRY_AFTER
        }
        KeyCheck::Unreachable(detail) => {
            tracing::warn!(provider, %detail, "email key check failed: retrying in an hour");
            // A temporary error does not hide an earlier refusal.
            if !state.mail_key.rejected() {
                state.mail_key.set(status("unreachable", Some(detail)));
            }
            RETRY_AFTER
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{crypto, db, heartbeat};

    async fn state() -> AppState {
        let pool = db::test_pool().await;
        let secret_box = crypto::SecretBox::new(&[1; 32], 1);
        let hb = Arc::new(heartbeat::Heartbeat::load(&pool, secret_box, None).await.unwrap());
        crate::test_state(pool, hb)
    }

    #[tokio::test]
    async fn records_outcomes() {
        let state = state().await;

        assert_eq!(record(&state, KeyCheck::Valid), CHECK_EVERY);
        assert_eq!(state.mail_key.get().unwrap().state, "ok");
        assert!(!state.mail_key.rejected());

        assert_eq!(record(&state, KeyCheck::Rejected("401".into())), RETRY_AFTER);
        let status = state.mail_key.get().unwrap();
        assert_eq!((status.state, status.error.as_deref()), (REJECTED, Some("401")));
        assert!(state.mail_key.rejected());

        // A network error later does not hide the refusal.
        assert_eq!(record(&state, KeyCheck::Unreachable("timeout".into())), RETRY_AFTER);
        assert!(state.mail_key.rejected());

        record(&state, KeyCheck::Valid);
        assert!(!state.mail_key.rejected());
        // A network error alone does not hold the pings.
        record(&state, KeyCheck::Unreachable("timeout".into()));
        assert_eq!(state.mail_key.get().unwrap().state, "unreachable");
        assert!(!state.mail_key.rejected());
    }

    #[tokio::test]
    async fn first_check_waits_only_with_a_key() {
        // No key: nothing to wait for.
        KeyMonitor::new(false).first_check().await;

        let monitor = Arc::new(KeyMonitor::new(true));
        let waiting = tokio::spawn({
            let monitor = monitor.clone();
            async move { monitor.first_check().await }
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        monitor.set(KeyStatus { provider: "Brevo", state: "ok", checked_at: 1, error: None });
        waiting.await.unwrap();
    }
}
