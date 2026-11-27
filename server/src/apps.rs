//! API della dashboard per le app dell'utente collegato.

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    clock::{DAY_MS, now_ms},
    crypto, db,
};

const MAX_APPS_PER_USER: i64 = 100;
const MAX_RECIPIENTS: usize = 10;
const MAX_NAME_CHARS: usize = 80;
const INTERVAL_RANGE: std::ops::RangeInclusive<i64> = 30..=7 * 86_400;
const GRACE_RANGE: std::ops::RangeInclusive<i64> = 0..=86_400;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/apps", get(list).post(create))
        .route("/api/apps/{id}", get(detail).patch(update).delete(remove))
        .route("/api/apps/{id}/rotate-key", post(rotate_key))
}

pub enum ApiError {
    NotFound,
    Invalid(String),
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND.into_response(),
            Self::Invalid(message) => {
                (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({ "error": message })))
                    .into_response()
            }
            Self::Internal(err) => crate::error::AppError::from(err).into_response(),
        }
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn invalid<T>(message: impl Into<String>) -> ApiResult<T> {
    Err(ApiError::Invalid(message.into()))
}

#[derive(Serialize)]
struct AppView {
    #[serde(flatten)]
    app: db::App,
    /// Destinatari configurati; vuoto = email dell'account.
    recipients: Vec<String>,
    failures_24h: i64,
}

#[derive(Serialize)]
struct AppDetail {
    #[serde(flatten)]
    view: AppView,
    failures: Vec<db::FailureBucket>,
}

#[derive(Serialize)]
struct Credentials {
    key_id: String,
    secret: String,
}

#[derive(Serialize)]
struct WithCredentials<T: Serialize> {
    app: T,
    /// Mostrato una volta sola: nel database il segreto è cifrato.
    credentials: Credentials,
}

async fn list(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Vec<AppView>>> {
    let apps = db::list_apps(&state.db, &user.id).await?;
    let mut recipients: HashMap<String, Vec<String>> = HashMap::new();
    for (app_id, email) in db::configured_recipients(&state.db, &user.id).await? {
        recipients.entry(app_id).or_default().push(email);
    }
    let failures: HashMap<String, i64> =
        db::failure_totals_for_user(&state.db, &user.id, now_ms() - DAY_MS).await?.into_iter().collect();
    Ok(Json(
        apps.into_iter()
            .map(|app| AppView {
                recipients: recipients.remove(&app.id).unwrap_or_default(),
                failures_24h: failures.get(&app.id).copied().unwrap_or(0),
                app,
            })
            .collect(),
    ))
}

async fn load_view(state: &AppState, user_id: &str, app_id: &str) -> ApiResult<AppDetail> {
    let app = db::get_app(&state.db, user_id, app_id).await?.ok_or(ApiError::NotFound)?;
    let recipients = db::configured_recipients(&state.db, user_id)
        .await?
        .into_iter()
        .filter(|(id, _)| id == app_id)
        .map(|(_, email)| email)
        .collect();
    let failures = db::failure_buckets_since(&state.db, app_id, now_ms() - DAY_MS).await?;
    let failures_24h = failures.iter().map(|b| b.counts.total()).sum();
    Ok(AppDetail { view: AppView { app, recipients, failures_24h }, failures })
}

async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<AppDetail>> {
    Ok(Json(load_view(&state, &user.id, &id).await?))
}

#[derive(Deserialize)]
struct CreateApp {
    name: String,
    interval_secs: i64,
    #[serde(default)]
    grace_secs: i64,
    #[serde(default)]
    recipients: Vec<String>,
}

async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(input): Json<CreateApp>,
) -> ApiResult<(StatusCode, Json<WithCredentials<AppDetail>>)> {
    let name = validate_name(&input.name)?;
    validate_timing(input.interval_secs, input.grace_secs)?;
    let recipients = normalize_recipients(&input.recipients)?;
    if db::count_apps(&state.db, &user.id).await? >= MAX_APPS_PER_USER {
        return invalid(format!("You can register at most {MAX_APPS_PER_USER} applications."));
    }

    let id = Uuid::now_v7().to_string();
    let key_id = crypto::generate_key_id();
    let secret = crypto::generate_secret();
    let secret_box = state.heartbeat.secret_box();
    db::insert_app(
        &state.db,
        &db::NewApp {
            id: &id,
            user_id: &user.id,
            name: &name,
            key_id: &key_id,
            secret_enc: &secret_box.seal(secret.as_bytes(), &id),
            secret_key_version: secret_box.version,
            interval_secs: input.interval_secs,
            grace_secs: input.grace_secs,
            now: now_ms(),
        },
        &recipients,
    )
    .await?;
    state.heartbeat.refresh_app(&state.db, &id).await?;

    let app = load_view(&state, &user.id, &id).await?;
    Ok((StatusCode::CREATED, Json(WithCredentials { app, credentials: Credentials { key_id, secret } })))
}

