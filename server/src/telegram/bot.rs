//! Bot commands: fixed answers built from IronHeart data.
//!
//! Only private chats. A chat that is not linked gets only the instructions to
//! link it, and learns nothing about apps or accounts. A linked chat sees the
//! apps of its account; every query is scoped to that account's `user_id`.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use super::{Message, text};
use crate::{
    AppState,
    alerts::{datetime, duration},
    apps,
    clock::{DAY_MS, MINUTE_MS},
    db,
};

/// Registered with `setMyCommands`, so Telegram suggests them on `/`.
/// `/start` is left out: it arrives through the link from the dashboard.
pub const COMMANDS: [(&str, &str); 9] = [
    ("apps", "All applications and their state"),
    ("down", "Applications that are down"),
    ("status", "Details of an application"),
    ("attempts", "Invalid heartbeat attempts"),
    ("pause", "Pause monitoring of an application"),
    ("resume", "Resume monitoring of an application"),
    ("test", "Send a test message"),
    ("stop", "Unlink this chat"),
    ("help", "List of commands"),
];

/// Commands per chat per minute; beyond this the bot stays silent.
const RATE_LIMIT: u32 = 20;
/// Apps shown as buttons by `/status` without an argument.
const MAX_BUTTONS: usize = 30;

pub struct Incoming {
    pub chat_id: i64,
    pub private: bool,
    /// "@username" or first name of the sender.
    pub label: Option<String>,
    pub text: String,
}

/// Command replies per second over all chats. Telegram allows about 30
/// messages per second per bot: the rest stays free for alerts, so a flood of
/// commands from many chats cannot get them refused (429).
const GLOBAL_REPLIES_PER_SEC: u32 = 10;

/// Commands per chat (windows of one minute) and replies over all chats
/// (windows of one second). Beyond either limit the bot stays silent.
#[derive(Default)]
pub struct Limiter {
    windows: HashMap<i64, (i64, u32)>,
    global: (i64, u32),
}

