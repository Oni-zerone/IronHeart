//! Heartbeat load test, inside the test suite.
//!
//! The server runs on a local port with the in-memory database. While many
//! workers send forged signatures (from different IPs), another task sends a
//! valid ping for every app every second. It checks that:
//! - every response is 204;
//! - all real pings are recorded;
//! - every forged attempt is counted, without writes to the database;
//! - IPs stay limited to 20 per app.
//!
//! Speed numbers are printed but not checked (they depend on the machine):
//! `IRONHEART_LOAD_SECS=30 cargo test --release load_tests -- --nocapture`.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{Router, routing::post};

use crate::{
    clock::now_ms,
    crypto::{self, HEARTBEAT_PATH, SecretBox},
    db,
    heartbeat::{self, Heartbeat},
    test_state,
};

const APPS: usize = 200;
const FLOOD_WORKERS: usize = 32;

fn load_duration() -> Duration {
    let secs = std::env::var("IRONHEART_LOAD_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    Duration::from_secs(secs)
}

struct Key {
    key_id: String,
    secret: String,
}

#[derive(Default)]
struct Counters {
    sent: AtomicU64,
    non_204: AtomicU64,
    errors: AtomicU64,
    max_latency_us: AtomicU64,
}

impl Counters {
    async fn send(&self, request: reqwest::RequestBuilder) {
        let start = Instant::now();
        match request.send().await {
            Ok(response) => {
                self.sent.fetch_add(1, Ordering::Relaxed);
                if response.status().as_u16() != 204 {
                    self.non_204.fetch_add(1, Ordering::Relaxed);
                }
                self.max_latency_us
                    .fetch_max(start.elapsed().as_micros() as u64, Ordering::Relaxed);
            }
            Err(_) => {
                self.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn request(
    client: &reqwest::Client,
    url: &str,
    key_id: &str,
    ts: i64,
    signature: &str,
) -> reqwest::RequestBuilder {
    client
        .post(url)
        .header(heartbeat::KEY_HEADER, key_id)
        .header(heartbeat::TIMESTAMP_HEADER, ts.to_string())
        .header(heartbeat::SIGNATURE_HEADER, signature)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn heartbeat_survives_a_flood_of_forged_requests() {
    // ---- Data: APPS apps with known keys ----
    let pool = db::test_pool().await;
    let user = db::upsert_google_user(&pool, "sub", "load@example.com", None, 1)
        .await
        .unwrap();
    let secret_box = SecretBox::new(&[9; 32], 1);
    let mut keys = Vec::new();
    for i in 0..APPS {
        let (id, key_id, secret) = (
            format!("app-{i}"),
            format!("key{i:05}"),
            format!("ihs_secret_{i}"),
        );
        db::insert_app(
            &pool,
            &db::NewApp {
                id: &id,
                user_id: &user.id,
                name: &id,
                key_id: &key_id,
                secret_enc: &secret_box.seal(secret.as_bytes(), &id),
                secret_key_version: 1,
                interval_secs: 60,
                grace_secs: 0,
                now: 1,
            },
            &[],
        )
        .await
        .unwrap();
        keys.push(Key { key_id, secret });
    }
    let keys = Arc::new(keys);
    let hb = Arc::new(
        Heartbeat::load(&pool, secret_box, Some("x-forwarded-for".into()))
            .await
            .unwrap(),
    );

    // ---- Server on a local port ----
    let state = test_state(pool.clone(), hb.clone());
    heartbeat::spawn_writer(state.clone());
    let app = Router::new()
        .route(HEARTBEAT_PATH, post(heartbeat::handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}{HEARTBEAT_PATH}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(256)
        .build()
        .unwrap();
    let duration = load_duration();
    let deadline = Instant::now() + duration;
    let started = Instant::now();
    let forged = Arc::new(Counters::default());
    let legit = Arc::new(Counters::default());

    // ---- Attack: forged signatures on existing keys, from many IPs ----
    let mut tasks = Vec::new();
    for worker in 0..FLOOD_WORKERS {
        let (client, url, keys, forged) =
            (client.clone(), url.clone(), keys.clone(), forged.clone());
        tasks.push(tokio::spawn(async move {
            let mut i = worker as u64;
            while Instant::now() < deadline {
                i = i
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let key = &keys[(i >> 33) as usize % APPS];
                let ip = format!(
                    "10.{}.{}.{}",
                    (i >> 16) % 256,
                    (i >> 24) % 256,
                    (i >> 40) % 256
                );
                let signature = format!("v1={i:064x}");
                let ts = now_ms() / 1000;
                forged
                    .send(
                        request(&client, &url, &key.key_id, ts, &signature)
                            .header("x-forwarded-for", ip),
                    )
                    .await;
            }
        }));
    }

    // ---- Real pings: one per app every second, always with a new timestamp ----
    let legit_task = {
        let (client, url, keys, legit) = (client.clone(), url.clone(), keys.clone(), legit.clone());
        tokio::spawn(async move {
            let mut last_ts = 0;
            let mut rounds = 0;
            while Instant::now() < deadline {
                let ts = now_ms() / 1000;
                if ts <= last_ts {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    continue;
                }
                let sends: Vec<_> = keys
                    .iter()
                    .map(|key| {
                        let canonical = crypto::canonical_string(
                            &key.key_id,
                            &ts.to_string(),
                            &crypto::sha256_hex(b""),
                        );
                        let signature = format!(
                            "v1={}",
                            hex::encode(crypto::sign(key.secret.as_bytes(), &canonical))
                        );
                        let (legit, request) = (
                            legit.clone(),
                            request(&client, &url, &key.key_id, ts, &signature),
                        );
                        tokio::spawn(async move { legit.send(request).await })
                    })
                    .collect();
                for send in sends {
                    send.await.unwrap();
                }
                last_ts = ts;
                rounds += 1;
            }
            (last_ts, rounds)
        })
    };

    for task in tasks {
        task.await.unwrap();
    }
    let (last_ts, rounds) = legit_task.await.unwrap();
    let elapsed = started.elapsed();

    // Valid pings are written in a separate task: wait for all of them to land.
    let mut recorded = 0;
    for _ in 0..100 {
        recorded =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM apps WHERE last_signed_ts = $1")
                .bind(last_ts)
                .fetch_one(&pool)
                .await
                .unwrap();
        if recorded == APPS as i64 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let failures = hb.take_failures();
    let forged_sent = forged.sent.load(Ordering::Relaxed);
    let counted: i64 = failures.iter().map(|(_, _, c)| c.bad_signature).sum();
    let replays: i64 = failures
        .iter()
        .map(|(_, _, c)| c.replay + c.clock_skew)
        .sum();
    let max_ips = failures
        .iter()
        .map(|(_, _, c)| c.ips.len())
        .max()
        .unwrap_or(0);
    let buckets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM failure_buckets")
        .fetch_one(&pool)
        .await
        .unwrap();

    let rate = |n: u64| n as f64 / elapsed.as_secs_f64();
    println!(
        "forged: {forged_sent} ({:.0}/s, max {:.1} ms) · real: {} in {rounds} rounds ({:.0}/s, max {:.1} ms) · recorded {recorded}/{APPS}",
        rate(forged_sent),
        forged.max_latency_us.load(Ordering::Relaxed) as f64 / 1000.0,
        legit.sent.load(Ordering::Relaxed),
        rate(legit.sent.load(Ordering::Relaxed)),
        legit.max_latency_us.load(Ordering::Relaxed) as f64 / 1000.0,
    );

    assert_eq!(
        forged.errors.load(Ordering::Relaxed) + legit.errors.load(Ordering::Relaxed),
        0,
        "network errors"
    );
    assert_eq!(
        forged.non_204.load(Ordering::Relaxed) + legit.non_204.load(Ordering::Relaxed),
        0,
        "responses other than 204"
    );
    assert!(rounds >= 1, "no round of real pings");
    assert_eq!(legit.sent.load(Ordering::Relaxed), rounds * APPS as u64);
    assert_eq!(recorded, APPS as i64, "real pings lost during the attack");
    assert_eq!(counted as u64, forged_sent, "forged attempts not counted");
    assert_eq!(replays, 0, "real pings mistaken for replays");
    assert!(max_ips <= 20, "IPs per app over the limit: {max_ips}");
    assert_eq!(buckets, 0, "database writes for invalid requests");
}
