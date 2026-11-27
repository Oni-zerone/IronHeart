//! Telegram bot: alerts in a chat and fixed commands answered with IronHeart data.
//!
//! One bot per instance (`TELEGRAM_BOT_TOKEN`). Without a token Telegram does
//! not exist anywhere. With a token, `getMe` checks it at startup, then daily
//! (hourly while it fails), like the email key (`mail_check::KeyMonitor`): the
//! bot is active only while Telegram accepts the token, and polling errors
//! count as an outage (`channels` treats one longer than an hour as broken).
//!
//! Updates arrive with long polling (`getUpdates`): no public endpoint. The Bot
//! API is called with `reqwest`, no SDK. The token is part of the URL, so
//! request errors are logged without it (`without_url`).

pub mod api;
mod bot;
pub mod text;

use std::{sync::RwLock, time::Duration};

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{
    AppState,
    clock::now_ms,
    db,
    mail::KeyCheck,
    mail_check::{KeyMonitor, record_check},
};

const API_BASE: &str = "https://api.telegram.org";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Long polling: Telegram holds the request up to this long when there is nothing new.
const POLL_SECS: u64 = 50;
const MAX_STARTUP_BACKOFF: Duration = Duration::from_secs(300);
/// Telegram refuses messages longer than 4096 characters.
const MAX_TEXT_CHARS: usize = 4000;
/// Waits honored on a 429 before giving up on a message.
const MAX_SEND_ATTEMPTS: usize = 3;
/// Name in the logs and in `KeyStatus`.
pub const PROVIDER: &str = "Telegram";

/// A message for a chat, with optional inline buttons.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub chat_id: i64,
    pub text: String,
    /// Rows of buttons: (label, callback data).
    pub buttons: Vec<Vec<(String, String)>>,
}

impl Message {
    pub fn text(chat_id: i64, text: impl Into<String>) -> Self {
        Self { chat_id, text: text.into(), buttons: Vec::new() }
    }
}

#[derive(Debug)]
pub enum ApiError {
    /// Token refused (401, or 404 for a malformed token).
    Unauthorized,
    /// The chat does not accept messages: bot blocked, chat deleted.
    Blocked(String),
    /// Too many requests: wait this many seconds.
    RetryAfter(u64),
    /// Another process is reading the updates (or a webhook is set).
    Conflict(String),
    Other(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "token refused"),
            Self::Blocked(d) => write!(f, "chat unavailable: {d}"),
            Self::RetryAfter(s) => write!(f, "rate limited, retry after {s} s"),
            Self::Conflict(d) => write!(f, "conflict: {d}"),
            Self::Other(d) => write!(f, "{d}"),
        }
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    ok: bool,
    result: Option<T>,
    error_code: Option<i64>,
    description: Option<String>,
    parameters: Option<ResponseParameters>,
}

#[derive(Deserialize)]
struct ResponseParameters {
    retry_after: Option<u64>,
}

impl<T> Envelope<T> {
    fn into_result(self) -> Result<T, ApiError> {
        if self.ok
            && let Some(result) = self.result
        {
            return Ok(result);
        }
        let description = self.description.unwrap_or_default();
        Err(match self.error_code {
            Some(401 | 404) => ApiError::Unauthorized,
            Some(403) => ApiError::Blocked(description),
            Some(400) if description.contains("chat not found") => ApiError::Blocked(description),
            Some(409) => ApiError::Conflict(description),
            Some(429) => ApiError::RetryAfter(self.parameters.and_then(|p| p.retry_after).unwrap_or(5)),
            code => ApiError::Other(format!("{} {description}", code.unwrap_or(0))),
        })
    }
}

struct Api {
    http: reqwest::Client,
    /// `https://api.telegram.org/bot<token>`: secret, never logged.
    base: String,
}

pub struct Telegram {
    api: Option<Api>,
    /// A token is set: Telegram is one of the instance's alert channels.
    configured: bool,
    /// Outcome of the token check and of polling.
    pub health: KeyMonitor,
    /// Set once `getMe` succeeded.
    username: RwLock<Option<String>>,
}

