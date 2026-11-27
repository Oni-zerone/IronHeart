//! Heartbeat endpoint: always answers 204, the same way for every request.
//!
//! App secrets live in memory (decrypted at startup), so a check costs a map
//! lookup and an HMAC, without the database. Valid pings are collected in
//! memory (the latest per app) and written in bulk once per second; failed
//! attempts are summed in memory and the periodic check writes them (see
//! `monitor`).

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
};

use crate::{
    AppState,
    clock::now_ms,
    crypto::{self, SecretBox},
    db::{self, FailureCounts, KeyRow, PingRecord},
};

/// How often the collected valid pings are written.
const PING_FLUSH_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

pub const KEY_HEADER: &str = "x-ironheart-key";
pub const TIMESTAMP_HEADER: &str = "x-ironheart-timestamp";
pub const SIGNATURE_HEADER: &str = "x-ironheart-signature";

const MAX_BODY_BYTES: usize = 1024;
/// Maximum distance between the signed timestamp and the server's clock.
const MAX_CLOCK_SKEW_SECS: i64 = 5 * 60;
/// Distinct IPs kept per app between two writes.
const MAX_IPS_PER_APP: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    BadSignature,
    Replay,
    ClockSkew,
}

struct KeyEntry {
    app_id: String,
    secret: Vec<u8>,
    interval_secs: AtomicI64,
    grace_secs: AtomicI64,
    /// Last accepted timestamp; `i64::MIN` if none.
    last_signed_ts: AtomicI64,
}

#[derive(Default)]
struct Keys {
    by_key_id: HashMap<String, Arc<KeyEntry>>,
    key_id_by_app: HashMap<String, String>,
}

struct PendingFailures {
    since: i64,
    counts: FailureCounts,
}

pub struct Heartbeat {
    secret_box: SecretBox,
    keys: RwLock<Keys>,
    failures: Mutex<HashMap<String, PendingFailures>>,
    /// Valid pings to write: per app the latest one is enough.
    pending_pings: Mutex<HashMap<String, PingRecord>>,
    unattributed: AtomicU64,
    /// Key used when the `key_id` does not exist, to compute an HMAC anyway.
    dummy_secret: Vec<u8>,
    client_ip_header: Option<String>,
}

pub enum Outcome {
    Valid { app_id: String, signed_ts: i64, deadline_after_ms: i64 },
    Rejected { app_id: String, reason: Reason },
    Unattributed,
}

impl Heartbeat {
    pub async fn load(
        pool: &db::Pool,
        secret_box: SecretBox,
        client_ip_header: Option<String>,
    ) -> anyhow::Result<Self> {
        let heartbeat = Self {
            secret_box,
            keys: RwLock::default(),
            failures: Mutex::default(),
            pending_pings: Mutex::default(),
            unattributed: AtomicU64::new(0),
            dummy_secret: crypto::generate_secret().into_bytes(),
            client_ip_header,
        };
        let rows = db::all_keys(pool).await?;
        let total = rows.len();
        for row in rows {
            let app_id = row.id.clone();
            if let Err(err) = heartbeat.insert_row(row) {
                tracing::error!(%app_id, err = ?err, "app key not loaded");
            }
        }
        tracing::info!("keys loaded: {total}");
        Ok(heartbeat)
    }

    pub fn secret_box(&self) -> &SecretBox {
        &self.secret_box
    }

    fn insert_row(&self, row: KeyRow) -> anyhow::Result<()> {
        let secret = self.secret_box.open(&row.secret_enc, row.secret_key_version, &row.id)?;
        let entry = Arc::new(KeyEntry {
            app_id: row.id.clone(),
            secret,
            interval_secs: AtomicI64::new(row.interval_secs),
            grace_secs: AtomicI64::new(row.grace_secs),
            last_signed_ts: AtomicI64::new(row.last_signed_ts.unwrap_or(i64::MIN)),
        });
        let mut keys = self.keys.write().expect("keys lock");
        if let Some(old) = keys.key_id_by_app.insert(row.id, row.key_id.clone()) {
            keys.by_key_id.remove(&old);
        }
        keys.by_key_id.insert(row.key_id, entry);
        Ok(())
    }

    /// Reloads an app's key from the database (after creation or rotation).
    pub async fn refresh_app(&self, pool: &db::Pool, app_id: &str) -> anyhow::Result<()> {
        match db::app_key(pool, app_id).await? {
            Some(row) => self.insert_row(row),
            None => {
                self.remove_app(app_id);
                Ok(())
            }
        }
    }

    pub fn remove_app(&self, app_id: &str) {
        let mut keys = self.keys.write().expect("keys lock");
        if let Some(key_id) = keys.key_id_by_app.remove(app_id) {
            keys.by_key_id.remove(&key_id);
        }
    }

