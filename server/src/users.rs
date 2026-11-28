//! Administration API: users and invites (admins only).
//!
//! The master (`ADMIN_EMAIL`) cannot be demoted or removed, and an admin
//! cannot demote or remove themselves.

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
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
    mail_layout::{Block, Letter, Pulse, fg, hi},
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
    groups: Vec<GroupTag>,
}

#[derive(Serialize)]
struct InviteView {
    #[serde(flatten)]
    invite: db::Invite,
    /// Groups joined at the first sign-in.
    groups: Vec<GroupTag>,
}

/// A group and the role in it, for the users table.
#[derive(Serialize)]
struct GroupTag {
    name: String,
    role: String,
}

#[derive(Serialize)]
struct Overview {
    users: Vec<UserView>,
    invites: Vec<InviteView>,
}

fn group_tags(rows: Vec<(String, String, String)>) -> HashMap<String, Vec<GroupTag>> {
    let mut tags: HashMap<String, Vec<GroupTag>> = HashMap::new();
    for (key, name, role) in rows {
        tags.entry(key).or_default().push(GroupTag { name, role });
    }
    tags
}

async fn overview(state: &AppState) -> ApiResult<Overview> {
    let mut memberships = group_tags(db::all_memberships(&state.db).await?);
    let mut invite_groups = group_tags(db::invite_groups(&state.db).await?);
    let users = db::list_users(&state.db)
        .await?
        .into_iter()
        .map(|user| UserView {
            is_master: user.email == state.admin_email.as_ref(),
            groups: memberships.remove(&user.id).unwrap_or_default(),
            user,
        })
        .collect();
    let invites = db::list_invites(&state.db)
        .await?
        .into_iter()
        .map(|invite| InviteView {
            groups: invite_groups.remove(&invite.email).unwrap_or_default(),
            invite,
        })
        .collect();
    Ok(Overview { users, invites })
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
        return Err(ApiError::Invalid(
            "This is the master user: they already have admin access.".into(),
        ));
    }
    match db::create_invite(&state.db, &email, &input.role, Some(&admin.id), now_ms()).await? {
        InviteOutcome::AlreadyUser => {
            return Err(ApiError::Invalid(
                "This address already has an account.".into(),
            ));
        }
        InviteOutcome::Created => send_invite_email(&state, &admin, &email, None).await,
        InviteOutcome::Updated => {}
    }
    Ok((StatusCode::CREATED, Json(overview(&state).await?)))
}

/// Notice to the invitee. If sending fails, the invite stays valid. With email
/// off the admin tells the invitee the instance's address. `group`: name and
/// role of the group joined at the first sign-in.
pub async fn send_invite_email(
    state: &AppState,
    admin: &db::User,
    to: &str,
    group: Option<(&str, &str)>,
) {
    if !state.mailer.is_enabled() {
        return;
    }
    let who = admin.name.clone().unwrap_or_else(|| admin.email.clone());
    let url = format!("{}/", state.public_base_url);
    let mut blocks = vec![Block::Para(vec![
        hi(who),
        fg(" invited you to IronHeart, the service that monitors application heartbeats."),
    ])];
    if let Some((name, role)) = group {
        blocks.push(Block::Para(vec![
            fg("At your first sign-in you join the group "),
            hi(name),
            fg(format!(
                " as {}: no further step needed.",
                crate::groups::role_label(role)
            )),
        ]));
    }
    blocks.push(Block::text(
        "Sign in with the Google account for this address.",
    ));
    let email = Letter {
        title: "Invitation".into(),
        hot: false,
        pulse: Pulse::Beat,
        blocks,
        action: Some(("sign in".into(), url)),
    }
    .into_email(to, "Invitation to IronHeart".into());
    if let Err(err) = state.mailer.send(&email).await {
        tracing::error!(err = ?err, "invite email not sent");
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

#[derive(Deserialize)]
struct RemoveQuery {
    /// Group that takes the user's personal apps; absent = they are deleted.
    move_apps_to: Option<String>,
}

async fn remove(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<String>,
    Query(query): Query<RemoveQuery>,
) -> ApiResult<Json<Overview>> {
    protect(&state, &admin, &id).await?;
    // A group left without members would keep apps that nobody sees or is alerted for.
    let sole = db::sole_member_groups(&state.db, &id).await?;
    if let Some((_, name, _)) = sole.iter().find(|(_, _, apps)| *apps > 0) {
        return Err(ApiError::Invalid(format!(
            "They are the only member of {name}, which owns applications: add another member first."
        )));
    }
    if let Some(group_id) = &query.move_apps_to {
        if sole.iter().any(|(gid, _, _)| gid == group_id) {
            return Err(ApiError::Invalid(
                "They are the only member of that group: choose a group with other members.".into(),
            ));
        }
        if db::get_group(&state.db, group_id).await?.is_none() {
            return Err(ApiError::Invalid("That group does not exist.".into()));
        }
        let moving = db::count_owned_apps(&state.db, db::Owner::User(&id)).await?;
        let there = db::count_owned_apps(&state.db, db::Owner::Group(group_id)).await?;
        if moving + there > crate::apps::MAX_APPS_PER_OWNER {
            return Err(ApiError::Invalid(format!(
                "The group has room for {} more applications, not {moving}.",
                (crate::apps::MAX_APPS_PER_OWNER - there).max(0)
            )));
        }
    }
    let app_ids = db::delete_user(&state.db, &id, query.move_apps_to.as_deref(), now_ms())
        .await?
        .ok_or(ApiError::NotFound)?;
    for app_id in &app_ids {
        state.heartbeat.remove_app(app_id);
    }
    tracing::info!(user_id = %id, apps = app_ids.len(), "user removed");
    Ok(Json(overview(&state).await?))
}

/// The master and the admin themselves are off limits.
async fn protect(state: &AppState, admin: &db::User, target_id: &str) -> ApiResult<()> {
    let target = db::find_user(&state.db, target_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if target.email == state.admin_email.as_ref() {
        return Err(ApiError::Invalid(
            "The master user cannot be changed.".into(),
        ));
    }
    if target.id == admin.id {
        return Err(ApiError::Invalid(
            "You cannot change your own account.".into(),
        ));
    }
    Ok(())
}

fn validate_role(role: &str) -> ApiResult<()> {
    match role {
        "admin" | "user" => Ok(()),
        _ => Err(ApiError::Invalid("Invalid role.".into())),
    }
}