impl Telegram {
    pub fn new(token: Option<&str>) -> anyhow::Result<Self> {
        let api = match token {
            Some(token) => Some(Api {
                http: reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?,
                base: format!("{API_BASE}/bot{token}"),
            }),
            None => None,
        };
        let configured = api.is_some();
        Ok(Self { api, configured, health: KeyMonitor::new(configured), username: RwLock::new(None) })
    }

    /// Active bot without a token: the dashboard and the commands work, sends fail.
    #[cfg(test)]
    pub fn for_tests(username: &str) -> Self {
        let telegram =
            Self { api: None, configured: true, health: KeyMonitor::new(true), username: RwLock::new(Some(username.into())) };
        record_check(&telegram.health, PROVIDER, KeyCheck::Valid, 0);
        telegram
    }

    pub fn is_configured(&self) -> bool {
        self.configured
    }

    /// The bot's `@username`, only while the bot is active.
    pub fn bot_username(&self) -> Option<String> {
        if self.health.rejected() {
            return None;
        }
        self.username.read().expect("telegram lock").clone()
    }

    /// Telegram accepted the token: linking and commands work. During an outage
    /// the bot stays active (sends fail and are logged).
    pub fn is_active(&self) -> bool {
        self.bot_username().is_some()
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, body: Value, timeout: Duration) -> Result<T, ApiError> {
        let Some(api) = &self.api else {
            return Err(ApiError::Other("no bot token".into()));
        };
        let response = api
            .http
            .post(format!("{}/{method}", api.base))
            .json(&body)
            .timeout(timeout)
            .send()
            .await
            .map_err(|err| ApiError::Other(err.without_url().to_string()))?;
        let envelope: Envelope<T> =
            response.json().await.map_err(|err| ApiError::Other(err.without_url().to_string()))?;
        envelope.into_result()
    }

    pub async fn send(&self, message: &Message) -> Result<(), ApiError> {
        let text: String = message.text.chars().take(MAX_TEXT_CHARS).collect();
        let mut body = json!({
            "chat_id": message.chat_id,
            "text": text,
            "link_preview_options": { "is_disabled": true },
        });
        if !message.buttons.is_empty() {
            let rows: Vec<Vec<Value>> = message
                .buttons
                .iter()
                .map(|row| row.iter().map(|(label, data)| json!({ "text": label, "callback_data": data })).collect())
                .collect();
            body["reply_markup"] = json!({ "inline_keyboard": rows });
        }
        self.call::<Value>("sendMessage", body, REQUEST_TIMEOUT).await.map(|_| ())
    }

    /// Alert from the queue: honors 429 waits; a chat that refuses the
    /// message is disabled so it is not tried again at every alert.
    pub async fn deliver(&self, pool: &db::Pool, message: &Message) {
        for _ in 0..MAX_SEND_ATTEMPTS {
            match self.send(message).await {
                Ok(()) => return,
                Err(ApiError::RetryAfter(secs)) => tokio::time::sleep(Duration::from_secs(secs.min(60))).await,
                Err(ApiError::Blocked(detail)) => {
                    tracing::warn!(chat_id = message.chat_id, %detail, "telegram: chat refuses messages, link disabled");
                    if let Err(err) = db::disable_telegram_chat(pool, message.chat_id, now_ms()).await {
                        tracing::error!(err = ?err, "telegram: link not disabled");
                    }
                    return;
                }
                Err(err) => {
                    tracing::error!(chat_id = message.chat_id, %err, "telegram: alert not sent");
                    return;
                }
            }
        }
        tracing::error!(chat_id = message.chat_id, "telegram: alert not sent (rate limited)");
    }
}

// ---- Updates ------------------------------------------------------------------

#[derive(Deserialize)]
struct BotUser {
    username: Option<String>,
}

#[derive(Deserialize)]
struct Update {
    update_id: i64,
    message: Option<IncomingMessage>,
    callback_query: Option<CallbackQuery>,
}

