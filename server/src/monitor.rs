//! Periodic check (every 30 seconds): writes failed attempts, moves overdue apps
//! to `down`, prepares alerts and cleans up old data.
//!
//! Alerts are grouped by recipient (one email per type each round) and go to
//! the alert queue (`outbox`): the check does not wait for sends. A recipient
//! email that belongs to an account with linked Telegram chats gets the same
//! alert in those chats.

use std::{
    collections::{BTreeMap, HashMap},
    time::{Duration, Instant},
};

use crate::{
    AppState,
    alerts::{self, DownItem, FailureItem, RecoveredItem},
    clock::{DAY_MS, MINUTE_MS, now_ms},
    db, telegram,
};

pub const TICK: Duration = Duration::from_secs(30);
const FAILURE_RETENTION_MS: i64 = 7 * DAY_MS;
const LOGIN_TTL_MS: i64 = 10 * MINUTE_MS;

/// Length of the windows between two summaries, per level.
pub const FAILURE_WINDOWS_MIN: [i64; 6] = [5, 10, 20, 40, 80, 120];
/// At most one attempt summary every 5 minutes per recipient, even
/// when the windows of their apps close at different times.
pub const FAILURE_DIGEST_MIN_MS: i64 = 5 * MINUTE_MS;
/// A round slower than this is reported in the log.
const SLOW_TICK: Duration = Duration::from_secs(5);

fn window_ms(level: i64) -> i64 {
    FAILURE_WINDOWS_MIN[level.clamp(0, 5) as usize] * MINUTE_MS
}

/// Next state of the failed-attempt alerts of an app.
#[derive(Debug, PartialEq, Eq)]
pub struct Escalation {
    pub report: bool,
    pub level: i64,
    pub window_end: Option<i64>,
}

/// - At rest, with attempts: alert right away and a 5-minute window.
/// - Window closed with attempts: alert, level +1 (at most 5).
/// - Window closed without attempts: level −1; from level 0 back to rest.
///
/// Returns `None` if nothing changes.
pub fn escalate(
    level: i64,
    window_end: Option<i64>,
    pending: bool,
    now: i64,
) -> Option<Escalation> {
    match window_end {
        None if pending => Some(Escalation {
            report: true,
            level: 0,
            window_end: Some(now + window_ms(0)),
        }),
        None => None,
        Some(end) if now < end => None,
        Some(_) if pending => {
            let level = (level + 1).min(5);
            Some(Escalation {
                report: true,
                level,
                window_end: Some(now + window_ms(level)),
            })
        }
        Some(_) if level > 0 => {
            let level = level - 1;
            Some(Escalation {
                report: false,
                level,
                window_end: Some(now + window_ms(level)),
            })
        }
        Some(_) => Some(Escalation {
            report: false,
            level: 0,
            window_end: None,
        }),
    }
}

/// At startup: deadlines passed while the service was down restart from now.
pub async fn reschedule_overdue(pool: &db::Pool, now: i64) -> anyhow::Result<()> {
    let overdue = db::overdue_up_apps(pool, now).await?;
    for (app_id, interval_secs, grace_secs) in &overdue {
        db::set_deadline(pool, app_id, now + (interval_secs + grace_secs) * 1000).await?;
    }
    if !overdue.is_empty() {
        tracing::info!("deadlines postponed after restart: {}", overdue.len());
    }
    Ok(())
}

/// State of the check kept in memory between rounds.
#[derive(Default)]
pub struct Memory {
    /// Pending attempt summaries, per recipient.
    failure_digests: HashMap<String, PendingDigest>,
}

#[derive(Default)]
struct PendingDigest {
    last_sent_at: Option<i64>,
    /// Per app: if an app comes back before sending, the counts add up.
    items: BTreeMap<String, FailureItem>,
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut memory = Memory::default();
        loop {
            interval.tick().await;
            let started = Instant::now();
            if let Err(err) = tick(&state, &mut memory, now_ms()).await {
                tracing::error!(err = ?err, "periodic check failed");
            }
            if started.elapsed() > SLOW_TICK {
                tracing::warn!(elapsed = ?started.elapsed(), "slow periodic check");
            }
        }
    });
}

