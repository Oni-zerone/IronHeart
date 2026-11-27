//! Alert email queue. The periodic check puts emails in the queue and moves
//! on; a separate task sends them, a few in parallel. This way a slow send
//! (or a wave of alerts) does not stop the deadline check.
//!
//! Two queues: urgent alerts (app down, recovery) take priority and are never
//! dropped; failed-attempt summaries are dropped if the queue is full.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use tokio::sync::{Semaphore, mpsc};

use crate::mail::{Email, Mailer};

const URGENT_CAPACITY: usize = 10_000;
const REPORT_CAPACITY: usize = 1_000;
const PARALLEL_SENDS: usize = 4;

pub struct Outbox {
    urgent: mpsc::Sender<Email>,
    reports: mpsc::Sender<Email>,
    dropped_reports: AtomicU64,
}

struct Receivers {
    urgent: mpsc::Receiver<Email>,
    reports: mpsc::Receiver<Email>,
}

impl Outbox {
    fn with_capacity(urgent: usize, reports: usize) -> (Self, Receivers) {
        let (urgent_tx, urgent_rx) = mpsc::channel(urgent);
        let (reports_tx, reports_rx) = mpsc::channel(reports);
        let outbox = Self { urgent: urgent_tx, reports: reports_tx, dropped_reports: AtomicU64::new(0) };
        (outbox, Receivers { urgent: urgent_rx, reports: reports_rx })
    }

    pub fn start(mailer: Mailer) -> Arc<Self> {
        let (outbox, receivers) = Self::with_capacity(URGENT_CAPACITY, REPORT_CAPACITY);
        tokio::spawn(run(receivers, mailer));
        Arc::new(outbox)
    }

    /// Urgent alert: waits only if the queue is full (10,000 emails).
    pub async fn urgent(&self, email: Email) {
        if self.urgent.send(email).await.is_err() {
            tracing::error!("email queue closed: alert lost");
        }
    }

    /// Summary: dropped if the queue is full. Returns `false` if dropped.
    pub fn report(&self, email: Email) -> bool {
        match self.reports.try_send(email) {
            Ok(()) => true,
            Err(_) => {
                let dropped = self.dropped_reports.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::warn!(dropped, "summary queue full: summary dropped");
                false
            }
        }
    }
}

async fn run(mut receivers: Receivers, mailer: Mailer) {
    let permits = Arc::new(Semaphore::new(PARALLEL_SENDS));
    loop {
        let email = tokio::select! {
            biased;
            Some(email) = receivers.urgent.recv() => email,
            Some(email) = receivers.reports.recv() => email,
            else => break,
        };
        let permit = permits.clone().acquire_owned().await.expect("semaphore open");
        let mailer = mailer.clone();
        tokio::spawn(async move {
            if let Err(err) = mailer.send(&email).await {
                tracing::error!(err = ?err, subject = %email.subject, "alert not sent");
            }
            drop(permit);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email(n: usize) -> Email {
        Email { to: format!("u{n}@example.com"), subject: "s".into(), text: "t".into(), html: "h".into() }
    }

    #[tokio::test]
    async fn reports_are_dropped_when_full_but_urgent_waits() {
        let (outbox, mut receivers) = Outbox::with_capacity(2, 2);
        assert!(outbox.report(email(1)));
        assert!(outbox.report(email(2)));
        assert!(!outbox.report(email(3)), "queue full: dropped");
        assert_eq!(outbox.dropped_reports.load(Ordering::Relaxed), 1);

        outbox.urgent(email(4)).await;
        outbox.urgent(email(5)).await;
        // The third one waits until someone drains the queue.
        let waiting = tokio::spawn(async move { outbox.urgent(email(6)).await });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        assert_eq!(receivers.urgent.recv().await.unwrap().to, "u4@example.com");
        waiting.await.unwrap();
        assert_eq!(receivers.reports.recv().await.unwrap().to, "u1@example.com");
    }
}