#[derive(Deserialize)]
struct IncomingMessage {
    chat: Chat,
    from: Option<Sender>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct Chat {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct Sender {
    first_name: String,
    username: Option<String>,
}

#[derive(Deserialize)]
struct CallbackQuery {
    id: String,
    message: Option<CallbackMessage>,
    data: Option<String>,
}

#[derive(Deserialize)]
struct CallbackMessage {
    chat: Chat,
}

/// Starts the bot if there is a token: the token check and the polling.
pub fn spawn(state: AppState) {
    if !state.telegram.configured {
        tracing::info!("telegram off: TELEGRAM_BOT_TOKEN not set");
        return;
    }
    tokio::spawn(check_loop(state.clone()));
    tokio::spawn(poll_loop(state));
}

/// `getMe` at startup, then daily (hourly while it fails). When the token
/// starts working, the commands are registered and the webhook removed.
async fn check_loop(state: AppState) {
    let telegram = &state.telegram;
    let mut backoff = Duration::from_secs(5);
    loop {
        let was_ok = telegram.health.get().is_some_and(|s| s.state == "ok");
        let check = match telegram.call::<BotUser>("getMe", json!({}), REQUEST_TIMEOUT).await {
            Ok(BotUser { username: Some(username) }) => {
                *telegram.username.write().expect("telegram lock") = Some(username.clone());
                if !was_ok {
                    setup(telegram).await;
                    tracing::info!("telegram: bot @{username} active");
                }
                KeyCheck::Valid
            }
            Ok(BotUser { username: None }) => KeyCheck::Rejected("getMe returned no bot username".into()),
            Err(ApiError::Unauthorized) => KeyCheck::Rejected("Telegram refused the bot token".into()),
            Err(err) => KeyCheck::Unreachable(format!("Telegram unreachable: {err}")),
        };
        let unreachable = matches!(check, KeyCheck::Unreachable(_));
        let mut wait = record_check(&telegram.health, PROVIDER, check, now_ms());
        // Until the first answer the bot is off: retry sooner than hourly.
        if unreachable && telegram.username.read().expect("telegram lock").is_none() {
            wait = backoff;
            backoff = (backoff * 2).min(MAX_STARTUP_BACKOFF);
        }
        tokio::time::sleep(wait).await;
    }
}

async fn setup(telegram: &Telegram) {
    // Polling does not work while a webhook is set.
    if let Err(err) = telegram.call::<bool>("deleteWebhook", json!({}), REQUEST_TIMEOUT).await {
        tracing::warn!(%err, "telegram: deleteWebhook failed");
    }
    let commands: Vec<Value> =
        bot::COMMANDS.iter().map(|(command, description)| json!({ "command": command, "description": description })).collect();
    if let Err(err) = telegram.call::<bool>("setMyCommands", json!({ "commands": commands }), REQUEST_TIMEOUT).await {
        tracing::warn!(%err, "telegram: setMyCommands failed");
    }
}

/// Long polling while the bot is active. Its outcome also feeds the health:
/// a network error starts an outage, the next answer ends it.
async fn poll_loop(state: AppState) {
    let telegram = &state.telegram;
    let mut offset = 0_i64;
    let mut limiter = bot::Limiter::default();
    loop {
        if !telegram.is_active() {
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        }
        let body = json!({
            "offset": offset,
            "timeout": POLL_SECS,
            "allowed_updates": ["message", "callback_query"],
        });
        let updates: Vec<Update> =
            match telegram.call("getUpdates", body, Duration::from_secs(POLL_SECS + 10)).await {
                Ok(updates) => updates,
                Err(ApiError::Unauthorized) => {
                    record_check(&telegram.health, PROVIDER, KeyCheck::Rejected("Telegram refused the bot token".into()), now_ms());
                    continue;
                }
                Err(ApiError::Conflict(detail)) => {
                    tracing::warn!(%detail, "telegram: another process reads this bot's updates");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
                Err(ApiError::RetryAfter(secs)) => {
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    continue;
                }
                Err(err) => {
                    if telegram.health.get().is_some_and(|s| s.state == "ok") {
                        let detail = format!("Telegram unreachable: {err}");
                        record_check(&telegram.health, PROVIDER, KeyCheck::Unreachable(detail), now_ms());
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
        if telegram.health.get().is_some_and(|s| s.state == "unreachable") {
            record_check(&telegram.health, PROVIDER, KeyCheck::Valid, now_ms());
        }
        for update in updates {
            offset = offset.max(update.update_id + 1);
            if let Err(err) = handle_update(&state, update, &mut limiter).await {
                tracing::error!(err = ?err, "telegram: update not handled");
            }
        }
    }
}

async fn handle_update(state: &AppState, update: Update, limiter: &mut bot::Limiter) -> anyhow::Result<()> {
    let now = now_ms();
    let reply = if let Some(message) = update.message {
        let Some(text) = message.text else { return Ok(()) };
        if !limiter.allow(message.chat.id, now) {
            return Ok(());
        }
        let incoming = bot::Incoming {
            chat_id: message.chat.id,
            private: message.chat.kind == "private",
            label: message.from.map(|from| match from.username {
                Some(username) => format!("@{username}"),
                None => from.first_name,
            }),
            text,
        };
        bot::handle_message(state, &incoming, now).await?
    } else if let Some(query) = update.callback_query {
        // Stops the spinner on the button, whatever happens next.
        let _ = state
            .telegram
            .call::<bool>("answerCallbackQuery", json!({ "callback_query_id": query.id }), REQUEST_TIMEOUT)
            .await;
        let (Some(message), Some(data)) = (query.message, query.data) else { return Ok(()) };
        if message.chat.kind != "private" || !limiter.allow(message.chat.id, now) {
            return Ok(());
        }
        bot::handle_callback(state, message.chat.id, &data, now).await?
    } else {
        None
    };
    if let Some(reply) = reply
        && let Err(err) = state.telegram.send(&reply).await
    {
        tracing::warn!(chat_id = reply.chat_id, %err, "telegram: reply not sent");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(body: &str) -> Result<Value, ApiError> {
        serde_json::from_str::<Envelope<Value>>(body).unwrap().into_result()
    }

    #[test]
    fn api_errors_are_classified() {
        assert!(envelope(r#"{"ok":true,"result":true}"#).is_ok());
        assert!(matches!(envelope(r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#), Err(ApiError::Unauthorized)));
        assert!(matches!(envelope(r#"{"ok":false,"error_code":404,"description":"Not Found"}"#), Err(ApiError::Unauthorized)));
        assert!(matches!(
            envelope(r#"{"ok":false,"error_code":403,"description":"Forbidden: bot was blocked by the user"}"#),
            Err(ApiError::Blocked(_))
        ));
        assert!(matches!(
            envelope(r#"{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}"#),
            Err(ApiError::Blocked(_))
        ));
        assert!(matches!(
            envelope(r#"{"ok":false,"error_code":400,"description":"Bad Request: message text is empty"}"#),
            Err(ApiError::Other(_))
        ));
        assert!(matches!(
            envelope(r#"{"ok":false,"error_code":429,"description":"Too Many Requests","parameters":{"retry_after":7}}"#),
            Err(ApiError::RetryAfter(7))
        ));
        assert!(matches!(envelope(r#"{"ok":false,"error_code":409,"description":"Conflict"}"#), Err(ApiError::Conflict(_))));
    }

    #[test]
    fn without_token_the_bot_is_off() {
        let telegram = Telegram::new(None).unwrap();
        assert!(!telegram.is_configured());
        assert!(!telegram.is_active());
        assert_eq!(telegram.bot_username(), None);
    }

    #[test]
    fn refused_token_turns_the_bot_off() {
        let telegram = Telegram::for_tests("ironheart_bot");
        assert!(telegram.is_active());
        record_check(&telegram.health, PROVIDER, KeyCheck::Unreachable("timeout".into()), 1);
        assert!(telegram.is_active(), "an outage keeps the bot active");
        record_check(&telegram.health, PROVIDER, KeyCheck::Rejected("401".into()), 2);
        assert!(!telegram.is_active());
        assert!(telegram.is_configured());
    }
}