pub async fn tick(state: &AppState, memory: &mut Memory, now: i64) -> anyhow::Result<()> {
    let pool = &state.db;

    db::insert_failure_buckets(pool, now, &state.heartbeat.take_failures()).await?;
    let unattributed = state.heartbeat.take_unattributed();
    if unattributed > 0 {
        tracing::warn!("heartbeats with unknown key: {unattributed}");
    }

    let mut downs: BTreeMap<String, Vec<DownItem>> = BTreeMap::new();
    for app in db::mark_overdue_down(pool, now).await? {
        let failures =
            db::failure_summary_since(pool, &app.id, app.last_seen_at.unwrap_or(0)).await?;
        let item = DownItem {
            url: app_url(state, &app.id),
            name: app.name,
            last_seen_at: app.last_seen_at,
            failures: failures.total(),
        };
        for to in db::alert_recipients(pool, &app.id).await? {
            downs.entry(to).or_default().push(item.clone());
        }
    }

    let mut recoveries: BTreeMap<String, Vec<RecoveredItem>> = BTreeMap::new();
    for app in db::take_recoveries(pool).await? {
        let item = RecoveredItem {
            url: app_url(state, &app.id),
            name: app.name,
            down_since: app.down_since,
            last_seen_at: app.last_seen_at,
        };
        for to in db::alert_recipients(pool, &app.id).await? {
            recoveries.entry(to).or_default().push(item.clone());
        }
    }

    for app in db::failure_alert_candidates(pool).await? {
        if app.fail_window_end.is_some_and(|end| now < end) {
            continue;
        }
        let since = app.fail_last_report_at;
        let counts = db::failure_summary_since(pool, &app.id, since.unwrap_or(0)).await?;
        let Some(next) = escalate(app.fail_level, app.fail_window_end, counts.total() > 0, now)
        else {
            continue;
        };
        let last_report_at = if next.report { Some(now) } else { since };
        // State first, then the email: if sending fails there is no burst of retries.
        db::set_failure_alert_state(pool, &app.id, next.level, next.window_end, last_report_at)
            .await?;
        if next.report {
            let item = FailureItem {
                url: app_url(state, &app.id),
                app_id: app.id.clone(),
                name: app.name.clone(),
                counts,
                since,
                next_report_in_ms: next.window_end.map_or(0, |end| end - now),
            };
            for to in db::alert_recipients(pool, &app.id).await? {
                queue_failure(memory, to, item.clone());
            }
        }
    }

    let apps_url = format!("{}/apps", state.public_base_url);
    let failure_digests = due_failure_digests(memory, now);
    let chats = telegram_chats(
        state,
        !downs.is_empty() || !recoveries.is_empty() || !failure_digests.is_empty(),
    )
    .await?;
    let chats_of = |to: &str| chats.get(to).map(Vec::as_slice).unwrap_or_default();
    let mail = state.mailer.is_enabled();
    if !mail {
        // Telegram only: an address without a linked account chat gets nothing.
        let unreached = downs
            .keys()
            .chain(recoveries.keys())
            .chain(failure_digests.iter().map(|(to, _)| to))
            .filter(|to| chats_of(to).is_empty())
            .count();
        if unreached > 0 {
            tracing::warn!(
                unreached,
                "alerts without a channel: recipients with no linked Telegram chat"
            );
        }
    }
    for (to, items) in &downs {
        if mail {
            state
                .outbox
                .urgent(alerts::down_digest(to, items, now, &apps_url))
                .await;
        }
        for &chat_id in chats_of(to) {
            state
                .outbox
                .urgent(telegram::Message::text(
                    chat_id,
                    telegram::text::down(items, now, &apps_url),
                ))
                .await;
        }
    }
    for (to, items) in &recoveries {
        if mail {
            state
                .outbox
                .urgent(alerts::recovered_digest(to, items, &apps_url))
                .await;
        }
        for &chat_id in chats_of(to) {
            state
                .outbox
                .urgent(telegram::Message::text(
                    chat_id,
                    telegram::text::recovered(items, &apps_url),
                ))
                .await;
        }
    }
    for (to, items) in failure_digests {
        if mail {
            state.outbox.report(alerts::failures_digest(
                &to,
                &items,
                FAILURE_DIGEST_MIN_MS,
                &apps_url,
            ));
        }
        for &chat_id in chats_of(&to) {
            state.outbox.report(telegram::Message::text(
                chat_id,
                telegram::text::failures(&items, &apps_url),
            ));
        }
    }

    db::delete_failure_buckets_before(pool, now - FAILURE_RETENTION_MS).await?;
    db::delete_expired_sessions(pool, now).await?;
    db::delete_oauth_flows_before(pool, now - LOGIN_TTL_MS).await?;
    db::delete_telegram_tokens_before(pool, now).await?;
    Ok(())
}

/// Linked chats per account email: read only when there is something to send
/// and the bot is active (one query per round).
async fn telegram_chats(
    state: &AppState,
    needed: bool,
) -> anyhow::Result<HashMap<String, Vec<i64>>> {
    let mut chats: HashMap<String, Vec<i64>> = HashMap::new();
    if needed && state.telegram.is_active() {
        for (email, chat_id) in db::telegram_routes(&state.db).await? {
            chats.entry(email).or_default().push(chat_id);
        }
    }
    Ok(chats)
}

fn queue_failure(memory: &mut Memory, to: String, item: FailureItem) {
    let digest = memory.failure_digests.entry(to).or_default();
    match digest.items.get_mut(&item.app_id) {
        Some(existing) => {
            existing.counts.add(&item.counts);
            existing.next_report_in_ms = item.next_report_in_ms;
        }
        None => {
            digest.items.insert(item.app_id.clone(), item);
        }
    }
}

