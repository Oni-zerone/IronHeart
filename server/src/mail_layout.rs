//! The look shared by every email: the dashboard's ASCII style.
//!
//! An email is drawn on a character grid `COLS` columns wide, like the
//! dashboard: the top bar (`<3 IRONHEART`), the ECG line, an `AsciiBox`-style
//! frame with the title in the top border and a footer.
//!
//! ```text
//! <3 IRONHEART
//! ________________________________.........._/\/\_________
//!
//! +-[ APP DOWN ]-----------------------------------------+
//! |                                                      |
//! | </3 DOWN  nightly-backup                             |
//! |                                                      |
//! | It did not send its heartbeat within the expected    |
//! | interval.                                            |
//! |                                                      |
//! | [ open the application ]                             |
//! |                                                      |
//! +------------------------------------------------------+
//! ```
//!
//! The HTML part is a single `<pre>` with inline styles (mail clients drop
//! external stylesheets and web fonts), on a black background; the text part
//! is the same drawing without colors. Text is wrapped here, in Rust, so the
//! side borders always line up. The colors are the tokens of
//! `web/src/styles.css`.

use crate::mail::Email;

/// Width of the drawing in columns: about 470 px at 14 px, and a media query
/// shrinks the font on phones so it still fits without scrolling.
pub const COLS: usize = 56;
/// Columns inside the frame: `| ` + content + ` |`.
const INNER: usize = COLS - 4;

const BG: &str = "#000000";
const FG: &str = "#a6b0b8";
const DIM: &str = "#5b666e";
const FAINT: &str = "#1f272c";
const ACCENT: &str = "#78c5ee";
const HI: &str = "#c6edff";
const MONO: &str = "'JetBrains Mono',ui-monospace,SFMono-Regular,Menlo,Consolas,'DejaVu Sans Mono','Courier New',monospace";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Fg,
    Dim,
    Faint,
    Accent,
    /// Accent with the glow of `.glow` / `.blip`.
    Glow,
    Hi,
    /// Inverse video, like the `DOWN` label of `StateTag`.
    Inverse,
    /// The label of the action button (the URL is in `Letter::action`).
    Link,
}

#[derive(Clone, Debug)]
pub struct Span {
    pub style: Style,
    pub text: String,
}

pub fn fg(text: impl Into<String>) -> Span {
    Span {
        style: Style::Fg,
        text: text.into(),
    }
}

pub fn hi(text: impl Into<String>) -> Span {
    Span {
        style: Style::Hi,
        text: text.into(),
    }
}

pub fn dim(text: impl Into<String>) -> Span {
    Span {
        style: Style::Dim,
        text: text.into(),
    }
}

/// App state, drawn like `StateTag`.
#[derive(Clone, Copy)]
pub enum State {
    Up,
    Down,
}

pub enum Block {
    /// A paragraph, wrapped to the width of the frame.
    Para(Vec<Span>),
    /// A list item (`- …`), with a hanging indent. Consecutive items, and
    /// items after the paragraph that introduces them, have no blank line
    /// between them.
    Item(Vec<Span>),
    /// `</3 DOWN  name`.
    State(State, String),
}

impl Block {
    pub fn text(text: impl Into<String>) -> Self {
        Block::Para(vec![fg(text)])
    }
}

/// The ECG line under the top bar.
#[derive(Clone, Copy)]
pub enum Pulse {
    /// A beat, as in the dashboard.
    Beat,
    /// Flat line: something stopped.
    Flat,
}

pub struct Letter {
    /// In the top border of the frame, in uppercase.
    pub title: String,
    /// Highlighted frame, like `AsciiBox hot`: something needs attention.
    pub hot: bool,
    pub pulse: Pulse,
    pub blocks: Vec<Block>,
    /// The `[ label ]` button at the end of the frame, with its URL.
    pub action: Option<(String, String)>,
}

/// A row of the drawing: runs of styled text.
type Row = Vec<(Style, String)>;

impl Letter {
    pub fn into_email(self, to: &str, subject: String) -> Email {
        let text = self.text();
        let html = self.html(&subject);
        Email {
            to: to.to_owned(),
            subject,
            text,
            html,
        }
    }

    fn text(&self) -> String {
        let mut out: String = self
            .rows(false)
            .iter()
            .map(|row| {
                let line: String = row.iter().map(|(_, t)| t.as_str()).collect();
                format!("{}\n", line.trim_end())
            })
            .collect();
        // In plain text the URL stays whole, outside the frame, so it can be clicked.
        if let Some((label, url)) = &self.action {
            out.push_str(&format!("\n{label}: {url}\n"));
        }
        out
    }

