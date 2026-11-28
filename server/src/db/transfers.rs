//! Moving apps between owners: immediate moves (between a user and their
//! groups) and transfer requests to another user, who accepts or rejects them.
//!
//! An app keeps its id, key, state and history when it changes owner: the
//! in-memory heartbeat registry does not know the owner and is not touched.

use serde::Serialize;

use super::{Connection, Owner, Pool, count_owned_apps};

/// Pending request, as the sender sees it on the app.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PendingTransfer {
    #[serde(skip)]
    pub app_id: String,
    pub to_email: String,
    pub created_at: i64,
    pub expires_at: i64,
}

/// Pending request, as the recipient sees it on the dashboard.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct IncomingTransfer {
    pub id: String,
    pub app_id: String,
    pub app_name: String,
    pub interval_secs: i64,
    /// Email of whoever asked; `None` if that account no longer exists.
    pub from_email: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

/// Owner columns of an app: the user, or the group.
fn owner_columns(owner: Owner<'_>) -> (Option<&str>, Option<&str>) {
    match owner {
        Owner::User(id) => (Some(id), None),
        Owner::Group(id) => (None, Some(id)),
    }
}

/// Moves an app to `to` if it still belongs to `from`, and cancels its pending
/// transfer request. `false` if the owner changed meanwhile.
async fn reassign(
    tx: &mut Connection,
    app_id: &str,
    from: Owner<'_>,
    to: Owner<'_>,
    now: i64,
) -> sqlx::Result<bool> {
    let (from_user, from_group) = owner_columns(from);
    let (to_user, to_group) = owner_columns(to);
    let moved = sqlx::query(
        "UPDATE apps SET user_id = $1, group_id = $2, updated_at = $3
         WHERE id = $4 AND COALESCE(user_id, '') = COALESCE($5, '')
           AND COALESCE(group_id, '') = COALESCE($6, '')",
    )
    .bind(to_user)
    .bind(to_group)
    .bind(now)
    .bind(app_id)
    .bind(from_user)
    .bind(from_group)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    if moved {
        close_pending(tx, app_id, "cancelled", now).await?;
    }
    Ok(moved)
}

async fn close_pending(
    tx: &mut Connection,
    app_id: &str,
    state: &str,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE app_transfers SET state = $1, closed_at = $2 WHERE app_id = $3 AND state = 'pending'",
    )
    .bind(state)
    .bind(now)
    .bind(app_id)
    .execute(&mut *tx)
    .await
    .map(|_| ())
}

#[derive(Debug, PartialEq, Eq)]
pub enum MoveOutcome {
    Moved,
    /// The target already has `max_apps` apps.
    Full,
    /// The app changed owner meanwhile (or no longer exists).
    Stale,
}

/// Immediate move from `from` to `to`, limits checked in the same transaction.
pub async fn move_app(
    pool: &Pool,
    app_id: &str,
    from: Owner<'_>,
    to: Owner<'_>,
    max_apps: i64,
    now: i64,
) -> sqlx::Result<MoveOutcome> {
    let mut tx = pool.begin().await?;
    if count_owned_apps(&mut *tx, to).await? >= max_apps {
        return Ok(MoveOutcome::Full);
    }
    if !reassign(&mut tx, app_id, from, to, now).await? {
        return Ok(MoveOutcome::Stale);
    }
    tx.commit().await?;
    Ok(MoveOutcome::Moved)
}

pub struct TransferRequest<'a> {
    pub id: &'a str,
    pub app_id: &'a str,
    pub from: Owner<'a>,
    pub requested_by: &'a str,
    /// Lowercase.
    pub to_email: &'a str,
    pub now: i64,
    pub expires_at: i64,
}

