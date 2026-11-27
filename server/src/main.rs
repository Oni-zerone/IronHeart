mod alerts;
mod apps;
mod auth;
mod clock;
mod config;
mod crypto;
mod db;
mod entanglement;
mod error;
mod heartbeat;
mod mail;
mod monitor;
mod users;

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

#[derive(Clone)]
pub struct AppState {
    db: db::Pool,
    auth: auth::Auth,
    mailer: mail::Mailer,
    heartbeat: Arc<heartbeat::Heartbeat>,
    public_base_url: Arc<str>,
    admin_email: Arc<str>,
    entanglement: Arc<entanglement::Sender>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `.env` in sviluppo; le variabili già presenti nell'ambiente hanno la precedenza.
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
    let auth = auth::Auth::discover(&config).await?;
    let mailer = mail::Mailer::new(&config.mail)?;
    tracing::info!("invio email con {}", mailer.describe());
    let secret_box = crypto::SecretBox::new(&config.secrets_key, config.secrets_key_version);
    let heartbeat =
        Arc::new(heartbeat::Heartbeat::load(&db, secret_box, config.client_ip_header.clone()).await?);
    monitor::reschedule_overdue(&db, clock::now_ms()).await?;

    let state = AppState {
        db,
        auth,
        mailer,
        heartbeat,
        public_base_url: config.public_base_url.as_str().into(),
        admin_email: config.admin_email.as_str().into(),
        entanglement: Arc::new(entanglement::Sender::new()?),
    };
    monitor::spawn(state.clone());
    entanglement::spawn(state.clone());

    let app = Router::new()
        .route("/", get(home))
        .route("/health", get(health))
        .route("/api/me", get(me))
        .route("/api/me/test-email", post(send_test_email))
        .route(crypto::HEARTBEAT_PATH, post(heartbeat::handler))
        .merge(auth::routes())
        .merge(apps::routes())
        .merge(users::routes())
        .merge(entanglement::routes())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!("in ascolto su {} ({})", config.bind_addr, config.public_base_url);
    // L'indirizzo del peer serve all'heartbeat quando non c'è un proxy davanti.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> StatusCode {
    match db::ping(&state.db).await {
        Ok(()) => StatusCode::OK,
        Err(err) => {
            tracing::error!(%err, "health: database non raggiungibile");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

#[derive(serde::Serialize)]
struct Me {
    #[serde(flatten)]
    user: db::User,
    is_master: bool,
}

async fn me(State(state): State<AppState>, auth::CurrentUser(user): auth::CurrentUser) -> Json<Me> {
    let is_master = user.email == state.admin_email.as_ref();
    Json(Me { user, is_master })
}

/// Manda una mail di prova all'email dell'account, per verificare la configurazione.
async fn send_test_email(
    State(state): State<AppState>,
    auth::CurrentUser(user): auth::CurrentUser,
) -> StatusCode {
    let email = mail::Email {
        to: user.email,
        subject: "IronHeart: email di prova".into(),
        text: "Se leggi questo messaggio, IronHeart riesce a mandarti gli avvisi.".into(),
        html: "<p>Se leggi questo messaggio, IronHeart riesce a mandarti gli avvisi.</p>".into(),
    };
    match state.mailer.send(&email).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(err) => {
            tracing::error!(err = ?err, user_id = %user.id, "email di prova non inviata");
            StatusCode::BAD_GATEWAY
        }
    }
}

/// Pagina provvisoria per provare il login finché non c'è la dashboard React.
async fn home(user: Option<auth::CurrentUser>) -> impl IntoResponse {
    let body = match user {
        Some(auth::CurrentUser(user)) => format!(
            "<p>Accesso eseguito come <b>{}</b>.</p>\
             <form method=post action=/api/me/test-email><button>Mandami una mail di prova</button></form>\
             <form method=post action=/auth/logout><button>Esci</button></form>",
            html_escape(&user.email)
        ),
        None => "<p><a href=/auth/google/login>Accedi con Google</a></p>".to_owned(),
    };
    Html(format!("<!doctype html><meta charset=utf-8><title>IronHeart</title><h1>IronHeart</h1>{body}"))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Cartella del file SQLite, da creare prima della connessione.
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
