//! Moving apps between owners.
//!
//! - Immediate moves (`POST /api/apps/{id}/move`): a user moves their personal
//!   app into one of their groups; a group admin moves a group app to
//!   themselves or into another group they belong to. A plain member can put
//!   their app into a group but not take it out again.
//! - Transfer requests to another person (`POST /api/apps/{id}/transfer`):
//!   the owner, or an admin of the owning group, types an email; the answer is
//!   the same whether or not an account has it (privacy). The recipient is
//!   told by email (Telegram with email off), sees the request on /apps and
//!   accepts or rejects it within 7 days. Accepting makes it a personal app.
//!
//! The app keeps its id, key, state, recipients and history.

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
    alerts::duration,
    apps::{ApiError, MAX_APPS_PER_OWNER},
    auth::CurrentUser,
    clock::{DAY_MS, now_ms},
    db::{self, AcceptOutcome, MoveOutcome, Owner},
    mail_layout::{Block, Letter, Pulse, fg, hi},
    telegram,
};

const TRANSFER_TTL_MS: i64 = 7 * DAY_MS;
/// Transfer requests per user per day (every state, stored ones only).
const TRANSFERS_PER_DAY: i64 = 20;
/// Closed requests are kept this long, then deleted by the periodic check.
pub const CLOSED_RETENTION_MS: i64 = 30 * DAY_MS;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/apps/{id}/transfer",
            post(request_transfer).delete(cancel),
        )
        .route("/api/apps/{id}/move", post(move_app))
        .route("/api/transfers/incoming", get(incoming))
        .route("/api/transfers/{id}/accept", post(accept))
        .route("/api/transfers/{id}/reject", post(reject))
}

type ApiResult<T> = Result<T, ApiError>;

fn invalid<T>(message: impl Into<String>) -> ApiResult<T> {
    Err(ApiError::Invalid(message.into()))
}

fn owner_of(app: &db::App) -> Owner<'_> {
    match (&app.group_id, &app.user_id) {
        (Some(group), _) => Owner::Group(group),
        (None, user) => Owner::User(user.as_deref().unwrap_or_default()),
    }
}

/// An app the user can hand over: their own, or of a group where they are admin.
async fn managed_app(state: &AppState, user: &db::User, app_id: &str) -> ApiResult<db::App> {
    let app = db::get_app(&state.db, &user.id, app_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !app.manageable() {
        return Err(ApiError::Forbidden);
    }
    Ok(app)
}

#[derive(Deserialize)]
struct TransferInput {
    email: String,
}

/// The same for every address.
#[derive(Serialize)]
struct TransferView {
    to_email: String,
    expires_at: i64,
}

async fn request_transfer(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(input): Json<TransferInput>,
) -> ApiResult<(StatusCode, Json<TransferView>)> {
    let app = managed_app(&state, &user, &id).await?;
    let email = input.email.trim().to_lowercase();
    if email.parse::<lettre::Address>().is_err() {
        return invalid(format!("Invalid address: {email}"));
    }
    if email == user.email {
        return invalid("This is your own address.");
    }
    let now = now_ms();
    if db::count_transfers_since(&state.db, &user.id, now - DAY_MS).await? >= TRANSFERS_PER_DAY {
        return invalid(format!(
            "You can send at most {TRANSFERS_PER_DAY} transfer requests a day."
        ));
    }
    let expires_at = now + TRANSFER_TTL_MS;
    db::create_transfer(
        &state.db,
        &db::TransferRequest {
            id: &Uuid::now_v7().to_string(),
            app_id: &app.id,
            from: owner_of(&app),
            requested_by: &user.id,
            to_email: &email,
            now,
            expires_at,
        },
    )
    .await?;
    tracing::info!(app_id = %app.id, by = %user.id, "transfer requested");
    notify_recipient(&state, &user, &app, &email).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(TransferView {
            to_email: email,
            expires_at,
        }),
    ))
}