/// New request; an older pending one for the same app is cancelled.
pub async fn create_transfer(pool: &Pool, request: &TransferRequest<'_>) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    close_pending(&mut tx, request.app_id, "cancelled", request.now).await?;
    let (from_user, from_group) = owner_columns(request.from);
    sqlx::query(
        "INSERT INTO app_transfers (id, app_id, from_user_id, from_group_id, requested_by,
                                    to_email, state, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8)",
    )
    .bind(request.id)
    .bind(request.app_id)
    .bind(from_user)
    .bind(from_group)
    .bind(request.requested_by)
    .bind(request.to_email)
    .bind(request.now)
    .bind(request.expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Requests made by a user since `since` (every state), for the daily limit.
pub async fn count_transfers_since(pool: &Pool, user_id: &str, since: i64) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM app_transfers WHERE requested_by = $1 AND created_at > $2",
    )
    .bind(user_id)
    .bind(since)
    .fetch_one(pool)
    .await
}

pub async fn cancel_transfer(pool: &Pool, app_id: &str, now: i64) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "UPDATE app_transfers SET state = 'cancelled', closed_at = $1
         WHERE app_id = $2 AND state = 'pending'",
    )
    .bind(now)
    .bind(app_id)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

/// Pending requests on the apps the user manages (theirs, or of the groups
/// where they are group admin): only they see where an app is being sent.
pub async fn pending_transfers(
    pool: &Pool,
    user_id: &str,
    now: i64,
) -> sqlx::Result<Vec<PendingTransfer>> {
    sqlx::query_as(
        "SELECT t.app_id, t.to_email, t.created_at, t.expires_at
         FROM app_transfers t JOIN apps a ON a.id = t.app_id
         WHERE t.state = 'pending' AND t.expires_at > $2
           AND (a.user_id = $1 OR a.group_id IN
                (SELECT group_id FROM group_members WHERE user_id = $1 AND role = 'admin'))",
    )
    .bind(user_id)
    .bind(now)
    .fetch_all(pool)
    .await
}

pub async fn incoming_transfers(
    pool: &Pool,
    email: &str,
    now: i64,
) -> sqlx::Result<Vec<IncomingTransfer>> {
    sqlx::query_as(
        "SELECT t.id, t.app_id, a.name AS app_name, a.interval_secs, u.email AS from_email,
                t.created_at, t.expires_at
         FROM app_transfers t JOIN apps a ON a.id = t.app_id
         LEFT JOIN users u ON u.id = t.requested_by
         WHERE t.to_email = $1 AND t.state = 'pending' AND t.expires_at > $2
         ORDER BY t.created_at",
    )
    .bind(email)
    .bind(now)
    .fetch_all(pool)
    .await
}

#[derive(Debug, PartialEq, Eq)]
pub enum AcceptOutcome {
    Accepted {
        app_id: String,
    },
    /// No pending request for this address (never sent, expired, answered).
    NotFound,
    /// The app changed owner after the request: the request is void.
    Stale,
    /// The recipient already has `max_apps` personal apps.
    Full,
}

#[derive(sqlx::FromRow)]
struct OpenTransfer {
    app_id: String,
    from_user_id: Option<String>,
    from_group_id: Option<String>,
    requested_by: Option<String>,
}

async fn open_transfer(
    tx: &mut Connection,
    id: &str,
    email: &str,
    now: i64,
) -> sqlx::Result<Option<OpenTransfer>> {
    sqlx::query_as(
        "SELECT app_id, from_user_id, from_group_id, requested_by FROM app_transfers
         WHERE id = $1 AND to_email = $2 AND state = 'pending' AND expires_at > $3",
    )
    .bind(id)
    .bind(email)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await
}

