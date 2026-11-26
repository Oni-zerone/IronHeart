//! Tentativi di heartbeat falliti, sommati per periodo.

use std::collections::BTreeMap;

use serde::Serialize;
use uuid::Uuid;

use super::Pool;

/// Conteggi di un periodo, o somma di più periodi.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct FailureCounts {
    pub bad_signature: i64,
    pub replay: i64,
    pub clock_skew: i64,
    /// IP → tentativi (al massimo 20 per periodo).
    pub ips: BTreeMap<String, i64>,
    /// Tentativi da IP oltre i primi 20.
    pub other_ips: i64,
}

impl FailureCounts {
    pub fn total(&self) -> i64 {
        self.bad_signature + self.replay + self.clock_skew
    }

    pub fn add(&mut self, other: &FailureCounts) {
        self.bad_signature += other.bad_signature;
        self.replay += other.replay;
        self.clock_skew += other.clock_skew;
        self.other_ips += other.other_ips;
        for (ip, count) in &other.ips {
            *self.ips.entry(ip.clone()).or_default() += count;
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FailureBucket {
    pub period_start: i64,
    pub flushed_at: i64,
    #[serde(flatten)]
    pub counts: FailureCounts,
}

pub async fn insert_failure_bucket(
    pool: &Pool,
    app_id: &str,
    period_start: i64,
    flushed_at: i64,
    counts: &FailureCounts,
) -> sqlx::Result<()> {
    let ips = serde_json::to_string(&counts.ips).expect("mappa serializzabile");
    let result = sqlx::query(
        "INSERT INTO failure_buckets (id, app_id, period_start, flushed_at, bad_signature,
                                      replay, clock_skew, ips, other_ips)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(app_id)
    .bind(period_start)
    .bind(flushed_at)
    .bind(counts.bad_signature)
    .bind(counts.replay)
    .bind(counts.clock_skew)
    .bind(ips)
    .bind(counts.other_ips)
    .execute(pool)
    .await;
    match result {
        // L'app può essere stata cancellata nel frattempo: i suoi conteggi non servono più.
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => Ok(()),
        other => other.map(|_| ()),
    }
}

/// Periodi di un'app scritti dopo `flushed_after`, dal più recente.
pub async fn failure_buckets_since(
    pool: &Pool,
    app_id: &str,
    flushed_after: i64,
) -> sqlx::Result<Vec<FailureBucket>> {
    let rows: Vec<(i64, i64, i64, i64, i64, String, i64)> = sqlx::query_as(
        "SELECT period_start, flushed_at, bad_signature, replay, clock_skew, ips, other_ips
         FROM failure_buckets WHERE app_id = $1 AND flushed_at > $2
         ORDER BY flushed_at DESC",
    )
    .bind(app_id)
    .bind(flushed_after)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(period_start, flushed_at, bad_signature, replay, clock_skew, ips, other_ips)| {
            FailureBucket {
                period_start,
                flushed_at,
                counts: FailureCounts {
                    bad_signature,
                    replay,
                    clock_skew,
                    ips: serde_json::from_str(&ips).unwrap_or_default(),
                    other_ips,
                },
            }
        })
        .collect())
}

pub async fn failure_summary_since(
    pool: &Pool,
    app_id: &str,
    flushed_after: i64,
) -> sqlx::Result<FailureCounts> {
    let mut total = FailureCounts::default();
    for bucket in failure_buckets_since(pool, app_id, flushed_after).await? {
        total.add(&bucket.counts);
    }
    Ok(total)
}

/// Totali per app di un utente, per l'elenco della dashboard.
pub async fn failure_totals_for_user(
    pool: &Pool,
    user_id: &str,
    flushed_after: i64,
) -> sqlx::Result<Vec<(String, i64)>> {
    sqlx::query_as(
        "SELECT b.app_id, SUM(b.bad_signature + b.replay + b.clock_skew)
         FROM failure_buckets b JOIN apps a ON a.id = b.app_id
         WHERE a.user_id = $1 AND b.flushed_at > $2
         GROUP BY b.app_id",
    )
    .bind(user_id)
    .bind(flushed_after)
    .fetch_all(pool)
    .await
}

pub async fn delete_failure_buckets_before(pool: &Pool, before: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM failure_buckets WHERE flushed_at < $1")
        .bind(before)
        .execute(pool)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{insert_test_app, test_pool, upsert_google_user};

    #[tokio::test]
    async fn summary_merges_buckets_after_cutoff() {
        let pool = test_pool().await;
        let user = upsert_google_user(&pool, "sub", "o@example.com", None, 1).await.unwrap();
        insert_test_app(&pool, "a", &user.id).await;

        let bucket = |bad, ip: &str| FailureCounts {
            bad_signature: bad,
            ips: [(ip.to_owned(), bad)].into(),
            ..Default::default()
        };
        insert_failure_bucket(&pool, "a", 0, 10, &bucket(1, "1.1.1.1")).await.unwrap();
        insert_failure_bucket(&pool, "a", 10, 20, &bucket(2, "1.1.1.1")).await.unwrap();
        insert_failure_bucket(&pool, "a", 20, 30, &bucket(4, "2.2.2.2")).await.unwrap();

        let summary = failure_summary_since(&pool, "a", 10).await.unwrap();
        assert_eq!(summary.bad_signature, 6);
        assert_eq!(summary.ips, [("1.1.1.1".into(), 2), ("2.2.2.2".into(), 4)].into());

        // Conteggi di un'app già cancellata: ignorati senza errore.
        insert_failure_bucket(&pool, "missing", 0, 40, &bucket(1, "x")).await.unwrap();
    }
}
