//! Groups of users: membership, group roles and invitations.
//!
//! A group always keeps at least one group admin: demoting or removing the last
//! one is refused, and when the last admin's account is removed the oldest
//! remaining member is promoted (`db::delete_user`).

use serde::Serialize;

use super::{Connection, Pool};

/// Members per group.
pub const MAX_GROUP_MEMBERS: i64 = 100;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GroupSummary {
    pub id: String,
    pub name: String,
    /// Role of the user who asked (`admin` or `member`), `None` if not a member.
    pub role: Option<String>,
    pub member_count: i64,
    pub app_count: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Member {
    pub user_id: String,
    pub email: String,
    pub name: Option<String>,
    pub role: String,
    pub created_at: i64,
}

/// A group the user belongs to, for `/api/me`.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Membership {
    pub id: String,
    pub name: String,
    pub role: String,
}

/// Pending invitation into a group, as the invitee sees it.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PendingGroupInvite {
    pub group_id: String,
    pub group_name: String,
    pub role: String,
    /// Email of whoever sent it; `None` if that account no longer exists.
    pub invited_by: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

/// Group summaries: the counts are subqueries so one statement answers.
macro_rules! group_summary {
    ($from:literal) => {
        concat!(
            "SELECT g.id, g.name, m.role,
                    (SELECT COUNT(*) FROM group_members x WHERE x.group_id = g.id) AS member_count,
                    (SELECT COUNT(*) FROM apps a WHERE a.group_id = g.id) AS app_count ",
            $from,
            " ORDER BY g.name_key, g.id"
        )
    };
}

