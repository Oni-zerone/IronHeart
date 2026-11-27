//! Entanglement: outgoing heartbeats to other IronHeart instances, so several
//! instances monitor each other. Only the master user configures it.
//!
//! The other instance always answers 204: from here we only know whether the ping
//! was delivered. If the key is wrong the other instance reports it, in its
//! invalid-attempt summaries.
//!
//! While this instance's Brevo API key is refused (`mail_check`) the pings are
//! held back: the other instances see this one go down and alert in its place.

use std::{collections::HashMap, sync::Mutex, time::Duration};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState,
    apps::ApiError,
    auth::MasterUser,
    clock::now_ms,
    crypto::{self, HEARTBEAT_PATH},
    db::{self, DuePing, EntanglementInput},
    heartbeat::{KEY_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER},
};

const TICK: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const INTERVAL_RANGE: std::ops::RangeInclusive<i64> = 10..=86_400;

pub struct Sender {
    http: reqwest::Client,
    /// Last signed timestamp per link: it must grow with every ping.
    last_ts: Mutex<HashMap<String, i64>>,
}

impl Sender {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                // The signed path is fixed: a redirect must not be followed.
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            last_ts: Mutex::default(),
        })
    }

    /// Timestamp in seconds, always greater than the previous one for the same
    /// link (at most 1 second ahead: within the ±5 minute tolerance).
    fn next_timestamp(&self, id: &str, now_secs: i64) -> i64 {
        let mut last = self.last_ts.lock().expect("timestamp lock");
        let ts = last.get(id).map_or(now_secs, |prev| now_secs.max(prev + 1));
        last.insert(id.to_owned(), ts);
        ts
    }
}

fn secret_aad(id: &str) -> String {
    format!("entanglement:{id}")
}

/// Sends a signed ping. `None` if it was delivered (204), otherwise the error.
async fn send(state: &AppState, target: &DuePing) -> Option<String> {
    let secret = match state.heartbeat.secret_box().open(
        &target.secret_enc,
        target.secret_key_version,
        &secret_aad(&target.id),
    ) {
        Ok(secret) => secret,
        Err(err) => {
            tracing::error!(id = %target.id, err = ?err, "entanglement secret cannot be decrypted");
            return Some("Secret cannot be decrypted on this instance".into());
        }
    };
    let ts = state.entanglement.next_timestamp(&target.id, now_ms() / 1000).to_string();
    let canonical = crypto::canonical_string(&target.key_id, &ts, &crypto::sha256_hex(b""));
    let signature = format!("v1={}", hex::encode(crypto::sign(&secret, &canonical)));

    let result = state
        .entanglement
        .http
        .post(format!("{}{HEARTBEAT_PATH}", target.base_url))
        .header(KEY_HEADER, &target.key_id)
        .header(TIMESTAMP_HEADER, ts)
        .header(SIGNATURE_HEADER, signature)
        .send()
        .await;
    match result {
        Ok(response) if response.status() == StatusCode::NO_CONTENT => None,
        Ok(response) => Some(format!("HTTP response {}", response.status().as_u16())),
        Err(err) if err.is_timeout() => Some("Timed out".into()),
        Err(err) if err.is_connect() => Some("Connection failed".into()),
        Err(err) => Some(format!("Request failed: {}", err.without_url()).chars().take(200).collect()),
    }
}

/// Recorded instead of sending while the Brevo key is refused.
pub const HELD_ERROR: &str = "Held: Brevo refused this instance's API key, alerts cannot be sent";

async fn send_and_record(state: &AppState, target: &DuePing) -> anyhow::Result<()> {
    // Without working email this instance cannot alert anyone: stopping the
    // pings lets the other instances see it go down and alert in its place.
    // Not logged per ping: `mail_check` already logged the refusal.
    let error = if state.mail_key.rejected() {
        Some(HELD_ERROR.to_owned())
    } else {
        let error = send(state, target).await;
        if let Some(error) = &error {
            tracing::warn!(id = %target.id, %error, "entanglement: ping not delivered");
        }
        error
    };
    db::record_ping_result(&state.db, &target.id, now_ms(), error.as_deref()).await?;
    Ok(())
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        // Pings stop if the Brevo key is refused: know that before the first one.
        state.mail_key.first_check().await;
        if let Err(err) = db::ping_all_now(&state.db, now_ms()).await {
            tracing::error!(err = ?err, "entanglement: startup failed");
        }
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let due = match db::take_due_pings(&state.db, now_ms()).await {
                Ok(due) => due,
                Err(err) => {
                    tracing::error!(err = ?err, "entanglement: read failed");
                    continue;
                }
            };
            for target in due {
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(err) = send_and_record(&state, &target).await {
                        tracing::error!(id = %target.id, err = ?err, "entanglement: outcome not saved");
                    }
                });
            }
        }
    });
}