#[derive(Deserialize)]
struct PatchApp {
    name: Option<String>,
    interval_secs: Option<i64>,
    grace_secs: Option<i64>,
    paused: Option<bool>,
    recipients: Option<Vec<String>>,
}

async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(input): Json<PatchApp>,
) -> ApiResult<Json<AppDetail>> {
    let app = db::get_app(&state.db, &user.id, &id).await?.ok_or(ApiError::NotFound)?;
    let name = match &input.name {
        Some(name) => validate_name(name)?,
        None => app.name.clone(),
    };
    let interval_secs = input.interval_secs.unwrap_or(app.interval_secs);
    let grace_secs = input.grace_secs.unwrap_or(app.grace_secs);
    validate_timing(interval_secs, grace_secs)?;
    let recipients = input.recipients.as_deref().map(normalize_recipients).transpose()?;

    let now = now_ms();
    let window_ms = (interval_secs + grace_secs) * 1000;
    let (app_state, next_deadline_at) = match (input.paused, app.state.as_str()) {
        (Some(true), _) => ("paused", None),
        // Ripresa: si aspetta un intervallo intero da adesso.
        (Some(false), "paused") => match app.last_seen_at {
            Some(_) => ("up", Some(now + window_ms)),
            None => ("new", None),
        },
        // Intervallo cambiato su un'app attiva: la scadenza parte dall'ultimo segnale.
        (_, "up") => ("up", app.last_seen_at.map(|seen| seen + window_ms).or(app.next_deadline_at)),
        (_, other) => (other, app.next_deadline_at),
    };

    db::update_app(
        &state.db,
        &user.id,
        &id,
        &db::AppUpdate {
            name: &name,
            interval_secs,
            grace_secs,
            state: app_state,
            next_deadline_at,
            recipients: recipients.as_deref(),
            now,
        },
    )
    .await?;
    state.heartbeat.set_timing(&id, interval_secs, grace_secs);
    Ok(Json(load_view(&state, &user.id, &id).await?))
}

async fn rotate_key(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<WithCredentials<AppDetail>>> {
    let key_id = crypto::generate_key_id();
    let secret = crypto::generate_secret();
    let secret_box = state.heartbeat.secret_box();
    let rotated = db::rotate_key(
        &state.db,
        &user.id,
        &id,
        &key_id,
        &secret_box.seal(secret.as_bytes(), &id),
        secret_box.version,
        now_ms(),
    )
    .await?;
    if !rotated {
        return Err(ApiError::NotFound);
    }
    state.heartbeat.refresh_app(&state.db, &id).await?;
    let app = load_view(&state, &user.id, &id).await?;
    Ok(Json(WithCredentials { app, credentials: Credentials { key_id, secret } }))
}

async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    db::delete_app(&state.db, &user.id, &id).await?.ok_or(ApiError::NotFound)?;
    state.heartbeat.remove_app(&id);
    Ok(StatusCode::NO_CONTENT)
}

fn validate_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return invalid(format!("The name must be 1 to {MAX_NAME_CHARS} characters long."));
    }
    Ok(name.to_owned())
}

fn validate_timing(interval_secs: i64, grace_secs: i64) -> ApiResult<()> {
    if !INTERVAL_RANGE.contains(&interval_secs) {
        return invalid("The interval must be between 30 seconds and 7 days.");
    }
    if !GRACE_RANGE.contains(&grace_secs) {
        return invalid("The grace period must be between 0 seconds and 1 day.");
    }
    Ok(())
}

fn normalize_recipients(input: &[String]) -> ApiResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for raw in input {
        let email = raw.trim().to_lowercase();
        if email.is_empty() {
            continue;
        }
        if email.parse::<lettre::Address>().is_err() {
            return invalid(format!("Invalid address: {email}"));
        }
        if !out.contains(&email) {
            out.push(email);
        }
    }
    if out.len() > MAX_RECIPIENTS {
        return invalid(format!("At most {MAX_RECIPIENTS} recipients."));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipients_are_normalized() {
        let input = vec![" Ops@Example.com ".into(), "ops@example.com".into(), "".into(), "dev@example.com".into()];
        assert_eq!(normalize_recipients(&input).ok().unwrap(), ["ops@example.com", "dev@example.com"]);
        assert!(normalize_recipients(&["non-una-mail".into()]).is_err());
        let many: Vec<String> = (0..11).map(|i| format!("u{i}@example.com")).collect();
        assert!(normalize_recipients(&many).is_err());
    }

    #[test]
    fn timing_limits() {
        assert!(validate_timing(30, 0).is_ok());
        assert!(validate_timing(29, 0).is_err());
        assert!(validate_timing(60, 86_401).is_err());
    }
}
