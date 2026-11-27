mod alerts;
mod apps;
mod auth;
mod channels;
mod clock;
mod config;
mod crypto;
mod db;
mod entanglement;
mod error;
mod heartbeat;
#[cfg(test)]
mod load_tests;
#[cfg(test)]
mod db_bench;
mod mail;
mod mail_check;
mod mail_layout;
mod monitor;
mod outbox;
mod telegram;
mod users;
mod web;

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

/// The Docker image is a static musl binary: musl's allocator halves the heartbeat
/// throughput under concurrency, mimalloc restores it.
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Clone)]
pub struct AppState {
    db: db::Pool,
    auth: auth::Auth,
    mailer: mail::Mailer,
    /// Alert queue: the periodic check does not wait for sends.
    outbox: Arc<outbox::Outbox>,
    heartbeat: Arc<heartbeat::Heartbeat>,
    public_base_url: Arc<str>,
    admin_email: Arc<str>,
    entanglement: Arc<entanglement::Sender>,
    /// Database in use, shown to the master.
    database: Arc<db::DatabaseInfo>,
    /// Outcome of the daily check of the email key, shown to the master.
    mail_key: Arc<mail_check::KeyMonitor>,
    /// Telegram bot: active only with a token that `getMe` accepted.
    telegram: Arc<telegram::Telegram>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `.env` in development; variables already in the environment take precedence.
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = config::Config::from_env()?;
    if let Some(dir) = sqlite_dir(&config.database_url) {
        std::fs::create_dir_all(dir)?;
    }
    let db = db::connect(&config.database_url).await?;
    let database = Arc::new(db::DatabaseInfo::from_url(&config.database_url));
    tracing::info!("database: {database}");
    let auth = auth::Auth::discover(&config).await?;
    let mailer = mail::Mailer::new(&config.mail)?;
    tracing::info!("sending email with {}", mailer.describe());
    let secret_box = crypto::SecretBox::new(&config.secrets_key, config.secrets_key_version);
    let heartbeat =
        Arc::new(heartbeat::Heartbeat::load(&db, secret_box, config.client_ip_header.clone()).await?);
    monitor::reschedule_overdue(&db, clock::now_ms()).await?;
    let telegram = Arc::new(telegram::Telegram::new(config.telegram_bot_token.as_deref(), &config.telegram_api_url)?);
    let mail_key = Arc::new(mail_check::KeyMonitor::new(mailer.has_key()));
    if !mailer.is_enabled() {
        tracing::info!("email off (MAIL_TRANSPORT=none): alerts only on Telegram");
    }

    let state = AppState {
        outbox: outbox::Outbox::start(outbox::Senders {
            mailer: mailer.clone(),
            mail_key: mail_key.clone(),
            telegram: telegram.clone(),
            db: db.clone(),
        }),
        telegram,
        db,
        auth,
        mail_key,
        mailer,
        heartbeat,
        public_base_url: config.public_base_url.as_str().into(),
        admin_email: config.admin_email.as_str().into(),
        entanglement: Arc::new(entanglement::Sender::new()?),
        database,
    };
    monitor::spawn(state.clone());
    mail_check::spawn(state.clone());
    heartbeat::spawn_writer(state.clone());
    entanglement::spawn(state.clone());
    telegram::spawn(state.clone());
    channels::spawn(state.clone());
    let shutdown_state = state.clone();

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/me", get(me))
        .route("/api/me/test-email", post(send_test_email))
        .route(crypto::HEARTBEAT_PATH, post(heartbeat::handler))
        .merge(auth::routes())
        .merge(apps::routes())
        .merge(users::routes())
        .merge(entanglement::routes())
        .merge(telegram::api::routes());
    let app = if web::available() {
        app.fallback(web::serve)
    } else {
        tracing::warn!("dashboard not built (web/dist): serving the temporary page");
        app.route("/", get(home))
    };
    let app = app
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!("listening on {} ({})", config.bind_addr, config.public_base_url);
    // The heartbeat needs the peer address when there is no proxy in front.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    // The valid pings of the last second must not be lost.
    let flushed = shutdown_state.heartbeat.flush_pings(&shutdown_state.db).await?;
    tracing::info!("stopped (pings written at shutdown: {flushed})");
    Ok(())
}