// ---- API (master only) -----------------------------------------------------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/entanglements", get(list).post(create))
        .route("/api/entanglements/{id}", axum::routing::patch(update).delete(remove))
        .route("/api/entanglements/{id}/ping", post(ping_now))
}

type ApiResult<T> = Result<T, ApiError>;

fn invalid<T>(message: impl Into<String>) -> ApiResult<T> {
    Err(ApiError::Invalid(message.into()))
}

#[derive(Serialize)]
struct Overview {
    /// Address of this instance, to use on the other one for the reverse link.
    instance_url: String,
    items: Vec<db::Entanglement>,
}

async fn list(State(state): State<AppState>, _: MasterUser) -> ApiResult<Json<Overview>> {
    Ok(Json(Overview {
        instance_url: state.public_base_url.to_string(),
        items: db::list_entanglements(&state.db).await?,
    }))
}

#[derive(Deserialize)]
struct CreateInput {
    name: String,
    base_url: String,
    key_id: String,
    secret: String,
    interval_secs: i64,
    #[serde(default = "enabled_default")]
    enabled: bool,
}

fn enabled_default() -> bool {
    true
}

async fn create(
    State(state): State<AppState>,
    _: MasterUser,
    Json(input): Json<CreateInput>,
) -> ApiResult<(StatusCode, Json<db::Entanglement>)> {
    let name = validate_name(&input.name)?;
    let base_url = normalize_base_url(&input.base_url)?;
    validate_interval(input.interval_secs)?;
    let (key_id, secret) = validate_credentials(&input.key_id, &input.secret)?;

    let id = Uuid::now_v7().to_string();
    let secret_box = state.heartbeat.secret_box();
    db::insert_entanglement(
        &state.db,
        &id,
        &EntanglementInput { name: &name, base_url: &base_url, interval_secs: input.interval_secs, enabled: input.enabled },
        &key_id,
        &secret_box.seal(secret.as_bytes(), &secret_aad(&id)),
        secret_box.version,
        now_ms(),
    )
    .await?;
    let created = db::get_entanglement(&state.db, &id).await?.ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(created)))
}

#[derive(Deserialize)]
struct UpdateInput {
    name: Option<String>,
    base_url: Option<String>,
    interval_secs: Option<i64>,
    enabled: Option<bool>,
    /// To be provided together, to replace the key.
    key_id: Option<String>,
    secret: Option<String>,
}

async fn update(
    State(state): State<AppState>,
    _: MasterUser,
    Path(id): Path<String>,
    Json(input): Json<UpdateInput>,
) -> ApiResult<Json<db::Entanglement>> {
    let current = db::get_entanglement(&state.db, &id).await?.ok_or(ApiError::NotFound)?;
    let name = match &input.name {
        Some(name) => validate_name(name)?,
        None => current.name,
    };
    let base_url = match &input.base_url {
        Some(url) => normalize_base_url(url)?,
        None => current.base_url,
    };
    let interval_secs = input.interval_secs.unwrap_or(current.interval_secs);
    validate_interval(interval_secs)?;
    let credentials = match (&input.key_id, &input.secret) {
        (None, None) => None,
        (Some(key_id), Some(secret)) => Some(validate_credentials(key_id, secret)?),
        _ => return invalid("To change the key, provide both the key ID and the secret."),
    };

    let secret_box = state.heartbeat.secret_box();
    let sealed = credentials
        .as_ref()
        .map(|(key_id, secret)| (key_id.as_str(), secret_box.seal(secret.as_bytes(), &secret_aad(&id))));
    db::update_entanglement(
        &state.db,
        &id,
        &EntanglementInput {
            name: &name,
            base_url: &base_url,
            interval_secs,
            enabled: input.enabled.unwrap_or(current.enabled.0),
        },
        sealed.as_ref().map(|(key_id, enc)| (*key_id, enc.as_slice(), secret_box.version)),
        now_ms(),
    )
    .await?;
    Ok(Json(db::get_entanglement(&state.db, &id).await?.ok_or(ApiError::NotFound)?))
}

async fn remove(
    State(state): State<AppState>,
    _: MasterUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    if !db::delete_entanglement(&state.db, &id).await? {
        return Err(ApiError::NotFound);
    }
    state.entanglement.last_ts.lock().expect("timestamp lock").remove(&id);
    Ok(StatusCode::NO_CONTENT)
}

