//! Telegram: chats linked to accounts and the one-time tokens that link them.

use serde::Serialize;
use uuid::Uuid;

use super::{Pool, User};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TelegramLink {
    pub id: String,
    /// "@username" or first name.
    pub label: String,
    pub created_at: i64,
    /// Telegram refused delivery (bot blocked): no alerts until the chat writes again.
    pub disabled_at: Option<i64>,
}

pub async fn insert_telegram_token(
    pool: &Pool,
    token_hash: &str,
    user_id: &str,
    expires_at: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO telegram_link_tokens (token_hash, user_id, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(expires_at)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Consumes the token (single use) and returns its user, if it has not expired.
pub async fn take_telegram_token(
    pool: &Pool,
    token_hash: &str,
    now: i64,
) -> sqlx::Result<Option<User>> {
    let mut tx = pool.begin().await?;
    let row: Option<(String, i64)> = sqlx::query_as(
        "DELETE FROM telegram_link_tokens WHERE token_hash = $1 RETURNING user_id, expires_at",
    )
    .bind(token_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let user = match row {
        Some((user_id, expires_at)) if expires_at > now => {
            sqlx::query_as("SELECT id, email, name, role FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?
        }
        _ => None,
    };
    tx.commit().await?;
    Ok(user)
}

pub async fn delete_telegram_tokens_before(pool: &Pool, now: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM telegram_link_tokens WHERE expires_at <= $1")
        .bind(now)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Links the chat to the user. A chat already linked to another account moves
/// to this one (a chat belongs to a single account).
pub async fn link_telegram_chat(
    pool: &Pool,
    user_id: &str,
    chat_id: i64,
    label: &str,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO telegram_links (id, user_id, chat_id, label, created_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (chat_id) DO UPDATE SET
             user_id = excluded.user_id, label = excluded.label,
             created_at = excluded.created_at, disabled_at = NULL",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(user_id)
    .bind(chat_id)
    .bind(label)
    .bind(now)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Account linked to the chat. A message from the chat proves the bot is no
/// longer blocked, so a disabled link is enabled again (one statement).
pub async fn telegram_chat_user(pool: &Pool, chat_id: i64) -> sqlx::Result<Option<User>> {
    sqlx::query("UPDATE telegram_links SET disabled_at = NULL WHERE chat_id = $1 AND disabled_at IS NOT NULL")
        .bind(chat_id)
        .execute(pool)
        .await?;
    sqlx::query_as(
        "SELECT u.id, u.email, u.name, u.role
         FROM telegram_links l JOIN users u ON u.id = l.user_id
         WHERE l.chat_id = $1",
    )
    .bind(chat_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_telegram_links(pool: &Pool, user_id: &str) -> sqlx::Result<Vec<TelegramLink>> {
    sqlx::query_as(
        "SELECT id, label, created_at, disabled_at FROM telegram_links
         WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Active linked chats of the members of a group.
pub async fn telegram_chats_for_group(pool: &Pool, group_id: &str) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar(
        "SELECT l.chat_id FROM telegram_links l JOIN group_members m ON m.user_id = l.user_id
         WHERE m.group_id = $1 AND l.disabled_at IS NULL ORDER BY l.created_at",
    )
    .bind(group_id)
    .fetch_all(pool)
    .await
}

/// Active chats of a user (for the test message).
pub async fn telegram_chats_for_user(pool: &Pool, user_id: &str) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar(
        "SELECT chat_id FROM telegram_links WHERE user_id = $1 AND disabled_at IS NULL ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Active chats of the account with this email (the master's, for notices).
pub async fn telegram_chats_for_email(pool: &Pool, email: &str) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar(
        "SELECT l.chat_id FROM telegram_links l JOIN users u ON u.id = l.user_id
         WHERE u.email = $1 AND l.disabled_at IS NULL ORDER BY l.created_at",
    )
    .bind(email)
    .fetch_all(pool)
    .await
}

pub async fn delete_telegram_link(pool: &Pool, user_id: &str, id: &str) -> sqlx::Result<bool> {
    sqlx::query("DELETE FROM telegram_links WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected() == 1)
}

/// `/stop` from the chat itself.
pub async fn delete_telegram_chat(pool: &Pool, chat_id: i64) -> sqlx::Result<bool> {
    sqlx::query("DELETE FROM telegram_links WHERE chat_id = $1")
        .bind(chat_id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected() == 1)
}

/// Telegram refused delivery (bot blocked or chat gone): stop sending to it.
pub async fn disable_telegram_chat(pool: &Pool, chat_id: i64, now: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE telegram_links SET disabled_at = $1 WHERE chat_id = $2 AND disabled_at IS NULL",
    )
    .bind(now)
    .bind(chat_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Active chats per account email: alerts follow the email recipients.
pub async fn telegram_routes(pool: &Pool) -> sqlx::Result<Vec<(String, i64)>> {
    sqlx::query_as(
        "SELECT u.email, l.chat_id FROM telegram_links l JOIN users u ON u.id = l.user_id
         WHERE l.disabled_at IS NULL ORDER BY l.created_at",
    )
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{test_pool, upsert_google_user};

    #[tokio::test]
    async fn token_is_single_use_and_expires() {
        let pool = test_pool().await;
        let user = upsert_google_user(&pool, "sub", "a@example.com", None, 1)
            .await
            .unwrap();
        insert_telegram_token(&pool, "h1", &user.id, 100)
            .await
            .unwrap();
        insert_telegram_token(&pool, "h2", &user.id, 100)
            .await
            .unwrap();

        assert_eq!(
            take_telegram_token(&pool, "h1", 50)
                .await
                .unwrap()
                .unwrap()
                .id,
            user.id
        );
        assert!(
            take_telegram_token(&pool, "h1", 50)
                .await
                .unwrap()
                .is_none(),
            "single use"
        );
        assert!(
            take_telegram_token(&pool, "h2", 100)
                .await
                .unwrap()
                .is_none(),
            "expired"
        );
        assert!(
            take_telegram_token(&pool, "nope", 0)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn chat_moves_between_accounts_and_routes_follow_email() {
        let pool = test_pool().await;
        let a = upsert_google_user(&pool, "a", "a@example.com", None, 1)
            .await
            .unwrap();
        let b = upsert_google_user(&pool, "b", "b@example.com", None, 1)
            .await
            .unwrap();
        link_telegram_chat(&pool, &a.id, 42, "@a", 1).await.unwrap();
        link_telegram_chat(&pool, &a.id, 43, "A2", 2).await.unwrap();
        assert_eq!(
            telegram_routes(&pool).await.unwrap(),
            [("a@example.com".into(), 42), ("a@example.com".into(), 43)]
        );

        link_telegram_chat(&pool, &b.id, 42, "@b", 3).await.unwrap();
        assert_eq!(
            telegram_chat_user(&pool, 42).await.unwrap().unwrap().id,
            b.id
        );
        assert_eq!(list_telegram_links(&pool, &a.id).await.unwrap().len(), 1);

        disable_telegram_chat(&pool, 43, 5).await.unwrap();
        assert_eq!(
            telegram_routes(&pool).await.unwrap(),
            [("b@example.com".into(), 42)]
        );
        assert!(
            telegram_chats_for_user(&pool, &a.id)
                .await
                .unwrap()
                .is_empty()
        );
        // A message from the chat enables it again.
        telegram_chat_user(&pool, 43).await.unwrap();
        assert_eq!(telegram_chats_for_user(&pool, &a.id).await.unwrap(), [43]);

        let link = &list_telegram_links(&pool, &a.id).await.unwrap()[0];
        assert!(
            !delete_telegram_link(&pool, &b.id, &link.id).await.unwrap(),
            "only the owner"
        );
        assert!(delete_telegram_link(&pool, &a.id, &link.id).await.unwrap());
        assert!(delete_telegram_chat(&pool, 42).await.unwrap());
        assert!(telegram_routes(&pool).await.unwrap().is_empty());
    }
}