/// Groups the user belongs to.
pub async fn list_user_groups(pool: &Pool, user_id: &str) -> sqlx::Result<Vec<GroupSummary>> {
    sqlx::query_as(group_summary!(
        "FROM groups g JOIN group_members m ON m.group_id = g.id AND m.user_id = $1"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Every group, with the user's role where they are a member (instance admins).
pub async fn list_all_groups(pool: &Pool, user_id: &str) -> sqlx::Result<Vec<GroupSummary>> {
    sqlx::query_as(group_summary!(
        "FROM groups g LEFT JOIN group_members m ON m.group_id = g.id AND m.user_id = $1"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn memberships(pool: &Pool, user_id: &str) -> sqlx::Result<Vec<Membership>> {
    sqlx::query_as(
        "SELECT g.id, g.name, m.role FROM groups g JOIN group_members m ON m.group_id = g.id
         WHERE m.user_id = $1 ORDER BY g.name_key, g.id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// `(user_id, group name, role)` of every membership, for the users page.
pub async fn all_memberships(pool: &Pool) -> sqlx::Result<Vec<(String, String, String)>> {
    sqlx::query_as(
        "SELECT m.user_id, g.name, m.role FROM group_members m JOIN groups g ON g.id = m.group_id
         ORDER BY g.name_key, g.id",
    )
    .fetch_all(pool)
    .await
}

/// `(email, group name, role)` of the groups attached to pending instance invites.
pub async fn invite_groups(pool: &Pool) -> sqlx::Result<Vec<(String, String, String)>> {
    sqlx::query_as(
        "SELECT i.email, g.name, i.role FROM user_invite_groups i JOIN groups g ON g.id = i.group_id
         ORDER BY g.name_key, g.id",
    )
    .fetch_all(pool)
    .await
}

pub async fn count_groups(pool: &Pool) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM groups")
        .fetch_one(pool)
        .await
}

pub async fn get_group(pool: &Pool, group_id: &str) -> sqlx::Result<Option<Group>> {
    sqlx::query_as("SELECT id, name, created_at FROM groups WHERE id = $1")
        .bind(group_id)
        .fetch_optional(pool)
        .await
}

/// `admin` or `member`; `None` if the user is not in the group.
pub async fn member_role(
    pool: &Pool,
    group_id: &str,
    user_id: &str,
) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("SELECT role FROM group_members WHERE group_id = $1 AND user_id = $2")
        .bind(group_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_members(pool: &Pool, group_id: &str) -> sqlx::Result<Vec<Member>> {
    sqlx::query_as(
        "SELECT m.user_id, u.email, u.name, m.role, m.created_at
         FROM group_members m JOIN users u ON u.id = m.user_id
         WHERE m.group_id = $1 ORDER BY m.created_at, u.email",
    )
    .bind(group_id)
    .fetch_all(pool)
    .await
}

#[derive(Debug, PartialEq, Eq)]
pub enum GroupWrite {
    Done,
    NotFound,
    /// Another group already has this name (case-insensitive).
    NameTaken,
}

/// New group with its first group admin (an existing account).
/// `name_key` is the lowercase name.
pub async fn create_group(
    pool: &Pool,
    id: &str,
    name: &str,
    name_key: &str,
    created_by: &str,
    first_admin: &str,
    now: i64,
) -> sqlx::Result<GroupWrite> {
    let mut tx = pool.begin().await?;
    let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(first_admin)
        .fetch_one(&mut *tx)
        .await?;
    if admins == 0 {
        return Ok(GroupWrite::NotFound);
    }
    let inserted = sqlx::query(
        "INSERT INTO groups (id, name, name_key, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT (name_key) DO NOTHING",
    )
    .bind(id)
    .bind(name)
    .bind(name_key)
    .bind(created_by)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(GroupWrite::NameTaken);
    }
    add_member(&mut tx, id, first_admin, "admin", now).await?;
    tx.commit().await?;
    Ok(GroupWrite::Done)
}

pub async fn rename_group(
    pool: &Pool,
    group_id: &str,
    name: &str,
    name_key: &str,
    now: i64,
) -> sqlx::Result<GroupWrite> {
    let mut tx = pool.begin().await?;
    let taken: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM groups WHERE name_key = $1 AND id <> $2")
            .bind(name_key)
            .bind(group_id)
            .fetch_one(&mut *tx)
            .await?;
    if taken > 0 {
        return Ok(GroupWrite::NameTaken);
    }
    let updated =
        sqlx::query("UPDATE groups SET name = $1, name_key = $2, updated_at = $3 WHERE id = $4")
            .bind(name)
            .bind(name_key)
            .bind(now)
            .bind(group_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    tx.commit().await?;
    Ok(if updated == 1 {
        GroupWrite::Done
    } else {
        GroupWrite::NotFound
    })
}

pub async fn delete_group(pool: &Pool, group_id: &str) -> sqlx::Result<bool> {
    Ok(sqlx::query("DELETE FROM groups WHERE id = $1")
        .bind(group_id)
        .execute(pool)
        .await?
        .rows_affected()
        == 1)
}

async fn add_member(
    tx: &mut Connection,
    group_id: &str,
    user_id: &str,
    role: &str,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO group_members (group_id, user_id, role, created_at) VALUES ($1, $2, $3, $4)
         ON CONFLICT (group_id, user_id) DO NOTHING",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(role)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map(|_| ())
}

async fn admin_count(tx: &mut Connection, group_id: &str) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM group_members WHERE group_id = $1 AND role = 'admin'")
        .bind(group_id)
        .fetch_one(&mut *tx)
        .await
}

#[derive(Debug, PartialEq, Eq)]
pub enum MemberChange {
    Done,
    NotMember,
    /// The change would leave the group without a group admin.
    LastAdmin,
}

pub async fn set_member_role(
    pool: &Pool,
    group_id: &str,
    user_id: &str,
    role: &str,
) -> sqlx::Result<MemberChange> {
    let mut tx = pool.begin().await?;
    let Some(current) = current_role(&mut tx, group_id, user_id).await? else {
        return Ok(MemberChange::NotMember);
    };
    if current == "admin" && role != "admin" && admin_count(&mut tx, group_id).await? <= 1 {
        return Ok(MemberChange::LastAdmin);
    }
    sqlx::query("UPDATE group_members SET role = $1 WHERE group_id = $2 AND user_id = $3")
        .bind(role)
        .bind(group_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(MemberChange::Done)
}

/// Removes a member (or the user leaves). The last group admin cannot go.
pub async fn remove_member(
    pool: &Pool,
    group_id: &str,
    user_id: &str,
) -> sqlx::Result<MemberChange> {
    let mut tx = pool.begin().await?;
    let Some(current) = current_role(&mut tx, group_id, user_id).await? else {
        return Ok(MemberChange::NotMember);
    };
    if current == "admin" && admin_count(&mut tx, group_id).await? <= 1 {
        return Ok(MemberChange::LastAdmin);
    }
    sqlx::query("DELETE FROM group_members WHERE group_id = $1 AND user_id = $2")
        .bind(group_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(MemberChange::Done)
}

async fn current_role(
    tx: &mut Connection,
    group_id: &str,
    user_id: &str,
) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("SELECT role FROM group_members WHERE group_id = $1 AND user_id = $2")
        .bind(group_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
}

/// Groups in which `user_id` is the only group admin and other members remain:
/// the oldest of them becomes admin. Called before the account is deleted.
pub(super) async fn promote_successors(tx: &mut Connection, user_id: &str) -> sqlx::Result<()> {
    let orphaned: Vec<String> = sqlx::query_scalar(
        "SELECT m.group_id FROM group_members m
         WHERE m.user_id = $1 AND m.role = 'admin'
           AND (SELECT COUNT(*) FROM group_members a
                WHERE a.group_id = m.group_id AND a.role = 'admin') = 1",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    for group_id in orphaned {
        sqlx::query(
            "UPDATE group_members SET role = 'admin'
             WHERE group_id = $1 AND user_id = (
                 SELECT user_id FROM group_members
                 WHERE group_id = $1 AND user_id <> $2
                 ORDER BY created_at, user_id LIMIT 1)",
        )
        .bind(&group_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Outcome of an invitation into a group. The API answers the same in every
/// case (privacy): only the caller of this function tells them apart.
#[derive(Debug, PartialEq, Eq)]
pub enum InviteTarget {
    /// No account with this email: nothing stored.
    NoAccount,
    /// Already a member: nothing stored.
    AlreadyMember,
    /// Invitation stored (or refreshed) for this account.
    Invited { user_id: String },
}

/// Invites the account with `email` (lowercase). Inviting again refreshes the
/// invitation: new role, new expiry.
pub async fn invite_to_group(
    pool: &Pool,
    group_id: &str,
    email: &str,
    role: &str,
    invited_by: &str,
    now: i64,
    expires_at: i64,
) -> sqlx::Result<InviteTarget> {
    let mut tx = pool.begin().await?;
    let Some(user_id): Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(&mut *tx)
        .await?
    else {
        return Ok(InviteTarget::NoAccount);
    };
    if current_role(&mut tx, group_id, &user_id).await?.is_some() {
        return Ok(InviteTarget::AlreadyMember);
    }
    sqlx::query(
        "INSERT INTO group_invites (group_id, user_id, role, invited_by, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (group_id, user_id) DO UPDATE SET
             role = excluded.role, invited_by = excluded.invited_by,
             created_at = excluded.created_at, expires_at = excluded.expires_at",
    )
    .bind(group_id)
    .bind(&user_id)
    .bind(role)
    .bind(invited_by)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(InviteTarget::Invited { user_id })
}

/// Attaches a group to a pending instance invite: at the first sign-in the
/// new account joins it with `role`.
pub async fn attach_invite_group(
    pool: &Pool,
    email: &str,
    group_id: &str,
    role: &str,
    invited_by: &str,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO user_invite_groups (email, group_id, role, invited_by, created_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (email, group_id) DO UPDATE SET role = excluded.role",
    )
    .bind(email)
    .bind(group_id)
    .bind(role)
    .bind(invited_by)
    .bind(now)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Groups attached to the instance invite of `email`: read before the invite is consumed.
pub(super) async fn take_invite_groups(
    tx: &mut Connection,
    email: &str,
) -> sqlx::Result<Vec<(String, String)>> {
    sqlx::query_as("DELETE FROM user_invite_groups WHERE email = $1 RETURNING group_id, role")
        .bind(email)
        .fetch_all(&mut *tx)
        .await
}

/// Memberships from an instance invite; a group that is full meanwhile is skipped.
pub(super) async fn join_groups(
    tx: &mut Connection,
    user_id: &str,
    groups: &[(String, String)],
    now: i64,
) -> sqlx::Result<()> {
    for (group_id, role) in groups {
        let members: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM group_members WHERE group_id = $1")
                .bind(group_id)
                .fetch_one(&mut *tx)
                .await?;
        if members < MAX_GROUP_MEMBERS {
            add_member(tx, group_id, user_id, role, now).await?;
        }
    }
    Ok(())
}

pub async fn pending_group_invites(
    pool: &Pool,
    user_id: &str,
    now: i64,
) -> sqlx::Result<Vec<PendingGroupInvite>> {
    sqlx::query_as(
        "SELECT i.group_id, g.name AS group_name, i.role, u.email AS invited_by,
                i.created_at, i.expires_at
         FROM group_invites i JOIN groups g ON g.id = i.group_id
         LEFT JOIN users u ON u.id = i.invited_by
         WHERE i.user_id = $1 AND i.expires_at > $2
         ORDER BY i.created_at, g.name_key",
    )
    .bind(user_id)
    .bind(now)
    .fetch_all(pool)
    .await
}

#[derive(Debug, PartialEq, Eq)]
pub enum JoinOutcome {
    Joined,
    /// No pending invitation (never sent, expired, or already answered).
    NotFound,
    /// The group reached `max_members`.
    Full,
}

pub async fn accept_group_invite(
    pool: &Pool,
    group_id: &str,
    user_id: &str,
    now: i64,
    max_members: i64,
) -> sqlx::Result<JoinOutcome> {
    let mut tx = pool.begin().await?;
    let Some(role): Option<String> = sqlx::query_scalar(
        "SELECT role FROM group_invites WHERE group_id = $1 AND user_id = $2 AND expires_at > $3",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(JoinOutcome::NotFound);
    };
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM group_members WHERE group_id = $1")
        .bind(group_id)
        .fetch_one(&mut *tx)
        .await?;
    if members >= max_members {
        return Ok(JoinOutcome::Full);
    }
    add_member(&mut tx, group_id, user_id, &role, now).await?;
    sqlx::query("DELETE FROM group_invites WHERE group_id = $1 AND user_id = $2")
        .bind(group_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(JoinOutcome::Joined)
}

pub async fn reject_group_invite(pool: &Pool, group_id: &str, user_id: &str) -> sqlx::Result<bool> {
    Ok(
        sqlx::query("DELETE FROM group_invites WHERE group_id = $1 AND user_id = $2")
            .bind(group_id)
            .bind(user_id)
            .execute(pool)
            .await?
            .rows_affected()
            == 1,
    )
}

pub async fn delete_group_invites_before(pool: &Pool, now: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM group_invites WHERE expires_at <= $1")
        .bind(now)
        .execute(pool)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_invite, delete_user, login_google_user, test_pool, upsert_google_user};

    async fn user(pool: &Pool, name: &str) -> String {
        upsert_google_user(pool, name, &format!("{name}@example.com"), None, 1)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn names_are_unique_ignoring_case() {
        let pool = test_pool().await;
        let a = user(&pool, "a").await;
        assert_eq!(
            create_group(&pool, "g1", "Ops", "ops", &a, &a, 1)
                .await
                .unwrap(),
            GroupWrite::Done
        );
        assert_eq!(
            create_group(&pool, "g2", "OPS", "ops", &a, &a, 1)
                .await
                .unwrap(),
            GroupWrite::NameTaken
        );
        assert_eq!(
            create_group(&pool, "g3", "x", "x", &a, "missing", 1)
                .await
                .unwrap(),
            GroupWrite::NotFound
        );
        create_group(&pool, "g4", "Billing", "billing", &a, &a, 1)
            .await
            .unwrap();
        assert_eq!(
            rename_group(&pool, "g4", "Ops", "ops", 2).await.unwrap(),
            GroupWrite::NameTaken
        );
        assert_eq!(
            rename_group(&pool, "g4", "Finance", "finance", 2)
                .await
                .unwrap(),
            GroupWrite::Done
        );
        let mine = list_user_groups(&pool, &a).await.unwrap();
        assert_eq!(
            mine.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            ["Finance", "Ops"]
        );
        assert_eq!(mine[0].role.as_deref(), Some("admin"));
        assert_eq!(mine[0].member_count, 1);
    }

    #[tokio::test]
    async fn the_last_group_admin_stays() {
        let pool = test_pool().await;
        let a = user(&pool, "a").await;
        let b = user(&pool, "b").await;
        create_group(&pool, "g", "Ops", "ops", &a, &a, 1)
            .await
            .unwrap();
        assert_eq!(
            set_member_role(&pool, "g", &a, "member").await.unwrap(),
            MemberChange::LastAdmin
        );
        assert_eq!(
            remove_member(&pool, "g", &a).await.unwrap(),
            MemberChange::LastAdmin
        );
        assert_eq!(
            remove_member(&pool, "g", &b).await.unwrap(),
            MemberChange::NotMember
        );

        invite_to_group(&pool, "g", "b@example.com", "member", &a, 1, 100)
            .await
            .unwrap();
        assert_eq!(
            accept_group_invite(&pool, "g", &b, 2, 100).await.unwrap(),
            JoinOutcome::Joined
        );
        assert_eq!(
            set_member_role(&pool, "g", &b, "admin").await.unwrap(),
            MemberChange::Done
        );
        assert_eq!(
            set_member_role(&pool, "g", &a, "member").await.unwrap(),
            MemberChange::Done
        );
        assert_eq!(
            member_role(&pool, "g", &a).await.unwrap().as_deref(),
            Some("member")
        );
    }

    #[tokio::test]
    async fn removing_the_last_admin_promotes_the_oldest_member() {
        let pool = test_pool().await;
        let a = user(&pool, "a").await;
        let b = user(&pool, "b").await;
        let c = user(&pool, "c").await;
        create_group(&pool, "g", "Ops", "ops", &a, &a, 1)
            .await
            .unwrap();
        for (who, at) in [("b@example.com", 2), ("c@example.com", 3)] {
            invite_to_group(&pool, "g", who, "member", &a, at, 100)
                .await
                .unwrap();
        }
        accept_group_invite(&pool, "g", &c, 4, 100).await.unwrap();
        accept_group_invite(&pool, "g", &b, 5, 100).await.unwrap();

        delete_user(&pool, &a).await.unwrap();
        let members = list_members(&pool, "g").await.unwrap();
        let role = |id: &str| {
            members
                .iter()
                .find(|m| m.user_id == id)
                .unwrap()
                .role
                .clone()
        };
        assert_eq!((role(&c), role(&b)), ("admin".into(), "member".into()));
    }

    #[tokio::test]
    async fn invitations_reach_only_accounts_that_are_not_members() {
        let pool = test_pool().await;
        let a = user(&pool, "a").await;
        let b = user(&pool, "b").await;
        create_group(&pool, "g", "Ops", "ops", &a, &a, 1)
            .await
            .unwrap();

        let invite = |email: &'static str, role: &'static str, now: i64| {
            let pool = pool.clone();
            let a = a.clone();
            async move {
                invite_to_group(&pool, "g", email, role, &a, now, now + 100)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(
            invite("nobody@example.com", "member", 1).await,
            InviteTarget::NoAccount
        );
        assert_eq!(
            invite("a@example.com", "member", 1).await,
            InviteTarget::AlreadyMember
        );
        assert_eq!(
            invite("b@example.com", "member", 1).await,
            InviteTarget::Invited { user_id: b.clone() }
        );
        // Inviting again refreshes role and expiry.
        invite("b@example.com", "admin", 50).await;
        let pending = pending_group_invites(&pool, &b, 120).await.unwrap();
        assert_eq!((pending.len(), pending[0].role.as_str()), (1, "admin"));
        assert_eq!(pending[0].invited_by.as_deref(), Some("a@example.com"));
        assert!(
            pending_group_invites(&pool, &b, 150)
                .await
                .unwrap()
                .is_empty(),
            "expired"
        );

        assert!(reject_group_invite(&pool, "g", &b).await.unwrap());
        assert_eq!(
            accept_group_invite(&pool, "g", &b, 60, 100).await.unwrap(),
            JoinOutcome::NotFound
        );

        invite("b@example.com", "member", 60).await;
        assert_eq!(
            accept_group_invite(&pool, "g", &b, 61, 1).await.unwrap(),
            JoinOutcome::Full
        );
        delete_group_invites_before(&pool, 1_000).await.unwrap();
        assert!(
            pending_group_invites(&pool, &b, 61)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn an_instance_invite_with_groups_joins_them_at_first_sign_in() {
        let pool = test_pool().await;
        let a = user(&pool, "a").await;
        create_group(&pool, "g", "Ops", "ops", &a, &a, 1)
            .await
            .unwrap();
        create_invite(&pool, "new@example.com", "user", Some(&a), 1)
            .await
            .unwrap();
        attach_invite_group(&pool, "new@example.com", "g", "member", &a, 1)
            .await
            .unwrap();
        assert_eq!(invite_groups(&pool).await.unwrap().len(), 1);

        let new = login_google_user(&pool, "sub-new", "new@example.com", None, false, 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            member_role(&pool, "g", &new.id).await.unwrap().as_deref(),
            Some("member")
        );
        assert!(invite_groups(&pool).await.unwrap().is_empty());
    }
}
