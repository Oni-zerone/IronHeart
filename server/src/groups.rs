//! Groups API: groups of users that share applications.
//!
//! - Instance admins create and delete groups and can manage any group.
//! - Group admins manage their group: invitations, roles, members, name.
//! - Joining is always an invitation the person accepts. Invitations reach only
//!   existing accounts and the API answers the same whatever the address
//!   (privacy): an unknown address is dropped silently. Exception: an instance
//!   admin who is also admin of the group turns an unknown address into an
//!   instance invite with the group attached (joined at the first sign-in).
//! - Group admins never see pending invitations: members appear once they accept.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState,
    apps::ApiError,
    auth::{AdminUser, CurrentUser},
    clock::{DAY_MS, now_ms},
    db::{self, GroupWrite, InviteOutcome, InviteTarget, JoinOutcome, MemberChange},
    mail_layout::{Block, Letter, Pulse, fg, hi},
};

pub const MAX_GROUPS: i64 = 50;
pub const MAX_MEMBERS: i64 = db::MAX_GROUP_MEMBERS;
const MAX_NAME_CHARS: usize = 80;
const INVITE_TTL_MS: i64 = 7 * DAY_MS;
/// Invitations per group admin per day, counting the dropped ones too, so the
/// limit does not tell which addresses have an account.
const INVITES_PER_DAY: usize = 50;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/groups", get(list))
        .route("/api/admin/groups", post(create))
        .route("/api/admin/groups/{id}", delete(remove))
        .route("/api/groups/{id}", get(detail).patch(rename))
        .route("/api/groups/{id}/invites", post(invite))
        .route(
            "/api/groups/{id}/members/{user_id}",
            patch(set_role).delete(remove_member),
        )
        .route("/api/groups/{id}/leave", post(leave))
        .route("/api/me/group-invites", get(my_invites))
        .route("/api/me/group-invites/{id}/accept", post(accept))
        .route("/api/me/group-invites/{id}/reject", post(reject))
}

type ApiResult<T> = Result<T, ApiError>;

fn invalid<T>(message: impl Into<String>) -> ApiResult<T> {
    Err(ApiError::Invalid(message.into()))
}

/// Requests per key in a sliding window of one day, in memory (one instance;
/// a restart resets the counts).
#[derive(Default)]
pub struct DailyLimit {
    seen: Mutex<HashMap<String, VecDeque<i64>>>,
}

impl DailyLimit {
    /// Records a request of `key` at `now`; `false` if the limit is reached.
    pub fn allow(&self, key: &str, now: i64, limit: usize) -> bool {
        let mut seen = self.seen.lock().expect("limit lock");
        seen.retain(|_, times| times.back().is_some_and(|&t| now - t < DAY_MS));
        let times = seen.entry(key.to_owned()).or_default();
        while times.front().is_some_and(|&t| now - t >= DAY_MS) {
            times.pop_front();
        }
        if times.len() >= limit {
            return false;
        }
        times.push_back(now);
        true
    }
}

/// What the signed-in user can do in a group.
struct Access {
    group: db::Group,
    role: Option<String>,
    /// Group admin, or instance admin.
    manage: bool,
}

