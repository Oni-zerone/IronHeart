//! Controllo periodico (ogni 30 secondi): scrive i tentativi falliti, porta a
//! `down` le app scadute, prepara gli avvisi e pulisce i dati vecchi.
//!
//! Gli avvisi si raggruppano per destinatario (una mail per tipo a ogni giro) e
//! vanno nella coda delle mail (`outbox`): il controllo non aspetta gli invii.

use std::{
    collections::{BTreeMap, HashMap},
    time::{Duration, Instant},
};

use crate::{
    AppState,
    alerts::{self, DownItem, FailureItem, RecoveredItem},
    clock::{DAY_MS, MINUTE_MS, now_ms},
    db,
};

pub const TICK: Duration = Duration::from_secs(30);
const FAILURE_RETENTION_MS: i64 = 7 * DAY_MS;
const LOGIN_TTL_MS: i64 = 10 * MINUTE_MS;

/// Durata delle finestre tra un riepilogo e l'altro, per livello.
pub const FAILURE_WINDOWS_MIN: [i64; 6] = [5, 10, 20, 40, 80, 120];
/// Al massimo un riepilogo di tentativi ogni 5 minuti per destinatario, anche
/// quando le finestre delle sue app si chiudono in momenti diversi.
pub const FAILURE_DIGEST_MIN_MS: i64 = 5 * MINUTE_MS;
/// Un giro più lento di così viene segnalato nel log.
const SLOW_TICK: Duration = Duration::from_secs(5);

fn window_ms(level: i64) -> i64 {
    FAILURE_WINDOWS_MIN[level.clamp(0, 5) as usize] * MINUTE_MS
}

/// Prossimo stato degli avvisi sui tentativi falliti di un'app.
#[derive(Debug, PartialEq, Eq)]
pub struct Escalation {
    pub report: bool,
    pub level: i64,
    pub window_end: Option<i64>,
}

/// - A riposo, con tentativi: avviso subito e finestra di 5 minuti.
/// - Finestra chiusa con tentativi: avviso, livello +1 (massimo 5).
/// - Finestra chiusa senza tentativi: livello −1; dal livello 0 si torna a riposo.
///
/// Restituisce `None` se non cambia nulla.
pub fn escalate(level: i64, window_end: Option<i64>, pending: bool, now: i64) -> Option<Escalation> {
    match window_end {
        None if pending => Some(Escalation { report: true, level: 0, window_end: Some(now + window_ms(0)) }),
        None => None,
        Some(end) if now < end => None,
        Some(_) if pending => {
            let level = (level + 1).min(5);
            Some(Escalation { report: true, level, window_end: Some(now + window_ms(level)) })
        }
        Some(_) if level > 0 => {
            let level = level - 1;
            Some(Escalation { report: false, level, window_end: Some(now + window_ms(level)) })
        }
        Some(_) => Some(Escalation { report: false, level: 0, window_end: None }),
    }
}

/// All'avvio: le scadenze superate mentre il servizio era spento ripartono da adesso.
pub async fn reschedule_overdue(pool: &db::Pool, now: i64) -> anyhow::Result<()> {
    let overdue = db::overdue_up_apps(pool, now).await?;
    for (app_id, interval_secs, grace_secs) in &overdue {
        db::set_deadline(pool, app_id, now + (interval_secs + grace_secs) * 1000).await?;
    }
    if !overdue.is_empty() {
        tracing::info!("scadenze rinviate dopo il riavvio: {}", overdue.len());
    }
    Ok(())
}

/// Stato del controllo che resta in memoria tra un giro e l'altro.
#[derive(Default)]
pub struct Memory {
    /// Riepiloghi dei tentativi in attesa, per destinatario.
    failure_digests: HashMap<String, PendingDigest>,
}

#[derive(Default)]
struct PendingDigest {
    last_sent_at: Option<i64>,
    /// Per app: se un'app torna prima dell'invio, i conteggi si sommano.
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
                tracing::error!(err = ?err, "controllo periodico non riuscito");
            }
            if started.elapsed() > SLOW_TICK {
                tracing::warn!(elapsed = ?started.elapsed(), "controllo periodico lento");
            }
        }
    });
}

