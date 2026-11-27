//! Alert channels of the instance: email and Telegram.
//!
//! - The mode comes from the configuration: `mail+telegram`, `mail` (no
//!   Telegram token) or `telegram` (`MAIL_TRANSPORT=none`).
//! - A channel is broken when its key is refused, or when it has been
//!   unreachable for more than an hour (`mail_check::Health`).
//! - When a channel breaks, the master is told once on the other channel; again
//!   when it works again.
//! - The entanglement pings are held when no channel can reach the master and at
//!   least one is broken: the linked instances then report this one as down and
//!   alert in its place. Telegram reaches the master only through a linked chat.
//!   SMTP without login cannot be checked: it never holds the pings by itself,
//!   but it does not count as reaching the master either (the notice is still
//!   attempted by email).

use std::time::Duration;

use serde::Serialize;

use crate::{
    AppState,
    clock::now_ms,
    db,
    mail_check::Health,
    mail_layout::{Block, Letter, Pulse},
    telegram,
};

const TICK: Duration = Duration::from_secs(60);

/// Health of each configured channel (`None` = not configured), and how many
/// active Telegram chats the master has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reach {
    pub mail: Option<Health>,
    pub telegram: Option<Health>,
    pub master_chats: usize,
}

/// Not broken: checked and working, briefly unreachable, or not checked yet.
fn usable(health: Health) -> bool {
    matches!(health, Health::Ok | Health::Unreachable | Health::Checking)
}

impl Reach {
    fn mail_reaches_master(&self) -> bool {
        self.mail.is_some_and(usable)
    }

    fn telegram_reaches_master(&self) -> bool {
        self.telegram.is_some_and(usable) && self.master_chats > 0
    }

    fn broken(health: Option<Health>) -> bool {
        health == Some(Health::Broken)
    }

    /// No channel reaches the master, and one of them is broken.
    pub fn held(&self) -> bool {
        let reached = self.mail_reaches_master() || self.telegram_reaches_master();
        let broken = Self::broken(self.mail) || Self::broken(self.telegram);
        !reached && broken
    }
}

pub async fn reach(state: &AppState, now: i64) -> anyhow::Result<Reach> {
    let mail = state.mailer.is_enabled().then(|| state.mail_key.health(now));
    let telegram = state.telegram.is_configured().then(|| state.telegram.health.health(now));
    let master_chats = match telegram {
        Some(_) => db::telegram_chats_for_email(&state.db, &state.admin_email).await?.len(),
        None => 0,
    };
    Ok(Reach { mail, telegram, master_chats })
}

/// The entanglement pings must not go out: see the module comment.
pub async fn held(state: &AppState) -> anyhow::Result<bool> {
    Ok(reach(state, now_ms()).await?.held())
}

/// What every user sees on the main page.
#[derive(Debug, Serialize)]
pub struct Status {
    /// `mail+telegram`, `mail` or `telegram`.
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mail: Option<Health>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telegram: Option<Health>,
    /// The user has at least one active Telegram chat.
    pub telegram_linked: bool,
    /// Master only: the entanglement pings are held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held: Option<bool>,
}

pub async fn status(state: &AppState, user: &db::User, is_master: bool) -> anyhow::Result<Status> {
    let reach = reach(state, now_ms()).await?;
    let mode = match (reach.mail.is_some(), reach.telegram.is_some()) {
        (true, true) => "mail+telegram",
        (false, true) => "telegram",
        _ => "mail",
    };
    let telegram_linked = reach.telegram.is_some() && !db::telegram_chats_for_user(&state.db, &user.id).await?.is_empty();
    Ok(Status {
        mode,
        mail: reach.mail,
        telegram: reach.telegram,
        telegram_linked,
        held: is_master.then(|| reach.held()),
    })
}

/// Which channel changed, for the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Mail,
    Telegram,
}

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Self::Mail => "Email",
            Self::Telegram => "Telegram",
        }
    }
}

/// Broken channels at the previous round, in memory: after a restart with a
/// channel still broken, the notice goes out once more.
#[derive(Default)]
struct Memory {
    mail_broken: bool,
    telegram_broken: bool,
    held: bool,
}

/// Notices for the master: `(channel, now broken?)` for every change.
fn changes(memory: &mut Memory, reach: &Reach) -> Vec<(Channel, bool)> {
    let mut out = Vec::new();
    let mail_broken = Reach::broken(reach.mail);
    let telegram_broken = Reach::broken(reach.telegram);
    if mail_broken != memory.mail_broken {
        out.push((Channel::Mail, mail_broken));
    }
    if telegram_broken != memory.telegram_broken {
        out.push((Channel::Telegram, telegram_broken));
    }
    memory.mail_broken = mail_broken;
    memory.telegram_broken = telegram_broken;
    out
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        // The first evaluation waits for the first checks, so a working
        // channel is not reported as broken at startup.
        state.mail_key.first_check().await;
        state.telegram.health.first_check().await;
        let mut memory = Memory::default();
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(err) = round(&state, &mut memory).await {
                tracing::error!(err = ?err, "alert channels: check failed");
            }
        }
    });
}

