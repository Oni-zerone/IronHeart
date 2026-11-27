//! Telegram alert texts: plain text (no parse mode, nothing to escape), the
//! same content as the emails in a shorter form. Times are in UTC.

use crate::{
    alerts::{DownItem, FailureItem, RecoveredItem, duration},
    db::FailureCounts,
};

/// Apps listed in full; the others are only counted.
const LIST_MAX: usize = 30;

pub fn down(items: &[DownItem], now: i64, apps_url: &str) -> String {
    let seen = |item: &DownItem| match item.last_seen_at {
        Some(at) => format!("last signal {} ago", duration(now - at)),
        None => "never a signal".to_owned(),
    };
    let failures: i64 = items.iter().map(|i| i.failures).sum();
    let mut lines = match items {
        [item] => vec![
            format!("!! DOWN: {}", item.name),
            format!("No heartbeat within the expected interval, {}.", seen(item)),
        ],
        _ => {
            let mut lines = vec![format!("!! {} applications are DOWN:", items.len())];
            lines.extend(list(items, |item| format!("{}: {}", item.name, seen(item))));
            lines
        }
    };
    if failures > 0 {
        lines.push(format!(
            "{failures} invalid attempts in the same period: the pings may be blocked by an attack."
        ));
    }
    lines.push(link(
        items.len(),
        items.first().map(|i| i.url.as_str()),
        apps_url,
    ));
    lines.join("\n")
}

pub fn recovered(items: &[RecoveredItem], apps_url: &str) -> String {
    let down_for = |item: &RecoveredItem| match (item.down_since, item.last_seen_at) {
        (Some(from), Some(to)) => format!(" (down for {})", duration(to - from)),
        _ => String::new(),
    };
    let mut lines = match items {
        [item] => vec![format!("UP: {} is back up{}.", item.name, down_for(item))],
        _ => {
            let mut lines = vec![format!("{} applications are back UP:", items.len())];
            lines.extend(list(items, |item| {
                format!("{}{}", item.name, down_for(item))
            }));
            lines
        }
    };
    lines.push(link(
        items.len(),
        items.first().map(|i| i.url.as_str()),
        apps_url,
    ));
    lines.join("\n")
}

pub fn failures(items: &[FailureItem], apps_url: &str) -> String {
    let mut total = FailureCounts::default();
    for item in items {
        total.add(&item.counts);
    }
    let mut lines = match items {
        [item] => vec![format!(
            "!! {} invalid heartbeat attempts for {}",
            total.total(),
            item.name
        )],
        _ => {
            let mut sorted: Vec<&FailureItem> = items.iter().collect();
            sorted.sort_by(|a, b| {
                b.counts
                    .total()
                    .cmp(&a.counts.total())
                    .then(a.name.cmp(&b.name))
            });
            let mut lines = vec![format!(
                "!! {} invalid heartbeat attempts on {} applications:",
                total.total(),
                items.len()
            )];
            lines.extend(list(&sorted, |item| {
                format!("{}: {}", item.name, item.counts.total())
            }));
            lines
        }
    };
    lines.extend(reasons(&total));
    lines.extend(sources(&total));
    lines.push("A forged heartbeat, or a client with the wrong secret or clock.".into());
    lines.push(link(
        items.len(),
        items.first().map(|i| i.url.as_str()),
        apps_url,
    ));
    lines.join("\n")
}

/// "bad signature 12, replayed 2".
pub fn reasons(counts: &FailureCounts) -> Option<String> {
    let parts: Vec<String> = [
        ("bad signature", counts.bad_signature),
        ("replayed", counts.replay),
        ("clock out of tolerance", counts.clock_skew),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .map(|(label, n)| format!("{label} {n}"))
    .collect();
    (!parts.is_empty()).then(|| format!("reasons: {}", parts.join(", ")))
}

/// The five IPs with the most attempts.
pub fn sources(counts: &FailureCounts) -> Option<String> {
    let mut ips: Vec<_> = counts.ips.iter().collect();
    if ips.is_empty() {
        return None;
    }
    ips.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut parts: Vec<String> = ips
        .iter()
        .take(5)
        .map(|(ip, n)| format!("{ip} ({n})"))
        .collect();
    let others = ips.iter().skip(5).map(|(_, n)| **n).sum::<i64>() + counts.other_ips;
    if others > 0 {
        parts.push(format!("{others} from other IPs"));
    }
    Some(format!("source: {}", parts.join(", ")))
}

fn list<T>(items: &[T], line: impl Fn(&T) -> String) -> Vec<String> {
    let mut lines: Vec<String> = items
        .iter()
        .take(LIST_MAX)
        .map(|i| format!("- {}", line(i)))
        .collect();
    if items.len() > LIST_MAX {
        lines.push(format!("- ... and {} more", items.len() - LIST_MAX));
    }
    lines
}

/// The app's page for a single app, the list otherwise.
fn link(count: usize, first: Option<&str>, apps_url: &str) -> String {
    match (count, first) {
        (1, Some(url)) => url.to_owned(),
        _ => apps_url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn down_item(name: &str, seen: Option<i64>, failures: i64) -> DownItem {
        DownItem {
            name: name.into(),
            url: format!("https://x/apps/{name}"),
            last_seen_at: seen,
            failures,
        }
    }

    #[test]
    fn single_and_multiple_down() {
        let one = down(&[down_item("nas", Some(0), 0)], 12 * MIN, "https://x/apps");
        assert_eq!(
            one,
            "!! DOWN: nas\nNo heartbeat within the expected interval, last signal 12 minutes ago.\nhttps://x/apps/nas"
        );

        let many = down(
            &[down_item("a", Some(0), 2), down_item("b", None, 1)],
            5 * MIN,
            "https://x/apps",
        );
        assert!(many.starts_with(
            "!! 2 applications are DOWN:\n- a: last signal 5 minutes ago\n- b: never a signal\n"
        ));
        assert!(many.contains("3 invalid attempts"));
        assert!(many.ends_with("https://x/apps"));
    }

    #[test]
    fn recovered_shows_downtime() {
        let item = RecoveredItem {
            name: "nas".into(),
            url: "https://x/apps/nas".into(),
            down_since: Some(0),
            last_seen_at: Some(90 * MIN),
        };
        assert_eq!(
            recovered(&[item], "https://x/apps"),
            "UP: nas is back up (down for 1 hour and 30 minutes).\nhttps://x/apps/nas"
        );
    }

    #[test]
    fn failures_list_reasons_and_sources() {
        let counts = FailureCounts {
            bad_signature: 9,
            replay: 1,
            ips: [("1.2.3.4".to_owned(), 8), ("5.6.7.8".to_owned(), 2)].into(),
            ..Default::default()
        };
        let item = FailureItem {
            app_id: "id".into(),
            name: "nas".into(),
            url: "https://x/apps/id".into(),
            counts,
            since: None,
            next_report_in_ms: 0,
        };
        let text = failures(&[item], "https://x/apps");
        assert!(text.starts_with("!! 10 invalid heartbeat attempts for nas\nreasons: bad signature 9, replayed 1\nsource: 1.2.3.4 (8), 5.6.7.8 (2)\n"), "{text}");
    }

    #[test]
    fn long_lists_are_cut() {
        let items: Vec<DownItem> = (0..40)
            .map(|i| down_item(&format!("app{i}"), None, 0))
            .collect();
        let text = down(&items, 0, "https://x/apps");
        assert!(text.contains("- ... and 10 more"));
        assert_eq!(text.lines().count(), 1 + LIST_MAX + 1 + 1);
    }
}