/// Members and instance admins see the group; everyone else gets 404.
async fn access(state: &AppState, user: &db::User, group_id: &str) -> ApiResult<Access> {
    let group = db::get_group(&state.db, group_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let role = db::member_role(&state.db, group_id, &user.id).await?;
    if role.is_none() && !user.is_admin() {
        return Err(ApiError::NotFound);
    }
    let manage = role.as_deref() == Some("admin") || user.is_admin();
    Ok(Access {
        group,
        role,
        manage,
    })
}

async fn manager(state: &AppState, user: &db::User, group_id: &str) -> ApiResult<Access> {
    let access = access(state, user, group_id).await?;
    if !access.manage {
        return Err(ApiError::Forbidden);
    }
    Ok(access)
}

#[derive(Serialize)]
struct GroupList {
    /// The user's groups; for instance admins every group (`role` null where
    /// they are not a member).
    groups: Vec<db::GroupSummary>,
}

async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<GroupList>> {
    let groups = if user.is_admin() {
        db::list_all_groups(&state.db, &user.id).await?
    } else {
        db::list_user_groups(&state.db, &user.id).await?
    };
    Ok(Json(GroupList { groups }))
}

#[derive(Serialize)]
struct GroupDetail {
    #[serde(flatten)]
    group: db::Group,
    /// Role of the signed-in user; null for an instance admin who is not a member.
    role: Option<String>,
    can_manage: bool,
    members: Vec<db::Member>,
}

async fn load_detail(state: &AppState, access: Access) -> ApiResult<GroupDetail> {
    let members = db::list_members(&state.db, &access.group.id).await?;
    Ok(GroupDetail {
        group: access.group,
        role: access.role,
        can_manage: access.manage,
        members,
    })
}

async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<GroupDetail>> {
    let access = access(&state, &user, &id).await?;
    Ok(Json(load_detail(&state, access).await?))
}

#[derive(Deserialize)]
struct CreateGroup {
    name: String,
    /// First group admin: an existing account.
    admin_id: String,
}

async fn create(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Json(input): Json<CreateGroup>,
) -> ApiResult<(StatusCode, Json<GroupDetail>)> {
    let (name, key) = validate_name(&input.name)?;
    if db::count_groups(&state.db).await? >= MAX_GROUPS {
        return invalid(format!(
            "This instance can have at most {MAX_GROUPS} groups."
        ));
    }
    let id = Uuid::now_v7().to_string();
    match db::create_group(
        &state.db,
        &id,
        &name,
        &key,
        &admin.id,
        &input.admin_id,
        now_ms(),
    )
    .await?
    {
        GroupWrite::Done => {}
        GroupWrite::NameTaken => return invalid("Another group already has this name."),
        GroupWrite::NotFound => return invalid("The first group admin must be an existing user."),
    }
    tracing::info!(group_id = %id, by = %admin.id, "group created");
    let access = access(&state, &admin, &id).await?;
    Ok((
        StatusCode::CREATED,
        Json(load_detail(&state, access).await?),
    ))
}

#[derive(Deserialize)]
struct RenameGroup {
    name: String,
}

async fn rename(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(input): Json<RenameGroup>,
) -> ApiResult<Json<GroupDetail>> {
    manager(&state, &user, &id).await?;
    let (name, key) = validate_name(&input.name)?;
    match db::rename_group(&state.db, &id, &name, &key, now_ms()).await? {
        GroupWrite::Done => {}
        GroupWrite::NameTaken => return invalid("Another group already has this name."),
        GroupWrite::NotFound => return Err(ApiError::NotFound),
    }
    let access = access(&state, &user, &id).await?;
    Ok(Json(load_detail(&state, access).await?))
}

async fn remove(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    if !db::delete_group(&state.db, &id).await? {
        return Err(ApiError::NotFound);
    }
    tracing::info!(group_id = %id, by = %admin.id, "group deleted");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct InviteInput {
    email: String,
    role: String,
}

#[derive(Serialize)]
struct InviteResult {
    /// `sent` for every invitation (whether or not the address has an account);
    /// `instance_invite` only for an instance admin who is also admin of the
    /// group, when the address had no account.
    result: &'static str,
}

async fn invite(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(input): Json<InviteInput>,
) -> ApiResult<(StatusCode, Json<InviteResult>)> {
    let access = manager(&state, &user, &id).await?;
    let email = input.email.trim().to_lowercase();
    if email.parse::<lettre::Address>().is_err() {
        return invalid(format!("Invalid address: {email}"));
    }
    validate_role(&input.role)?;
    let now = now_ms();
    if !state.group_invites.allow(&user.id, now, INVITES_PER_DAY) {
        return invalid(format!(
            "You can send at most {INVITES_PER_DAY} invitations a day."
        ));
    }
    let target = db::invite_to_group(
        &state.db,
        &id,
        &email,
        &input.role,
        &user.id,
        now,
        now + INVITE_TTL_MS,
    )
    .await?;
    let mut result = "sent";
    match target {
        InviteTarget::Invited { .. } => {
            queue_invitation_email(&state, &user, &access.group, &input.role, &email).await;
        }
        InviteTarget::NoAccount
            if user.is_admin()
                && access.role.as_deref() == Some("admin")
                && email != state.admin_email.as_ref() =>
        {
            result = "instance_invite";
            // An existing instance invite keeps its role: only the group is added.
            let outcome = db::ensure_invite(&state.db, &email, "user", Some(&user.id), now).await?;
            if outcome != InviteOutcome::AlreadyUser {
                db::attach_invite_group(&state.db, &email, &id, &input.role, &user.id, now).await?;
            }
            if outcome == InviteOutcome::Created {
                let group = (access.group.name.as_str(), input.role.as_str());
                crate::users::send_invite_email(&state, &user, &email, Some(group)).await;
            }
        }
        InviteTarget::NoAccount | InviteTarget::AlreadyMember => {}
    }
    Ok((StatusCode::ACCEPTED, Json(InviteResult { result })))
}

/// Email to the invitee, through the queue: the request does not wait for the
/// send, so its duration does not tell whether the address has an account. The
/// urgent queue never drops it (the summary queue would, when full).
async fn queue_invitation_email(
    state: &AppState,
    by: &db::User,
    group: &db::Group,
    role: &str,
    to: &str,
) {
    if !state.mailer.is_enabled() {
        return;
    }
    let who = by.name.clone().unwrap_or_else(|| by.email.clone());
    let email = Letter {
        title: "Group invitation".into(),
        hot: false,
        pulse: Pulse::Beat,
        blocks: vec![
            Block::Para(vec![
                hi(who.clone()),
                fg(" invited you to the group "),
                hi(group.name.clone()),
                fg(format!(", as {}.", role_label(role))),
            ]),
            Block::text(
                "Members see and manage the group's applications and receive their alerts.",
            ),
            Block::text("Accept or reject the invitation on your dashboard. It expires in 7 days."),
        ],
        action: Some((
            "open the dashboard".into(),
            format!("{}/apps", state.public_base_url),
        )),
    }
    .into_email(
        to,
        format!("{who} invited you to {} on IronHeart", group.name),
    );
    state.outbox.urgent(email).await;
}

pub fn role_label(role: &str) -> &'static str {
    if role == "admin" {
        "group admin"
    } else {
        "member"
    }
}

#[derive(Deserialize)]
struct RoleInput {
    role: String,
}

async fn set_role(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((id, member_id)): Path<(String, String)>,
    Json(input): Json<RoleInput>,
) -> ApiResult<Json<GroupDetail>> {
    let access = manager(&state, &user, &id).await?;
    validate_role(&input.role)?;
    member_change(db::set_member_role(&state.db, &id, &member_id, &input.role).await?)?;
    let access = Access {
        role: db::member_role(&state.db, &id, &user.id).await?,
        ..access
    };
    Ok(Json(load_detail(&state, access).await?))
}

async fn remove_member(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((id, member_id)): Path<(String, String)>,
) -> ApiResult<Json<GroupDetail>> {
    let access = manager(&state, &user, &id).await?;
    member_change(db::remove_member(&state.db, &id, &member_id).await?)?;
    let access = Access {
        role: db::member_role(&state.db, &id, &user.id).await?,
        ..access
    };
    Ok(Json(load_detail(&state, access).await?))
}

async fn leave(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    member_change(db::remove_member(&state.db, &id, &user.id).await?)?;
    Ok(StatusCode::NO_CONTENT)
}

fn member_change(change: MemberChange) -> ApiResult<()> {
    match change {
        MemberChange::Done => Ok(()),
        MemberChange::NotMember => Err(ApiError::NotFound),
        MemberChange::LastAdmin => {
            invalid("A group needs at least one group admin: appoint another one first.")
        }
    }
}

async fn my_invites(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<db::PendingGroupInvite>>> {
    Ok(Json(
        db::pending_group_invites(&state.db, &user.id, now_ms()).await?,
    ))
}

async fn accept(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    match db::accept_group_invite(&state.db, &id, &user.id, now_ms(), MAX_MEMBERS).await? {
        JoinOutcome::Joined => Ok(StatusCode::NO_CONTENT),
        JoinOutcome::NotFound => Err(ApiError::NotFound),
        JoinOutcome::Full => invalid(format!("The group is full: at most {MAX_MEMBERS} members.")),
    }
}

async fn reject(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    if !db::reject_group_invite(&state.db, &id, &user.id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Trimmed name and its lowercase key (names are unique ignoring case).
fn validate_name(name: &str) -> ApiResult<(String, String)> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return invalid(format!(
            "The name must be 1 to {MAX_NAME_CHARS} characters long."
        ));
    }
    Ok((name.to_owned(), name.to_lowercase()))
}

fn validate_role(role: &str) -> ApiResult<()> {
    match role {
        "admin" | "member" => Ok(()),
        _ => invalid("Invalid role."),
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

    async fn account(state: &AppState, name: &str, role: &str) -> db::User {
        let user = db::upsert_google_user(&state.db, name, &format!("{name}@example.com"), None, 1)
            .await
            .unwrap();
        db::set_user_role(&state.db, &user.id, role).await.unwrap();
        db::User {
            role: role.into(),
            ..user
        }
    }

    fn invite_input(email: &str) -> Json<InviteInput> {
        Json(InviteInput {
            email: email.into(),
            role: "member".into(),
        })
    }

    #[test]
    fn daily_limit_slides() {
        let limit = DailyLimit::default();
        assert!(limit.allow("a", 0, 2));
        assert!(limit.allow("a", 1, 2));
        assert!(!limit.allow("a", 2, 2));
        assert!(limit.allow("b", 2, 2), "per key");
        assert!(limit.allow("a", DAY_MS, 2), "the oldest left the window");
    }

    /// The group admin gets the same answer for an account, a member and an
    /// unknown address; only the account gets an invitation.
    #[tokio::test]
    async fn invitations_answer_the_same_for_every_address() {
        let state = state().await;
        let admin = account(&state, "boss", "admin").await;
        let owner = account(&state, "owner", "user").await;
        let other = account(&state, "other", "user").await;
        let (_, Json(group)) = create(
            State(state.clone()),
            AdminUser(admin),
            Json(CreateGroup {
                name: "Ops".into(),
                admin_id: owner.id.clone(),
            }),
        )
        .await
        .ok()
        .unwrap();

        for email in [
            "other@example.com",
            "owner@example.com",
            "nobody@example.com",
        ] {
            let (status, Json(result)) = invite(
                State(state.clone()),
                CurrentUser(owner.clone()),
                Path(group.group.id.clone()),
                invite_input(email),
            )
            .await
            .ok()
            .unwrap();
            assert_eq!((status, result.result), (StatusCode::ACCEPTED, "sent"));
        }
        assert_eq!(
            db::pending_group_invites(&state.db, &other.id, now_ms())
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(db::list_invites(&state.db).await.unwrap().is_empty());

        // A plain member cannot invite.
        db::accept_group_invite(&state.db, &group.group.id, &other.id, now_ms(), MAX_MEMBERS)
            .await
            .unwrap();
        let refused = invite(
            State(state.clone()),
            CurrentUser(other),
            Path(group.group.id.clone()),
            invite_input("x@example.com"),
        )
        .await;
        assert!(matches!(refused, Err(ApiError::Forbidden)));
    }

    /// Instance admin and group admin: an unknown address becomes an instance
    /// invite with the group attached.
    #[tokio::test]
    async fn instance_admin_and_group_admin_invites_new_people() {
        let state = state().await;
        let admin = account(&state, "boss", "admin").await;
        let (_, Json(group)) = create(
            State(state.clone()),
            AdminUser(admin.clone()),
            Json(CreateGroup {
                name: "Ops".into(),
                admin_id: admin.id.clone(),
            }),
        )
        .await
        .ok()
        .unwrap();
        let (_, Json(result)) = invite(
            State(state.clone()),
            CurrentUser(admin.clone()),
            Path(group.group.id.clone()),
            invite_input("new@example.com"),
        )
        .await
        .ok()
        .unwrap();
        assert_eq!(result.result, "instance_invite");
        assert_eq!(db::list_invites(&state.db).await.unwrap().len(), 1);
        assert_eq!(db::invite_groups(&state.db).await.unwrap().len(), 1);

        // A pending instance invite keeps its role: only the group is attached.
        db::create_invite(&state.db, "lead@example.com", "admin", None, 1)
            .await
            .unwrap();
        assert!(
            invite(
                State(state.clone()),
                CurrentUser(admin.clone()),
                Path(group.group.id.clone()),
                invite_input("lead@example.com"),
            )
            .await
            .is_ok()
        );
        let invites = db::list_invites(&state.db).await.unwrap();
        let lead = invites
            .iter()
            .find(|i| i.email == "lead@example.com")
            .unwrap();
        assert_eq!(lead.role, "admin");
        assert_eq!(db::invite_groups(&state.db).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn outsiders_do_not_see_a_group() {
        let state = state().await;
        let admin = account(&state, "boss", "admin").await;
        let outsider = account(&state, "out", "user").await;
        let (_, Json(group)) = create(
            State(state.clone()),
            AdminUser(admin.clone()),
            Json(CreateGroup {
                name: "Ops".into(),
                admin_id: admin.id.clone(),
            }),
        )
        .await
        .ok()
        .unwrap();
        let seen = detail(
            State(state.clone()),
            CurrentUser(outsider),
            Path(group.group.id.clone()),
        )
        .await;
        assert!(matches!(seen, Err(ApiError::NotFound)));
    }
}