/// Tells the recipient, if an account has this address: by email, or with
/// email off in its linked Telegram chats. Through the queue, so the request
/// takes the same time either way; the urgent queue never drops it.
async fn notify_recipient(
    state: &AppState,
    from: &db::User,
    app: &db::App,
    to: &str,
) -> anyhow::Result<()> {
    if db::user_id_by_email(&state.db, to).await?.is_none() {
        return Ok(());
    }
    let who = from.name.clone().unwrap_or_else(|| from.email.clone());
    let apps_url = format!("{}/apps", state.public_base_url);
    let every = duration(app.interval_secs * 1000);
    if state.mailer.is_enabled() {
        let email = Letter {
            title: "Transfer request".into(),
            hot: false,
            pulse: Pulse::Beat,
            blocks: vec![
                Block::Para(vec![
                    hi(who.clone()),
                    fg(" asks you to take over "),
                    hi(app.name.clone()),
                    fg(format!(" (every {every}).")),
                ]),
                Block::text(
                    "If you accept, it becomes one of your applications, with the same key: its clients keep working without changes.",
                ),
                Block::text("Accept or reject it on your dashboard. The request expires in 7 days."),
            ],
            action: Some(("open the dashboard".into(), apps_url)),
        }
        .into_email(to, format!("{who} wants to give you {}", app.name));
        state.outbox.urgent(email).await;
    } else if state.telegram.is_active() {
        let text = telegram::text::transfer_request(&who, &app.name, app.interval_secs, &apps_url);
        for chat_id in db::telegram_chats_for_email(&state.db, to).await? {
            state
                .outbox
                .urgent(telegram::Message::text(chat_id, text.clone()))
                .await;
        }
    }
    Ok(())
}

