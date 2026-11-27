//! Daily check of the email provider's key (hourly while it fails).
//!
//! Only the configured transport is checked (`MAIL_TRANSPORT`): the other
//! providers are off. Every provider with a key is checked the same way:
//! Brevo, Resend, Postmark, Mailgun, Scaleway, and SMTP with a login.
//!
//! - Brevo deactivates API keys that go unused for 90 days: if every app keeps
//!   pinging, no alert goes out and the key expires silently. The check is an
//!   authenticated read, which keeps the key in use.
//! - The others do not expire unused keys, but a key can be revoked or deleted,
//!   and on Mailgun and Scaleway it can be created with an expiration date. The
//!   check finds out within a day instead of at the next alert.
//!
//! A refused key cannot be reported by email through the same key: `channels`
//! warns the master on Telegram if it works, and holds the entanglement pings
//! when no channel can reach the master. The outcome is also logged and shown
//! to the master (`/api/me` → `mail_key`).
//!
//! `KeyMonitor` and `record_check` also serve the Telegram token check.

use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;

use crate::{AppState, clock, mail::KeyCheck};

/// An unreachable provider counts as broken after this long without a good check.
pub const LONG_OUTAGE_MS: i64 = 3600 * 1000;

/// Daily: an expiration date or a revocation must not leave the alerts broken
/// for long. Far within Brevo's 90 days.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
/// After a network error, an unexpected answer or a refusal: a refusal by
/// mistake (an incident at the provider) must not hold the pings for a day.
const RETRY_AFTER: Duration = Duration::from_secs(3600);
/// Minimum time between a check and an early one asked by a failed send.
const EARLY_CHECK_MIN: Duration = Duration::from_secs(300);

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
    /// First failed check since the last good one (Unix milliseconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failing_since: Option<i64>,
}

/// State of an alert channel, from its latest check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// Not checked yet (startup).
    Checking,
    Ok,
    /// Network error or outage, for less than `LONG_OUTAGE_MS`.
    Unreachable,
    /// Nothing to check (SMTP without login): it may or may not work.
    Unverified,
    /// Key refused, or unreachable for more than `LONG_OUTAGE_MS`.
    Broken,
}

/// Shared outcome of the latest check: `None` before the first one, or without a key.
pub struct KeyMonitor {
    /// `false` without a key (SMTP without login): no check.
    enabled: bool,
    status: watch::Sender<Option<KeyStatus>>,
    /// Wakes the check before its time (a failed send).
    wake: tokio::sync::Notify,
}

impl KeyMonitor {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            status: watch::Sender::new(None),
            wake: tokio::sync::Notify::new(),
        }
    }

    pub fn get(&self) -> Option<KeyStatus> {
        self.status.borrow().clone()
    }

    pub(crate) fn set(&self, status: KeyStatus) {
        self.status.send_replace(Some(status));
    }

    /// A send failed: check again soon instead of waiting a day, so a long
    /// outage is noticed (at most once every `EARLY_CHECK_MIN`).
    pub fn recheck_soon(&self) {
        let recent = self
            .get()
            .is_some_and(|s| clock::now_ms() - s.checked_at < EARLY_CHECK_MIN.as_millis() as i64);
        if self.enabled && !recent {
            self.wake.notify_one();
        }
    }

    pub fn health(&self, now: i64) -> Health {
        if !self.enabled {
            return Health::Unverified;
        }
        match self.status.borrow().as_ref() {
            None => Health::Checking,
            Some(s) if s.state == REJECTED => Health::Broken,
            Some(s) if s.state == "ok" => Health::Ok,
            Some(s)
                if s.failing_since
                    .is_some_and(|since| now - since >= LONG_OUTAGE_MS) =>
            {
                Health::Broken
            }
            Some(_) => Health::Unreachable,
        }
    }

    /// The key was refused (a long outage is not a refusal: see `health`).
    pub fn rejected(&self) -> bool {
        self.status
            .borrow()
            .as_ref()
            .is_some_and(|s| s.state == REJECTED)
    }

    /// Waits for the first check (immediately without a key), so that at startup
    /// no entanglement ping goes out before we know whether the key works.
    pub async fn first_check(&self) {
        if self.enabled {
            let _ = self.status.subscribe().wait_for(Option::is_some).await;
        }
    }
}

/// Checks the key at startup and then every day. Nothing without a key.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        while let Some(check) = state.mailer.check_key().await {
            let wait = record(&state, check);
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = state.mail_key.wake.notified() => {}
            }
        }
    });
}