/// The recipient (`user_id`, with `email`) takes the app over as a personal app.
pub async fn accept_transfer(
    pool: &Pool,
    id: &str,
    user_id: &str,
    email: &str,
    max_apps: i64,
    now: i64,
) -> sqlx::Result<AcceptOutcome> {
    let mut tx = pool.begin().await?;
    let Some(open) = open_transfer(&mut tx, id, email, now).await? else {
        return Ok(AcceptOutcome::NotFound);
    };
    let from = match (&open.from_user_id, &open.from_group_id) {
        (Some(user), _) => Owner::User(user),
        (None, Some(group)) => Owner::Group(group),
        (None, None) => return Ok(AcceptOutcome::Stale),
    };
    // The sender must still be allowed to hand it over: the owner, or an
    // admin of the owning group (they may have been demoted or removed since).
    let still_allowed = match (&open.requested_by, from) {
        (Some(by), Owner::User(owner)) => by == owner,
        (Some(by), Owner::Group(group)) => {
            let admins: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM group_members
                 WHERE group_id = $1 AND user_id = $2 AND role = 'admin'",
            )
            .bind(group)
            .bind(by)
            .fetch_one(&mut *tx)
            .await?;
            admins > 0
        }
        (None, _) => false,
    };
    if !still_allowed {
        close_pending(&mut tx, &open.app_id, "cancelled", now).await?;
        tx.commit().await?;
        return Ok(AcceptOutcome::Stale);
    }
    if count_owned_apps(&mut *tx, Owner::User(user_id)).await? >= max_apps {
        return Ok(AcceptOutcome::Full);
    }
    if !reassign(&mut tx, &open.app_id, from, Owner::User(user_id), now).await? {
        // The app moved meanwhile: the request is void.
        close_pending(&mut tx, &open.app_id, "cancelled", now).await?;
        tx.commit().await?;
        return Ok(AcceptOutcome::Stale);
    }
    sqlx::query("UPDATE app_transfers SET state = 'accepted', closed_at = $1 WHERE id = $2")
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(AcceptOutcome::Accepted {
        app_id: open.app_id,
    })
}

/// What the sender is told when a request is rejected.
#[derive(Debug, sqlx::FromRow)]
pub struct RejectedTransfer {
    pub app_name: String,
    /// Email of the sender; `None` if that account no longer exists.
    pub requested_by: Option<String>,
}

