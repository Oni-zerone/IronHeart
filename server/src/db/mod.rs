//! Accesso al database. Tutte le query stanno in questo modulo (vedi CLAUDE.md).

mod apps;
mod auth;
mod failures;

use std::{str::FromStr, time::Duration};

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};

pub use apps::*;
pub use auth::*;
pub use failures::*;

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

#[cfg(test)]
pub async fn test_pool() -> Pool {
    connect("sqlite::memory:").await.unwrap()
}

/// App minima per i test: intervallo 60 s, tolleranza 0, stato `new`.
#[cfg(test)]
pub async fn insert_test_app(pool: &Pool, id: &str, user_id: &str) {
    insert_app(
        pool,
        &NewApp {
            id,
            user_id,
            name: "App",
            key_id: &format!("key-{id}"),
            secret_enc: b"secret",
            secret_key_version: 1,
            interval_secs: 60,
            grace_secs: 0,
            now: 1,
        },
        &[],
    )
    .await
    .unwrap();
}