/// Stores the outcome of the email check and returns when to check again.
fn record(state: &AppState, check: KeyCheck) -> Duration {
    record_check(
        &state.mail_key,
        state.mailer.describe(),
        check,
        clock::now_ms(),
    )
}

/// Stores the outcome of a key check (email or Telegram) and returns when to
/// check again: daily when it works, hourly while it fails.
pub fn record_check(
    monitor: &KeyMonitor,
    provider: &'static str,
    check: KeyCheck,
    now: i64,
) -> Duration {
    let previous = monitor.get();
    let was_rejected = monitor.rejected();
    let failing_since = previous
        .as_ref()
        .and_then(|p| p.failing_since)
        .or(Some(now));
    let status = |state, error, failing_since| KeyStatus {
        provider,
        state,
        checked_at: now,
        error,
        failing_since,
    };
    match check {
        KeyCheck::Valid => {
            if was_rejected {
                tracing::info!(provider, "key valid again");
            } else {
                tracing::info!(provider, "key valid");
            }
            monitor.set(status("ok", None, None));
            CHECK_EVERY
        }
        KeyCheck::Rejected(detail) => {
            tracing::error!(provider, %detail, "the provider refused the key: alerts on this channel cannot be sent");
            monitor.set(status(REJECTED, Some(detail), failing_since));
            RETRY_AFTER
        }
        KeyCheck::Unreachable(detail) => {
            tracing::warn!(provider, %detail, "key check failed: retrying in an hour");
            // A temporary error does not hide an earlier refusal.
            if !was_rejected {
                monitor.set(status("unreachable", Some(detail), failing_since));
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
        let hb = Arc::new(
            heartbeat::Heartbeat::load(&pool, secret_box, None)
                .await
                .unwrap(),
        );
        crate::test_state(pool, hb)
    }

    #[tokio::test]
    async fn records_outcomes() {
        let state = state().await;

        assert_eq!(record(&state, KeyCheck::Valid), CHECK_EVERY);
        assert_eq!(state.mail_key.get().unwrap().state, "ok");
        assert!(!state.mail_key.rejected());

        assert_eq!(
            record(&state, KeyCheck::Rejected("401".into())),
            RETRY_AFTER
        );
        let status = state.mail_key.get().unwrap();
        assert_eq!(
            (status.state, status.error.as_deref()),
            (REJECTED, Some("401"))
        );
        assert!(state.mail_key.rejected());

        // A network error later does not hide the refusal.
        assert_eq!(
            record(&state, KeyCheck::Unreachable("timeout".into())),
            RETRY_AFTER
        );
        assert!(state.mail_key.rejected());

        record(&state, KeyCheck::Valid);
        assert!(!state.mail_key.rejected());
        // A network error alone does not hold the pings.
        record(&state, KeyCheck::Unreachable("timeout".into()));
        assert_eq!(state.mail_key.get().unwrap().state, "unreachable");
        assert!(!state.mail_key.rejected());
    }

    #[test]
    fn long_outage_counts_as_broken() {
        let monitor = KeyMonitor::new(true);
        assert_eq!(monitor.health(0), Health::Checking);
        record_check(
            &monitor,
            "Telegram",
            KeyCheck::Unreachable("timeout".into()),
            1_000,
        );
        assert_eq!(
            monitor.health(1_000 + LONG_OUTAGE_MS - 1),
            Health::Unreachable
        );
        // The next failure keeps the start of the outage.
        record_check(
            &monitor,
            "Telegram",
            KeyCheck::Unreachable("timeout".into()),
            1_000 + LONG_OUTAGE_MS,
        );
        assert_eq!(monitor.get().unwrap().failing_since, Some(1_000));
        assert_eq!(monitor.health(1_000 + LONG_OUTAGE_MS), Health::Broken);
        record_check(&monitor, "Telegram", KeyCheck::Valid, 2 * LONG_OUTAGE_MS);
        assert_eq!(monitor.health(2 * LONG_OUTAGE_MS), Health::Ok);
        assert_eq!(monitor.get().unwrap().failing_since, None);
        record_check(
            &monitor,
            "Telegram",
            KeyCheck::Rejected("401".into()),
            3 * LONG_OUTAGE_MS,
        );
        assert_eq!(monitor.health(3 * LONG_OUTAGE_MS), Health::Broken);
        assert_eq!(KeyMonitor::new(false).health(0), Health::Unverified);
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
        monitor.set(KeyStatus {
            provider: "Brevo",
            state: "ok",
            checked_at: 1,
            error: None,
            failing_since: None,
        });
        waiting.await.unwrap();
    }
}