/// Summaries to send now: recipients with something pending and
/// no summary in the last `FAILURE_DIGEST_MIN_MS`.
fn due_failure_digests(memory: &mut Memory, now: i64) -> Vec<(String, Vec<FailureItem>)> {
    let mut due = Vec::new();
    for (to, digest) in memory.failure_digests.iter_mut() {
        let cooled = digest
            .last_sent_at
            .is_none_or(|at| now - at >= FAILURE_DIGEST_MIN_MS);
        if cooled && !digest.items.is_empty() {
            digest.last_sent_at = Some(now);
            due.push((
                to.clone(),
                std::mem::take(&mut digest.items).into_values().collect(),
            ));
        }
    }
    // Recipients quiet for more than an interval: no need to remember them.
    memory.failure_digests.retain(|_, d| {
        !d.items.is_empty()
            || d.last_sent_at
                .is_some_and(|at| now - at < FAILURE_DIGEST_MIN_MS)
    });
    due
}

fn app_url(state: &AppState, app_id: &str) -> String {
    format!("{}/apps/{app_id}", state.public_base_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: i64 = MINUTE_MS;

    #[test]
    fn first_attempt_reports_immediately() {
        assert_eq!(
            escalate(0, None, true, 1_000),
            Some(Escalation {
                report: true,
                level: 0,
                window_end: Some(1_000 + 5 * M)
            })
        );
        assert_eq!(escalate(0, None, false, 1_000), None);
    }

    #[test]
    fn open_window_waits() {
        assert_eq!(escalate(2, Some(10 * M), true, 10 * M - 1), None);
    }

    #[test]
    fn steady_attack_climbs_to_two_hours() {
        let (mut level, mut end) = (0, Some(5 * M));
        let mut now = 5 * M;
        let mut windows = vec![];
        for _ in 0..7 {
            let next = escalate(level, end, true, now).unwrap();
            assert!(next.report);
            windows.push((next.window_end.unwrap() - now) / M);
            (level, end) = (next.level, next.window_end);
            now = end.unwrap();
        }
        assert_eq!(windows, [10, 20, 40, 80, 120, 120, 120]);
    }

    #[test]
    fn quiet_windows_step_down_one_level_at_a_time() {
        let next = escalate(3, Some(0), false, 0).unwrap();
        assert_eq!(
            next,
            Escalation {
                report: false,
                level: 2,
                window_end: Some(20 * M)
            }
        );
        assert_eq!(
            escalate(0, Some(0), false, 0),
            Some(Escalation {
                report: false,
                level: 0,
                window_end: None
            })
        );
    }

    /// An attacker sending an attempt as soon as each window closes
    /// gets no more than one summary every 2 hours, at steady state.
    #[test]
    fn paced_attacker_is_rate_limited() {
        let (mut level, mut end): (i64, Option<i64>) = (0, None);
        let mut reports = 0;
        let mut now = 0;
        while now < DAY_MS {
            if let Some(next) = escalate(level, end, true, now) {
                reports += next.report as i32;
                (level, end) = (next.level, next.window_end);
            }
            now = end.unwrap_or(now) + 1;
        }
        assert!(reports <= 16, "summaries in a day: {reports}");
    }

    fn item(app: &str, bad: i64) -> FailureItem {
        FailureItem {
            app_id: app.into(),
            name: app.into(),
            url: String::new(),
            counts: crate::db::FailureCounts {
                bad_signature: bad,
                ..Default::default()
            },
            since: None,
            next_report_in_ms: 5 * M,
        }
    }

    /// Apps with windows closing at different times: the recipient still
    /// gets at most one summary every 5 minutes, without losing counts.
    #[test]
    fn failure_digests_are_rate_limited_per_recipient() {
        let mut memory = Memory::default();
        let mut sent = vec![];
        let mut now = 0;
        for i in 0..20 {
            // Every round (30 s) a different app closes its window.
            queue_failure(
                &mut memory,
                "a@example.com".into(),
                item(&format!("app{}", i % 4), 1),
            );
            for (_, items) in due_failure_digests(&mut memory, now) {
                sent.push((
                    now,
                    items.iter().map(|i| i.counts.bad_signature).sum::<i64>(),
                ));
            }
            now += M / 2;
        }
        assert_eq!(
            sent.iter().map(|(at, _)| *at).collect::<Vec<_>>(),
            [0, 5 * M],
            "one every 5 minutes"
        );
        let delivered: i64 = sent.iter().map(|(_, n)| n).sum();
        let pending: i64 = memory
            .failure_digests
            .values()
            .flat_map(|d| d.items.values())
            .map(|i| i.counts.bad_signature)
            .sum();
        assert_eq!(delivered + pending, 20, "no count lost");
    }
}
