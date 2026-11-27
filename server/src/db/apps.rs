//! App: configurazione, chiavi, stato del controllo.

use serde::Serialize;

use super::Pool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct App {
    pub id: String,
    pub name: String,
    pub key_id: String,
    pub interval_secs: i64,
    pub grace_secs: i64,
    pub state: String,
    pub last_seen_at: Option<i64>,
    pub next_deadline_at: Option<i64>,
    pub down_since: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Colonne di `App`: una macro, così le query restano stringhe letterali (sqlx 0.9).
macro_rules! app_columns {
    () => {
        "id, name, key_id, interval_secs, grace_secs, state, \
         last_seen_at, next_deadline_at, down_since, created_at, updated_at"
    };
}

pub struct NewApp<'a> {
    pub id: &'a str,
    pub user_id: &'a str,
    pub name: &'a str,
    pub key_id: &'a str,
    pub secret_enc: &'a [u8],
    pub secret_key_version: i64,
    pub interval_secs: i64,
    pub grace_secs: i64,
    pub now: i64,
}

pub async fn insert_app(pool: &Pool, app: &NewApp<'_>, recipients: &[String]) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO apps (id, user_id, name, key_id, secret_enc, secret_key_version,
                           interval_secs, grace_secs, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
    )
    .bind(app.id)
    .bind(app.user_id)
    .bind(app.name)
    .bind(app.key_id)
    .bind(app.secret_enc)
    .bind(app.secret_key_version)
    .bind(app.interval_secs)
    .bind(app.grace_secs)
    .bind(app.now)
    .execute(&mut *tx)
    .await?;
    replace_recipients(&mut tx, app.id, recipients, app.now).await?;
    tx.commit().await
}

pub async fn count_apps(pool: &Pool, user_id: &str) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM apps WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
}

