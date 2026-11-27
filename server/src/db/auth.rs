//! Users, pending logins, sessions and alert recipients.

use serde::Serialize;
use uuid::Uuid;

use super::Pool;

// ---- Users ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    /// `admin` or `user`.
    pub role: String,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

/// Google sign-in. Allowed in: whoever is already registered, the master user, or
/// whoever has an invite for this email (the invite is consumed). Everyone else:
/// `None`, and nothing is created.
///
/// `email` must be passed in lowercase.
pub async fn login_google_user(
    pool: &Pool,
    google_sub: &str,
    email: &str,
    name: Option<&str>,
    is_master: bool,
    now: i64,
) -> sqlx::Result<Option<User>> {
    let mut tx = pool.begin().await?;
    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE google_sub = $1")
        .bind(google_sub)
        .fetch_optional(&mut *tx)
        .await?;
    let role: Option<String> = if is_master {
        Some("admin".into())
    } else if existing.is_some() {
        None
    } else {
        sqlx::query_scalar("DELETE FROM user_invites WHERE email = $1 RETURNING role")
            .bind(email)
            .fetch_optional(&mut *tx)
            .await?
    };
    if existing.is_none() && role.is_none() {
        return Ok(None);
    }
    let user = sqlx::query_as(
        "INSERT INTO users (id, google_sub, email, name, role, created_at)
         VALUES ($1, $2, $3, $4, COALESCE($5, 'user'), $6)
         ON CONFLICT (google_sub) DO UPDATE SET
             email = excluded.email, name = excluded.name, role = COALESCE($5, users.role)
         RETURNING id, email, name, role",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(google_sub)
    .bind(email)
    .bind(name)
    .bind(role)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(user))
}

/// Test user with the `user` role.
#[cfg(test)]
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
         RETURNING id, email, name, role",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(google_sub)
    .bind(email)
    .bind(name)
    .bind(now)
    .fetch_one(pool)
    .await
}

// ---- Alert recipients -------------------------------------------------------

/// Addresses to send an app's alerts to: the configured ones,
/// or the account email if the list is empty.
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

// ---- Pending logins ---------------------------------------------------------

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

/// Fetches and deletes the pending login, if it is not older than `created_after`.
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
        .map(|(nonce, pkce_verifier, _)| OAuthFlow {
            nonce,
            pkce_verifier,
        }))
}

pub async fn delete_oauth_flows_before(pool: &Pool, before: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM oauth_flows WHERE created_at <= $1")
        .bind(before)
        .execute(pool)
        .await
        .map(|_| ())
}

// ---- Sessions ---------------------------------------------------------------

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
        "SELECT u.id, u.email, u.name, u.role
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

    use crate::db::test_pool as memory_pool;

    #[tokio::test]
    async fn upsert_keeps_id_and_updates_email() {
        let pool = memory_pool().await;
        let first = upsert_google_user(&pool, "sub-1", "a@example.com", Some("A"), 1)
            .await
            .unwrap();
        let second = upsert_google_user(&pool, "sub-1", "b@example.com", None, 2)
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.email, "b@example.com");
        assert_eq!(second.name, None);
    }

    #[tokio::test]
    async fn alert_recipients_fall_back_to_account_email() {
        let pool = memory_pool().await;
        let user = upsert_google_user(&pool, "sub-1", "owner@example.com", None, 1)
            .await
            .unwrap();
        crate::db::insert_test_app(&pool, "app-1", &user.id).await;

        assert_eq!(
            alert_recipients(&pool, "app-1").await.unwrap(),
            ["owner@example.com"]
        );

        for email in ["ops@example.com", "dev@example.com"] {
            sqlx::query(
                "INSERT INTO app_recipients (app_id, email, created_at) VALUES ('app-1', $1, 1)",
            )
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
    async fn login_requires_master_invite_or_existing_user() {
        let pool = memory_pool().await;
        let login = |sub: &'static str, email: &'static str, master: bool| {
            let pool = pool.clone();
            async move {
                login_google_user(&pool, sub, email, None, master, 1)
                    .await
                    .unwrap()
            }
        };

        assert!(
            login("sub-x", "x@example.com", false).await.is_none(),
            "without an invite"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );

        let master = login("sub-m", "m@example.com", true).await.unwrap();
        assert_eq!(master.role, "admin");

        crate::db::create_invite(&pool, "x@example.com", "user", Some(&master.id), 2)
            .await
            .unwrap();
        let invited = login("sub-x", "x@example.com", false).await.unwrap();
        assert_eq!(invited.role, "user");
        assert!(
            crate::db::list_invites(&pool).await.unwrap().is_empty(),
            "invite consumed"
        );

        // Already registered: gets in without an invite and keeps the role.
        crate::db::set_user_role(&pool, &invited.id, "admin")
            .await
            .unwrap();
        assert_eq!(
            login("sub-x", "x2@example.com", false).await.unwrap().role,
            "admin"
        );

        // The master becomes admin again even if the role in the database changed.
        crate::db::set_user_role(&pool, &master.id, "user")
            .await
            .unwrap();
        assert_eq!(
            login("sub-m", "m@example.com", true).await.unwrap().role,
            "admin"
        );
    }

    #[tokio::test]
    async fn oauth_flow_is_single_use_and_expires() {
        let pool = memory_pool().await;
        let flow = OAuthFlow {
            nonce: "n".into(),
            pkce_verifier: "v".into(),
        };
        insert_oauth_flow(&pool, "s1", &flow, 100).await.unwrap();
        insert_oauth_flow(&pool, "s2", &flow, 100).await.unwrap();

        assert!(take_oauth_flow(&pool, "s1", 50).await.unwrap().is_some());
        assert!(take_oauth_flow(&pool, "s1", 50).await.unwrap().is_none());
        assert!(take_oauth_flow(&pool, "s2", 100).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_lookup_respects_expiry() {
        let pool = memory_pool().await;
        let user = upsert_google_user(&pool, "sub-1", "a@example.com", None, 1)
            .await
            .unwrap();
        insert_session(&pool, "h", &user.id, 1, 1_000)
            .await
            .unwrap();

        assert_eq!(
            session_user(&pool, "h", 999).await.unwrap().map(|u| u.id),
            Some(user.id)
        );
        assert!(session_user(&pool, "h", 1_000).await.unwrap().is_none());
        delete_expired_sessions(&pool, 1_000).await.unwrap();
        assert!(session_user(&pool, "h", 0).await.unwrap().is_none());
    }
}