/// Full state for tests: database and registry, no Google, email to a closed port.
#[cfg(test)]
pub fn test_state(db: db::Pool, heartbeat: Arc<heartbeat::Heartbeat>) -> AppState {
    let mail = config::MailConfig {
        from_email: "test@ironheart.local".into(),
        from_name: "IronHeart".into(),
        transport: config::MailTransport::Smtp(config::SmtpConfig {
            host: "127.0.0.1".into(),
            port: 9,
            tls: config::SmtpTls::None,
            credentials: None,
        }),
    };
    let mailer = mail::Mailer::new(&mail).unwrap();
    let telegram = Arc::new(telegram::Telegram::new(None, telegram::DEFAULT_API_URL).unwrap());
    let mail_key = Arc::new(mail_check::KeyMonitor::new(false));
    AppState {
        outbox: outbox::Outbox::start(outbox::Senders {
            mailer: mailer.clone(),
            mail_key: mail_key.clone(),
            telegram: telegram.clone(),
            db: db.clone(),
        }),
        telegram,
        db,
        auth: auth::Auth::for_tests(),
        mailer,
        heartbeat,
        public_base_url: "http://localhost".into(),
        admin_email: "master@example.com".into(),
        entanglement: Arc::new(entanglement::Sender::new().unwrap()),
        database: Arc::new(db::DatabaseInfo::from_url("sqlite::memory:")),
        mail_key,
    }
}