pub async fn list_apps(pool: &Pool, user_id: &str) -> sqlx::Result<Vec<App>> {
    sqlx::query_as(concat!(
        "SELECT ",
        app_columns!(),
        " FROM apps WHERE user_id = $1 ORDER BY name, id"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn get_app(pool: &Pool, user_id: &str, app_id: &str) -> sqlx::Result<Option<App>> {
    sqlx::query_as(concat!("SELECT ", app_columns!(), " FROM apps WHERE id = $1 AND user_id = $2"))
        .bind(app_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

/// Destinatari configurati delle app di un utente (senza il ripiego sull'account).
pub async fn configured_recipients(
    pool: &Pool,
    user_id: &str,
) -> sqlx::Result<Vec<(String, String)>> {
    sqlx::query_as(
        "SELECT r.app_id, r.email FROM app_recipients r JOIN apps a ON a.id = r.app_id
         WHERE a.user_id = $1 ORDER BY r.email",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Nuovi valori di un'app, già calcolati in Rust (stato e scadenza compresi).
pub struct AppUpdate<'a> {
    pub name: &'a str,
    pub interval_secs: i64,
    pub grace_secs: i64,
    pub state: &'a str,
    pub next_deadline_at: Option<i64>,
    pub recipients: Option<&'a [String]>,
    pub now: i64,
}

pub async fn update_app(
    pool: &Pool,
    user_id: &str,
    app_id: &str,
    update: &AppUpdate<'_>,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE apps SET name = $1, interval_secs = $2, grace_secs = $3, state = $4,
                         next_deadline_at = $5, updated_at = $6
         WHERE id = $7 AND user_id = $8",
    )
    .bind(update.name)
    .bind(update.interval_secs)
    .bind(update.grace_secs)
    .bind(update.state)
    .bind(update.next_deadline_at)
    .bind(update.now)
    .bind(app_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    if updated && let Some(recipients) = update.recipients {
        replace_recipients(&mut tx, app_id, recipients, update.now).await?;
    }
    tx.commit().await?;
    Ok(updated)
}

async fn replace_recipients(
    tx: &mut super::Connection,
    app_id: &str,
    recipients: &[String],
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM app_recipients WHERE app_id = $1")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    for email in recipients {
        sqlx::query("INSERT INTO app_recipients (app_id, email, created_at) VALUES ($1, $2, $3)")
            .bind(app_id)
            .bind(email)
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// Nuova chiave: il vecchio `key_id` smette di valere e il timestamp riparte.
pub async fn rotate_key(
    pool: &Pool,
    user_id: &str,
    app_id: &str,
    key_id: &str,
    secret_enc: &[u8],
    secret_key_version: i64,
    now: i64,
) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "UPDATE apps SET key_id = $1, secret_enc = $2, secret_key_version = $3,
                         last_signed_ts = NULL, updated_at = $4
         WHERE id = $5 AND user_id = $6",
    )
    .bind(key_id)
    .bind(secret_enc)
    .bind(secret_key_version)
    .bind(now)
    .bind(app_id)
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

/// Cancella l'app e restituisce il suo `key_id`, da togliere dal registro.
pub async fn delete_app(pool: &Pool, user_id: &str, app_id: &str) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("DELETE FROM apps WHERE id = $1 AND user_id = $2 RETURNING key_id")
        .bind(app_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

// ---- Registro delle chiavi --------------------------------------------------

#[derive(sqlx::FromRow)]
pub struct KeyRow {
    pub id: String,
    pub key_id: String,
    pub secret_enc: Vec<u8>,
    pub secret_key_version: i64,
    pub interval_secs: i64,
    pub grace_secs: i64,
    pub last_signed_ts: Option<i64>,
}

macro_rules! key_columns {
    () => {
        "id, key_id, secret_enc, secret_key_version, interval_secs, grace_secs, last_signed_ts"
    };
}

pub async fn all_keys(pool: &Pool) -> sqlx::Result<Vec<KeyRow>> {
    sqlx::query_as(concat!("SELECT ", key_columns!(), " FROM apps"))
        .fetch_all(pool)
        .await
}

pub async fn app_key(pool: &Pool, app_id: &str) -> sqlx::Result<Option<KeyRow>> {
    sqlx::query_as(concat!("SELECT ", key_columns!(), " FROM apps WHERE id = $1"))
        .bind(app_id)
        .fetch_optional(pool)
        .await
}

// ---- Heartbeat e controllo delle scadenze -----------------------------------

/// Un ping valido da registrare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingRecord {
    pub app_id: String,
    pub signed_ts: i64,
    pub seen_at: i64,
    pub next_deadline_at: i64,
}

/// Registra un ping valido. Restituisce `false` se il timestamp non è nuovo.
/// Se l'app era `down`, segna il ripristino: la mail la manda il controllo.
#[cfg(test)]
pub async fn record_ping(
    pool: &Pool,
    app_id: &str,
    signed_ts: i64,
    now: i64,
    next_deadline_at: i64,
) -> sqlx::Result<bool> {
    let record = PingRecord { app_id: app_id.into(), signed_ts, seen_at: now, next_deadline_at };
    Ok(record_pings(pool, &[record]).await?[0])
}

/// Ping per istruzione: 4 parametri ciascuno, ben sotto i limiti di SQLite
/// (32.766) e di Postgres (65.535).
const PINGS_PER_STATEMENT: usize = 500;

/// `UPDATE … FROM (VALUES …)` per `rows` ping. Le colonne di VALUES si chiamano
/// `column1…` sia in SQLite sia in Postgres. Nelle espressioni di SET, `apps.state`
/// è il valore prima dell'aggiornamento (in entrambi).
fn record_pings_sql(rows: usize) -> String {
    let values: Vec<String> = (0..rows)
        .map(|i| format!("(${}, ${}, ${}, ${})", 4 * i + 1, 4 * i + 2, 4 * i + 3, 4 * i + 4))
        .collect();
    format!(
        "UPDATE apps SET
             last_signed_ts = v.column2,
             last_seen_at = v.column3,
             recovery_pending = CASE WHEN apps.state = 'down' THEN 1 ELSE apps.recovery_pending END,
             next_deadline_at = CASE WHEN apps.state = 'paused' THEN NULL ELSE v.column4 END,
             state = CASE WHEN apps.state = 'paused' THEN 'paused' ELSE 'up' END,
             updated_at = v.column3
         FROM (VALUES {}) AS v
         WHERE apps.id = v.column1
           AND (apps.last_signed_ts IS NULL OR apps.last_signed_ts < v.column2)
         RETURNING apps.id",
        values.join(", ")
    )
}

/// Registra più ping (al massimo uno per app) in una transazione, con una sola
/// istruzione ogni 500 ping: con un database remoto ogni istruzione è un viaggio
/// di rete. Per ciascuno: `false` se il timestamp non era nuovo (o l'app non
/// esiste più).
pub async fn record_pings(pool: &Pool, pings: &[PingRecord]) -> sqlx::Result<Vec<bool>> {
    let mut tx = pool.begin().await?;
    let mut updated = std::collections::HashSet::new();
    for chunk in pings.chunks(PINGS_PER_STATEMENT) {
        // Contiene solo segnaposto: i valori passano come parametri.
        let sql = record_pings_sql(chunk.len());
        let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql));
        for ping in chunk {
            query = query
                .bind(ping.app_id.as_str())
                .bind(ping.signed_ts)
                .bind(ping.seen_at)
                .bind(ping.next_deadline_at);
        }
        updated.extend(query.fetch_all(&mut *tx).await?);
    }
    tx.commit().await?;
    Ok(pings.iter().map(|ping| updated.contains(&ping.app_id)).collect())
}

#[derive(Debug, sqlx::FromRow)]
pub struct DownApp {
    pub id: String,
    pub name: String,
    pub last_seen_at: Option<i64>,
}

/// Porta a `down` le app scadute. Atomico: ogni app passa una volta sola.
pub async fn mark_overdue_down(pool: &Pool, now: i64) -> sqlx::Result<Vec<DownApp>> {
    sqlx::query_as(
        "UPDATE apps SET state = 'down', down_since = $1, next_deadline_at = NULL, updated_at = $1
         WHERE state = 'up' AND next_deadline_at <= $1
         RETURNING id, name, last_seen_at",
    )
    .bind(now)
    .fetch_all(pool)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct RecoveredApp {
    pub id: String,
    pub name: String,
    pub down_since: Option<i64>,
    pub last_seen_at: Option<i64>,
}

pub async fn take_recoveries(pool: &Pool) -> sqlx::Result<Vec<RecoveredApp>> {
    sqlx::query_as(
        "UPDATE apps SET recovery_pending = 0 WHERE recovery_pending = 1
         RETURNING id, name, down_since, last_seen_at",
    )
    .fetch_all(pool)
    .await
}

/// App `up` con la scadenza già passata: all'avvio si rinviano, perché i ping
/// persi mentre il servizio era spento non sono colpa dell'app.
pub async fn overdue_up_apps(pool: &Pool, now: i64) -> sqlx::Result<Vec<(String, i64, i64)>> {
    sqlx::query_as(
        "SELECT id, interval_secs, grace_secs FROM apps
         WHERE state = 'up' AND next_deadline_at <= $1",
    )
    .bind(now)
    .fetch_all(pool)
    .await
}

pub async fn set_deadline(pool: &Pool, app_id: &str, next_deadline_at: i64) -> sqlx::Result<()> {
    sqlx::query("UPDATE apps SET next_deadline_at = $1 WHERE id = $2 AND state = 'up'")
        .bind(next_deadline_at)
        .bind(app_id)
        .execute(pool)
        .await
        .map(|_| ())
}

// ---- Stato degli avvisi sui tentativi falliti -------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FailureAlertState {
    pub id: String,
    pub name: String,
    pub fail_level: i64,
    pub fail_window_end: Option<i64>,
    pub fail_last_report_at: Option<i64>,
}

/// App con una finestra aperta o con tentativi non ancora segnalati.
pub async fn failure_alert_candidates(pool: &Pool) -> sqlx::Result<Vec<FailureAlertState>> {
    sqlx::query_as(
        "SELECT id, name, fail_level, fail_window_end, fail_last_report_at FROM apps a
         WHERE fail_window_end IS NOT NULL
            OR EXISTS (SELECT 1 FROM failure_buckets b
                       WHERE b.app_id = a.id AND b.flushed_at > COALESCE(a.fail_last_report_at, 0))",
    )
    .fetch_all(pool)
    .await
}

pub async fn set_failure_alert_state(
    pool: &Pool,
    app_id: &str,
    level: i64,
    window_end: Option<i64>,
    last_report_at: Option<i64>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE apps SET fail_level = $1, fail_window_end = $2, fail_last_report_at = $3
         WHERE id = $4",
    )
    .bind(level)
    .bind(window_end)
    .bind(last_report_at)
    .bind(app_id)
    .execute(pool)
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{insert_test_app, test_pool, upsert_google_user};

    async fn setup() -> Pool {
        let pool = test_pool().await;
        let user = upsert_google_user(&pool, "sub", "o@example.com", None, 1).await.unwrap();
        insert_test_app(&pool, "a", &user.id).await;
        pool
    }

    async fn state(pool: &Pool, id: &str) -> (String, Option<i64>, i64) {
        sqlx::query_as("SELECT state, next_deadline_at, recovery_pending FROM apps WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn ping_requires_increasing_timestamp() {
        let pool = setup().await;
        assert!(record_ping(&pool, "a", 100, 1_000, 61_000).await.unwrap());
        assert!(!record_ping(&pool, "a", 100, 2_000, 62_000).await.unwrap());
        assert!(!record_ping(&pool, "a", 99, 2_000, 62_000).await.unwrap());
        assert!(record_ping(&pool, "a", 101, 2_000, 62_000).await.unwrap());
        assert_eq!(state(&pool, "a").await, ("up".into(), Some(62_000), 0));
    }

    #[tokio::test]
    async fn down_then_recovery() {
        let pool = setup().await;
        record_ping(&pool, "a", 1, 1_000, 61_000).await.unwrap();

        assert!(mark_overdue_down(&pool, 60_999).await.unwrap().is_empty());
        let down = mark_overdue_down(&pool, 61_000).await.unwrap();
        assert_eq!(down.len(), 1);
        assert!(mark_overdue_down(&pool, 70_000).await.unwrap().is_empty(), "una volta sola");

        record_ping(&pool, "a", 2, 80_000, 140_000).await.unwrap();
        assert_eq!(state(&pool, "a").await, ("up".into(), Some(140_000), 1));
        let recovered = take_recoveries(&pool).await.unwrap();
        assert_eq!(recovered[0].down_since, Some(61_000));
        assert!(take_recoveries(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn paused_app_records_ping_without_deadline() {
        let pool = setup().await;
        sqlx::query("UPDATE apps SET state = 'paused'").execute(&pool).await.unwrap();
        assert!(record_ping(&pool, "a", 1, 1_000, 61_000).await.unwrap());
        assert_eq!(state(&pool, "a").await, ("paused".into(), None, 0));
    }

    #[tokio::test]
    async fn record_pings_in_blocks() {
        let pool = setup().await;
        let user = upsert_google_user(&pool, "sub2", "p@example.com", None, 1).await.unwrap();
        let ids: Vec<String> = (0..1_201).map(|i| format!("b{i}")).collect();
        for id in &ids {
            insert_test_app(&pool, id, &user.id).await;
        }
        let pings = |ts: i64| -> Vec<PingRecord> {
            ids.iter()
                .map(|id| PingRecord { app_id: id.clone(), signed_ts: ts, seen_at: ts * 1000, next_deadline_at: ts * 1000 + 60_000 })
                .chain([PingRecord { app_id: "missing".into(), signed_ts: ts, seen_at: 0, next_deadline_at: 0 }])
                .collect()
        };
        let accepted = record_pings(&pool, &pings(5)).await.unwrap();
        assert_eq!(accepted.iter().filter(|ok| **ok).count(), ids.len());
        assert!(!accepted[ids.len()], "app inesistente");
        assert_eq!(state(&pool, "b1200").await, ("up".into(), Some(65_000), 0));

        // Stesso timestamp: nessuno accettato.
        assert!(record_pings(&pool, &pings(5)).await.unwrap().iter().all(|ok| !ok));
    }
}
