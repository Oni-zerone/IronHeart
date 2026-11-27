//! API di amministrazione: utenti e inviti (solo admin).
//!
//! Il master (`ADMIN_EMAIL`) non si può declassare né rimuovere, e un admin
//! non può declassare né rimuovere se stesso.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch},
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    apps::ApiError,
    auth::AdminUser,
    clock::now_ms,
    db::{self, InviteOutcome},
    mail::Email,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(list).post(invite))
        .route("/api/admin/users/{id}", patch(update).delete(remove))
        .route("/api/admin/invites/{email}", delete(revoke_invite))
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Serialize)]
struct UserView {
    #[serde(flatten)]
    user: db::UserSummary,
    is_master: bool,
}

#[derive(Serialize)]
struct Overview {
    users: Vec<UserView>,
    invites: Vec<db::Invite>,
}

async fn overview(state: &AppState) -> ApiResult<Overview> {
    let users = db::list_users(&state.db)
        .await?
        .into_iter()
        .map(|user| UserView { is_master: user.email == state.admin_email.as_ref(), user })
        .collect();
    Ok(Overview { users, invites: db::list_invites(&state.db).await? })
}

async fn list(State(state): State<AppState>, _: AdminUser) -> ApiResult<Json<Overview>> {
    Ok(Json(overview(&state).await?))
}

#[derive(Deserialize)]
struct InviteInput {
    email: String,
    role: String,
}

async fn invite(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Json(input): Json<InviteInput>,
) -> ApiResult<(StatusCode, Json<Overview>)> {
    let email = input.email.trim().to_lowercase();
    if email.parse::<lettre::Address>().is_err() {
        return Err(ApiError::Invalid(format!("Invalid address: {email}")));
    }
    validate_role(&input.role)?;
    if email == state.admin_email.as_ref() {
        return Err(ApiError::Invalid("This is the master user: they already have admin access.".into()));
    }
    match db::create_invite(&state.db, &email, &input.role, Some(&admin.id), now_ms()).await? {
        InviteOutcome::AlreadyUser => {
            return Err(ApiError::Invalid("This address already has an account.".into()));
        }
        InviteOutcome::Created => send_invite_email(&state, &admin, &email).await,
        InviteOutcome::Updated => {}
    }
    Ok((StatusCode::CREATED, Json(overview(&state).await?)))
}

/// Avviso all'invitato. Se l'invio fallisce, l'invito resta valido.
async fn send_invite_email(state: &AppState, admin: &db::User, to: &str) {
    let who = admin.name.clone().unwrap_or_else(|| admin.email.clone());
    let url = format!("{}/", state.public_base_url);
    let text = format!(
        "{who} ti ha invitato su IronHeart, il servizio che controlla l'heartbeat delle applicazioni.\n\n\
         Accedi con l'account Google di questo indirizzo: {url}\n"
    );
    let html = format!(
        "<p>{} ti ha invitato su IronHeart, il servizio che controlla l'heartbeat delle applicazioni.</p>\
         <p><a href=\"{url}\">Accedi</a> con l'account Google di questo indirizzo.</p>",
        who.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    );
    let email = Email { to: to.to_owned(), subject: "Invito a IronHeart".into(), text, html };
    if let Err(err) = state.mailer.send(&email).await {
        tracing::error!(err = ?err, "mail di invito non inviata");
    }
}

async fn revoke_invite(
    State(state): State<AppState>,
    _: AdminUser,
    Path(email): Path<String>,
) -> ApiResult<Json<Overview>> {
    if !db::delete_invite(&state.db, &email.to_lowercase()).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(overview(&state).await?))
}

#[derive(Deserialize)]
struct RoleInput {
    role: String,
}

async fn update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<String>,
    Json(input): Json<RoleInput>,
) -> ApiResult<Json<Overview>> {
    validate_role(&input.role)?;
    protect(&state, &admin, &id).await?;
    db::set_user_role(&state.db, &id, &input.role).await?;
    Ok(Json(overview(&state).await?))
}

async fn remove(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Overview>> {
    protect(&state, &admin, &id).await?;
    let app_ids = db::delete_user(&state.db, &id).await?.ok_or(ApiError::NotFound)?;
    for app_id in &app_ids {
        state.heartbeat.remove_app(app_id);
    }
    tracing::info!(user_id = %id, apps = app_ids.len(), "utente rimosso");
    Ok(Json(overview(&state).await?))
}

/// Il master e l'admin stesso non si toccano.
async fn protect(state: &AppState, admin: &db::User, target_id: &str) -> ApiResult<()> {
    let target = db::find_user(&state.db, target_id).await?.ok_or(ApiError::NotFound)?;
    if target.email == state.admin_email.as_ref() {
        return Err(ApiError::Invalid("The master user cannot be changed.".into()));
    }
    if target.id == admin.id {
        return Err(ApiError::Invalid("You cannot change your own account.".into()));
    }
    Ok(())
}

fn validate_role(role: &str) -> ApiResult<()> {
    match role {
        "admin" | "user" => Ok(()),
        _ => Err(ApiError::Invalid("Invalid role.".into())),
    }
}
