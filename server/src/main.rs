mod auth;
mod clock;
mod config;
mod db;
mod error;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    db: db::Pool,
    auth: auth::Auth,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `.env` in sviluppo; le variabili già presenti nell'ambiente hanno la precedenza.
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .init();

    let config = config::Config::from_env()?;
    if let Some(dir) = sqlite_dir(&config.database_url) {
        std::fs::create_dir_all(dir)?;
    }
    let db = db::connect(&config.database_url).await?;
    let auth = auth::Auth::discover(&config).await?;

    let app = Router::new()
        .route("/", get(home))
        .route("/health", get(health))
        .route("/api/me", get(me))
        .merge(auth::routes())
        .layer(TraceLayer::new_for_http())
        .with_state(AppState { db, auth });

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!("in ascolto su {} ({})", config.bind_addr, config.public_base_url);
    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;
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

async fn me(auth::CurrentUser(user): auth::CurrentUser) -> Json<db::User> {
    Json(user)
}

/// Pagina provvisoria per provare il login finché non c'è la dashboard React.
async fn home(user: Option<auth::CurrentUser>) -> impl IntoResponse {
    let body = match user {
        Some(auth::CurrentUser(user)) => format!(
            "<p>Accesso eseguito come <b>{}</b>.</p>\
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