impl Limiter {
    pub fn allow(&mut self, chat_id: i64, now: i64) -> bool {
        if self.windows.len() > 10_000 {
            self.windows
                .retain(|_, (start, _)| now - *start < MINUTE_MS);
        }
        let (start, count) = self.windows.entry(chat_id).or_insert((now, 0));
        if now - *start >= MINUTE_MS {
            (*start, *count) = (now, 0);
        }
        *count += 1;
        if *count > RATE_LIMIT {
            return false;
        }
        let (start, count) = &mut self.global;
        if now - *start >= 1000 {
            (*start, *count) = (now, 0);
        }
        *count += 1;
        *count <= GLOBAL_REPLIES_PER_SEC
    }
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub async fn handle_message(
    state: &AppState,
    incoming: &Incoming,
    now: i64,
) -> anyhow::Result<Option<Message>> {
    let chat_id = incoming.chat_id;
    let Some((command, arg)) = parse_command(&incoming.text) else {
        if !incoming.private {
            return Ok(None);
        }
        // Any message proves the bot is no longer blocked: this re-enables a
        // disabled chat, as the dashboard promises ("write to the bot to resume").
        db::telegram_chat_user(&state.db, chat_id).await?;
        return Ok(Some(Message::text(
            chat_id,
            "Commands start with /. Try /help.",
        )));
    };
    // Phone keyboards often capitalize the first letter (/Apps).
    let command = command.to_ascii_lowercase();
    let command = command.as_str();
    if !incoming.private {
        return Ok(Some(Message::text(
            chat_id,
            "IronHeart answers only in private chats.",
        )));
    }
    let reply = |text: String| Ok(Some(Message::text(chat_id, text)));

    if command == "start" && !arg.is_empty() {
        let Some(user) = db::take_telegram_token(&state.db, &hash_token(arg), now).await? else {
            return reply(format!(
                "This link is not valid any more (it lasts 10 minutes and works once). Create a new one from {}.",
                telegram_page(state)
            ));
        };
        let label = incoming.label.clone().unwrap_or_else(|| "Telegram".into());
        let label: String = label.chars().take(64).collect();
        db::link_telegram_chat(&state.db, &user.id, chat_id, &label, now).await?;
        return reply(format!(
            "Linked to {}. Alerts for this address arrive here too.\n\n{}",
            user.email,
            help_text()
        ));
    }

    let Some(user) = db::telegram_chat_user(&state.db, chat_id).await? else {
        return reply(format!(
            "This chat is not linked to IronHeart. Sign in at {} and press \"link telegram\": the link opens this chat and links it.",
            telegram_page(state)
        ));
    };

    match command {
        "start" | "help" => reply(format!("Linked to {}.\n\n{}", user.email, help_text())),
        "apps" => reply(apps_text(state, &user, now).await?),
        "down" => reply(down_text(state, &user, now).await?),
        "status" if arg.is_empty() => Ok(Some(app_picker(state, &user, chat_id).await?)),
        "status" => match find_app(state, &user, arg).await? {
            Ok(app) => reply(status_text(state, &user, &app, now).await?),
            Err(message) => reply(message),
        },
        "attempts" if arg.is_empty() => reply(attempts_overview(state, &user, now).await?),
        "attempts" => match find_app(state, &user, arg).await? {
            Ok(app) => reply(attempts_text(state, &app, now).await?),
            Err(message) => reply(message),
        },
        "pause" | "resume" if arg.is_empty() => {
            reply(format!("Usage: /{command} <application name>"))
        }
        "pause" | "resume" => match find_app(state, &user, arg).await? {
            Ok(app) => Ok(Some(confirm_pause(chat_id, &app, command == "pause"))),
            Err(message) => reply(message),
        },
        "test" => reply("Test message: IronHeart can send you alerts here.".into()),
        "stop" => {
            db::delete_telegram_chat(&state.db, chat_id).await?;
            reply("Chat unlinked: alerts will no longer arrive here. To link it again, use the dashboard.".into())
        }
        _ => reply("Unknown command. Try /help.".into()),
    }
}

/// Inline buttons: `status:<id>`, `pause:<id>`, `resume:<id>`, `cancel`.
pub async fn handle_callback(
    state: &AppState,
    chat_id: i64,
    data: &str,
    now: i64,
) -> anyhow::Result<Option<Message>> {
    let Some(user) = db::telegram_chat_user(&state.db, chat_id).await? else {
        return Ok(None);
    };
    let reply = |text: String| Ok(Some(Message::text(chat_id, text)));
    let (action, app_id) = data.split_once(':').unwrap_or((data, ""));
    if action == "cancel" {
        return reply("Cancelled.".into());
    }
    // The app is looked up among the account's apps: data from a button cannot reach others.
    let Some(app) = db::get_app(&state.db, &user.id, app_id).await? else {
        return reply("This application no longer exists.".into());
    };
    match action {
        "status" => reply(status_text(state, &user, &app, now).await?),
        "pause" | "resume" => {
            let pause = action == "pause";
            if pause == (app.state == "paused") {
                return reply(format!(
                    "{} is already {}.",
                    app.name,
                    if pause { "paused" } else { "active" }
                ));
            }
            apps::set_paused(state, &user.id, &app, pause, now).await?;
            reply(if pause {
                format!(
                    "{} paused: no alerts until you resume it (/resume {}).",
                    app.name, app.name
                )
            } else {
                format!(
                    "{} resumed: the next heartbeat is expected within {}.",
                    app.name,
                    duration((app.interval_secs + app.grace_secs) * 1000)
                )
            })
        }
        _ => Ok(None),
    }
}

/// `/status@ironheart_bot nas` → ("status", "nas").
fn parse_command(text: &str) -> Option<(&str, &str)> {
    let text = text.trim().strip_prefix('/')?;
    let (head, arg) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let command = head.split('@').next().unwrap_or(head);
    Some((command, arg.trim()))
}

fn help_text() -> String {
    let mut lines = vec!["Commands:".to_owned()];
    lines.extend(
        COMMANDS
            .iter()
            .map(|(command, description)| format!("/{command} - {description}")),
    );
    lines.join("\n")
}

fn telegram_page(state: &AppState) -> String {
    format!("{}/telegram", state.public_base_url)
}

/// By name, ignoring case: exact match first, then a unique prefix.
async fn find_app(
    state: &AppState,
    user: &db::User,
    query: &str,
) -> anyhow::Result<Result<db::App, String>> {
    let apps = db::list_apps(&state.db, &user.id).await?;
    let query = query.to_lowercase();
    if let Some(app) = apps.iter().find(|a| a.name.to_lowercase() == query) {
        return Ok(Ok(app.clone()));
    }
    let matches: Vec<&db::App> = apps
        .iter()
        .filter(|a| a.name.to_lowercase().starts_with(&query))
        .collect();
    Ok(match matches.as_slice() {
        [app] => Ok((*app).clone()),
        [] => Err("No application with that name. Try /apps.".into()),
        many => Err(format!(
            "More than one application matches: {}.",
            many.iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    })
}

/// One line per app: state and what comes next.
fn app_line(app: &db::App, now: i64) -> String {
    match app.state.as_str() {
        "up" => match app.next_deadline_at {
            Some(at) if at > now => {
                format!("[up] {} - deadline in {}", app.name, duration(at - now))
            }
            _ => format!("[up] {} - deadline passed, checking", app.name),
        },
        "down" => match app.down_since {
            Some(since) => format!("[DOWN] {} - for {}", app.name, duration(now - since)),
            None => format!("[DOWN] {}", app.name),
        },
        "paused" => format!("[paused] {}", app.name),
        _ => format!("[new] {} - waiting for the first heartbeat", app.name),
    }
}

async fn apps_text(state: &AppState, user: &db::User, now: i64) -> anyhow::Result<String> {
    let apps = db::list_apps(&state.db, &user.id).await?;
    if apps.is_empty() {
        return Ok(format!(
            "No applications. Create one at {}/apps",
            state.public_base_url
        ));
    }
    let down = apps.iter().filter(|a| a.state == "down").count();
    let mut lines = vec![format!("{} applications, {down} down:", apps.len())];
    lines.extend(apps.iter().map(|app| app_line(app, now)));
    Ok(lines.join("\n"))
}

async fn down_text(state: &AppState, user: &db::User, now: i64) -> anyhow::Result<String> {
    let apps = db::list_apps(&state.db, &user.id).await?;
    let down: Vec<&db::App> = apps.iter().filter(|a| a.state == "down").collect();
    if down.is_empty() {
        return Ok("No application is down.".into());
    }
    let lines: Vec<String> = down
        .iter()
        .map(|app| {
            let seen = app.last_seen_at.map_or("never a signal".into(), |at| {
                format!("last signal {} ago", duration(now - at))
            });
            format!("{} - {seen}", app_line(app, now))
        })
        .collect();
    Ok(lines.join("\n"))
}

async fn app_picker(state: &AppState, user: &db::User, chat_id: i64) -> anyhow::Result<Message> {
    let apps = db::list_apps(&state.db, &user.id).await?;
    if apps.is_empty() {
        return Ok(Message::text(chat_id, "No applications."));
    }
    let buttons = apps
        .iter()
        .take(MAX_BUTTONS)
        .map(|app| (app.name.clone(), format!("status:{}", app.id)))
        .collect::<Vec<_>>()
        .chunks(2)
        .map(<[_]>::to_vec)
        .collect();
    Ok(Message {
        chat_id,
        text: "Which application?".into(),
        buttons,
    })
}

async fn status_text(
    state: &AppState,
    user: &db::User,
    app: &db::App,
    now: i64,
) -> anyhow::Result<String> {
    let mut lines = vec![app_line(app, now)];
    lines.push(match app.last_seen_at {
        Some(at) => format!("last signal: {} ago ({})", duration(now - at), datetime(at)),
        None => "last signal: never".into(),
    });
    lines.push(format!(
        "interval: every {}, grace {}",
        duration(app.interval_secs * 1000),
        duration(app.grace_secs * 1000)
    ));
    let recipients: Vec<String> = db::configured_recipients(&state.db, &user.id)
        .await?
        .into_iter()
        .filter(|(id, _)| *id == app.id)
        .map(|(_, email)| email)
        .collect();
    lines.push(if recipients.is_empty() {
        format!("alerts to: {}", user.email)
    } else {
        format!("alerts to: {}", recipients.join(", "))
    });
    let failures = db::failure_summary_since(&state.db, &app.id, now - DAY_MS).await?;
    lines.push(format!("invalid attempts (24h): {}", failures.total()));
    lines.push(format!("{}/apps/{}", state.public_base_url, app.id));
    Ok(lines.join("\n"))
}

async fn attempts_overview(state: &AppState, user: &db::User, now: i64) -> anyhow::Result<String> {
    let apps = db::list_apps(&state.db, &user.id).await?;
    let names: HashMap<&str, &str> = apps
        .iter()
        .map(|a| (a.id.as_str(), a.name.as_str()))
        .collect();
    let mut totals: Vec<(String, i64)> =
        db::failure_totals_for_user(&state.db, &user.id, now - DAY_MS)
            .await?
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .filter_map(|(id, n)| names.get(id.as_str()).map(|name| (name.to_string(), n)))
            .collect();
    if totals.is_empty() {
        return Ok("No invalid attempts in the last 24 hours.".into());
    }
    totals.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut lines = vec!["Invalid attempts in the last 24 hours:".to_owned()];
    lines.extend(
        totals
            .iter()
            .take(MAX_BUTTONS)
            .map(|(name, n)| format!("- {name}: {n}")),
    );
    lines.push("Details: /attempts <application name>".into());
    Ok(lines.join("\n"))
}

async fn attempts_text(state: &AppState, app: &db::App, now: i64) -> anyhow::Result<String> {
    let day = db::failure_summary_since(&state.db, &app.id, now - DAY_MS).await?;
    let week = db::failure_summary_since(&state.db, &app.id, now - 7 * DAY_MS).await?;
    let mut lines = vec![format!(
        "{}: {} invalid attempts in 24 hours, {} in 7 days",
        app.name,
        day.total(),
        week.total()
    )];
    lines.extend(text::reasons(&day));
    lines.extend(text::sources(&day));
    Ok(lines.join("\n"))
}

fn confirm_pause(chat_id: i64, app: &db::App, pause: bool) -> Message {
    let (verb, question) = if pause {
        (
            "pause",
            format!("Pause {}? No alerts until you resume it.", app.name),
        )
    } else {
        (
            "resume",
            format!(
                "Resume {}? The next heartbeat is expected within a full interval.",
                app.name
            ),
        )
    };
    Message {
        chat_id,
        text: question,
        buttons: vec![vec![
            (format!("yes, {verb}"), format!("{verb}:{}", app.id)),
            ("cancel".into(), "cancel".into()),
        ]],
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{crypto, heartbeat};

    async fn state() -> AppState {
        let pool = db::test_pool().await;
        let hb = Arc::new(
            heartbeat::Heartbeat::load(&pool, crypto::SecretBox::new(&[1; 32], 1), None)
                .await
                .unwrap(),
        );
        crate::test_state(pool, hb)
    }

    fn msg(chat_id: i64, text: &str) -> Incoming {
        Incoming {
            chat_id,
            private: true,
            label: Some("@me".into()),
            text: text.into(),
        }
    }

    async fn say(state: &AppState, chat_id: i64, text: &str) -> String {
        handle_message(state, &msg(chat_id, text), 1_000_000)
            .await
            .unwrap()
            .unwrap()
            .text
    }

    #[test]
    fn commands_are_parsed() {
        assert_eq!(
            parse_command("/status nas box"),
            Some(("status", "nas box"))
        );
        assert_eq!(parse_command(" /apps@ironheart_bot "), Some(("apps", "")));
        assert_eq!(parse_command("hello"), None);
    }

    #[tokio::test]
    async fn commands_ignore_case_and_plain_text_reenables_a_blocked_chat() {
        let state = state().await;
        let user = db::upsert_google_user(&state.db, "s", "a@example.com", None, 1)
            .await
            .unwrap();
        db::link_telegram_chat(&state.db, &user.id, 7, "@me", 1)
            .await
            .unwrap();
        assert!(
            say(&state, 7, "/Help")
                .await
                .starts_with("Linked to a@example.com.")
        );
        assert!(
            !say(&state, 7, "/APPS@ironheart_bot")
                .await
                .contains("Unknown command")
        );

        db::disable_telegram_chat(&state.db, 7, 2).await.unwrap();
        let disabled = || async {
            db::list_telegram_links(&state.db, &user.id).await.unwrap()[0]
                .disabled_at
                .is_some()
        };
        assert!(disabled().await);
        assert!(say(&state, 7, "hi").await.contains("Try /help"));
        assert!(!disabled().await, "plain text re-enables the chat");
    }

    #[test]
    fn limiter_resets_every_minute() {
        let mut limiter = Limiter::default();
        // One command per second, so the global limit does not interfere.
        assert!((0..RATE_LIMIT as i64).all(|i| limiter.allow(1, i * 1000)));
        assert!(!limiter.allow(1, 30_000));
        assert!(limiter.allow(2, 30_000), "per chat");
        assert!(limiter.allow(1, MINUTE_MS));
    }

    /// Many chats at once: at most `GLOBAL_REPLIES_PER_SEC` replies per second.
    #[test]
    fn flood_from_many_chats_is_capped() {
        let mut limiter = Limiter::default();
        let allowed = (0..1000).filter(|chat| limiter.allow(*chat, 5)).count();
        assert_eq!(allowed, GLOBAL_REPLIES_PER_SEC as usize);
        assert!(limiter.allow(5000, 1005), "next second");
    }

    #[tokio::test]
    async fn unlinked_chat_learns_nothing() {
        let state = state().await;
        let user = db::upsert_google_user(&state.db, "s", "a@example.com", None, 1)
            .await
            .unwrap();
        crate::db::insert_test_app(&state.db, "app-1", &user.id).await;
        for command in ["/apps", "/down", "/status app", "/help", "/start"] {
            let text = say(&state, 7, command).await;
            assert!(
                text.contains("not linked") && !text.contains("a@example.com"),
                "{command}: {text}"
            );
        }
        assert!(say(&state, 7, "/start bogus").await.contains("not valid"));
        let group = Incoming {
            private: false,
            ..msg(7, "/apps")
        };
        assert!(
            handle_message(&state, &group, 0)
                .await
                .unwrap()
                .unwrap()
                .text
                .contains("private chats")
        );
    }

    #[tokio::test]
    async fn link_then_commands_see_only_own_apps() {
        let state = state().await;
        let owner = db::upsert_google_user(&state.db, "s1", "a@example.com", None, 1)
            .await
            .unwrap();
        let other = db::upsert_google_user(&state.db, "s2", "b@example.com", None, 1)
            .await
            .unwrap();
        crate::db::insert_test_app(&state.db, "mine", &owner.id).await;
        crate::db::insert_test_app(&state.db, "theirs", &other.id).await;
        let update = db::AppUpdate {
            name: "Theirs",
            interval_secs: 60,
            grace_secs: 0,
            state: "new",
            next_deadline_at: None,
            recipients: None,
            now: 1,
        };
        db::update_app(&state.db, &other.id, "theirs", &update)
            .await
            .unwrap();
        db::insert_telegram_token(&state.db, &hash_token("tok"), &owner.id, 2_000_000)
            .await
            .unwrap();

        assert!(
            say(&state, 7, "/start tok")
                .await
                .starts_with("Linked to a@example.com.")
        );
        assert_eq!(
            db::list_telegram_links(&state.db, &owner.id).await.unwrap()[0].label,
            "@me"
        );
        assert!(
            say(&state, 8, "/start tok").await.contains("not valid"),
            "single use"
        );

        let apps = say(&state, 7, "/apps").await;
        assert!(
            apps.contains("[new] App") && !apps.contains("Theirs"),
            "{apps}"
        );
        assert_eq!(say(&state, 7, "/down").await, "No application is down.");
        assert!(
            say(&state, 7, "/status app")
                .await
                .contains("alerts to: a@example.com")
        );
        assert!(
            say(&state, 7, "/status theirs")
                .await
                .starts_with("No application")
        );
        let theirs = db::get_app(&state.db, &other.id, "theirs")
            .await
            .unwrap()
            .unwrap();

        // A button with someone else's app id finds nothing.
        let text = handle_callback(&state, 7, &format!("pause:{}", theirs.id), 0)
            .await
            .unwrap()
            .unwrap()
            .text;
        assert_eq!(text, "This application no longer exists.");

        assert!(say(&state, 7, "/stop").await.starts_with("Chat unlinked"));
        assert!(say(&state, 7, "/apps").await.contains("not linked"));
    }

    #[tokio::test]
    async fn pause_needs_confirmation() {
        let state = state().await;
        let owner = db::upsert_google_user(&state.db, "s1", "a@example.com", None, 1)
            .await
            .unwrap();
        crate::db::insert_test_app(&state.db, "app-1", &owner.id).await;
        db::link_telegram_chat(&state.db, &owner.id, 7, "@me", 1)
            .await
            .unwrap();
        let app = db::list_apps(&state.db, &owner.id).await.unwrap().remove(0);

        let ask = handle_message(&state, &msg(7, &format!("/pause {}", app.name)), 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ask.buttons[0][0].1, format!("pause:{}", app.id));
        assert_eq!(
            db::get_app(&state.db, &owner.id, &app.id)
                .await
                .unwrap()
                .unwrap()
                .state,
            "new",
            "not yet"
        );

        let done = handle_callback(&state, 7, &ask.buttons[0][0].1, 0)
            .await
            .unwrap()
            .unwrap();
        assert!(done.text.contains("paused"), "{}", done.text);
        assert_eq!(
            db::get_app(&state.db, &owner.id, &app.id)
                .await
                .unwrap()
                .unwrap()
                .state,
            "paused"
        );
        assert!(
            handle_callback(&state, 7, &ask.buttons[0][0].1, 0)
                .await
                .unwrap()
                .unwrap()
                .text
                .contains("already")
        );

        handle_callback(&state, 7, &format!("resume:{}", app.id), 0)
            .await
            .unwrap();
        assert_eq!(
            db::get_app(&state.db, &owner.id, &app.id)
                .await
                .unwrap()
                .unwrap()
                .state,
            "new"
        );
    }
}
