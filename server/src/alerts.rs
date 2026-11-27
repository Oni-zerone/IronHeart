//! Alert email texts. Times are in UTC.
//!
//! On each round of the check every recipient gets at most one email per
//! type: with several apps involved, a single summary (`*_digest`).

use time::{OffsetDateTime, macros::format_description};

use crate::{db::FailureCounts, mail::Email};

/// Apps listed in full in a summary; the others are only counted.
const DIGEST_LIST_MAX: usize = 50;

#[derive(Clone)]
pub struct DownItem {
    pub name: String,
    pub url: String,
    pub last_seen_at: Option<i64>,
    /// Invalid attempts since the last signal.
    pub failures: i64,
}

#[derive(Clone)]
pub struct RecoveredItem {
    pub name: String,
    pub url: String,
    pub down_since: Option<i64>,
    pub last_seen_at: Option<i64>,
}

#[derive(Clone)]
pub struct FailureItem {
    pub app_id: String,
    pub name: String,
    pub url: String,
    pub counts: FailureCounts,
    pub since: Option<i64>,
    pub next_report_in_ms: i64,
}

pub fn down_digest(to: &str, items: &[DownItem], now: i64, apps_url: &str) -> Email {
    if let [item] = items {
        let app = AppRef { name: &item.name, url: item.url.clone() };
        return down(&app, to, item.last_seen_at, now, item.failures);
    }
    let mut lines = vec![format!("{} applications did not send their heartbeat within the expected interval:", items.len())];
    lines.extend(list(items, |item| {
        let seen = item.last_seen_at.map_or("never a signal".to_owned(), |at| format!("last signal {} ago", duration(now - at)));
        format!("• {} — {seen}", item.name)
    }));
    let failures: i64 = items.iter().map(|i| i.failures).sum();
    if failures > 0 {
        lines.push(format!(
            "In the same period {failures} invalid attempts arrived: the pings may not be \
             reaching the service because of an attack."
        ));
    }
    build(to, format!("IronHeart: {} applications are not responding", items.len()), lines, apps_url)
}

pub fn recovered_digest(to: &str, items: &[RecoveredItem], apps_url: &str) -> Email {
    if let [item] = items {
        let app = AppRef { name: &item.name, url: item.url.clone() };
        return recovered(&app, to, item.down_since, item.last_seen_at);
    }
    let mut lines = vec![format!("{} applications have resumed sending their heartbeat:", items.len())];
    lines.extend(list(items, |item| match (item.down_since, item.last_seen_at) {
        (Some(from), Some(to_at)) => format!("• {} — down for {}", item.name, duration(to_at - from)),
        _ => format!("• {}", item.name),
    }));
    build(to, format!("IronHeart: {} applications are back up", items.len()), lines, apps_url)
}

pub fn failures_digest(to: &str, items: &[FailureItem], min_interval_ms: i64, apps_url: &str) -> Email {
    if let [item] = items {
        let app = AppRef { name: &item.name, url: item.url.clone() };
        let next = item.next_report_in_ms.max(min_interval_ms);
        return failures(&app, to, &item.counts, item.since, next);
    }
    let mut total = FailureCounts::default();
    for item in items {
        total.add(&item.counts);
    }
    let mut sorted: Vec<&FailureItem> = items.iter().collect();
    sorted.sort_by(|a, b| b.counts.total().cmp(&a.counts.total()).then(a.name.cmp(&b.name)));
    let mut lines = vec![format!(
        "{} invalid heartbeat attempts arrived for {} applications:",
        total.total(),
        items.len()
    )];
    lines.extend(reason_lines(&total));
    lines.push("Most targeted applications:".to_owned());
    lines.extend(list(&sorted, |item| format!("• {}: {}", item.name, item.counts.total())));
    lines.extend(ip_line(&total));
    lines.push(
        "It may be an attempt to forge the heartbeat, or misconfigured clients \
         (wrong secret or clock)."
            .to_owned(),
    );
    let next = items.iter().map(|i| i.next_report_in_ms).min().unwrap_or(0).max(min_interval_ms);
    lines.push(format!("If the attempts continue, the next summary will arrive in {}.", duration(next)));
    build(to, format!("IronHeart: invalid attempts on {} applications", items.len()), lines, apps_url)
}