/// Sends a ping right away and returns the outcome.
async fn ping_now(
    State(state): State<AppState>,
    _: MasterUser,
    Path(id): Path<String>,
) -> ApiResult<Json<db::Entanglement>> {
    let target = db::ping_target(&state.db, &id).await?.ok_or(ApiError::NotFound)?;
    send_and_record(&state, &target).await?;
    Ok(Json(db::get_entanglement(&state.db, &id).await?.ok_or(ApiError::NotFound)?))
}

fn validate_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return invalid("The name must be 1 to 80 characters long.");
    }
    Ok(name.to_owned())
}

fn validate_interval(interval_secs: i64) -> ApiResult<()> {
    if !INTERVAL_RANGE.contains(&interval_secs) {
        return invalid("The interval must be between 10 seconds and 1 day.");
    }
    Ok(())
}

fn validate_credentials(key_id: &str, secret: &str) -> ApiResult<(String, String)> {
    let (key_id, secret) = (key_id.trim(), secret.trim());
    if key_id.is_empty() || key_id.len() > 64 || !key_id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return invalid("Invalid key ID.");
    }
    if secret.is_empty() || secret.len() > 200 || !secret.is_ascii() {
        return invalid("Invalid secret.");
    }
    Ok((key_id.to_owned(), secret.to_owned()))
}

/// Only the origin (scheme, host, port). HTTPS, or HTTP to localhost for development.
fn normalize_base_url(raw: &str) -> ApiResult<String> {
    let Ok(url) = reqwest::Url::parse(raw.trim()) else {
        return invalid("Invalid address: use for example https://heartbeat.example.com");
    };
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        _ => return invalid("The address must use HTTPS (HTTP only for localhost)."),
    }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return invalid("The address cannot contain credentials, a query or a fragment.");
    }
    if !matches!(url.path(), "" | "/") {
        return invalid("Enter only the instance address, without a path.");
    }
    Ok(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_reduced_to_origin() {
        let ok = |raw| normalize_base_url(raw).ok().unwrap();
        assert_eq!(ok("https://hb.example.com/"), "https://hb.example.com");
        assert_eq!(ok(" https://hb.example.com:8443 "), "https://hb.example.com:8443");
        assert_eq!(ok("http://localhost:8081"), "http://localhost:8081");
        for bad in ["http://hb.example.com", "ftp://x", "https://u:p@hb.example.com", "https://hb.example.com/api", "https://hb.example.com/?a=1", "nope"] {
            assert!(normalize_base_url(bad).is_err(), "{bad}");
        }
    }

    /// With the Brevo key refused nothing is sent: the link records the hold.
    #[tokio::test]
    async fn pings_are_held_while_the_mail_key_is_rejected() {
        use std::sync::Arc;

        use crate::{heartbeat, mail_check::{KeyStatus, REJECTED}};

        let pool = db::test_pool().await;
        let secret_box = crypto::SecretBox::new(&[1; 32], 1);
        let hb = Arc::new(heartbeat::Heartbeat::load(&pool, secret_box, None).await.unwrap());
        let state = crate::test_state(pool, hb);
        let id = Uuid::now_v7().to_string();
        let secret_enc = state.heartbeat.secret_box().seal(b"ihs_secret", &secret_aad(&id));
        // Closed port: a real send would fail with "Connection failed".
        let input = EntanglementInput { name: "peer", base_url: "http://127.0.0.1:9", interval_secs: 60, enabled: true };
        db::insert_entanglement(&state.db, &id, &input, "abcdefghijkl", &secret_enc, 1, now_ms()).await.unwrap();
        let target = db::ping_target(&state.db, &id).await.unwrap().unwrap();
        let last_error = || async { db::get_entanglement(&state.db, &id).await.unwrap().unwrap().last_error };

        state.mail_key.set(KeyStatus { state: REJECTED, checked_at: 1, error: None });
        send_and_record(&state, &target).await.unwrap();
        assert_eq!(last_error().await.as_deref(), Some(HELD_ERROR));

        state.mail_key.set(KeyStatus { state: "ok", checked_at: 2, error: None });
        send_and_record(&state, &target).await.unwrap();
        assert_eq!(last_error().await.as_deref(), Some("Connection failed"));
    }

    #[test]
    fn timestamps_always_increase_per_link() {
        let sender = Sender::new().unwrap();
        assert_eq!(sender.next_timestamp("a", 100), 100);
        assert_eq!(sender.next_timestamp("a", 100), 101);
        assert_eq!(sender.next_timestamp("a", 100), 102);
        assert_eq!(sender.next_timestamp("a", 200), 200);
        assert_eq!(sender.next_timestamp("b", 100), 100);
    }
}
