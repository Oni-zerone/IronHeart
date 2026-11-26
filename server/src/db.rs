//! Accesso al database. Tutte le query stanno in questo modulo (vedi CLAUDE.md).

use std::{str::FromStr, time::Duration};

use serde::Serialize;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use uuid::Uuid;

pub type Pool = SqlitePool;

pub async fn connect(database_url: &str) -> anyhow::Result<Pool> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new().connect_with(options).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

pub async fn ping(pool: &Pool) -> sqlx::Result<()> {
    sqlx::query("SELECT 1").execute(pool).await.map(|_| ())
}

// ---- Utenti -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
}

/// Crea l'utente al primo accesso, oppure aggiorna email e nome da Google.
pub async fn upsert_google_user(
    pool: &Pool,
    google_sub: &str,
    email: &str,
    name: Option<&str>,
    now: i64,
) -> sqlx::Result<User> {
    sqlx::query_as(
        "INSERT INTO users (id, google_sub, email, name, created_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (google_sub) DO UPDATE SET email = excluded.email, name = excluded.name
         RETURNING id, email, name",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(google_sub)
    .bind(email)
    .bind(name)
    .bind(now)
    .fetch_one(pool)
    .await
}

// ---- Destinatari degli avvisi ----------------------------------------------

/// Indirizzi a cui mandare gli avvisi di un'app: quelli configurati,
/// oppure l'email dell'account se l'elenco è vuoto.
pub async fn alert_recipients(pool: &Pool, app_id: &str) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT r.email FROM app_recipients r WHERE r.app_id = $1
         UNION ALL
         SELECT u.email FROM apps a JOIN users u ON u.id = a.user_id
         WHERE a.id = $1 AND NOT EXISTS (SELECT 1 FROM app_recipients WHERE app_id = $1)",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await
}

// ---- Login in corso ---------------------------------------------------------

pub struct OAuthFlow {
    pub nonce: String,
    pub pkce_verifier: String,
}

pub async fn insert_oauth_flow(
    pool: &Pool,
    state: &str,
    flow: &OAuthFlow,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO oauth_flows (state, nonce, pkce_verifier, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(state)
    .bind(&flow.nonce)
    .bind(&flow.pkce_verifier)
    .bind(now)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Recupera e cancella il login in corso, se non è più vecchio di `created_after`.
pub async fn take_oauth_flow(
    pool: &Pool,
    state: &str,
    created_after: i64,
) -> sqlx::Result<Option<OAuthFlow>> {
    let row: Option<(String, String, i64)> = sqlx::query_as(
        "DELETE FROM oauth_flows WHERE state = $1 RETURNING nonce, pkce_verifier, created_at",
    )
    .bind(state)
    .fetch_optional(pool)
    .await?;
    Ok(row
        .filter(|(_, _, created_at)| *created_at > created_after)
        .map(|(nonce, pkce_verifier, _)| OAuthFlow { nonce, pkce_verifier }))
}

pub async fn delete_oauth_flows_before(pool: &Pool, before: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM oauth_flows WHERE created_at <= $1")
        .bind(before)
        .execute(pool)
        .await
        .map(|_| ())
}

// ---- Sessioni ---------------------------------------------------------------

pub async fn insert_session(
    pool: &Pool,
    token_hash: &str,
    user_id: &str,
    now: i64,
    expires_at: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await
    .map(|_| ())
}

pub async fn session_user(pool: &Pool, token_hash: &str, now: i64) -> sqlx::Result<Option<User>> {
    sqlx::query_as(
        "SELECT u.id, u.email, u.name
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > $2",
    )
    .bind(token_hash)
    .bind(now)
    .fetch_optional(pool)
    .await
}

pub async fn delete_session(pool: &Pool, token_hash: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await
        .map(|_| ())
}

pub async fn delete_expired_sessions(pool: &Pool, now: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE expires_at <= $1")
        .bind(now)
        .execute(pool)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn memory_pool() -> Pool {
        connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn upsert_keeps_id_and_updates_email() {
        let pool = memory_pool().await;
        let first = upsert_google_user(&pool, "sub-1", "a@example.com", Some("A"), 1).await.unwrap();
        let second = upsert_google_user(&pool, "sub-1", "b@example.com", None, 2).await.unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.email, "b@example.com");
        assert_eq!(second.name, None);
    }

    #[tokio::test]
    async fn alert_recipients_fall_back_to_account_email() {
        let pool = memory_pool().await;
        let user = upsert_google_user(&pool, "sub-1", "owner@example.com", None, 1).await.unwrap();
        sqlx::query(
            "INSERT INTO apps (id, user_id, name, api_key_prefix, api_key_hash, interval_secs, created_at, updated_at)
             VALUES ('app-1', $1, 'App', 'ih_x', 'hash', 60, 1, 1)",
        )
        .bind(&user.id)
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(alert_recipients(&pool, "app-1").await.unwrap(), ["owner@example.com"]);

        for email in ["ops@example.com", "dev@example.com"] {
            sqlx::query("INSERT INTO app_recipients (app_id, email, created_at) VALUES ('app-1', $1, 1)")
                .bind(email)
                .execute(&pool)
                .await
                .unwrap();
        }
        let mut recipients = alert_recipients(&pool, "app-1").await.unwrap();
        recipients.sort();
        assert_eq!(recipients, ["dev@example.com", "ops@example.com"]);
        assert!(alert_recipients(&pool, "missing").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn oauth_flow_is_single_use_and_expires() {
        let pool = memory_pool().await;
        let flow = OAuthFlow { nonce: "n".into(), pkce_verifier: "v".into() };
        insert_oauth_flow(&pool, "s1", &flow, 100).await.unwrap();
        insert_oauth_flow(&pool, "s2", &flow, 100).await.unwrap();

        assert!(take_oauth_flow(&pool, "s1", 50).await.unwrap().is_some());
        assert!(take_oauth_flow(&pool, "s1", 50).await.unwrap().is_none());
        assert!(take_oauth_flow(&pool, "s2", 100).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_lookup_respects_expiry() {
        let pool = memory_pool().await;
        let user = upsert_google_user(&pool, "sub-1", "a@example.com", None, 1).await.unwrap();
        insert_session(&pool, "h", &user.id, 1, 1_000).await.unwrap();

        assert_eq!(session_user(&pool, "h", 999).await.unwrap().map(|u| u.id), Some(user.id));
        assert!(session_user(&pool, "h", 1_000).await.unwrap().is_none());
        delete_expired_sessions(&pool, 1_000).await.unwrap();
        assert!(session_user(&pool, "h", 0).await.unwrap().is_none());
    }
}
