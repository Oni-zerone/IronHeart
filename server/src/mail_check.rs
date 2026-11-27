//! Weekly check of the Brevo API key.
//!
//! Brevo deactivates API keys that go unused for 90 days: if every app keeps
//! pinging, no alert goes out and the key expires silently. An authenticated
//! read once a week keeps the key in use and tells us when it stops working.
//!
//! A refused key cannot be reported by email through the same key, so the
//! outcome is also logged and shown to the master in the dashboard
//! (`/api/me` → `mail_key`). The email is still attempted.

use std::{sync::Mutex, time::Duration};

use serde::Serialize;

use crate::{AppState, alerts, clock, mail::KeyCheck};

const CHECK_EVERY: Duration = Duration::from_secs(7 * 24 * 3600);
/// After a network error or an unexpected answer.
const RETRY_AFTER: Duration = Duration::from_secs(3600);

/// Last outcome of the check, shown to the master.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct KeyStatus {
    /// `ok`, `rejected` or `unreachable`.
    pub state: &'static str,
    /// Unix milliseconds.
    pub checked_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Shared outcome of the latest check: `None` before the first one, or with SMTP.
#[derive(Default)]
pub struct KeyMonitor(Mutex<Option<KeyStatus>>);

impl KeyMonitor {
    pub fn get(&self) -> Option<KeyStatus> {
        self.0.lock().expect("key status lock").clone()
    }

    pub(crate) fn set(&self, status: KeyStatus) {
        *self.0.lock().expect("key status lock") = Some(status);
    }
}

/// Checks the key at startup and then every week. Nothing with SMTP.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        // Alert only once per refused key, not at every check.
        let mut alerted = false;
        while let Some(check) = state.mailer.check_key().await {
            let wait = record(&state, check, &mut alerted).await;
            tokio::time::sleep(wait).await;
        }
    });
}

/// Stores the outcome, alerts if needed and returns when to check again.
async fn record(state: &AppState, check: KeyCheck, alerted: &mut bool) -> Duration {
    let checked_at = clock::now_ms();
    match check {
        KeyCheck::Valid => {
            tracing::info!("Brevo API key valid");
            state.mail_key.set(KeyStatus { state: "ok", checked_at, error: None });
            *alerted = false;
            CHECK_EVERY
        }
        KeyCheck::Rejected(detail) => {
            tracing::error!(%detail, "Brevo refused the API key: alert emails cannot be sent");
            state.mail_key.set(KeyStatus { state: "rejected", checked_at, error: Some(detail.clone()) });
            if !*alerted {
                *alerted = true;
                let email = alerts::mail_key_rejected(&state.admin_email, &detail, &state.public_base_url);
                if let Err(err) = state.mailer.send(&email).await {
                    tracing::error!(err = ?err, "Brevo key alert not sent");
                }
            }
            CHECK_EVERY
        }
        KeyCheck::Unreachable(detail) => {
            tracing::warn!(%detail, "Brevo API key check failed: retrying in an hour");
            // A temporary error does not hide an earlier refusal.
            if state.mail_key.get().is_none_or(|s| s.state != "rejected") {
                state.mail_key.set(KeyStatus { state: "unreachable", checked_at, error: Some(detail) });
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
    async fn records_outcomes_and_alerts_once() {
        let state = state().await;
        let mut alerted = false;

        assert_eq!(record(&state, KeyCheck::Valid, &mut alerted).await, CHECK_EVERY);
        assert_eq!(state.mail_key.get().unwrap().state, "ok");

        let wait = record(&state, KeyCheck::Rejected("401".into()), &mut alerted).await;
        assert_eq!(wait, CHECK_EVERY);
        assert!(alerted);
        let status = state.mail_key.get().unwrap();
        assert_eq!((status.state, status.error.as_deref()), ("rejected", Some("401")));

        // A network error later does not hide the refusal.
        assert_eq!(record(&state, KeyCheck::Unreachable("timeout".into()), &mut alerted).await, RETRY_AFTER);
        assert_eq!(state.mail_key.get().unwrap().state, "rejected");

        record(&state, KeyCheck::Valid, &mut alerted).await;
        assert!(!alerted, "a valid key re-arms the alert");
        record(&state, KeyCheck::Unreachable("timeout".into()), &mut alerted).await;
        assert_eq!(state.mail_key.get().unwrap().state, "unreachable");
    }
}
