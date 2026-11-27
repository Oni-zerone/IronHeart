//! Collegamenti verso altre istanze (entanglement).

use serde::Serialize;

use super::Pool;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Entanglement {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub key_id: String,
    pub interval_secs: i64,
    pub enabled: bool,
    pub next_ping_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: i64,
}

macro_rules! entanglement_columns {
    () => {
        "id, name, base_url, key_id, interval_secs, enabled, next_ping_at, \
         last_attempt_at, last_success_at, last_error, created_at"
    };
}

pub struct EntanglementInput<'a> {
    pub name: &'a str,
    pub base_url: &'a str,
    pub interval_secs: i64,
    pub enabled: bool,
}

pub async fn list_entanglements(pool: &Pool) -> sqlx::Result<Vec<Entanglement>> {
    sqlx::query_as(concat!("SELECT ", entanglement_columns!(), " FROM entanglements ORDER BY name, id"))
        .fetch_all(pool)
        .await
}

pub async fn get_entanglement(pool: &Pool, id: &str) -> sqlx::Result<Option<Entanglement>> {
    sqlx::query_as(concat!("SELECT ", entanglement_columns!(), " FROM entanglements WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Il primo ping parte subito (`next_ping_at = now`).
#[allow(clippy::too_many_arguments)]
pub async fn insert_entanglement(
    pool: &Pool,
    id: &str,
    input: &EntanglementInput<'_>,
    key_id: &str,
    secret_enc: &[u8],
    secret_key_version: i64,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO entanglements (id, name, base_url, key_id, secret_enc, secret_key_version,
                                    interval_secs, enabled, next_ping_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, $9)",
    )
    .bind(id)
    .bind(input.name)
    .bind(input.base_url)
    .bind(key_id)
    .bind(secret_enc)
    .bind(secret_key_version)
    .bind(input.interval_secs)
    .bind(input.enabled)
    .bind(now)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Aggiorna i dati; il ping successivo parte subito. Chiave e segreto solo se forniti.
pub async fn update_entanglement(
    pool: &Pool,
    id: &str,
    input: &EntanglementInput<'_>,
    credentials: Option<(&str, &[u8], i64)>,
    now: i64,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE entanglements SET name = $1, base_url = $2, interval_secs = $3, enabled = $4,
                                  next_ping_at = $5, updated_at = $5
         WHERE id = $6",
    )
    .bind(input.name)
    .bind(input.base_url)
    .bind(input.interval_secs)
    .bind(input.enabled)
    .bind(now)
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    if updated && let Some((key_id, secret_enc, version)) = credentials {
        sqlx::query(
            "UPDATE entanglements SET key_id = $1, secret_enc = $2, secret_key_version = $3,
                                      last_success_at = NULL, last_error = NULL
             WHERE id = $4",
        )
        .bind(key_id)
        .bind(secret_enc)
        .bind(version)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(updated)
}

pub async fn delete_entanglement(pool: &Pool, id: &str) -> sqlx::Result<bool> {
    Ok(sqlx::query("DELETE FROM entanglements WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        == 1)
}

#[derive(sqlx::FromRow)]
pub struct DuePing {
    pub id: String,
    pub base_url: String,
    pub key_id: String,
    pub secret_enc: Vec<u8>,
    pub secret_key_version: i64,
}

/// Prende i collegamenti da pingare e sposta subito la prossima scadenza, così
/// un ping lento non viene mandato due volte.
pub async fn take_due_pings(pool: &Pool, now: i64) -> sqlx::Result<Vec<DuePing>> {
    sqlx::query_as(
        "UPDATE entanglements SET next_ping_at = $1 + interval_secs * 1000
         WHERE enabled = 1 AND next_ping_at <= $1
         RETURNING id, base_url, key_id, secret_enc, secret_key_version",
    )
    .bind(now)
    .fetch_all(pool)
    .await
}

/// Da usare per «Invia ora»: stessi dati, senza toccare la scadenza.
pub async fn ping_target(pool: &Pool, id: &str) -> sqlx::Result<Option<DuePing>> {
    sqlx::query_as(
        "SELECT id, base_url, key_id, secret_enc, secret_key_version FROM entanglements WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn record_ping_result(
    pool: &Pool,
    id: &str,
    now: i64,
    error: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE entanglements SET last_attempt_at = $1,
             last_success_at = CASE WHEN $2 IS NULL THEN $1 ELSE last_success_at END,
             last_error = $2
         WHERE id = $3",
    )
    .bind(now)
    .bind(error)
    .bind(id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// All'avvio: i collegamenti attivi mandano subito un ping.
pub async fn ping_all_now(pool: &Pool, now: i64) -> sqlx::Result<()> {
    sqlx::query("UPDATE entanglements SET next_ping_at = $1 WHERE enabled = 1")
        .bind(now)
        .execute(pool)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    fn input(enabled: bool) -> EntanglementInput<'static> {
        EntanglementInput { name: "B", base_url: "https://b.example.com", interval_secs: 60, enabled }
    }

    #[tokio::test]
    async fn due_pings_are_taken_once_per_interval() {
        let pool = test_pool().await;
        insert_entanglement(&pool, "e1", &input(true), "k", b"s", 1, 1_000).await.unwrap();
        insert_entanglement(&pool, "e2", &input(false), "k", b"s", 1, 1_000).await.unwrap();

        let due = take_due_pings(&pool, 1_000).await.unwrap();
        assert_eq!(due.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["e1"]);
        assert!(take_due_pings(&pool, 60_999).await.unwrap().is_empty());
        assert_eq!(take_due_pings(&pool, 61_000).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn results_keep_last_success() {
        let pool = test_pool().await;
        insert_entanglement(&pool, "e1", &input(true), "k", b"s", 1, 1).await.unwrap();
        record_ping_result(&pool, "e1", 10, None).await.unwrap();
        record_ping_result(&pool, "e1", 20, Some("timeout")).await.unwrap();
        let e = get_entanglement(&pool, "e1").await.unwrap().unwrap();
        assert_eq!((e.last_attempt_at, e.last_success_at, e.last_error.as_deref()), (Some(20), Some(10), Some("timeout")));
    }
}
