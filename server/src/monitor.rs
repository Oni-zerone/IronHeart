//! Controllo periodico (ogni 30 secondi): scrive i tentativi falliti, porta a
//! `down` le app scadute, manda gli avvisi e pulisce i dati vecchi.

use std::time::Duration;

use crate::{
    AppState, alerts,
    clock::{DAY_MS, MINUTE_MS, now_ms},
    db,
    mail::Email,
};

pub const TICK: Duration = Duration::from_secs(30);
const FAILURE_RETENTION_MS: i64 = 7 * DAY_MS;
const LOGIN_TTL_MS: i64 = 10 * MINUTE_MS;

/// Durata delle finestre tra un riepilogo e l'altro, per livello.
pub const FAILURE_WINDOWS_MIN: [i64; 6] = [5, 10, 20, 40, 80, 120];

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

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(err) = tick(&state, now_ms()).await {
                tracing::error!(err = ?err, "controllo periodico non riuscito");
            }
        }
    });
}

pub async fn tick(state: &AppState, now: i64) -> anyhow::Result<()> {
    let pool = &state.db;

    for (app_id, since, counts) in state.heartbeat.take_failures() {
        db::insert_failure_bucket(pool, &app_id, since, now, &counts).await?;
    }
    let unattributed = state.heartbeat.take_unattributed();
    if unattributed > 0 {
        tracing::warn!("heartbeat con chiave sconosciuta: {unattributed}");
    }

    for app in db::mark_overdue_down(pool, now).await? {
        let failures = db::failure_summary_since(pool, &app.id, app.last_seen_at.unwrap_or(0)).await?;
        let app_ref = app_ref(state, &app.id, &app.name);
        notify(state, &app.id, |to| alerts::down(&app_ref, to, app.last_seen_at, now, failures.total())).await?;
    }

    for app in db::take_recoveries(pool).await? {
        let app_ref = app_ref(state, &app.id, &app.name);
        notify(state, &app.id, |to| alerts::recovered(&app_ref, to, app.down_since, app.last_seen_at)).await?;
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
            let app_ref = app_ref(state, &app.id, &app.name);
            let next_in = next.window_end.map_or(0, |end| end - now);
            notify(state, &app.id, |to| alerts::failures(&app_ref, to, &counts, since, next_in)).await?;
        }
    }

    db::delete_failure_buckets_before(pool, now - FAILURE_RETENTION_MS).await?;
    db::delete_expired_sessions(pool, now).await?;
    db::delete_oauth_flows_before(pool, now - LOGIN_TTL_MS).await?;
    Ok(())
}

fn app_ref<'a>(state: &AppState, app_id: &str, name: &'a str) -> alerts::AppRef<'a> {
    alerts::AppRef { name, url: format!("{}/apps/{app_id}", state.public_base_url) }
}

/// Una mail per destinatario. Un invio fallito si registra e non blocca gli altri.
async fn notify(state: &AppState, app_id: &str, build: impl Fn(&str) -> Email) -> anyhow::Result<()> {
    for to in db::alert_recipients(&state.db, app_id).await? {
        if let Err(err) = state.mailer.send(&build(&to)).await {
            tracing::error!(%app_id, err = ?err, "avviso non inviato");
        }
    }
    Ok(())
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
}
