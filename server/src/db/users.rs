//! Management of users and invites (admins only).

use serde::Serialize;

use super::Pool;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UserSummary {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub role: String,
    pub created_at: i64,
    pub app_count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Invite {
    pub email: String,
    pub role: String,
    pub created_at: i64,
}

pub async fn list_users(pool: &Pool) -> sqlx::Result<Vec<UserSummary>> {
    sqlx::query_as(
        "SELECT u.id, u.email, u.name, u.role, u.created_at,
                (SELECT COUNT(*) FROM apps a WHERE a.user_id = u.id) AS app_count
         FROM users u ORDER BY u.email",
    )
    .fetch_all(pool)
    .await
}

pub async fn find_user(pool: &Pool, user_id: &str) -> sqlx::Result<Option<UserSummary>> {
    sqlx::query_as(
        "SELECT u.id, u.email, u.name, u.role, u.created_at,
                (SELECT COUNT(*) FROM apps a WHERE a.user_id = u.id) AS app_count
         FROM users u WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_invites(pool: &Pool) -> sqlx::Result<Vec<Invite>> {
    sqlx::query_as("SELECT email, role, created_at FROM user_invites ORDER BY email")
        .fetch_all(pool)
        .await
}

#[derive(Debug, PartialEq, Eq)]
pub enum InviteOutcome {
    Created,
    /// Already existed: role updated.
    Updated,
    AlreadyUser,
}

/// `email` must be passed in lowercase.
pub async fn create_invite(
    pool: &Pool,
    email: &str,
    role: &str,
    invited_by: Option<&str>,
    now: i64,
) -> sqlx::Result<InviteOutcome> {
    let mut tx = pool.begin().await?;
    // COUNT instead of EXISTS: EXISTS is a boolean in Postgres and an integer in SQLite.
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email = $1")
        .bind(email)
        .fetch_one(&mut *tx)
        .await?;
    if users > 0 {
        return Ok(InviteOutcome::AlreadyUser);
    }
    let invites: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_invites WHERE email = $1")
        .bind(email)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO user_invites (email, role, invited_by, created_at) VALUES ($1, $2, $3, $4)
         ON CONFLICT (email) DO UPDATE SET role = excluded.role",
    )
    .bind(email)
    .bind(role)
    .bind(invited_by)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(if invites > 0 {
        InviteOutcome::Updated
    } else {
        InviteOutcome::Created
    })
}

pub async fn delete_invite(pool: &Pool, email: &str) -> sqlx::Result<bool> {
    Ok(sqlx::query("DELETE FROM user_invites WHERE email = $1")
        .bind(email)
        .execute(pool)
        .await?
        .rows_affected()
        == 1)
}

pub async fn set_user_role(pool: &Pool, user_id: &str, role: &str) -> sqlx::Result<bool> {
    Ok(sqlx::query("UPDATE users SET role = $1 WHERE id = $2")
        .bind(role)
        .bind(user_id)
        .execute(pool)
        .await?
        .rows_affected()
        == 1)
}

/// Deletes the user with apps and sessions (cascading). Returns the ids of the
/// deleted apps, to remove from the in-memory registry, or `None` if it did not exist.
pub async fn delete_user(pool: &Pool, user_id: &str) -> sqlx::Result<Option<Vec<String>>> {
    let mut tx = pool.begin().await?;
    let app_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM apps WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
    let deleted = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    tx.commit().await?;
    Ok(deleted.then_some(app_ids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{insert_session, insert_test_app, session_user, test_pool, upsert_google_user};

    #[tokio::test]
    async fn invites_skip_existing_users_and_update_role() {
        let pool = test_pool().await;
        upsert_google_user(&pool, "sub", "u@example.com", None, 1)
            .await
            .unwrap();
        assert_eq!(
            create_invite(&pool, "u@example.com", "user", None, 1)
                .await
                .unwrap(),
            InviteOutcome::AlreadyUser
        );
        assert_eq!(
            create_invite(&pool, "n@example.com", "user", None, 1)
                .await
                .unwrap(),
            InviteOutcome::Created
        );
        assert_eq!(
            create_invite(&pool, "n@example.com", "admin", None, 2)
                .await
                .unwrap(),
            InviteOutcome::Updated
        );
        let invites = list_invites(&pool).await.unwrap();
        assert_eq!((invites.len(), invites[0].role.as_str()), (1, "admin"));
        assert!(delete_invite(&pool, "n@example.com").await.unwrap());
        assert!(!delete_invite(&pool, "n@example.com").await.unwrap());
    }

    #[tokio::test]
    async fn deleting_a_user_removes_apps_and_sessions() {
        let pool = test_pool().await;
        let user = upsert_google_user(&pool, "sub", "u@example.com", None, 1)
            .await
            .unwrap();
        insert_test_app(&pool, "a1", &user.id).await;
        insert_session(&pool, "h", &user.id, 1, 1_000)
            .await
            .unwrap();
        assert_eq!(list_users(&pool).await.unwrap()[0].app_count, 1);

        assert_eq!(
            delete_user(&pool, &user.id).await.unwrap(),
            Some(vec!["a1".to_owned()])
        );
        assert!(session_user(&pool, "h", 0).await.unwrap().is_none());
        assert_eq!(delete_user(&pool, &user.id).await.unwrap(), None);
    }
}
