//! Benchmark of the database operations used by the hot paths: bulk ping
//! writes, deadline check, failed attempts, sessions and dashboard. It does
//! not check timings (they depend on the machine), it prints them.
//!
//! ```sh
//! cargo test --release db_bench -- --ignored --nocapture
//! IRONHEART_BENCH_DATABASE_URL=postgres://ironheart:ironheart@localhost:5432/ironheart_bench \
//!     cargo test --release db_bench -- --ignored --nocapture
//! ```
//!
//! Without the variable it uses a temporary SQLite file (WAL, as in production).
//! On Postgres the given database is wiped before the benchmark.

use std::time::{Duration, Instant};

use crate::db::{self, FailureCounts, PingRecord};

const USERS: usize = 20;
const APPS_PER_USER: usize = 100;
const ROUNDS: usize = 10;

fn report(label: &str, total: Duration, count: usize) {
    println!(
        "{label:<44} {:>9.2} ms/op  ({count} op, {:.0} ms total)",
        total.as_secs_f64() * 1000.0 / count as f64,
        total.as_secs_f64() * 1000.0
    );
}

async fn pool() -> db::Pool {
    match std::env::var("IRONHEART_BENCH_DATABASE_URL") {
        Ok(url) => {
            let pool = db::connect(&url).await.unwrap();
            for sql in [
                "DELETE FROM failure_buckets",
                "DELETE FROM app_recipients",
                "DELETE FROM sessions",
                "DELETE FROM apps",
                "DELETE FROM user_invites",
                "DELETE FROM users",
            ] {
                sqlx::query(sql).execute(&pool).await.unwrap();
            }
            pool
        }
        Err(_) => {
            let dir =
                std::env::temp_dir().join(format!("ironheart-bench-{}", uuid::Uuid::now_v7()));
            std::fs::create_dir_all(&dir).unwrap();
            db::connect(&format!("sqlite://{}/bench.db", dir.display()))
                .await
                .unwrap()
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark, run manually"]
async fn db_bench() {
    let pool = pool().await;
    let apps = USERS * APPS_PER_USER;

    // ---- Users, sessions and apps ----
    let started = Instant::now();
    let mut users = Vec::new();
    for u in 0..USERS {
        let user = db::upsert_google_user(
            &pool,
            &format!("sub-{u}"),
            &format!("u{u}@example.com"),
            None,
            1,
        )
        .await
        .unwrap();
        db::insert_session(&pool, &format!("token-{u}"), &user.id, 1, i64::MAX)
            .await
            .unwrap();
        users.push(user.id);
    }
    let mut app_ids = Vec::new();
    for (u, user_id) in users.iter().enumerate() {
        for a in 0..APPS_PER_USER {
            let id = format!("app-{u}-{a}");
            db::insert_app(
                &pool,
                &db::NewApp {
                    id: &id,
                    user_id,
                    name: &id,
                    key_id: &format!("k{u:03}{a:05}"),
                    secret_enc: &[7; 60],
                    secret_key_version: 1,
                    interval_secs: 60,
                    grace_secs: 0,
                    now: 1,
                },
                &[format!("ops{u}@example.com")],
            )
            .await
            .unwrap();
            app_ids.push(id);
        }
    }
    report("app insert (with recipient)", started.elapsed(), apps);

    // ---- Startup: key registry ----
    let started = Instant::now();
    for _ in 0..ROUNDS {
        assert_eq!(db::all_keys(&pool).await.unwrap().len(), apps);
    }
    report("all_keys (2,000 apps)", started.elapsed(), ROUNDS);

    // ---- Bulk pings: one spawn_writer round with all apps ----
    let mut flush_total = Duration::ZERO;
    for round in 1..=ROUNDS as i64 {
        let pings: Vec<PingRecord> = app_ids
            .iter()
            .map(|id| PingRecord {
                app_id: id.clone(),
                signed_ts: round,
                seen_at: round * 1000,
                next_deadline_at: 1_000_000,
            })
            .collect();
        let started = Instant::now();
        let accepted = db::record_pings(&pool, &pings).await.unwrap();
        flush_total += started.elapsed();
        assert!(accepted.iter().all(|ok| *ok));
    }
    report("record_pings (2,000 pings per round)", flush_total, ROUNDS);

    // ---- Check: no deadlines, then all at once ----
    let started = Instant::now();
    for _ in 0..ROUNDS {
        assert!(db::mark_overdue_down(&pool, 10).await.unwrap().is_empty());
        assert!(db::take_recoveries(&pool).await.unwrap().is_empty());
        db::failure_alert_candidates(&pool).await.unwrap();
    }
    report("check round at rest", started.elapsed(), ROUNDS);

    let started = Instant::now();
    assert_eq!(
        db::mark_overdue_down(&pool, 2_000_000).await.unwrap().len(),
        apps
    );
    report("mark_overdue_down (2,000 apps down)", started.elapsed(), 1);

    let started = Instant::now();
    for id in &app_ids {
        db::alert_recipients(&pool, id).await.unwrap();
    }
    report("alert_recipients", started.elapsed(), apps);

    // ---- Failed attempts: one round with all apps under attack ----
    let counts = FailureCounts {
        bad_signature: 100,
        ips: (0..20).map(|i| (format!("10.0.0.{i}"), 5)).collect(),
        ..Default::default()
    };
    let pending: Vec<db::PendingFailures> = app_ids
        .iter()
        .map(|id| (id.clone(), 1, counts.clone()))
        .collect();
    let started = Instant::now();
    db::insert_failure_buckets(&pool, 2, &pending)
        .await
        .unwrap();
    report(
        "insert_failure_buckets (2,000 apps, one round)",
        started.elapsed(),
        1,
    );

    let started = Instant::now();
    assert_eq!(
        db::failure_alert_candidates(&pool).await.unwrap().len(),
        apps
    );
    report(
        "failure_alert_candidates (2,000 apps)",
        started.elapsed(),
        1,
    );

    // ---- Dashboard requests ----
    let started = Instant::now();
    for i in 0..1000 {
        assert!(
            db::session_user(&pool, &format!("token-{}", i % USERS), 2)
                .await
                .unwrap()
                .is_some()
        );
    }
    report("session_user", started.elapsed(), 1000);

    let started = Instant::now();
    for i in 0..200 {
        let user_id = &users[i % USERS];
        assert_eq!(
            db::list_apps(&pool, user_id).await.unwrap().len(),
            APPS_PER_USER
        );
        db::failure_totals_for_user(&pool, user_id, 0)
            .await
            .unwrap();
    }
    report(
        "app list + attempt totals (100 apps)",
        started.elapsed(),
        200,
    );
}