pub async fn reject_transfer(
    pool: &Pool,
    id: &str,
    email: &str,
    now: i64,
) -> sqlx::Result<Option<RejectedTransfer>> {
    let mut tx = pool.begin().await?;
    let rejected: Option<RejectedTransfer> = sqlx::query_as(
        "SELECT a.name AS app_name, u.email AS requested_by
         FROM app_transfers t JOIN apps a ON a.id = t.app_id
         LEFT JOIN users u ON u.id = t.requested_by
         WHERE t.id = $1 AND t.to_email = $2 AND t.state = 'pending' AND t.expires_at > $3",
    )
    .bind(id)
    .bind(email)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    if rejected.is_some() {
        sqlx::query("UPDATE app_transfers SET state = 'rejected', closed_at = $1 WHERE id = $2")
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(rejected)
}

/// Periodic clean-up: pending requests past their expiry become `expired`,
/// closed ones older than `closed_before` are deleted.
pub async fn expire_transfers(pool: &Pool, now: i64, closed_before: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE app_transfers SET state = 'expired', closed_at = $1
         WHERE state = 'pending' AND expires_at <= $1",
    )
    .bind(now)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM app_transfers WHERE state <> 'pending' AND closed_at < $1")
        .bind(closed_before)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Before an account is removed: its personal apps move to `group_id` (if
/// given) and the pending requests it sent or received are cancelled.
pub(super) async fn release_user(
    tx: &mut Connection,
    user_id: &str,
    move_apps_to: Option<&str>,
    now: i64,
) -> sqlx::Result<()> {
    if let Some(group_id) = move_apps_to {
        sqlx::query(
            "UPDATE apps SET user_id = NULL, group_id = $1, updated_at = $2 WHERE user_id = $3",
        )
        .bind(group_id)
        .bind(now)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE app_transfers SET state = 'cancelled', closed_at = $1
         WHERE state = 'pending'
           AND (requested_by = $2 OR to_email = (SELECT email FROM users WHERE id = $2))",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        accept_group_invite, create_group, get_app, insert_test_app, invite_to_group, test_pool,
        upsert_google_user,
    };

    async fn user(pool: &Pool, name: &str) -> String {
        upsert_google_user(pool, name, &format!("{name}@example.com"), None, 1)
            .await
            .unwrap()
            .id
    }

    fn request<'a>(
        id: &'a str,
        from: Owner<'a>,
        by: &'a str,
        to: &'a str,
        now: i64,
    ) -> TransferRequest<'a> {
        TransferRequest {
            id,
            app_id: "a",
            from,
            requested_by: by,
            to_email: to,
            now,
            expires_at: now + 100,
        }
    }

    #[tokio::test]
    async fn accepted_transfer_keeps_the_app_and_cancels_nothing_else() {
        let pool = test_pool().await;
        let luca = user(&pool, "luca").await;
        let marco = user(&pool, "marco").await;
        insert_test_app(&pool, "a", &luca).await;
        sqlx::query("UPDATE apps SET state = 'up', last_signed_ts = 7, next_deadline_at = 9")
            .execute(&pool)
            .await
            .unwrap();

        create_transfer(
            &pool,
            &request("t1", Owner::User(&luca), &luca, "marco@example.com", 1),
        )
        .await
        .unwrap();
        // A new request replaces the old one.
        create_transfer(
            &pool,
            &request("t2", Owner::User(&luca), &luca, "marco@example.com", 2),
        )
        .await
        .unwrap();
        assert_eq!(pending_transfers(&pool, &luca, 3).await.unwrap().len(), 1);
        assert!(
            pending_transfers(&pool, &marco, 3)
                .await
                .unwrap()
                .is_empty()
        );
        let incoming = incoming_transfers(&pool, "marco@example.com", 3)
            .await
            .unwrap();
        assert_eq!(
            (
                incoming.len(),
                incoming[0].id.as_str(),
                incoming[0].from_email.as_deref()
            ),
            (1, "t2", Some("luca@example.com"))
        );
        assert_eq!(count_transfers_since(&pool, &luca, 0).await.unwrap(), 2);

        // Only the addressee can accept, before the expiry, within the limit.
        assert_eq!(
            accept_transfer(&pool, "t2", &luca, "luca@example.com", 100, 3)
                .await
                .unwrap(),
            AcceptOutcome::NotFound
        );
        assert_eq!(
            accept_transfer(&pool, "t2", &marco, "marco@example.com", 0, 3)
                .await
                .unwrap(),
            AcceptOutcome::Full
        );
        assert_eq!(
            accept_transfer(&pool, "t2", &marco, "marco@example.com", 100, 3)
                .await
                .unwrap(),
            AcceptOutcome::Accepted { app_id: "a".into() }
        );
        assert!(get_app(&pool, &luca, "a").await.unwrap().is_none());
        let moved = get_app(&pool, &marco, "a").await.unwrap().unwrap();
        assert_eq!(
            (moved.state.as_str(), moved.next_deadline_at),
            ("up", Some(9))
        );
        let signed: Option<i64> = sqlx::query_scalar("SELECT last_signed_ts FROM apps")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(signed, Some(7), "the key and its timestamp are untouched");
        assert!(
            incoming_transfers(&pool, "marco@example.com", 3)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_request_is_void_once_the_app_moved() {
        let pool = test_pool().await;
        let luca = user(&pool, "luca").await;
        let marco = user(&pool, "marco").await;
        create_group(&pool, "g", "Ops", "ops", &luca, &luca, 1)
            .await
            .unwrap();
        insert_test_app(&pool, "a", &luca).await;
        create_transfer(
            &pool,
            &request("t", Owner::User(&luca), &luca, "marco@example.com", 1),
        )
        .await
        .unwrap();
        assert_eq!(
            move_app(&pool, "a", Owner::User(&luca), Owner::Group("g"), 100, 2)
                .await
                .unwrap(),
            MoveOutcome::Moved
        );
        assert_eq!(
            accept_transfer(&pool, "t", &marco, "marco@example.com", 100, 3)
                .await
                .unwrap(),
            AcceptOutcome::NotFound,
            "moving cancels the pending request"
        );
        assert_eq!(
            move_app(&pool, "a", Owner::User(&luca), Owner::Group("g"), 100, 4)
                .await
                .unwrap(),
            MoveOutcome::Stale
        );
        assert_eq!(
            move_app(&pool, "a", Owner::Group("g"), Owner::User(&luca), 0, 4)
                .await
                .unwrap(),
            MoveOutcome::Full
        );
    }

    /// Removing a user can move their personal apps to a group; their pending
    /// requests (sent or received) are cancelled.
    #[tokio::test]
    async fn removing_a_user_moves_apps_and_cancels_requests() {
        let pool = test_pool().await;
        let luca = user(&pool, "luca").await;
        let marco = user(&pool, "marco").await;
        create_group(&pool, "g", "Ops", "ops", &marco, &marco, 1)
            .await
            .unwrap();
        insert_test_app(&pool, "a", &luca).await;
        create_transfer(
            &pool,
            &request("t", Owner::User(&luca), &luca, "marco@example.com", 1),
        )
        .await
        .unwrap();
        assert_eq!(
            crate::db::delete_user(&pool, &luca, Some("g"), 2)
                .await
                .unwrap(),
            Some(vec![]),
            "nothing deleted: the app moved"
        );
        let moved = get_app(&pool, &marco, "a").await.unwrap().unwrap();
        assert_eq!(moved.group_id.as_deref(), Some("g"));
        assert!(
            incoming_transfers(&pool, "marco@example.com", 3)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn rejecting_tells_the_sender_and_expiry_closes() {
        let pool = test_pool().await;
        let luca = user(&pool, "luca").await;
        let marco = user(&pool, "marco").await;
        create_group(&pool, "g", "Ops", "ops", &luca, &luca, 1)
            .await
            .unwrap();
        invite_to_group(&pool, "g", "marco@example.com", "member", &luca, 1, 100)
            .await
            .unwrap();
        accept_group_invite(&pool, "g", &marco, 1, 100)
            .await
            .unwrap();
        insert_test_app(&pool, "a", &luca).await;
        move_app(&pool, "a", Owner::User(&luca), Owner::Group("g"), 100, 1)
            .await
            .unwrap();

        // A request from a group admin who is demoted meanwhile is void.
        create_transfer(
            &pool,
            &request("d", Owner::Group("g"), &luca, "marco@example.com", 2),
        )
        .await
        .unwrap();
        crate::db::set_member_role(&pool, "g", &marco, "admin")
            .await
            .unwrap();
        crate::db::set_member_role(&pool, "g", &luca, "member")
            .await
            .unwrap();
        assert_eq!(
            accept_transfer(&pool, "d", &marco, "marco@example.com", 100, 3)
                .await
                .unwrap(),
            AcceptOutcome::Stale
        );
        crate::db::set_member_role(&pool, "g", &luca, "admin")
            .await
            .unwrap();
        crate::db::set_member_role(&pool, "g", &marco, "member")
            .await
            .unwrap();

        // Group admins see the group's pending requests, plain members do not.
        create_transfer(
            &pool,
            &request("t", Owner::Group("g"), &luca, "x@example.com", 2),
        )
        .await
        .unwrap();
        assert_eq!(pending_transfers(&pool, &luca, 3).await.unwrap().len(), 1);
        assert!(
            pending_transfers(&pool, &marco, 3)
                .await
                .unwrap()
                .is_empty()
        );
        let rejected = reject_transfer(&pool, "t", "x@example.com", 3)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rejected.requested_by.as_deref(), Some("luca@example.com"));
        assert!(
            reject_transfer(&pool, "t", "x@example.com", 3)
                .await
                .unwrap()
                .is_none()
        );

        create_transfer(
            &pool,
            &request("u", Owner::Group("g"), &luca, "y@example.com", 10),
        )
        .await
        .unwrap();
        expire_transfers(&pool, 200, 0).await.unwrap();
        assert!(
            !cancel_transfer(&pool, "a", 201).await.unwrap(),
            "already expired"
        );
        expire_transfers(&pool, 300, 1_000).await.unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_transfers")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "closed requests are deleted after a while");
    }
}
