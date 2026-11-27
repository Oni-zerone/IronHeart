//! Testi delle mail di avviso. Gli orari sono in UTC.
//!
//! A ogni giro del controllo ogni destinatario riceve al massimo una mail per
//! tipo: con più app coinvolte, un riepilogo unico (`*_digest`).

use time::{OffsetDateTime, macros::format_description};

use crate::{db::FailureCounts, mail::Email};

/// App elencate per esteso in un riepilogo; le altre si contano soltanto.
const DIGEST_LIST_MAX: usize = 50;

#[derive(Clone)]
pub struct DownItem {
    pub name: String,
    pub url: String,
    pub last_seen_at: Option<i64>,
    /// Tentativi non validi dall'ultimo segnale.
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
    let mut lines = vec![format!("{} applicazioni non hanno inviato l'heartbeat entro l'intervallo previsto:", items.len())];
    lines.extend(list(items, |item| {
        let seen = item.last_seen_at.map_or("mai un segnale".to_owned(), |at| format!("ultimo segnale {} fa", duration(now - at)));
        format!("• {} — {seen}", item.name)
    }));
    let failures: i64 = items.iter().map(|i| i.failures).sum();
    if failures > 0 {
        lines.push(format!(
            "Nello stesso periodo sono arrivati {failures} tentativi non validi: i ping potrebbero \
             non raggiungere il servizio a causa di un attacco."
        ));
    }
    build(to, format!("IronHeart: {} applicazioni non rispondono", items.len()), lines, apps_url)
}

pub fn recovered_digest(to: &str, items: &[RecoveredItem], apps_url: &str) -> Email {
    if let [item] = items {
        let app = AppRef { name: &item.name, url: item.url.clone() };
        return recovered(&app, to, item.down_since, item.last_seen_at);
    }
    let mut lines = vec![format!("{} applicazioni hanno ripreso a inviare l'heartbeat:", items.len())];
    lines.extend(list(items, |item| match (item.down_since, item.last_seen_at) {
        (Some(from), Some(to_at)) => format!("• {} — non attiva per {}", item.name, duration(to_at - from)),
        _ => format!("• {}", item.name),
    }));
    build(to, format!("IronHeart: {} applicazioni sono di nuovo attive", items.len()), lines, apps_url)
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
        "Sono arrivati {} tentativi di heartbeat non validi su {} applicazioni:",
        total.total(),
        items.len()
    )];
    lines.extend(reason_lines(&total));
    lines.push("Applicazioni più colpite:".to_owned());
    lines.extend(list(&sorted, |item| format!("• {}: {}", item.name, item.counts.total())));
    lines.extend(ip_line(&total));
    lines.push(
        "Può trattarsi di un tentativo di falsificare l'heartbeat oppure di client \
         configurati male (segreto o orologio sbagliati)."
            .to_owned(),
    );
    let next = items.iter().map(|i| i.next_report_in_ms).min().unwrap_or(0).max(min_interval_ms);
    lines.push(format!("Se i tentativi continuano, il prossimo riepilogo arriverà tra {}.", duration(next)));
    build(to, format!("IronHeart: tentativi non validi su {} applicazioni", items.len()), lines, apps_url)
}

/// Le prime `DIGEST_LIST_MAX` righe, più «… e altre N».
fn list<T>(items: &[T], line: impl Fn(&T) -> String) -> Vec<String> {
    let mut lines: Vec<String> = items.iter().take(DIGEST_LIST_MAX).map(line).collect();
    if items.len() > DIGEST_LIST_MAX {
        lines.push(format!("… e altre {}", items.len() - DIGEST_LIST_MAX));
    }
    lines
}

fn reason_lines(counts: &FailureCounts) -> Vec<String> {
    [
        ("firma errata", counts.bad_signature),
        ("richiesta ripetuta", counts.replay),
        ("orologio fuori tolleranza", counts.clock_skew),
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
    let mut line = format!("Provenienza: {}", top.join(", "));
    if others > 0 {
        line.push_str(&format!(", altri {others}"));
    }
    Some(line)
}

pub struct AppRef<'a> {
    pub name: &'a str,
    /// Indirizzo della pagina dell'app nella dashboard.
    pub url: String,
}

pub fn down(app: &AppRef, to: &str, last_seen_at: Option<i64>, now: i64, failures: i64) -> Email {
    let last_seen = match last_seen_at {
        Some(at) => format!("L'ultimo segnale è arrivato {} fa ({}).", duration(now - at), datetime(at)),
        None => "Non è mai arrivato un segnale.".to_owned(),
    };
    let mut lines = vec![
        format!("L'applicazione «{}» non ha inviato l'heartbeat entro l'intervallo previsto.", app.name),
        last_seen,
    ];
    if failures > 0 {
        lines.push(format!(
            "Nello stesso periodo sono arrivati {failures} tentativi non validi: i ping potrebbero \
             non raggiungere il servizio a causa di un attacco."
        ));
    }
    build(to, format!("IronHeart: «{}» non risponde", app.name), lines, &app.url)
}

