//! Accesso al database. Tutte le query stanno in questo modulo (vedi CLAUDE.md).
//!
//! Il database si sceglie all'avvio da `DATABASE_URL`: `postgres://…` per un
//! Postgres esterno, altrimenti il file SQLite locale. Le query sono le stesse
//! per entrambi (pool `Any` di sqlx); cambiano solo le migrazioni e i PRAGMA.

mod apps;
mod auth;
mod entanglements;
mod failures;
mod users;

use std::time::Duration;

use sqlx::{
    Executor,
    AnyConnection, AnyPool, any::AnyPoolOptions,
};

pub use apps::*;
pub use auth::*;
pub use entanglements::*;
pub use failures::*;
pub use users::*;

pub type Pool = AnyPool;
pub type Connection = AnyConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

impl Backend {
    pub fn from_url(database_url: &str) -> anyhow::Result<Self> {
        if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
            Ok(Self::Postgres)
        } else if database_url.starts_with("sqlite:") {
            Ok(Self::Sqlite)
        } else {
            anyhow::bail!("DATABASE_URL non riconosciuto: deve iniziare con sqlite: o postgres://")
        }
    }
}

/// Impostazioni di ogni connessione SQLite. Dopo ogni checkpoint il file WAL
/// torna al massimo a 16 MB: senza questo limite, sotto carico, cresce e non si
/// riduce più.
const SQLITE_PRAGMAS: &str = "PRAGMA journal_mode = WAL;
     PRAGMA synchronous = NORMAL;
     PRAGMA foreign_keys = ON;
     PRAGMA busy_timeout = 5000;
     PRAGMA journal_size_limit = 16777216;";

pub async fn connect(database_url: &str) -> anyhow::Result<Pool> {
    sqlx::any::install_default_drivers();
    let pool = match Backend::from_url(database_url)? {
        Backend::Sqlite => {
            let pool = AnyPoolOptions::new()
                .after_connect(|conn, _| {
                    Box::pin(async move { conn.execute(SQLITE_PRAGMAS).await.map(|_| ()) })
                })
                .connect(&sqlite_create_if_missing(database_url))
                .await?;
            sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
            pool
        }
        Backend::Postgres => {
            let pool = AnyPoolOptions::new()
                .max_connections(10)
                .acquire_timeout(Duration::from_secs(10))
                .connect(database_url)
                .await?;
            sqlx::migrate!("./migrations/postgres").run(&pool).await?;
            pool
        }
    };
    Ok(pool)
}

/// Descrizione per il log, senza credenziali.
pub fn describe(database_url: &str) -> String {
    match Backend::from_url(database_url) {
        Ok(Backend::Postgres) => match reqwest::Url::parse(database_url) {
            Ok(url) => format!(
                "Postgres su {}:{}{}",
                url.host_str().unwrap_or("?"),
                url.port().unwrap_or(5432),
                url.path()
            ),
            Err(_) => "Postgres".into(),
        },
        _ => format!("SQLite locale ({})", database_url.split('?').next().unwrap_or_default()),
    }
}

/// Il file SQLite si crea se manca (`mode=rwc`), salvo che l'URL dica altro.
fn sqlite_create_if_missing(url: &str) -> String {
    if url.contains("mode=") || url.contains(":memory:") {
        url.to_owned()
    } else if url.contains('?') {
        format!("{url}&mode=rwc")
    } else {
        format!("{url}?mode=rwc")
    }
}

pub async fn ping(pool: &Pool) -> sqlx::Result<()> {
    sqlx::query("SELECT 1").execute(pool).await.map(|_| ())
}

/// Database vuoto per un test. SQLite in memoria, condiviso fra le connessioni
/// del pool; con `IRONHEART_TEST_POSTGRES_URL` uno schema nuovo di quel Postgres.
#[cfg(test)]
pub async fn test_pool() -> Pool {
    let name = format!("t{}", uuid::Uuid::now_v7().simple());
    match std::env::var("IRONHEART_TEST_POSTGRES_URL") {
        Ok(url) => {
            sqlx::any::install_default_drivers();
            let admin = AnyPoolOptions::new().max_connections(1).connect(&url).await.unwrap();
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {name}")))
                .execute(&admin)
                .await
                .unwrap();
            admin.close().await;
            let sep = if url.contains('?') { '&' } else { '?' };
            connect(&format!("{url}{sep}options[search_path]={name}")).await.unwrap()
        }
        Err(_) => connect(&format!("sqlite:file:{name}?mode=memory&cache=shared")).await.unwrap(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_from_url() {
        assert_eq!(Backend::from_url("sqlite://data/ironheart.db").unwrap(), Backend::Sqlite);
        assert_eq!(Backend::from_url("postgres://u:p@h/db").unwrap(), Backend::Postgres);
        assert_eq!(Backend::from_url("postgresql://u:p@h/db").unwrap(), Backend::Postgres);
        assert!(Backend::from_url("mysql://h/db").is_err());
        assert_eq!(describe("postgres://u:segreto@db.example.com/ih?sslmode=require"), "Postgres su db.example.com:5432/ih");
        assert_eq!(sqlite_create_if_missing("sqlite://data/x.db"), "sqlite://data/x.db?mode=rwc");
    }

    /// Le connessioni del pool di prova vedono lo stesso database.
    #[tokio::test]
    async fn test_pool_is_shared_between_connections() {
        let pool = test_pool().await;
        let mut a = pool.acquire().await.unwrap();
        let mut b = pool.acquire().await.unwrap();
        sqlx::query("INSERT INTO user_invites (email, role, created_at) VALUES ('x@example.com', 'user', 1)")
            .execute(&mut *a)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_invites")
            .fetch_one(&mut *b)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