pub async fn tick(state: &AppState, memory: &mut Memory, now: i64) -> anyhow::Result<()> {
    let pool = &state.db;

    for (app_id, since, counts) in state.heartbeat.take_failures() {
        db::insert_failure_bucket(pool, &app_id, since, now, &counts).await?;
    }
    let unattributed = state.heartbeat.take_unattributed();
    if unattributed > 0 {
        tracing::warn!("heartbeat con chiave sconosciuta: {unattributed}");
    }

    let mut downs: BTreeMap<String, Vec<DownItem>> = BTreeMap::new();
    for app in db::mark_overdue_down(pool, now).await? {
        let failures = db::failure_summary_since(pool, &app.id, app.last_seen_at.unwrap_or(0)).await?;
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
        let Some(next) = escalate(app.fail_level, app.fail_window_end, counts.total() > 0, now) else {
            continue;
        };
        let last_report_at = if next.report { Some(now) } else { since };
        // Prima lo stato, poi la mail: se l'invio fallisce non si riprova a raffica.
        db::set_failure_alert_state(pool, &app.id, next.level, next.window_end, last_report_at).await?;
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
    for (to, items) in &downs {
        state.outbox.urgent(alerts::down_digest(to, items, now, &apps_url)).await;
    }
    for (to, items) in &recoveries {
        state.outbox.urgent(alerts::recovered_digest(to, items, &apps_url)).await;
    }
    for (to, items) in due_failure_digests(memory, now) {
        state.outbox.report(alerts::failures_digest(&to, &items, FAILURE_DIGEST_MIN_MS, &apps_url));
    }

    db::delete_failure_buckets_before(pool, now - FAILURE_RETENTION_MS).await?;
    db::delete_expired_sessions(pool, now).await?;
    db::delete_oauth_flows_before(pool, now - LOGIN_TTL_MS).await?;
    Ok(())
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

/// Riepiloghi da mandare adesso: destinatari con qualcosa in attesa e
/// nessun riepilogo negli ultimi `FAILURE_DIGEST_MIN_MS`.
fn due_failure_digests(memory: &mut Memory, now: i64) -> Vec<(String, Vec<FailureItem>)> {
    let mut due = Vec::new();
    for (to, digest) in memory.failure_digests.iter_mut() {
        let cooled = digest.last_sent_at.is_none_or(|at| now - at >= FAILURE_DIGEST_MIN_MS);
        if cooled && !digest.items.is_empty() {
            digest.last_sent_at = Some(now);
            due.push((to.clone(), std::mem::take(&mut digest.items).into_values().collect()));
        }
    }
    // Destinatari tranquilli da più di un intervallo: non serve ricordarli.
    memory.failure_digests.retain(|_, d| {
        !d.items.is_empty() || d.last_sent_at.is_some_and(|at| now - at < FAILURE_DIGEST_MIN_MS)
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
            Some(Escalation { report: true, level: 0, window_end: Some(1_000 + 5 * M) })
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
        assert_eq!(next, Escalation { report: false, level: 2, window_end: Some(20 * M) });
        assert_eq!(escalate(0, Some(0), false, 0), Some(Escalation { report: false, level: 0, window_end: None }));
    }

    /// Un attaccante che manda un tentativo appena si chiude ogni finestra
    /// non ottiene più di un riepilogo ogni 2 ore, a regime.
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
        assert!(reports <= 16, "riepiloghi in un giorno: {reports}");
    }

    fn item(app: &str, bad: i64) -> FailureItem {
        FailureItem {
            app_id: app.into(),
            name: app.into(),
            url: String::new(),
            counts: crate::db::FailureCounts { bad_signature: bad, ..Default::default() },
            since: None,
            next_report_in_ms: 5 * M,
        }
    }

    /// App con finestre che si chiudono in momenti diversi: il destinatario
    /// riceve comunque al massimo un riepilogo ogni 5 minuti, senza perdere conteggi.
    #[test]
    fn failure_digests_are_rate_limited_per_recipient() {
        let mut memory = Memory::default();
        let mut sent = vec![];
        let mut now = 0;
        for i in 0..20 {
            // Ogni giro (30 s) un'app diversa chiude la sua finestra.
            queue_failure(&mut memory, "a@example.com".into(), item(&format!("app{}", i % 4), 1));
            for (_, items) in due_failure_digests(&mut memory, now) {
                sent.push((now, items.iter().map(|i| i.counts.bad_signature).sum::<i64>()));
            }
            now += M / 2;
        }
        assert_eq!(sent.iter().map(|(at, _)| *at).collect::<Vec<_>>(), [0, 5 * M], "uno ogni 5 minuti");
        let delivered: i64 = sent.iter().map(|(_, n)| n).sum();
        let pending: i64 = memory.failure_digests.values().flat_map(|d| d.items.values()).map(|i| i.counts.bad_signature).sum();
        assert_eq!(delivered + pending, 20, "nessun conteggio perso");
    }
}