pub fn recovered(app: &AppRef, to: &str, down_since: Option<i64>, last_seen_at: Option<i64>) -> Email {
    let mut lines = vec![format!("L'applicazione «{}» ha ripreso a inviare l'heartbeat.", app.name)];
    if let (Some(from), Some(to_at)) = (down_since, last_seen_at) {
        lines.push(format!("È risultata non attiva per {} (dalle {}).", duration(to_at - from), datetime(from)));
    }
    build(to, format!("IronHeart: «{}» è di nuovo attiva", app.name), lines, &app.url)
}

pub fn failures(
    app: &AppRef,
    to: &str,
    counts: &FailureCounts,
    since: Option<i64>,
    next_report_in_ms: i64,
) -> Email {
    let period = match since {
        Some(at) => format!("dal precedente avviso ({})", datetime(at)),
        None => "di recente".to_owned(),
    };
    let mut lines = vec![
        format!(
            "Per l'applicazione «{}» sono arrivati {} tentativi di heartbeat non validi {period}:",
            app.name,
            counts.total()
        ),
    ];
    lines.extend(reason_lines(counts));
    lines.extend(ip_line(counts));
    lines.push(
        "Può trattarsi di un tentativo di falsificare l'heartbeat oppure di un client \
         configurato male (segreto o orologio sbagliati)."
            .to_owned(),
    );
    lines.push(format!(
        "Se i tentativi continuano, il prossimo riepilogo arriverà tra {}.",
        duration(next_report_in_ms)
    ));
    build(to, format!("IronHeart: tentativi non validi per «{}»", app.name), lines, &app.url)
}

fn build(to: &str, subject: String, lines: Vec<String>, url: &str) -> Email {
    let text = format!("{}\n\nDettagli: {url}\n", lines.join("\n"));
    let html = format!(
        "{}<p><a href=\"{}\">Apri l'applicazione in IronHeart</a></p>",
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
        .and_then(|dt| dt.format(format_description!("[day]/[month]/[year] [hour]:[minute] UTC")).ok())
        .unwrap_or_else(|| "?".to_owned())
}

/// «45 secondi», «12 minuti», «2 ore e 5 minuti», «3 giorni e 4 ore».
pub fn duration(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    let unit = |n: i64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let (days, hours, minutes) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (days, hours, minutes) {
        (0, 0, 0) => unit(secs, "secondo", "secondi"),
        (0, 0, m) => unit(m, "minuto", "minuti"),
        (0, h, 0) => unit(h, "ora", "ore"),
        (0, h, m) => format!("{} e {}", unit(h, "ora", "ore"), unit(m, "minuto", "minuti")),
        (d, 0, _) => unit(d, "giorno", "giorni"),
        (d, h, _) => format!("{} e {}", unit(d, "giorno", "giorni"), unit(h, "ora", "ore")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration(45_000), "45 secondi");
        assert_eq!(duration(60_000), "1 minuto");
        assert_eq!(duration(2 * 3_600_000), "2 ore");
        assert_eq!(duration(3_600_000 + 5 * 60_000), "1 ora e 5 minuti");
        assert_eq!(duration(26 * 3_600_000), "1 giorno e 2 ore");
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
        let app = AppRef { name: "Backup <notturno>", url: "https://x/apps/1".into() };
        let email = failures(&app, "a@example.com", &counts, None, 40 * 60_000);
        assert!(email.text.contains("8 tentativi"));
        assert!(email.text.contains("• firma errata: 7"));
        assert!(!email.text.contains("orologio fuori tolleranza"));
        assert!(email.text.contains("1.1.1.1 (6), 2.2.2.2 (2), altri 3"));
        assert!(email.text.contains("tra 40 minuti"));
        assert!(email.html.contains("Backup &lt;notturno&gt;"));
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
        assert_eq!(email.subject, "IronHeart: tentativi non validi per «A»");
        assert!(email.text.contains("tra 10 minuti"));
    }

    #[test]
    fn digest_lists_apps_by_attempts_and_caps_the_list() {
        let items: Vec<_> = (0..60).map(|i| failure_item(&format!("app{i:02}"), i + 1)).collect();
        let email = failures_digest("a@example.com", &items, 5 * 60_000, "https://x/apps");
        assert_eq!(email.subject, "IronHeart: tentativi non validi su 60 applicazioni");
        let first = email.text.find("• app59: 60").unwrap();
        let second = email.text.find("• app58: 59").unwrap();
        assert!(first < second, "ordinate per tentativi");
        assert!(email.text.contains("… e altre 10"));
        assert!(email.text.contains("1.1.1.1 (1830)"));
        assert!(email.text.contains("tra 10 minuti"));

        let downs: Vec<_> = (0..3)
            .map(|i| DownItem { name: format!("d{i}"), url: String::new(), last_seen_at: Some(0), failures: 0 })
            .collect();
        let email = down_digest("a@example.com", &downs, 120_000, "https://x/apps");
        assert_eq!(email.subject, "IronHeart: 3 applicazioni non rispondono");
        assert!(email.text.contains("• d1 — ultimo segnale 2 minuti fa"));
    }
}