async fn cancel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    managed_app(&state, &user, &id).await?;
    if !db::cancel_transfer(&state.db, &id, now_ms()).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct MoveInput {
    to: MoveTarget,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum MoveTarget {
    /// Only the user themselves: giving it to someone else is a transfer request.
    User {
        id: Option<String>,
    },
    Group {
        id: String,
    },
}

async fn move_app(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(input): Json<MoveInput>,
) -> ApiResult<StatusCode> {
    let app = db::get_app(&state.db, &user.id, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let from = owner_of(&app);
    let to = match &input.to {
        MoveTarget::User { id } => {
            if id.as_deref().is_some_and(|id| id != user.id) {
                return invalid("To give it to someone else, send a transfer request.");
            }
            Owner::User(&user.id)
        }
        MoveTarget::Group { id } => {
            if db::member_role(&state.db, id, &user.id).await?.is_none() {
                return invalid("You can move it only into your groups.");
            }
            Owner::Group(id)
        }
    };
    if from == to {
        return invalid("It already belongs there.");
    }
    // Out of a group: group admins only. Into a group: their own apps, any member.
    if matches!(from, Owner::Group(_)) && !app.manageable() {
        return Err(ApiError::Forbidden);
    }
    match db::move_app(&state.db, &id, from, to, MAX_APPS_PER_OWNER, now_ms()).await? {
        MoveOutcome::Moved => {}
        MoveOutcome::Full => {
            return invalid(format!(
                "The new owner already has {MAX_APPS_PER_OWNER} applications."
            ));
        }
        MoveOutcome::Stale => return invalid("The application changed owner meanwhile: reload."),
    }
    tracing::info!(app_id = %id, by = %user.id, to = ?to, "application moved");
    Ok(StatusCode::NO_CONTENT)
}

async fn incoming(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<db::IncomingTransfer>>> {
    Ok(Json(
        db::incoming_transfers(&state.db, &user.email, now_ms()).await?,
    ))
}

async fn accept(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    match db::accept_transfer(
        &state.db,
        &id,
        &user.id,
        &user.email,
        MAX_APPS_PER_OWNER,
        now_ms(),
    )
    .await?
    {
        AcceptOutcome::Accepted { app_id } => {
            tracing::info!(%app_id, to = %user.id, "transfer accepted");
            Ok(StatusCode::NO_CONTENT)
        }
        AcceptOutcome::NotFound => Err(ApiError::NotFound),
        AcceptOutcome::Stale => {
            invalid("The application changed owner meanwhile: the request is no longer valid.")
        }
        AcceptOutcome::Full => invalid(format!(
            "You already have {MAX_APPS_PER_OWNER} personal applications."
        )),
    }
}

async fn reject(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let rejected = db::reject_transfer(&state.db, &id, &user.email, now_ms())
        .await?
        .ok_or(ApiError::NotFound)?;
    if let Some(sender) = &rejected.requested_by {
        notify_rejection(&state, &user, &rejected.app_name, sender).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Tells the sender that the request was rejected (email, or Telegram with email off).
async fn notify_rejection(
    state: &AppState,
    by: &db::User,
    app_name: &str,
    to: &str,
) -> anyhow::Result<()> {
    let who = by.name.clone().unwrap_or_else(|| by.email.clone());
    let apps_url = format!("{}/apps", state.public_base_url);
    if state.mailer.is_enabled() {
        let email = Letter {
            title: "Transfer rejected".into(),
            hot: false,
            pulse: Pulse::Beat,
            blocks: vec![Block::Para(vec![
                hi(who.clone()),
                fg(" rejected the transfer of "),
                hi(app_name.to_owned()),
                fg(": it stays where it was."),
            ])],
            action: Some(("open the dashboard".into(), apps_url)),
        }
        .into_email(to, format!("{who} rejected {app_name}"));
        state.outbox.urgent(email).await;
    } else if state.telegram.is_active() {
        let text = telegram::text::transfer_rejected(&who, app_name, &apps_url);
        for chat_id in db::telegram_chats_for_email(&state.db, to).await? {
            state
                .outbox
                .urgent(telegram::Message::text(chat_id, text.clone()))
                .await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{crypto, heartbeat};

    async fn setup() -> (AppState, db::User, db::User) {
        let pool = db::test_pool().await;
        let hb = Arc::new(
            heartbeat::Heartbeat::load(&pool, crypto::SecretBox::new(&[1; 32], 1), None)
                .await
                .unwrap(),
        );
        let state = crate::test_state(pool, hb);
        let admin = db::upsert_google_user(&state.db, "a", "admin@example.com", None, 1)
            .await
            .unwrap();
        let member = db::upsert_google_user(&state.db, "m", "member@example.com", None, 1)
            .await
            .unwrap();
        db::create_group(&state.db, "g", "Ops", "ops", &admin.id, &admin.id, 1)
            .await
            .unwrap();
        db::invite_to_group(
            &state.db,
            "g",
            "member@example.com",
            "member",
            &admin.id,
            1,
            i64::MAX,
        )
        .await
        .unwrap();
        db::accept_group_invite(&state.db, "g", &member.id, 2, 100)
            .await
            .unwrap();
        (state, admin, member)
    }

    fn to_group() -> Json<MoveInput> {
        Json(MoveInput {
            to: MoveTarget::Group { id: "g".into() },
        })
    }

    fn to_me() -> Json<MoveInput> {
        Json(MoveInput {
            to: MoveTarget::User { id: None },
        })
    }

    /// A plain member puts their app into the group but cannot take it back;
    /// the group admin can.
    #[tokio::test]
    async fn members_move_in_group_admins_move_out() {
        let (state, admin, member) = setup().await;
        db::insert_test_app(&state.db, "a", &member.id).await;
        let mv = |user: &db::User, input: Json<MoveInput>| {
            move_app(
                State(state.clone()),
                CurrentUser(user.clone()),
                Path("a".to_owned()),
                input,
            )
        };
        assert!(matches!(
            mv(&member, to_group()).await,
            Ok(StatusCode::NO_CONTENT)
        ));
        assert!(matches!(
            mv(&member, to_me()).await,
            Err(ApiError::Forbidden)
        ));
        assert!(matches!(
            mv(&admin, to_group()).await,
            Err(ApiError::Invalid(_))
        ));
        assert!(matches!(
            mv(&admin, to_me()).await,
            Ok(StatusCode::NO_CONTENT)
        ));
        assert!(
            db::get_app(&state.db, &admin.id, "a")
                .await
                .unwrap()
                .unwrap()
                .user_id
                .is_some()
        );
    }

    /// The answer to a transfer request is the same whether or not the address
    /// has an account; a plain member of the owning group cannot send one.
    #[tokio::test]
    async fn transfer_requests_answer_the_same_for_every_address() {
        let (state, admin, member) = setup().await;
        db::insert_test_app(&state.db, "a", &admin.id).await;
        let send = |user: &db::User, email: &str| {
            request_transfer(
                State(state.clone()),
                CurrentUser(user.clone()),
                Path("a".to_owned()),
                Json(TransferInput {
                    email: email.into(),
                }),
            )
        };
        let (status, Json(known)) = send(&admin, "member@example.com").await.ok().unwrap();
        let (_, Json(unknown)) = send(&admin, "Nobody@Example.com").await.ok().unwrap();
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(
            (known.to_email.as_str(), unknown.to_email.as_str()),
            ("member@example.com", "nobody@example.com")
        );
        assert!(matches!(
            send(&admin, "admin@example.com").await,
            Err(ApiError::Invalid(_))
        ));

        db::move_app(
            &state.db,
            "a",
            Owner::User(&admin.id),
            Owner::Group("g"),
            100,
            3,
        )
        .await
        .unwrap();
        assert!(matches!(
            send(&member, "x@example.com").await,
            Err(ApiError::Forbidden)
        ));
    }
}
