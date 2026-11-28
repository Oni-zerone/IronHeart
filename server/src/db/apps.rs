//! Apps: configuration, keys, monitoring state.

use serde::Serialize;

use super::Pool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct App {
    pub id: String,
    /// Owner: a user (always the viewer, for personal apps) or a group.
    #[serde(skip)]
    pub user_id: Option<String>,
    #[serde(skip)]
    pub group_id: Option<String>,
    #[serde(skip)]
    pub group_name: Option<String>,
    /// Role of the viewer in the owning group.
    #[serde(skip)]
    pub group_role: Option<String>,
    pub name: String,
    pub key_id: String,
    pub interval_secs: i64,
    pub grace_secs: i64,
    pub state: String,
    pub last_seen_at: Option<i64>,
    pub next_deadline_at: Option<i64>,
    pub down_since: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl App {
    /// The viewer can delete it and move it to another owner: it is theirs, or
    /// they are admin of the owning group.
    pub fn manageable(&self) -> bool {
        self.user_id.is_some() || self.group_role.as_deref() == Some("admin")
    }
}

/// `App` as seen by the user `$1`: the owning group and the viewer's role in it.
/// A macro, so queries stay string literals (sqlx 0.9).
macro_rules! select_apps {
    () => {
        "SELECT apps.id, apps.user_id, apps.group_id, g.name AS group_name, m.role AS group_role, \
                apps.name, apps.key_id, apps.interval_secs, apps.grace_secs, apps.state, \
                apps.last_seen_at, apps.next_deadline_at, apps.down_since, \
                apps.created_at, apps.updated_at \
         FROM apps LEFT JOIN groups g ON g.id = apps.group_id \
         LEFT JOIN group_members m ON m.group_id = apps.group_id AND m.user_id = $1 \
         WHERE (apps.user_id = $1 OR m.role IS NOT NULL)"
    };
}

/// Apps that the user `$p` sees and works on: their own and those of their
/// groups. Every query on a user's apps goes through this rule.
macro_rules! visible_to {
    ($p:literal) => {
        concat!(
            "(user_id = ",
            $p,
            " OR group_id IN ",
            "(SELECT group_id FROM group_members WHERE user_id = ",
            $p,
            "))"
        )
    };
}

/// Apps that the user `$p` can delete and move: their own and those of the
/// groups where they are group admin.
macro_rules! managed_by {
    ($p:literal) => {
        concat!(
            "(user_id = ",
            $p,
            " OR group_id IN ",
            "(SELECT group_id FROM group_members WHERE user_id = ",
            $p,
            " AND role = 'admin'))"
        )
    };
}

/// Who owns an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner<'a> {
    User(&'a str),
    Group(&'a str),
}

impl<'a> Owner<'a> {
    fn user(self) -> Option<&'a str> {
        match self {
            Self::User(id) => Some(id),
            Self::Group(_) => None,
        }
    }

    fn group(self) -> Option<&'a str> {
        match self {
            Self::User(_) => None,
            Self::Group(id) => Some(id),
        }
    }
}

pub struct NewApp<'a> {
    pub id: &'a str,
    pub owner: Owner<'a>,
    pub name: &'a str,
    pub key_id: &'a str,
    pub secret_enc: &'a [u8],
    pub secret_key_version: i64,
    pub interval_secs: i64,
    pub grace_secs: i64,
    pub now: i64,
}

pub async fn insert_app(pool: &Pool, app: &NewApp<'_>, recipients: &[String]) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO apps (id, user_id, group_id, name, key_id, secret_enc, secret_key_version,
                           interval_secs, grace_secs, created_at, updated_at)
         VALUES ($1, $2, $10, $3, $4, $5, $6, $7, $8, $9, $9)",
    )
    .bind(app.id)
    .bind(app.owner.user())
    .bind(app.name)
    .bind(app.key_id)
    .bind(app.secret_enc)
    .bind(app.secret_key_version)
    .bind(app.interval_secs)
    .bind(app.grace_secs)
    .bind(app.now)
    .bind(app.owner.group())
    .execute(&mut *tx)
    .await?;
    replace_recipients(&mut tx, app.id, recipients, app.now).await?;
    tx.commit().await
}

/// Apps of an owner: limits are per user (personal apps) and per group.
/// Takes the pool or a transaction.
pub async fn count_owned_apps<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Any>,
    owner: Owner<'_>,
) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM apps
         WHERE ($1 IS NOT NULL AND user_id = $1) OR ($2 IS NOT NULL AND group_id = $2)",
    )
    .bind(owner.user())
    .bind(owner.group())
    .fetch_one(executor)
    .await
}