async fn health(State(state): State<AppState>) -> StatusCode {
    match db::ping(&state.db).await {
        Ok(()) => StatusCode::OK,
        Err(err) => {
            tracing::error!(%err, "health: database unreachable");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

#[derive(serde::Serialize)]
struct Me {
    #[serde(flatten)]
    user: db::User,
    is_master: bool,
    /// Master only: which database the instance runs on.
    #[serde(skip_serializing_if = "Option::is_none")]
    database: Option<db::DatabaseInfo>,
    /// Master only: last check of the email key (absent without a key).
    #[serde(skip_serializing_if = "Option::is_none")]
    mail_key: Option<mail_check::KeyStatus>,
    /// Present only while the Telegram bot is active: linking and commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    telegram: Option<TelegramInfo>,
    /// Alert channels and their state, for the main page.
    alerts: channels::Status,
}

#[derive(serde::Serialize)]
struct TelegramInfo {
    bot_username: String,
}

async fn me(
    State(state): State<AppState>,
    auth::CurrentUser(user): auth::CurrentUser,
) -> error::AppResult<Json<Me>> {
    let is_master = user.email == state.admin_email.as_ref();
    let alerts = channels::status(&state, &user, is_master).await?;
    let database = is_master.then(|| (*state.database).clone());
    let mail_key = if is_master { state.mail_key.get() } else { None };
    let telegram = state.telegram.bot_username().map(|bot_username| TelegramInfo { bot_username });
    Ok(Json(Me { user, is_master, database, mail_key, telegram, alerts }))
}

/// Sends a test email to the account's address, to check the configuration.
async fn send_test_email(
    State(state): State<AppState>,
    auth::CurrentUser(user): auth::CurrentUser,
) -> StatusCode {
    if !state.mailer.is_enabled() {
        return StatusCode::NOT_FOUND;
    }
    let email = mail_layout::Letter {
        title: "Test email".into(),
        hot: false,
        pulse: mail_layout::Pulse::Beat,
        blocks: vec![mail_layout::Block::text("If you are reading this message, IronHeart can send you alerts.")],
        action: Some(("open IronHeart".into(), format!("{}/", state.public_base_url))),
    }
    .into_email(&user.email, "IronHeart: test email".into());
    match state.mailer.send(&email).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(err) => {
            tracing::error!(err = ?err, user_id = %user.id, "test email not sent");
            StatusCode::BAD_GATEWAY
        }
    }
}

/// Temporary page to try the login when the dashboard is not built.
async fn home(user: Option<auth::CurrentUser>) -> impl IntoResponse {
    let body = match user {
        Some(auth::CurrentUser(user)) => format!(
            "<p>Signed in as <b>{}</b>.</p>\
             <form method=post action=/api/me/test-email><button>Send me a test email</button></form>\
             <form method=post action=/auth/logout><button>Sign out</button></form>",
            html_escape(&user.email)
        ),
        None => "<p><a href=/auth/google/login>Sign in with Google</a></p>".to_owned(),
    };
    Html(format!("<!doctype html><meta charset=utf-8><title>IronHeart</title><h1>IronHeart</h1>{body}"))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Folder of the SQLite file, to be created before connecting.
fn sqlite_dir(url: &str) -> Option<&std::path::Path> {
    let path = url.strip_prefix("sqlite://").or_else(|| url.strip_prefix("sqlite:"))?;
    let path = path.split('?').next()?;
    std::path::Path::new(path).parent().filter(|p| !p.as_os_str().is_empty())
}

async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.expect("ctrl-c") };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The database in use appears in `/api/me` only for the master.
    #[tokio::test]
    async fn me_shows_database_and_mail_key_only_to_master() {
        let pool = db::test_pool().await;
        let secret_box = crypto::SecretBox::new(&[1; 32], 1);
        let hb = Arc::new(heartbeat::Heartbeat::load(&pool, secret_box, None).await.unwrap());
        let state = test_state(pool, hb);
        let user = |email: &str| db::User { id: "u".into(), email: email.into(), name: None, role: "admin".into() };
        let json = |me: &Me| serde_json::to_value(me).unwrap();

        // Before the first check (or without a key) there is no key status.
        let Json(master) = me(State(state.clone()), auth::CurrentUser(user("master@example.com"))).await.ok().unwrap();
        assert!(json(&master).get("mail_key").is_none());
        state.mail_key.set(mail_check::KeyStatus { provider: "Brevo", state: "rejected", checked_at: 1, error: Some("401".into()), failing_since: Some(1) });

        let Json(master) = me(State(state.clone()), auth::CurrentUser(user("master@example.com"))).await.ok().unwrap();
        assert_eq!(json(&master)["database"]["backend"], "sqlite");
        assert_eq!(json(&master)["mail_key"]["state"], "rejected");

        let Json(admin) = me(State(state), auth::CurrentUser(user("admin@example.com"))).await.ok().unwrap();
        assert!(json(&admin).get("database").is_none());
        assert!(json(&admin).get("mail_key").is_none());
    }

    /// Telegram appears in `/api/me` only while the bot is active.
    #[tokio::test]
    async fn me_shows_telegram_only_when_bot_is_active() {
        let pool = db::test_pool().await;
        let secret_box = crypto::SecretBox::new(&[1; 32], 1);
        let hb = Arc::new(heartbeat::Heartbeat::load(&pool, secret_box, None).await.unwrap());
        let mut state = test_state(pool, hb);
        let user = || auth::CurrentUser(db::User { id: "u".into(), email: "a@example.com".into(), name: None, role: "user".into() });

        let Json(off) = me(State(state.clone()), user()).await.ok().unwrap();
        assert!(serde_json::to_value(&off).unwrap().get("telegram").is_none());

        state.telegram = Arc::new(telegram::Telegram::for_tests("ironheart_bot"));
        let Json(on) = me(State(state), user()).await.ok().unwrap();
        assert_eq!(serde_json::to_value(&on).unwrap()["telegram"]["bot_username"], "ironheart_bot");
    }
}
