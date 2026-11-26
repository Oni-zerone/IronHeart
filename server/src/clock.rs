//! Tempi in millisecondi Unix UTC, come nel database (vedi CLAUDE.md).

use std::time::{SystemTime, UNIX_EPOCH};

pub const MINUTE_MS: i64 = 60_000;
pub const DAY_MS: i64 = 24 * 60 * MINUTE_MS;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("orologio prima del 1970")
        .as_millis() as i64
}