/// Apps the user sees: their own and those of their groups.
pub async fn list_apps(pool: &Pool, user_id: &str) -> sqlx::Result<Vec<App>> {
    sqlx::query_as(concat!(select_apps!(), " ORDER BY apps.name, apps.id"))
        .bind(user_id)
        .fetch_all(pool)
        .await
}

pub async fn get_app(pool: &Pool, user_id: &str, app_id: &str) -> sqlx::Result<Option<App>> {
    sqlx::query_as(concat!(select_apps!(), " AND apps.id = $2"))
        .bind(user_id)
        .bind(app_id)
        .fetch_optional(pool)
        .await
}

/// Configured recipients of the apps a user sees (without the fallback).
pub async fn configured_recipients(
    pool: &Pool,
    user_id: &str,
) -> sqlx::Result<Vec<(String, String)>> {
    sqlx::query_as(concat!(
        "SELECT r.app_id, r.email FROM app_recipients r WHERE r.app_id IN (SELECT id FROM apps WHERE ",
        visible_to!("$1"),
        ") ORDER BY r.email"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// New values of an app, already computed in Rust (state and deadline included).
pub struct AppUpdate<'a> {
    pub name: &'a str,
    pub interval_secs: i64,
    pub grace_secs: i64,
    pub state: &'a str,
    pub next_deadline_at: Option<i64>,
    pub recipients: Option<&'a [String]>,
    pub now: i64,
}

pub async fn update_app(
    pool: &Pool,
    user_id: &str,
    app_id: &str,
    update: &AppUpdate<'_>,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query(concat!(
        "UPDATE apps SET name = $1, interval_secs = $2, grace_secs = $3, state = $4,
                         next_deadline_at = $5, updated_at = $6
         WHERE id = $7 AND ",
        visible_to!("$8")
    ))
    .bind(update.name)
    .bind(update.interval_secs)
    .bind(update.grace_secs)
    .bind(update.state)
    .bind(update.next_deadline_at)
    .bind(update.now)
    .bind(app_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    if updated && let Some(recipients) = update.recipients {
        replace_recipients(&mut tx, app_id, recipients, update.now).await?;
    }
    tx.commit().await?;
    Ok(updated)
}

async fn replace_recipients(
    tx: &mut super::Connection,
    app_id: &str,
    recipients: &[String],
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM app_recipients WHERE app_id = $1")
        .bind(app_id)
        .execute(&mut *tx)
        .await?;
    for email in recipients {
        sqlx::query("INSERT INTO app_recipients (app_id, email, created_at) VALUES ($1, $2, $3)")
            .bind(app_id)
            .bind(email)
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// New key: the old `key_id` stops being valid and the timestamp restarts.
pub async fn rotate_key(
    pool: &Pool,
    user_id: &str,
    app_id: &str,
    key_id: &str,
    secret_enc: &[u8],
    secret_key_version: i64,
    now: i64,
) -> sqlx::Result<bool> {
    Ok(sqlx::query(concat!(
        "UPDATE apps SET key_id = $1, secret_enc = $2, secret_key_version = $3,
                         last_signed_ts = NULL, updated_at = $4
         WHERE id = $5 AND ",
        visible_to!("$6")
    ))
    .bind(key_id)
    .bind(secret_enc)
    .bind(secret_key_version)
    .bind(now)
    .bind(app_id)
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

/// Deletes the app (owner, or admin of the owning group) and returns its
/// `key_id`, to remove from the registry.
pub async fn delete_app(pool: &Pool, user_id: &str, app_id: &str) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar(concat!(
        "DELETE FROM apps WHERE id = $1 AND ",
        managed_by!("$2"),
        " RETURNING key_id"
    ))
    .bind(app_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

// ---- Key registry -----------------------------------------------------------

#[derive(sqlx::FromRow)]
pub struct KeyRow {
    pub id: String,
    pub key_id: String,
    pub secret_enc: Vec<u8>,
    pub secret_key_version: i64,
    pub interval_secs: i64,
    pub grace_secs: i64,
    pub last_signed_ts: Option<i64>,
}

macro_rules! key_columns {
    () => {
        "id, key_id, secret_enc, secret_key_version, interval_secs, grace_secs, last_signed_ts"
    };
}

pub async fn all_keys(pool: &Pool) -> sqlx::Result<Vec<KeyRow>> {
    sqlx::query_as(concat!("SELECT ", key_columns!(), " FROM apps"))
        .fetch_all(pool)
        .await
}

pub async fn app_key(pool: &Pool, app_id: &str) -> sqlx::Result<Option<KeyRow>> {
    sqlx::query_as(concat!(
        "SELECT ",
        key_columns!(),
        " FROM apps WHERE id = $1"
    ))
    .bind(app_id)
    .fetch_optional(pool)
    .await
}

// ---- Heartbeat and deadline check ------------------------------------------

/// A valid ping to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingRecord {
    pub app_id: String,
    pub signed_ts: i64,
    pub seen_at: i64,
    pub next_deadline_at: i64,
}

/// Records a valid ping. Returns `false` if the timestamp is not new.
/// If the app was `down`, flags the recovery: the check sends the email.
#[cfg(test)]
pub async fn record_ping(
    pool: &Pool,
    app_id: &str,
    signed_ts: i64,
    now: i64,
    next_deadline_at: i64,
) -> sqlx::Result<bool> {
    let record = PingRecord {
        app_id: app_id.into(),
        signed_ts,
        seen_at: now,
        next_deadline_at,
    };
    Ok(record_pings(pool, &[record]).await?[0])
}

/// Pings per statement: 4 parameters each, well below the limits of SQLite
/// (32,766) and Postgres (65,535).
const PINGS_PER_STATEMENT: usize = 500;

/// `UPDATE … FROM (VALUES …)` for `rows` pings. The VALUES columns are named
/// `column1…` in both SQLite and Postgres. In the SET expressions, `apps.state`
/// is the value before the update (in both).
fn record_pings_sql(rows: usize) -> String {
    let values: Vec<String> = (0..rows)
        .map(|i| {
            format!(
                "(${}, ${}, ${}, ${})",
                4 * i + 1,
                4 * i + 2,
                4 * i + 3,
                4 * i + 4
            )
        })
        .collect();
    format!(
        "UPDATE apps SET
             last_signed_ts = v.column2,
             last_seen_at = v.column3,
             recovery_pending = CASE WHEN apps.state = 'down' THEN 1 ELSE apps.recovery_pending END,
             next_deadline_at = CASE WHEN apps.state = 'paused' THEN NULL ELSE v.column4 END,
             state = CASE WHEN apps.state = 'paused' THEN 'paused' ELSE 'up' END,
             updated_at = v.column3
         FROM (VALUES {}) AS v
         WHERE apps.id = v.column1
           AND (apps.last_signed_ts IS NULL OR apps.last_signed_ts < v.column2)
         RETURNING apps.id",
        values.join(", ")
    )
}

/// Records several pings (at most one per app) in one transaction, with a single
/// statement per 500 pings: with a remote database every statement is a network
/// round trip. For each one: `false` if the timestamp was not new (or the app no
/// longer exists).
pub async fn record_pings(pool: &Pool, pings: &[PingRecord]) -> sqlx::Result<Vec<bool>> {
    let mut tx = pool.begin().await?;
    let mut updated = std::collections::HashSet::new();
    for chunk in pings.chunks(PINGS_PER_STATEMENT) {
        // Contains only placeholders: the values are passed as parameters.
        let sql = record_pings_sql(chunk.len());
        let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql));
        for ping in chunk {
            query = query
                .bind(ping.app_id.as_str())
                .bind(ping.signed_ts)
                .bind(ping.seen_at)
                .bind(ping.next_deadline_at);
        }
        updated.extend(query.fetch_all(&mut *tx).await?);
    }
    tx.commit().await?;
    Ok(pings
        .iter()
        .map(|ping| updated.contains(&ping.app_id))
        .collect())
}

#[derive(Debug, sqlx::FromRow)]
pub struct DownApp {
    pub id: String,
    pub name: String,
    pub last_seen_at: Option<i64>,
}

/// Moves overdue apps to `down`. Atomic: each app transitions only once.
pub async fn mark_overdue_down(pool: &Pool, now: i64) -> sqlx::Result<Vec<DownApp>> {
    sqlx::query_as(
        "UPDATE apps SET state = 'down', down_since = $1, next_deadline_at = NULL, updated_at = $1
         WHERE state = 'up' AND next_deadline_at <= $1
         RETURNING id, name, last_seen_at",
    )
    .bind(now)
    .fetch_all(pool)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct RecoveredApp {
    pub id: String,
    pub name: String,
    pub down_since: Option<i64>,
    pub last_seen_at: Option<i64>,
}

pub async fn take_recoveries(pool: &Pool) -> sqlx::Result<Vec<RecoveredApp>> {
    sqlx::query_as(
        "UPDATE apps SET recovery_pending = 0 WHERE recovery_pending = 1
         RETURNING id, name, down_since, last_seen_at",
    )
    .fetch_all(pool)
    .await
}

/// `up` apps whose deadline has already passed: at startup they are postponed, because
/// pings lost while the service was down are not the app's fault.
pub async fn overdue_up_apps(pool: &Pool, now: i64) -> sqlx::Result<Vec<(String, i64, i64)>> {
    sqlx::query_as(
        "SELECT id, interval_secs, grace_secs FROM apps
         WHERE state = 'up' AND next_deadline_at <= $1",
    )
    .bind(now)
    .fetch_all(pool)
    .await
}

pub async fn set_deadline(pool: &Pool, app_id: &str, next_deadline_at: i64) -> sqlx::Result<()> {
    sqlx::query("UPDATE apps SET next_deadline_at = $1 WHERE id = $2 AND state = 'up'")
        .bind(next_deadline_at)
        .bind(app_id)
        .execute(pool)
        .await
        .map(|_| ())
}

// ---- Failed-attempt alert state ---------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FailureAlertState {
    pub id: String,
    pub name: String,
    pub fail_level: i64,
    pub fail_window_end: Option<i64>,
    pub fail_last_report_at: Option<i64>,
}

/// Apps with an open window or with attempts not yet reported.
pub async fn failure_alert_candidates(pool: &Pool) -> sqlx::Result<Vec<FailureAlertState>> {
    sqlx::query_as(
        "SELECT id, name, fail_level, fail_window_end, fail_last_report_at FROM apps a
         WHERE fail_window_end IS NOT NULL
            OR EXISTS (SELECT 1 FROM failure_buckets b
                       WHERE b.app_id = a.id AND b.flushed_at > COALESCE(a.fail_last_report_at, 0))",
    )
    .fetch_all(pool)
    .await
}

pub async fn set_failure_alert_state(
    pool: &Pool,
    app_id: &str,
    level: i64,
    window_end: Option<i64>,
    last_report_at: Option<i64>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE apps SET fail_level = $1, fail_window_end = $2, fail_last_report_at = $3
         WHERE id = $4",
    )
    .bind(level)
    .bind(window_end)
    .bind(last_report_at)
    .bind(app_id)
    .execute(pool)
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{insert_test_app, test_pool, upsert_google_user};

    async fn setup() -> Pool {
        let pool = test_pool().await;
        let user = upsert_google_user(&pool, "sub", "o@example.com", None, 1)
            .await
            .unwrap();
        insert_test_app(&pool, "a", &user.id).await;
        pool
    }

    async fn state(pool: &Pool, id: &str) -> (String, Option<i64>, i64) {
        sqlx::query_as("SELECT state, next_deadline_at, recovery_pending FROM apps WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Group app: every member sees and edits it, only group admins delete it,
    /// outsiders do not see it, and it survives the removal of its creator.
    #[tokio::test]
    async fn group_apps_follow_the_group() {
        use crate::db::{
            accept_group_invite, alert_recipients, create_group, delete_user, invite_to_group,
        };
        let pool = test_pool().await;
        let mut ids = Vec::new();
        for name in ["admin", "member", "outsider"] {
            let user = upsert_google_user(&pool, name, &format!("{name}@example.com"), None, 1)
                .await
                .unwrap();
            ids.push(user.id);
        }
        let [admin, member, outsider] = [&ids[0], &ids[1], &ids[2]];
        create_group(&pool, "g", "Ops", "ops", admin, admin, 1)
            .await
            .unwrap();
        invite_to_group(&pool, "g", "member@example.com", "member", admin, 1, 100)
            .await
            .unwrap();
        accept_group_invite(&pool, "g", member, 2, 100)
            .await
            .unwrap();
        insert_app(
            &pool,
            &NewApp {
                id: "ga",
                owner: Owner::Group("g"),
                name: "Shared",
                key_id: "k-ga",
                secret_enc: b"secret",
                secret_key_version: 1,
                interval_secs: 60,
                grace_secs: 0,
                now: 1,
            },
            &[],
        )
        .await
        .unwrap();

        let seen = get_app(&pool, member, "ga").await.unwrap().unwrap();
        assert_eq!(
            (
                seen.group_name.as_deref(),
                seen.group_role.as_deref(),
                seen.manageable()
            ),
            (Some("Ops"), Some("member"), false)
        );
        assert!(
            get_app(&pool, admin, "ga")
                .await
                .unwrap()
                .unwrap()
                .manageable()
        );
        assert!(get_app(&pool, outsider, "ga").await.unwrap().is_none());
        assert!(list_apps(&pool, outsider).await.unwrap().is_empty());
        assert_eq!(list_apps(&pool, member).await.unwrap().len(), 1);

        let update = AppUpdate {
            name: "Renamed",
            interval_secs: 120,
            grace_secs: 0,
            state: "new",
            next_deadline_at: None,
            recipients: None,
            now: 3,
        };
        assert!(update_app(&pool, member, "ga", &update).await.unwrap());
        assert!(!update_app(&pool, outsider, "ga", &update).await.unwrap());
        assert!(
            rotate_key(&pool, member, "ga", "k2", b"s", 1, 4)
                .await
                .unwrap()
        );
        assert!(
            !rotate_key(&pool, outsider, "ga", "k3", b"s", 1, 4)
                .await
                .unwrap()
        );

        let mut recipients = alert_recipients(&pool, "ga").await.unwrap();
        recipients.sort();
        assert_eq!(recipients, ["admin@example.com", "member@example.com"]);
        assert_eq!(count_owned_apps(&pool, Owner::Group("g")).await.unwrap(), 1);
        assert_eq!(
            count_owned_apps(&pool, Owner::User(admin)).await.unwrap(),
            0
        );

        assert!(delete_app(&pool, member, "ga").await.unwrap().is_none());
        // The creator leaves: the app stays with the group, the member takes over.
        assert_eq!(
            delete_user(&pool, admin, None, 1).await.unwrap(),
            Some(vec![])
        );
        let seen = get_app(&pool, member, "ga").await.unwrap().unwrap();
        assert_eq!(seen.group_role.as_deref(), Some("admin"));
        assert_eq!(
            delete_app(&pool, member, "ga").await.unwrap().as_deref(),
            Some("k2")
        );
    }

    #[tokio::test]
    async fn ping_requires_increasing_timestamp() {
        let pool = setup().await;
        assert!(record_ping(&pool, "a", 100, 1_000, 61_000).await.unwrap());
        assert!(!record_ping(&pool, "a", 100, 2_000, 62_000).await.unwrap());
        assert!(!record_ping(&pool, "a", 99, 2_000, 62_000).await.unwrap());
        assert!(record_ping(&pool, "a", 101, 2_000, 62_000).await.unwrap());
        assert_eq!(state(&pool, "a").await, ("up".into(), Some(62_000), 0));
    }

    #[tokio::test]
    async fn down_then_recovery() {
        let pool = setup().await;
        record_ping(&pool, "a", 1, 1_000, 61_000).await.unwrap();

        assert!(mark_overdue_down(&pool, 60_999).await.unwrap().is_empty());
        let down = mark_overdue_down(&pool, 61_000).await.unwrap();
        assert_eq!(down.len(), 1);
        assert!(
            mark_overdue_down(&pool, 70_000).await.unwrap().is_empty(),
            "only once"
        );

        record_ping(&pool, "a", 2, 80_000, 140_000).await.unwrap();
        assert_eq!(state(&pool, "a").await, ("up".into(), Some(140_000), 1));
        let recovered = take_recoveries(&pool).await.unwrap();
        assert_eq!(recovered[0].down_since, Some(61_000));
        assert!(take_recoveries(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn paused_app_records_ping_without_deadline() {
        let pool = setup().await;
        sqlx::query("UPDATE apps SET state = 'paused'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(record_ping(&pool, "a", 1, 1_000, 61_000).await.unwrap());
        assert_eq!(state(&pool, "a").await, ("paused".into(), None, 0));
    }

    #[tokio::test]
    async fn record_pings_in_blocks() {
        let pool = setup().await;
        let user = upsert_google_user(&pool, "sub2", "p@example.com", None, 1)
            .await
            .unwrap();
        let ids: Vec<String> = (0..1_201).map(|i| format!("b{i}")).collect();
        for id in &ids {
            insert_test_app(&pool, id, &user.id).await;
        }
        let pings = |ts: i64| -> Vec<PingRecord> {
            ids.iter()
                .map(|id| PingRecord {
                    app_id: id.clone(),
                    signed_ts: ts,
                    seen_at: ts * 1000,
                    next_deadline_at: ts * 1000 + 60_000,
                })
                .chain([PingRecord {
                    app_id: "missing".into(),
                    signed_ts: ts,
                    seen_at: 0,
                    next_deadline_at: 0,
                }])
                .collect()
        };
        let accepted = record_pings(&pool, &pings(5)).await.unwrap();
        assert_eq!(accepted.iter().filter(|ok| **ok).count(), ids.len());
        assert!(!accepted[ids.len()], "nonexistent app");
        assert_eq!(state(&pool, "b1200").await, ("up".into(), Some(65_000), 0));

        // Same timestamp: none accepted.
        assert!(
            record_pings(&pool, &pings(5))
                .await
                .unwrap()
                .iter()
                .all(|ok| !ok)
        );
    }
}