/// The first `DIGEST_LIST_MAX` lines, plus "… and N more".
fn list<T>(items: &[T], line: impl Fn(&T) -> String) -> Vec<String> {
    let mut lines: Vec<String> = items.iter().take(DIGEST_LIST_MAX).map(line).collect();
    if items.len() > DIGEST_LIST_MAX {
        lines.push(format!("… and {} more", items.len() - DIGEST_LIST_MAX));
    }
    lines
}

fn reason_lines(counts: &FailureCounts) -> Vec<String> {
    [
        ("bad signature", counts.bad_signature),
        ("replayed request", counts.replay),
        ("clock out of tolerance", counts.clock_skew),
    ]
    .into_iter()
    .filter(|(_, value)| *value > 0)
    .map(|(label, value)| format!("• {label}: {value}"))
    .collect()
}

fn ip_line(counts: &FailureCounts) -> Option<String> {
    let mut ips: Vec<_> = counts.ips.iter().collect();
    if ips.is_empty() {
        return None;
    }
    ips.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let top: Vec<String> = ips.iter().take(5).map(|(ip, n)| format!("{ip} ({n})")).collect();
    let others = ips.iter().skip(5).map(|(_, n)| **n).sum::<i64>() + counts.other_ips;
    let mut line = format!("Source: {}", top.join(", "));
    if others > 0 {
        line.push_str(&format!(", {others} others"));
    }
    Some(line)
}

pub struct AppRef<'a> {
    pub name: &'a str,
    /// Address of the app's page in the dashboard.
    pub url: String,
}

pub fn down(app: &AppRef, to: &str, last_seen_at: Option<i64>, now: i64, failures: i64) -> Email {
    let last_seen = match last_seen_at {
        Some(at) => format!("The last signal arrived {} ago ({}).", duration(now - at), datetime(at)),
        None => "No signal has ever arrived.".to_owned(),
    };
    let mut lines = vec![
        format!("The application “{}” did not send its heartbeat within the expected interval.", app.name),
        last_seen,
    ];
    if failures > 0 {
        lines.push(format!(
            "In the same period {failures} invalid attempts arrived: the pings may not be \
             reaching the service because of an attack."
        ));
    }
    build(to, format!("IronHeart: “{}” is not responding", app.name), lines, &app.url)
}

pub fn recovered(app: &AppRef, to: &str, down_since: Option<i64>, last_seen_at: Option<i64>) -> Email {
    let mut lines = vec![format!("The application “{}” has resumed sending its heartbeat.", app.name)];
    if let (Some(from), Some(to_at)) = (down_since, last_seen_at) {
        lines.push(format!("It was down for {} (since {}).", duration(to_at - from), datetime(from)));
    }
    build(to, format!("IronHeart: “{}” is back up", app.name), lines, &app.url)
}

pub fn failures(
    app: &AppRef,
    to: &str,
    counts: &FailureCounts,
    since: Option<i64>,
    next_report_in_ms: i64,
) -> Email {
    let period = match since {
        Some(at) => format!("since the previous alert ({})", datetime(at)),
        None => "recently".to_owned(),
    };
    let mut lines = vec![
        format!(
            "For the application “{}”, {} invalid heartbeat attempts arrived {period}:",
            app.name,
            counts.total()
        ),
    ];
    lines.extend(reason_lines(counts));
    lines.extend(ip_line(counts));
    lines.push(
        "It may be an attempt to forge the heartbeat, or a misconfigured client \
         (wrong secret or clock)."
            .to_owned(),
    );
    lines.push(format!(
        "If the attempts continue, the next summary will arrive in {}.",
        duration(next_report_in_ms)
    ));
    build(to, format!("IronHeart: invalid attempts for “{}”", app.name), lines, &app.url)
}

fn build(to: &str, subject: String, lines: Vec<String>, url: &str) -> Email {
    let text = format!("{}\n\nDetails: {url}\n", lines.join("\n"));
    let html = format!(
        "{}<p><a href=\"{}\">Open the application in IronHeart</a></p>",
        lines.iter().map(|l| format!("<p>{}</p>", escape(l))).collect::<String>(),
        escape(url)
    );
    Email { to: to.to_owned(), subject, text, html }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn datetime(ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|dt| dt.format(format_description!("[year]-[month]-[day] [hour]:[minute] UTC")).ok())
        .unwrap_or_else(|| "?".to_owned())
}