    pub fn set_timing(&self, app_id: &str, interval_secs: i64, grace_secs: i64) {
        let keys = self.keys.read().expect("keys lock");
        if let Some(entry) = keys.key_id_by_app.get(app_id).and_then(|k| keys.by_key_id.get(k)) {
            entry.interval_secs.store(interval_secs, Ordering::Relaxed);
            entry.grace_secs.store(grace_secs, Ordering::Relaxed);
        }
    }

    /// Checks a request. Does the same work (lookup + HMAC) even
    /// when the `key_id` does not exist.
    pub fn check(
        &self,
        key_id: Option<&str>,
        timestamp: Option<&str>,
        signature: Option<&str>,
        body: Option<&[u8]>,
        now_secs: i64,
    ) -> Outcome {
        let key_id = key_id.unwrap_or("");
        let timestamp = timestamp.unwrap_or("");
        let entry = self.keys.read().expect("keys lock").by_key_id.get(key_id).cloned();

        let secret = entry.as_ref().map_or(self.dummy_secret.as_slice(), |e| e.secret.as_slice());
        let body_hash = crypto::sha256_hex(body.unwrap_or_default());
        let canonical = crypto::canonical_string(key_id, timestamp, &body_hash);
        let signature = parse_signature(signature).unwrap_or([0; 32]);
        let signature_ok = crypto::verify(secret, &canonical, &signature) && body.is_some();

        let Some(entry) = entry else {
            return Outcome::Unattributed;
        };
        let rejected = |reason| Outcome::Rejected { app_id: entry.app_id.clone(), reason };
        if !signature_ok {
            return rejected(Reason::BadSignature);
        }
        let Ok(signed_ts) = timestamp.parse::<i64>() else {
            return rejected(Reason::BadSignature);
        };
        if (now_secs - signed_ts).abs() > MAX_CLOCK_SKEW_SECS {
            return rejected(Reason::ClockSkew);
        }
        // Atomic: of two requests with the same timestamp, only one passes.
        if entry.last_signed_ts.fetch_max(signed_ts, Ordering::SeqCst) >= signed_ts {
            return rejected(Reason::Replay);
        }
        let window = entry.interval_secs.load(Ordering::Relaxed) + entry.grace_secs.load(Ordering::Relaxed);
        Outcome::Valid { app_id: entry.app_id.clone(), signed_ts, deadline_after_ms: window * 1000 }
    }

    pub fn record_failure(&self, app_id: &str, reason: Reason, ip: Option<String>, now: i64) {
        let mut failures = self.failures.lock().expect("failures lock");
        let pending = failures
            .entry(app_id.to_owned())
            .or_insert_with(|| PendingFailures { since: now, counts: FailureCounts::default() });
        let counts = &mut pending.counts;
        match reason {
            Reason::BadSignature => counts.bad_signature += 1,
            Reason::Replay => counts.replay += 1,
            Reason::ClockSkew => counts.clock_skew += 1,
        }
        match ip {
            Some(ip) if counts.ips.contains_key(&ip) || counts.ips.len() < MAX_IPS_PER_APP => {
                *counts.ips.entry(ip).or_default() += 1;
            }
            _ => counts.other_ips += 1,
        }
    }

    /// Attempts accumulated since the last call: (app, period start, counts).
    pub fn take_failures(&self) -> Vec<(String, i64, FailureCounts)> {
        let failures = std::mem::take(&mut *self.failures.lock().expect("failures lock"));
        failures.into_iter().map(|(app, p)| (app, p.since, p.counts)).collect()
    }

    fn queue_ping(&self, ping: PingRecord) {
        let mut pending = self.pending_pings.lock().expect("pings lock");
        match pending.get(&ping.app_id) {
            Some(existing) if existing.signed_ts >= ping.signed_ts => {}
            _ => {
                pending.insert(ping.app_id.clone(), ping);
            }
        }
    }

    /// Writes the collected pings in one transaction. Those the database
    /// rejects (timestamp not new) are counted as replays.
    pub async fn flush_pings(&self, pool: &db::Pool) -> anyhow::Result<usize> {
        let pings: Vec<PingRecord> =
            std::mem::take(&mut *self.pending_pings.lock().expect("pings lock")).into_values().collect();
        if pings.is_empty() {
            return Ok(0);
        }
        let accepted = db::record_pings(pool, &pings).await?;
        for (ping, ok) in pings.iter().zip(&accepted) {
            if !ok {
                self.record_failure(&ping.app_id, Reason::Replay, None, ping.seen_at);
            }
        }
        Ok(pings.len())
    }

    pub fn take_unattributed(&self) -> u64 {
        self.unattributed.swap(0, Ordering::Relaxed)
    }

