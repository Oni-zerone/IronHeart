//! Load generator for the heartbeat endpoint.
//!
//! ```sh
//! cargo run --release --example loadtest -- --url http://127.0.0.1:8080 --mode bad \
//!     --keys keys.csv --concurrency 64 --duration 30 [--spoof-ips 5000] [--valid-every 10]
//! ```
//!
//! Modes:
//! - `unknown`: nonexistent keys (the cheapest case for the server);
//! - `bad`: existing keys with a wrong signature (counted as failed attempts);
//! - `valid`: valid pings, at most one per second per key;
//! - `mixed`: `bad` attack with all the concurrency and, in parallel, a valid ping
//!   for every key every `--valid-every` seconds (do the real pings survive?).
//!
//! `--keys` is a CSV with `key_id,secret` per line. `--spoof-ips N` adds
//! `X-Forwarded-For` with N different addresses (the server must be started with
//! `CLIENT_IP_HEADER=x-forwarded-for`).

use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

const EMPTY_BODY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Unknown,
    Bad,
    Valid,
    Mixed,
}

struct Opts {
    url: String,
    mode: Mode,
    concurrency: usize,
    duration: Duration,
    keys: Vec<(String, String)>,
    spoof_ips: usize,
    valid_every: Duration,
}

fn parse() -> Opts {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let mode = match get("--mode").as_deref().unwrap_or("unknown") {
        "unknown" => Mode::Unknown,
        "bad" => Mode::Bad,
        "valid" => Mode::Valid,
        "mixed" => Mode::Mixed,
        other => panic!("unknown mode: {other}"),
    };
    let keys = get("--keys")
        .map(|path| {
            std::fs::read_to_string(path)
                .expect("keys file")
                .lines()
                .filter_map(|l| l.split_once(','))
                .map(|(k, s)| (k.trim().to_owned(), s.trim().to_owned()))
                .collect()
        })
        .unwrap_or_default();
    let opts = Opts {
        url: format!("{}/api/v1/heartbeat", get("--url").expect("--url").trim_end_matches('/')),
        mode,
        concurrency: get("--concurrency").map_or(64, |v| v.parse().unwrap()),
        duration: Duration::from_secs(get("--duration").map_or(30, |v| v.parse().unwrap())),
        keys,
        spoof_ips: get("--spoof-ips").map_or(0, |v| v.parse().unwrap()),
        valid_every: Duration::from_secs(get("--valid-every").map_or(10, |v| v.parse().unwrap())),
    };
    if mode != Mode::Unknown && opts.keys.is_empty() {
        panic!("this mode requires --keys");
    }
    opts
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

fn signature(secret: &str, key_id: &str, ts: i64) -> String {
    let canonical = format!("v1\nPOST\n/api/v1/heartbeat\n{key_id}\n{ts}\n{EMPTY_BODY_SHA256}");
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(canonical.as_bytes());
    format!("v1={}", hex::encode(mac.finalize().into_bytes()))
}

#[derive(Default)]
struct Stats {
    latencies_us: Vec<u32>,
    non_204: u64,
    errors: u64,
}

impl Stats {
    fn merge(&mut self, other: Stats) {
        self.latencies_us.extend(other.latencies_us);
        self.non_204 += other.non_204;
        self.errors += other.errors;
    }

    fn report(mut self, label: &str, elapsed: Duration) {
        self.latencies_us.sort_unstable();
        let n = self.latencies_us.len();
        let pct = |p: f64| {
            if n == 0 { 0.0 } else { self.latencies_us[((n as f64 * p) as usize).min(n - 1)] as f64 / 1000.0 }
        };
        println!(
            "{label:<10} requests {n:>8}  {:>8.0}/s  p50 {:>6.2} ms  p95 {:>6.2} ms  p99 {:>7.2} ms  max {:>7.2} ms  non-204 {}  errors {}",
            n as f64 / elapsed.as_secs_f64(),
            pct(0.50),
            pct(0.95),
            pct(0.99),
            pct(1.0),
            self.non_204,
            self.errors,
        );
    }
}

async fn send(
    client: &reqwest::Client,
    url: &str,
    key_id: &str,
    ts: i64,
    sig: &str,
    ip: Option<String>,
    stats: &mut Stats,
) {
    let mut request = client
        .post(url)
        .header("x-ironheart-key", key_id)
        .header("x-ironheart-timestamp", ts.to_string())
        .header("x-ironheart-signature", sig);
    if let Some(ip) = ip {
        request = request.header("x-forwarded-for", ip);
    }
    let start = Instant::now();
    match request.send().await {
        Ok(response) => {
            stats.latencies_us.push(start.elapsed().as_micros() as u32);
            if response.status().as_u16() != 204 {
                stats.non_204 += 1;
            }
        }
        Err(_) => stats.errors += 1,
    }
}

#[tokio::main]
async fn main() {
    let opts = Arc::new(parse());
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(opts.concurrency * 2)
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let deadline = Instant::now() + opts.duration;
    let started = Instant::now();
    let next_key = Arc::new(AtomicUsize::new(0));
    let last_ts: Arc<Vec<AtomicI64>> = Arc::new((0..opts.keys.len()).map(|_| AtomicI64::new(0)).collect());

    let flood_mode = if opts.mode == Mode::Mixed { Mode::Bad } else { opts.mode };
    let mut workers = Vec::new();
    for worker in 0..opts.concurrency {
        let (opts, client, next_key, last_ts) = (opts.clone(), client.clone(), next_key.clone(), last_ts.clone());
        workers.push(tokio::spawn(async move {
            let mut stats = Stats::default();
            let mut i: u64 = worker as u64;
            while Instant::now() < deadline {
                i = i.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let ip = (opts.spoof_ips > 0)
                    .then(|| format!("10.{}.{}.{}", (i >> 40) % 256, (i >> 32) % 256, (i >> 24) as usize % opts.spoof_ips % 256));
                let ts = now_secs();
                match flood_mode {
                    Mode::Unknown => {
                        let key = format!("{:012x}", i >> 16);
                        send(&client, &opts.url, &key, ts, &format!("v1={:064x}", i), ip, &mut stats).await;
                    }
                    Mode::Bad => {
                        let (key, _) = &opts.keys[(i >> 20) as usize % opts.keys.len()];
                        send(&client, &opts.url, key, ts, &format!("v1={:064x}", i), ip, &mut stats).await;
                    }
                    _ => {
                        // A key that is free in this second, otherwise wait a moment.
                        let n = opts.keys.len();
                        let mut sent = false;
                        for _ in 0..n {
                            let k = next_key.fetch_add(1, Ordering::Relaxed) % n;
                            if last_ts[k].fetch_max(ts, Ordering::SeqCst) < ts {
                                let (key, secret) = &opts.keys[k];
                                send(&client, &opts.url, key, ts, &signature(secret, key, ts), ip.clone(), &mut stats).await;
                                sent = true;
                                break;
                            }
                        }
                        if !sent {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    }
                }
            }
            stats
        }));
    }

    // Real pings during the attack: one per key every `valid_every`.
    let legit = (opts.mode == Mode::Mixed).then(|| {
        let (opts, client) = (opts.clone(), client.clone());
        tokio::spawn(async move {
            let mut stats = Stats::default();
            let mut rounds = 0u64;
            let mut last = 0i64;
            while Instant::now() < deadline {
                let round_start = Instant::now();
                let mut ts = now_secs().max(last + 1);
                for chunk in opts.keys.chunks(32) {
                    let futures = chunk.iter().map(|(key, secret)| {
                        let client = client.clone();
                        let url = opts.url.clone();
                        let sig = signature(secret, key, ts);
                        let key = key.clone();
                        async move {
                            let mut s = Stats::default();
                            send(&client, &url, &key, ts, &sig, None, &mut s).await;
                            s
                        }
                    });
                    for s in futures_join(futures).await {
                        stats.merge(s);
                    }
                }
                last = ts;
                rounds += 1;
                let elapsed = round_start.elapsed();
                if elapsed < opts.valid_every {
                    tokio::time::sleep(opts.valid_every - elapsed).await;
                }
                ts = now_secs();
                let _ = ts;
            }
            (stats, rounds)
        })
    });

    let mut total = Stats::default();
    for worker in workers {
        total.merge(worker.await.unwrap());
    }
    let elapsed = started.elapsed();
    let label = match flood_mode {
        Mode::Unknown => "unknown",
        Mode::Bad => "forged",
        _ => "valid",
    };
    total.report(label, elapsed);
    if let Some(legit) = legit {
        let (stats, rounds) = legit.await.unwrap();
        println!("real pings sent: {} rounds × {} keys", rounds, opts.keys.len());
        stats.report("real", elapsed);
    }
}

/// Runs the futures in parallel without extra dependencies.
async fn futures_join<F: std::future::Future<Output = Stats> + Send + 'static>(
    futures: impl Iterator<Item = F>,
) -> Vec<Stats> {
    let handles: Vec<_> = futures.map(tokio::spawn).collect();
    let mut out = Vec::new();
    for handle in handles {
        out.push(handle.await.unwrap());
    }
    out
}
