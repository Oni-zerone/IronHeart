//! Dashboard API for the signed-in user's Telegram chats.
//!
//! Every endpoint answers 404 while the bot is not active (no token, or a
//! token Telegram refused): for the dashboard the feature does not exist.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;

use super::{ApiError as SendError, Message, bot::hash_token};
use crate::{
    AppState,
    apps::ApiError,
    auth::CurrentUser,
    clock::{MINUTE_MS, now_ms},
    db,
};

/// The link from the dashboard lasts 10 minutes and works once.
const LINK_TTL_MS: i64 = 10 * MINUTE_MS;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/me/telegram", get(overview))
        .route("/api/me/telegram/link", post(create_link))
        .route("/api/me/telegram/test", post(send_test))
        .route("/api/me/telegram/{id}", delete(remove))
}

type ApiResult<T> = Result<T, ApiError>;

fn bot_username(state: &AppState) -> ApiResult<String> {
    state.telegram.bot_username().ok_or(ApiError::NotFound)
}

#[derive(Serialize)]
struct Overview {
    bot_username: String,
    bot_url: String,
    links: Vec<db::TelegramLink>,
}

async fn overview(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Overview>> {
    let bot_username = bot_username(&state)?;
    Ok(Json(Overview {
        bot_url: format!("https://t.me/{bot_username}"),
        bot_username,
        links: db::list_telegram_links(&state.db, &user.id).await?,
    }))
}

#[derive(Serialize)]
struct LinkInvite {
    /// `https://t.me/<bot>?start=<token>`: opening it links the chat.
    url: String,
    expires_at: i64,
}

async fn create_link(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<LinkInvite>> {
    let bot_username = bot_username(&state)?;
    // 32 characters from [A-Za-z0-9_-]: what Telegram accepts in `start` (at most 64).
    let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 24]>());
    let expires_at = now_ms() + LINK_TTL_MS;
    db::insert_telegram_token(&state.db, &hash_token(&token), &user.id, expires_at).await?;
    Ok(Json(LinkInvite { url: format!("https://t.me/{bot_username}?start={token}"), expires_at }))
}

/// Sends a test message to every active chat of the account.
async fn send_test(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<StatusCode> {
    bot_username(&state)?;
    let chats = db::telegram_chats_for_user(&state.db, &user.id).await?;
    if chats.is_empty() {
        return Err(ApiError::Invalid("No linked Telegram chat.".into()));
    }
    let mut failed = 0;
    for chat_id in chats {
        let message = Message::text(chat_id, "Test message: IronHeart can send you alerts here.");
        match state.telegram.send(&message).await {
            Ok(()) => {}
            Err(SendError::Blocked(detail)) => {
                tracing::warn!(chat_id, %detail, "telegram: chat refuses messages, link disabled");
                db::disable_telegram_chat(&state.db, chat_id, now_ms()).await?;
                failed += 1;
            }
            Err(err) => {
                tracing::error!(chat_id, %err, "telegram: test message not sent");
                failed += 1;
            }
        }
    }
    Ok(if failed == 0 { StatusCode::NO_CONTENT } else { StatusCode::BAD_GATEWAY })
}

async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    bot_username(&state)?;
    if db::delete_telegram_link(&state.db, &user.id, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{crypto, heartbeat, telegram::Telegram};

    async fn state(telegram: Telegram) -> (AppState, db::User) {
        let pool = db::test_pool().await;
        let hb = Arc::new(heartbeat::Heartbeat::load(&pool, crypto::SecretBox::new(&[1; 32], 1), None).await.unwrap());
        let mut state = crate::test_state(pool, hb);
        state.telegram = Arc::new(telegram);
        let user = db::upsert_google_user(&state.db, "s", "a@example.com", None, 1).await.unwrap();
        (state, user)
    }

    #[tokio::test]
    async fn inactive_bot_hides_everything() {
        let (state, user) = state(Telegram::new(None, crate::telegram::DEFAULT_API_URL).unwrap()).await;
        let me = || CurrentUser(user.clone());
        assert!(matches!(overview(State(state.clone()), me()).await, Err(ApiError::NotFound)));
        assert!(matches!(create_link(State(state.clone()), me()).await, Err(ApiError::NotFound)));
        assert!(matches!(send_test(State(state.clone()), me()).await, Err(ApiError::NotFound)));
        assert!(matches!(remove(State(state), me(), Path("x".into())).await, Err(ApiError::NotFound)));
    }

    #[tokio::test]
    async fn link_token_is_stored_hashed() {
        let (state, user) = state(Telegram::for_tests("ironheart_bot")).await;
        let Json(invite) = create_link(State(state.clone()), CurrentUser(user.clone())).await.ok().unwrap();
        let token = invite.url.strip_prefix("https://t.me/ironheart_bot?start=").unwrap();
        assert_eq!(token.len(), 32);
        assert!(db::take_telegram_token(&state.db, token, now_ms()).await.unwrap().is_none(), "not in clear");
        let linked = db::take_telegram_token(&state.db, &hash_token(token), now_ms()).await.unwrap().unwrap();
        assert_eq!(linked.id, user.id);

        let Json(view) = overview(State(state.clone()), CurrentUser(user.clone())).await.ok().unwrap();
        assert_eq!(view.bot_url, "https://t.me/ironheart_bot");
        assert!(view.links.is_empty());
        assert!(matches!(send_test(State(state), CurrentUser(user)).await, Err(ApiError::Invalid(_))));
    }
}