async fn round(state: &AppState, memory: &mut Memory) -> anyhow::Result<()> {
    let reach = reach(state, now_ms()).await?;
    let held = reach.held();
    if held != memory.held {
        if held {
            tracing::error!("no alert channel reaches the master: entanglement pings are held");
        } else {
            tracing::info!("an alert channel reaches the master again: entanglement pings resume");
        }
        memory.held = held;
    }
    for (channel, broken) in changes(memory, &reach) {
        notify_master(state, &reach, channel, broken).await?;
    }
    Ok(())
}

/// Tells the master on every channel that is not broken (SMTP without login
/// included: it may work).
async fn notify_master(state: &AppState, reach: &Reach, channel: Channel, broken: bool) -> anyhow::Result<()> {
    let detail = match channel {
        Channel::Mail => state.mail_key.get(),
        Channel::Telegram => state.telegram.health.get(),
    }
    .and_then(|s| s.error)
    .unwrap_or_default();
    let name = channel.name();
    let mut lines = vec![if broken {
        format!("{name} stopped working on IronHeart: {detail}")
    } else {
        format!("{name} works again on IronHeart.")
    }];
    if broken {
        lines.push(if reach.held() {
            "No alert channel reaches you: the entanglement pings are held, so the linked instances report this one as down.".to_owned()
        } else {
            format!("Alerts keep going out on the other channel. Fix the {name} configuration and restart.")
        });
    }
    let url = format!("{}/", state.public_base_url);
    tracing::warn!(channel = name, broken, "alert channel changed: notifying the master");

    if channel != Channel::Mail && reach.mail.is_some_and(|h| h != Health::Broken) {
        let email = Letter {
            title: if broken { format!("{name} broken") } else { format!("{name} back") },
            hot: broken,
            pulse: if broken { Pulse::Flat } else { Pulse::Beat },
            blocks: lines.iter().map(|l| Block::text(l.clone())).collect(),
            action: Some(("open IronHeart".into(), url.clone())),
        }
        .into_email(
            &state.admin_email,
            format!("IronHeart: {name} {}", if broken { "stopped working" } else { "works again" }),
        );
        state.outbox.urgent(email).await;
    }
    if channel != Channel::Telegram && reach.telegram.is_some_and(|h| h != Health::Broken) {
        let text = format!("!! {}\n{url}", lines.join("\n"));
        for chat_id in db::telegram_chats_for_email(&state.db, &state.admin_email).await? {
            state.outbox.urgent(telegram::Message::text(chat_id, text.clone())).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use Health::*;

    fn reach(mail: Option<Health>, telegram: Option<Health>, master_chats: usize) -> Reach {
        Reach { mail, telegram, master_chats }
    }

    #[test]
    fn pings_are_held_only_when_nothing_reaches_the_master() {
        // Both working, or one of them broken while the other reaches the master.
        assert!(!reach(Some(Ok), Some(Ok), 1).held());
        assert!(!reach(Some(Ok), Some(Broken), 1).held(), "email still works");
        assert!(!reach(Some(Broken), Some(Ok), 1).held(), "telegram still works");
        // Email broken and the master has no chat: nothing reaches the master.
        assert!(reach(Some(Broken), Some(Ok), 0).held());
        assert!(reach(Some(Broken), Some(Broken), 1).held());
        // Only one channel configured.
        assert!(reach(Some(Broken), None, 0).held());
        assert!(reach(None, Some(Broken), 1).held());
        assert!(!reach(None, Some(Ok), 0).held(), "nothing broken: no hold");
        // A short outage or the first check is not a failure.
        assert!(!reach(Some(Unreachable), Some(Checking), 0).held());
        // SMTP without login: unknown, it does not hold by itself...
        assert!(!reach(Some(Unverified), None, 0).held());
        assert!(!reach(Some(Unverified), Some(Ok), 0).held());
        // ...but it does not keep the pings going when Telegram breaks.
        assert!(reach(Some(Unverified), Some(Broken), 1).held());
    }

    #[test]
    fn notices_only_on_changes() {
        let mut memory = Memory::default();
        assert!(changes(&mut memory, &reach(Some(Ok), Some(Ok), 1)).is_empty());
        assert_eq!(changes(&mut memory, &reach(Some(Ok), Some(Broken), 1)), [(Channel::Telegram, true)]);
        assert!(changes(&mut memory, &reach(Some(Ok), Some(Broken), 1)).is_empty(), "once");
        assert_eq!(
            changes(&mut memory, &reach(Some(Broken), Some(Ok), 1)),
            [(Channel::Mail, true), (Channel::Telegram, false)]
        );
    }
}