    fn client_ip(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<String> {
        match self.client_ip_header.as_deref() {
            // nginx (CapRover) appends the address it receives the request from:
            // the values before it are written by the client and are not trustworthy.
            Some("x-forwarded-for") => headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit(',').next())
                .map(|ip| ip.trim().to_owned()),
            Some(name) => headers.get(name).and_then(|v| v.to_str().ok()).map(|v| v.trim().to_owned()),
            None => peer.map(|p| p.ip().to_string()),
        }
        .filter(|ip| !ip.is_empty() && ip.len() <= 64)
    }
}

/// Writes valid pings once per second.
pub fn spawn_writer(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(PING_FLUSH_EVERY);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(err) = state.heartbeat.flush_pings(&state.db).await {
                tracing::error!(err = ?err, "pings not recorded");
            }
        }
    });
}

fn parse_signature(header: Option<&str>) -> Option<[u8; 32]> {
    let hex_sig = header?.trim().strip_prefix("v1=")?;
    let mut out = [0u8; 32];
    hex::decode_to_slice(hex_sig, &mut out).ok()?;
    Some(out)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

pub async fn handler(State(state): State<AppState>, request: Request) -> StatusCode {
    let (parts, body) = request.into_parts();
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let body = axum::body::to_bytes(Body::new(body), MAX_BODY_BYTES).await.ok();

    let heartbeat = &state.heartbeat;
    let now = now_ms();
    let outcome = heartbeat.check(
        header(&parts.headers, KEY_HEADER),
        header(&parts.headers, TIMESTAMP_HEADER),
        header(&parts.headers, SIGNATURE_HEADER),
        body.as_deref(),
        now / 1000,
    );
    match outcome {
        Outcome::Valid { app_id, signed_ts, deadline_after_ms } => {
            // Memory only: a valid ping must not be answered more slowly than a forged one.
            heartbeat.queue_ping(PingRecord {
                app_id,
                signed_ts,
                seen_at: now,
                next_deadline_at: now + deadline_after_ms,
            });
        }
        Outcome::Rejected { app_id, reason } => {
            let ip = heartbeat.client_ip(&parts.headers, peer);
            heartbeat.record_failure(&app_id, reason, ip, now);
        }
        Outcome::Unattributed => {
            heartbeat.unattributed.fetch_add(1, Ordering::Relaxed);
        }
    }
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{insert_app, test_pool, upsert_google_user, NewApp};

    const NOW: i64 = 1_764_000_000;

    async fn setup() -> (Heartbeat, String) {
        let (hb, secret, _) = setup_with_pool().await;
        (hb, secret)
    }

    async fn setup_with_pool() -> (Heartbeat, String, db::Pool) {
        let pool = test_pool().await;
        let user = upsert_google_user(&pool, "sub", "o@example.com", None, 1).await.unwrap();
        let secret_box = SecretBox::new(&[1; 32], 1);
        let secret = "ihs_segreto".to_owned();
        insert_app(
            &pool,
            &NewApp {
                id: "app",
                user_id: &user.id,
                name: "App",
                key_id: "key",
                secret_enc: &secret_box.seal(secret.as_bytes(), "app"),
                secret_key_version: 1,
                interval_secs: 60,
                grace_secs: 30,
                now: 1,
            },
            &[],
        )
        .await
        .unwrap();
        (Heartbeat::load(&pool, secret_box, None).await.unwrap(), secret, pool)
    }

    fn signed(secret: &str, ts: i64, body: &[u8]) -> String {
        let canonical = crypto::canonical_string("key", &ts.to_string(), &crypto::sha256_hex(body));
        format!("v1={}", hex::encode(crypto::sign(secret.as_bytes(), &canonical)))
    }

    fn check(hb: &Heartbeat, key: &str, ts: i64, sig: &str) -> Outcome {
        hb.check(Some(key), Some(&ts.to_string()), Some(sig), Some(b""), NOW)
    }

    fn reason(outcome: Outcome) -> Option<Reason> {
        match outcome {
            Outcome::Rejected { reason, .. } => Some(reason),
            _ => None,
        }
    }

    #[tokio::test]
    async fn valid_ping_then_replay_is_rejected() {
        let (hb, secret) = setup().await;
        let sig = signed(&secret, NOW, b"");
        match check(&hb, "key", NOW, &sig) {
            Outcome::Valid { app_id, signed_ts, deadline_after_ms } => {
                assert_eq!((app_id.as_str(), signed_ts, deadline_after_ms), ("app", NOW, 90_000));
            }
            _ => panic!("expected valid"),
        }
        assert_eq!(reason(check(&hb, "key", NOW, &sig)), Some(Reason::Replay));
        let older = signed(&secret, NOW - 1, b"");
        assert_eq!(reason(check(&hb, "key", NOW - 1, &older)), Some(Reason::Replay));
        let newer = signed(&secret, NOW + 1, b"");
        assert!(matches!(check(&hb, "key", NOW + 1, &newer), Outcome::Valid { .. }));
    }

    #[tokio::test]
    async fn rejections_are_classified() {
        let (hb, secret) = setup().await;
        assert_eq!(reason(check(&hb, "key", NOW, &signed("ihs_altro", NOW, b""))), Some(Reason::BadSignature));
        assert_eq!(reason(check(&hb, "key", NOW, "v1=zz")), Some(Reason::BadSignature));
        let late = NOW - MAX_CLOCK_SKEW_SECS - 1;
        assert_eq!(reason(check(&hb, "key", late, &signed(&secret, late, b""))), Some(Reason::ClockSkew));
        // Body different from the signed one, or too large (None).
        let sig = signed(&secret, NOW, b"");
        let tampered = hb.check(Some("key"), Some(&NOW.to_string()), Some(&sig), Some(b"x"), NOW);
        assert_eq!(reason(tampered), Some(Reason::BadSignature));
        let too_big = hb.check(Some("key"), Some(&NOW.to_string()), Some(&sig), None, NOW);
        assert_eq!(reason(too_big), Some(Reason::BadSignature));
        assert!(matches!(check(&hb, "sconosciuta", NOW, &sig), Outcome::Unattributed));
    }

    #[tokio::test]
    async fn rotated_key_replaces_the_old_one() {
        let (hb, secret, pool) = setup_with_pool().await;
        let sealed = hb.secret_box().seal(b"ihs_new", "app");
        sqlx::query("UPDATE apps SET key_id = 'key2', secret_enc = $1, last_signed_ts = NULL")
            .bind(sealed)
            .execute(&pool)
            .await
            .unwrap();
        hb.refresh_app(&pool, "app").await.unwrap();

        assert!(matches!(check(&hb, "key", NOW, &signed(&secret, NOW, b"")), Outcome::Unattributed));
        let canonical = crypto::canonical_string("key2", &NOW.to_string(), &crypto::sha256_hex(b""));
        let sig = format!("v1={}", hex::encode(crypto::sign(b"ihs_new", &canonical)));
        assert!(matches!(check(&hb, "key2", NOW, &sig), Outcome::Valid { .. }));

        hb.remove_app("app");
        assert!(matches!(check(&hb, "key2", NOW + 1, &sig), Outcome::Unattributed));
    }

    #[tokio::test]
    async fn queued_pings_keep_the_latest_and_flush_in_one_go() {
        let (hb, _, pool) = setup_with_pool().await;
        let ping = |ts| PingRecord { app_id: "app".into(), signed_ts: ts, seen_at: ts * 1000, next_deadline_at: ts * 1000 + 90_000 };
        hb.queue_ping(ping(NOW + 1));
        hb.queue_ping(ping(NOW));
        assert_eq!(hb.flush_pings(&pool).await.unwrap(), 1);
        let (ts,): (i64,) = sqlx::query_as("SELECT last_signed_ts FROM apps").fetch_one(&pool).await.unwrap();
        assert_eq!(ts, NOW + 1);
        assert_eq!(hb.flush_pings(&pool).await.unwrap(), 0);

        // A timestamp already written is rejected by the database and counted as a replay.
        hb.queue_ping(ping(NOW + 1));
        hb.flush_pings(&pool).await.unwrap();
        assert_eq!(hb.take_failures()[0].2.replay, 1);
    }

    #[tokio::test]
    async fn failures_are_aggregated_with_capped_ips() {
        let (hb, _) = setup().await;
        for i in 0..25 {
            hb.record_failure("app", Reason::BadSignature, Some(format!("10.0.0.{i}")), 5);
        }
        hb.record_failure("app", Reason::Replay, Some("10.0.0.1".into()), 6);
        let taken = hb.take_failures();
        let (_, since, counts) = &taken[0];
        assert_eq!(*since, 5);
        assert_eq!((counts.bad_signature, counts.replay), (25, 1));
        assert_eq!(counts.ips.len(), MAX_IPS_PER_APP);
        assert_eq!(counts.ips["10.0.0.1"], 2);
        assert_eq!(counts.other_ips, 5);
        assert!(hb.take_failures().is_empty());
    }

    #[test]
    fn client_ip_uses_last_forwarded_entry() {
        let hb = Heartbeat {
            secret_box: SecretBox::new(&[0; 32], 1),
            keys: RwLock::default(),
            failures: Mutex::default(),
            pending_pings: Mutex::default(),
            unattributed: AtomicU64::new(0),
            dummy_secret: vec![],
            client_ip_header: Some("x-forwarded-for".into()),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "6.6.6.6, 203.0.113.9".parse().unwrap());
        assert_eq!(hb.client_ip(&headers, None).as_deref(), Some("203.0.113.9"));
    }
}