/// "45 seconds", "12 minutes", "2 hours and 5 minutes", "3 days and 4 hours".
pub fn duration(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    let unit = |n: i64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let (days, hours, minutes) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (days, hours, minutes) {
        (0, 0, 0) => unit(secs, "second", "seconds"),
        (0, 0, m) => unit(m, "minute", "minutes"),
        (0, h, 0) => unit(h, "hour", "hours"),
        (0, h, m) => format!("{} and {}", unit(h, "hour", "hours"), unit(m, "minute", "minutes")),
        (d, 0, _) => unit(d, "day", "days"),
        (d, h, _) => format!("{} and {}", unit(d, "day", "days"), unit(h, "hour", "hours")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration(45_000), "45 seconds");
        assert_eq!(duration(60_000), "1 minute");
        assert_eq!(duration(2 * 3_600_000), "2 hours");
        assert_eq!(duration(3_600_000 + 5 * 60_000), "1 hour and 5 minutes");
        assert_eq!(duration(26 * 3_600_000), "1 day and 2 hours");
    }

    #[test]
    fn failure_report_lists_reasons_and_top_ips() {
        let counts = FailureCounts {
            bad_signature: 7,
            replay: 1,
            clock_skew: 0,
            ips: [("1.1.1.1".into(), 6), ("2.2.2.2".into(), 2)].into(),
            other_ips: 3,
        };
        let app = AppRef { name: "Nightly <backup>", url: "https://x/apps/1".into() };
        let email = failures(&app, "a@example.com", &counts, None, 40 * 60_000);
        assert!(email.text.contains("8 invalid"));
        assert!(email.text.contains("• bad signature: 7"));
        assert!(!email.text.contains("clock out of tolerance"));
        assert!(email.text.contains("1.1.1.1 (6), 2.2.2.2 (2), 3 others"));
        assert!(email.text.contains("in 40 minutes"));
        assert!(email.html.contains("Nightly &lt;backup&gt;"));
    }

    fn failure_item(name: &str, bad: i64) -> FailureItem {
        FailureItem {
            app_id: name.into(),
            name: name.into(),
            url: format!("https://x/apps/{name}"),
            counts: FailureCounts { bad_signature: bad, ips: [("1.1.1.1".into(), bad)].into(), ..Default::default() },
            since: None,
            next_report_in_ms: 10 * 60_000,
        }
    }

    #[test]
    fn digest_with_one_app_is_the_single_mail() {
        let email = failures_digest("a@example.com", &[failure_item("A", 3)], 5 * 60_000, "https://x/apps");
        assert_eq!(email.subject, "IronHeart: invalid attempts for “A”");
        assert!(email.text.contains("in 10 minutes"));
    }

    #[test]
    fn digest_lists_apps_by_attempts_and_caps_the_list() {
        let items: Vec<_> = (0..60).map(|i| failure_item(&format!("app{i:02}"), i + 1)).collect();
        let email = failures_digest("a@example.com", &items, 5 * 60_000, "https://x/apps");
        assert_eq!(email.subject, "IronHeart: invalid attempts on 60 applications");
        let first = email.text.find("• app59: 60").unwrap();
        let second = email.text.find("• app58: 59").unwrap();
        assert!(first < second, "sorted by attempts");
        assert!(email.text.contains("… and 10 more"));
        assert!(email.text.contains("1.1.1.1 (1830)"));
        assert!(email.text.contains("in 10 minutes"));

        let downs: Vec<_> = (0..3)
            .map(|i| DownItem { name: format!("d{i}"), url: String::new(), last_seen_at: Some(0), failures: 0 })
            .collect();
        let email = down_digest("a@example.com", &downs, 120_000, "https://x/apps");
        assert_eq!(email.subject, "IronHeart: 3 applications are not responding");
        assert!(email.text.contains("• d1 — last signal 2 minutes ago"));
    }
}
