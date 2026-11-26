//! Testi delle mail di avviso. Gli orari sono in UTC.

use time::{OffsetDateTime, macros::format_description};

use crate::{db::FailureCounts, mail::Email};

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
    for (label, value) in [
        ("firma errata", counts.bad_signature),
        ("richiesta ripetuta", counts.replay),
        ("orologio fuori tolleranza", counts.clock_skew),
    ] {
        if value > 0 {
            lines.push(format!("• {label}: {value}"));
        }
    }
    let mut ips: Vec<_> = counts.ips.iter().collect();
    ips.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    if !ips.is_empty() {
        let top: Vec<String> = ips.iter().take(5).map(|(ip, n)| format!("{ip} ({n})")).collect();
        let others = ips.iter().skip(5).map(|(_, n)| **n).sum::<i64>() + counts.other_ips;
        let mut line = format!("Provenienza: {}", top.join(", "));
        if others > 0 {
            line.push_str(&format!(", altri {others}"));
        }
        lines.push(line);
    }
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
}