    fn html(&self, subject: &str) -> String {
        let url = self
            .action
            .as_ref()
            .map(|(_, url)| url.as_str())
            .unwrap_or("");
        let body: String = self
            .rows(true)
            .iter()
            .map(|row| {
                row.iter()
                    .map(|(style, text)| span_html(*style, text, url))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "<!doctype html>\
<html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"color-scheme\" content=\"dark\"><meta name=\"supported-color-schemes\" content=\"dark\">\
<title>{title}</title>\
<style>@media (max-width:520px){{.ih{{font-size:10px!important;line-height:15px!important}}}}</style>\
</head>\
<body style=\"margin:0;padding:0;background:{BG};\">\
<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" bgcolor=\"{BG}\" style=\"background:{BG};\">\
<tr><td align=\"center\" style=\"padding:24px 12px;\">\
<pre class=\"ih\" style=\"display:inline-block;margin:0;text-align:left;white-space:pre;background:{BG};color:{FG};\
font-family:{MONO};font-size:14px;line-height:20px;font-variant-ligatures:none;font-feature-settings:'liga' 0,'calt' 0;\">\
{body}</pre></td></tr></table></body></html>",
            title = escape(subject),
        )
    }

    /// The whole drawing. `html` decides whether the action button is drawn
    /// (in plain text the URL goes below the frame instead).
    fn rows(&self, html: bool) -> Vec<Row> {
        let mut rows: Vec<Row> = vec![
            vec![
                (Style::Glow, "<3".into()),
                (Style::Fg, " ".into()),
                (Style::Hi, "IRONHEART".into()),
            ],
            ecg(self.pulse),
            vec![],
        ];

        let (edge, title) = if self.hot {
            (Style::Accent, Style::Hi)
        } else {
            (Style::Dim, Style::Accent)
        };
        let mut name = self.title.to_uppercase();
        if name.chars().count() > COLS - 8 {
            name = name.chars().take(COLS - 9).collect::<String>() + "~";
        }
        let fill = COLS - 7 - name.chars().count();
        rows.push(vec![
            (edge, "+-[ ".into()),
            (title, name),
            (edge, format!(" ]{}+", "-".repeat(fill))),
        ]);

        let mut content: Vec<Row> = vec![vec![]];
        let mut previous: Option<&Block> = None;
        for block in &self.blocks {
            let joined = matches!(
                (previous, block),
                (Some(Block::Item(_)), Block::Item(_)) | (Some(Block::Para(_)), Block::Item(_))
            );
            if previous.is_some() && !joined {
                content.push(vec![]);
            }
            content.extend(block_rows(block));
            previous = Some(block);
        }
        if let (true, Some((label, _))) = (html, &self.action) {
            content.push(vec![]);
            content.push(vec![
                (Style::Dim, "[ ".into()),
                (Style::Link, label.clone()),
                (Style::Dim, " ]".into()),
            ]);
        }
        content.push(vec![]);

        for row in content {
            let width: usize = row.iter().map(|(_, t)| t.chars().count()).sum();
            let mut line = vec![(edge, "| ".to_owned())];
            line.extend(row);
            line.push((Style::Fg, " ".repeat(INNER.saturating_sub(width))));
            line.push((edge, " |".into()));
            rows.push(line);
        }
        rows.push(vec![(edge, format!("+{}+", "-".repeat(COLS - 2)))]);
        rows.push(vec![(
            Style::Dim,
            "  times in UTC. -- IronHeart, heartbeat monitor".into(),
        )]);
        rows.into_iter().map(merge).collect()
    }
}

fn ecg(pulse: Pulse) -> Row {
    match pulse {
        Pulse::Flat => vec![(Style::Dim, "_".repeat(COLS))],
        Pulse::Beat => {
            // Where the dashboard draws it with reduced motion: at 60%.
            let blip = "_/\\/\\_";
            let at = COLS * 6 / 10;
            let trail = 10;
            vec![
                (Style::Faint, "_".repeat(at - trail)),
                (Style::Dim, ".".repeat(trail)),
                (Style::Glow, blip.into()),
                (Style::Faint, "_".repeat(COLS - at - blip.len())),
            ]
        }
    }
}

fn block_rows(block: &Block) -> Vec<Row> {
    match block {
        Block::Para(spans) => wrap(spans, INNER, 0),
        Block::Item(spans) => {
            let mut rows = wrap(spans, INNER - 4, 4);
            if let Some(first) = rows.first_mut() {
                first[0] = (Style::Dim, "  - ".into());
            }
            rows
        }
        Block::State(state, name) => {
            let (glyph, label, label_style) = match state {
                State::Up => ("<3  ", "UP", Style::Fg),
                State::Down => ("</3 ", "DOWN", Style::Inverse),
            };
            let glyph_style = match state {
                State::Up => Style::Accent,
                State::Down => Style::Hi,
            };
            let lead = vec![
                (glyph_style, glyph.to_owned()),
                (label_style, label.to_owned()),
                (Style::Fg, "  ".into()),
            ];
            let indent = glyph.len() + label.len() + 2;
            let mut rows = wrap(&[hi(name.clone())], INNER - indent, indent);
            rows[0].splice(0..1, lead);
            rows
        }
    }
}

/// Wraps on spaces (words longer than the line are cut). Every row starts
/// with an `indent`-wide run of spaces, so callers can replace it.
fn wrap(spans: &[Span], width: usize, indent: usize) -> Vec<Row> {
    let mut words: Vec<Vec<(Style, char)>> = vec![];
    let mut word: Vec<(Style, char)> = vec![];
    for span in spans {
        for c in span.text.chars() {
            if c == ' ' || c == '\n' {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            } else {
                word.push((span.style, c));
            }
        }
        // Spans are joined without spaces: a word may run across two styles.
    }
    if !word.is_empty() {
        words.push(word);
    }

    let mut lines: Vec<Vec<(Style, char)>> = vec![];
    let mut line: Vec<(Style, char)> = vec![];
    for word in words {
        if !line.is_empty() && line.len() + 1 + word.len() <= width {
            // Between two words of the same style the space keeps it, so a run is not split.
            let before = line.last().map(|(style, _)| *style);
            let space = if before == word.first().map(|(style, _)| *style) {
                before.unwrap()
            } else {
                Style::Fg
            };
            line.push((space, ' '));
            line.extend(word);
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        let mut chunks = word.chunks(width).peekable();
        while let Some(chunk) = chunks.next() {
            if chunks.peek().is_some() {
                lines.push(chunk.to_vec());
            } else {
                line = chunk.to_vec();
            }
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }

    lines
        .into_iter()
        .map(|chars| {
            let mut row: Row = vec![(Style::Fg, " ".repeat(indent))];
            row.extend(chars.into_iter().map(|(style, c)| (style, c.to_string())));
            row
        })
        .collect()
}

/// Joins adjacent runs of the same style and drops empty ones.
fn merge(row: Row) -> Row {
    let mut out: Row = vec![];
    for (style, text) in row {
        if text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((last, acc)) if *last == style => acc.push_str(&text),
            _ => out.push((style, text)),
        }
    }
    out
}

fn span_html(style: Style, text: &str, url: &str) -> String {
    let text = escape(text);
    let css = match style {
        Style::Fg => return text,
        Style::Dim => format!("color:{DIM};"),
        Style::Faint => format!("color:{FAINT};"),
        Style::Accent => format!("color:{ACCENT};"),
        Style::Glow => format!("color:{ACCENT};text-shadow:0 0 8px rgba(120,197,238,0.6);"),
        Style::Hi => format!("color:{HI};"),
        Style::Inverse => format!("background:{HI};color:{BG};"),
        Style::Link => {
            return format!(
                "<a href=\"{}\" style=\"color:{ACCENT};text-decoration:none;\">{text}</a>",
                escape(url)
            );
        }
    };
    format!("<span style=\"{css}\">{text}</span>")
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The content of the frame as running text: borders removed and wrapped
/// lines joined, to check texts in tests regardless of where they wrap.
#[cfg(test)]
pub fn unwrapped(text: &str) -> String {
    text.lines()
        .filter_map(|line| line.strip_prefix("| "))
        .map(|line| line.trim_end().trim_end_matches('|').trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter() -> Letter {
        Letter {
            title: "App down".into(),
            hot: true,
            pulse: Pulse::Flat,
            blocks: vec![
                Block::State(State::Down, "nightly <backup>".into()),
                Block::text(
                    "It did not send its heartbeat within the expected interval, and this sentence is long enough to wrap.",
                ),
                Block::text("Apps:"),
                Block::Item(vec![
                    hi("a-very-long-application-name-that-does-not-fit-on-one-line-at-all"),
                    fg(": never"),
                ]),
            ],
            action: Some((
                "open the application".into(),
                "https://x/apps/1?a=1&b=2".into(),
            )),
        }
    }

    #[test]
    fn every_row_of_the_frame_is_as_wide_as_the_grid() {
        let email = letter().into_email("a@example.com", "Subject".into());
        let framed: Vec<&str> = email
            .text
            .lines()
            .filter(|l| l.starts_with('|') || l.starts_with('+'))
            .collect();
        assert!(framed.len() > 8);
        for line in framed {
            assert_eq!(line.chars().count(), COLS, "{line:?}");
            assert!(line.ends_with('|') || line.ends_with('+'), "{line:?}");
        }
        assert!(email.text.starts_with("<3 IRONHEART\n"));
        assert!(email.text.contains("+-[ APP DOWN ]---"));
        assert!(email.text.contains("| </3 DOWN  nightly <backup>"));
        assert!(
            email
                .text
                .contains("\nopen the application: https://x/apps/1?a=1&b=2\n")
        );
        assert!(email.text.is_ascii());
    }

    #[test]
    fn html_is_escaped_and_links_the_action() {
        let email = letter().into_email("a@example.com", "Subject <x>".into());
        assert!(email.html.contains("nightly &lt;backup&gt;"));
        assert!(!email.html.contains("<backup>"));
        assert!(email.html.contains("<title>Subject &lt;x&gt;</title>"));
        assert!(email.html.contains("href=\"https://x/apps/1?a=1&amp;b=2\""));
        assert!(email.html.contains(">open the application</a>"));
    }

    #[test]
    fn long_words_are_cut_and_items_keep_their_indent() {
        let text = letter().into_email("a@example.com", "s".into()).text;
        assert!(text.contains("|   - a-very-long"));
        assert!(text.contains("\n|     "), "continuation lines are indented");
        assert!(unwrapped(&text).contains("this sentence is long enough to wrap."));
    }
}
