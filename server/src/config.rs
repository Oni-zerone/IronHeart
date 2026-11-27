use std::{collections::HashMap, net::SocketAddr};

use anyhow::{Context, bail};

pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    /// Address the browser uses to reach the service, without a trailing `/`.
    /// In development with Vite it is `http://localhost:5173`.
    pub public_base_url: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    /// Master user: always admin, cannot be demoted or removed. Lowercase.
    pub admin_email: String,
    pub mail: MailConfig,
    /// AES-256 master key for app secrets (`SECRETS_KEY`, base64 of 32 bytes).
    pub secrets_key: [u8; 32],
    pub secrets_key_version: i64,
    /// Header to read the client IP from behind a proxy:
    /// `x-forwarded-for` (CapRover) or `cf-connecting-ip` (Cloudflare).
    pub client_ip_header: Option<String>,
    /// Bot token from BotFather (`TELEGRAM_BOT_TOKEN`): without it Telegram is off.
    pub telegram_bot_token: Option<String>,
}

pub struct MailConfig {
    pub from_email: String,
    pub from_name: String,
    pub transport: MailTransport,
}

/// Email provider (`MAIL_TRANSPORT`): an HTTP API or standard SMTP.
#[derive(PartialEq)]
#[cfg_attr(test, derive(Debug))]
pub enum MailTransport {
    Brevo { api_key: String },
    Resend { api_key: String },
    Postmark { server_token: String, message_stream: String },
    Mailgun { api_key: String, domain: String, region: MailgunRegion },
    /// Scaleway Transactional Email.
    Scaleway { secret_key: String, project_id: String, region: String },
    /// Any SMTP server: Mailpit locally, fallback for every other provider.
    Smtp(SmtpConfig),
    /// `MAIL_TRANSPORT=none`: no email at all, alerts only on Telegram.
    None,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MailgunRegion {
    Us,
    Eu,
}

#[derive(PartialEq)]
#[cfg_attr(test, derive(Debug))]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub tls: SmtpTls,
    /// Username and password, given together or not at all.
    pub credentials: Option<(String, String)>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SmtpTls {
    /// Plain text: only towards a local host (Mailpit).
    None,
    StartTls,
    /// Implicit TLS (SMTPS).
    Tls,
}

const MAIL_TRANSPORTS: &str = "brevo, resend, postmark, mailgun, scaleway, smtp, none";

impl MailConfig {
    fn from_env() -> anyhow::Result<Self> {
        // `vars()` panics on a non-UTF-8 variable, even one unrelated to email:
        // those are skipped (an email variable that is not UTF-8 counts as missing).
        let vars = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect();
        Self::from_vars(&vars)
    }

    /// Reads the email configuration from a map of variables (the environment, or a test).
    /// Empty values count as missing. Values are trimmed, except `SMTP_PASSWORD`.
    pub fn from_vars(vars: &HashMap<String, String>) -> anyhow::Result<Self> {
        let get = |key: &str| {
            vars.get(key).map(|v| v.trim()).filter(|v| !v.is_empty()).map(str::to_owned)
        };
        let transport_name = get("MAIL_TRANSPORT").unwrap_or_else(|| "smtp".into());
        let need = |key: &str| {
            get(key).with_context(|| format!("missing variable {key} (MAIL_TRANSPORT={transport_name})"))
        };
        let transport = match transport_name.to_lowercase().as_str() {
            "brevo" => MailTransport::Brevo { api_key: need("BREVO_API_KEY")? },
            "resend" => MailTransport::Resend { api_key: need("RESEND_API_KEY")? },
            "postmark" => MailTransport::Postmark {
                server_token: need("POSTMARK_SERVER_TOKEN")?,
                message_stream: get("POSTMARK_MESSAGE_STREAM").unwrap_or_else(|| "outbound".into()),
            },
            "mailgun" => MailTransport::Mailgun {
                api_key: need("MAILGUN_API_KEY")?,
                domain: need("MAILGUN_DOMAIN")?,
                region: match get("MAILGUN_REGION").map(|v| v.to_lowercase()).as_deref() {
                    None | Some("us") => MailgunRegion::Us,
                    Some("eu") => MailgunRegion::Eu,
                    Some(other) => bail!("invalid MAILGUN_REGION: {other} (us or eu)"),
                },
            },
            "scaleway" => MailTransport::Scaleway {
                secret_key: need("SCW_SECRET_KEY")?,
                project_id: need("SCW_PROJECT_ID")?,
                region: get("SCW_REGION").unwrap_or_else(|| "fr-par".into()),
            },
            "smtp" => MailTransport::Smtp(smtp_config(&get, vars)?),
            "none" => MailTransport::None,
            other => bail!("unknown MAIL_TRANSPORT: {other} (valid: {MAIL_TRANSPORTS})"),
        };
        Ok(Self {
            from_email: get("MAIL_FROM_EMAIL").unwrap_or_else(|| "noreply@ironheart.local".into()),
            from_name: get("MAIL_FROM_NAME").unwrap_or_else(|| "IronHeart".into()),
            transport,
        })
    }
}

fn smtp_config(
    get: &dyn Fn(&str) -> Option<String>,
    vars: &HashMap<String, String>,
) -> anyhow::Result<SmtpConfig> {
    let host = get("SMTP_HOST").unwrap_or_else(|| "localhost".into());
    let local = is_local_host(&host);
    let port: Option<u16> = get("SMTP_PORT").map(|p| p.parse()).transpose().context("invalid SMTP_PORT")?;
    let tls = match get("SMTP_TLS").map(|v| v.to_lowercase()).as_deref() {
        // 465 is implicit TLS: STARTTLS there would wait for a greeting that never comes.
        None if port == Some(465) => SmtpTls::Tls,
        None if local => SmtpTls::None,
        None => SmtpTls::StartTls,
        Some("none") => SmtpTls::None,
        Some("starttls") => SmtpTls::StartTls,
        Some("tls") => SmtpTls::Tls,
        Some(other) => bail!("invalid SMTP_TLS: {other} (none, starttls or tls)"),
    };
    let port = match port {
        Some(port) => port,
        None => match tls {
            SmtpTls::None => 1025,
            SmtpTls::StartTls => 587,
            SmtpTls::Tls => 465,
        },
    };
    // The password is kept as written: it may start or end with a space.
    let password = vars.get("SMTP_PASSWORD").filter(|v| !v.trim().is_empty()).cloned();
    let credentials = match (get("SMTP_USERNAME"), password) {
        (Some(user), Some(password)) => Some((user, password)),
        (None, None) => None,
        (Some(_), None) => bail!("missing variable SMTP_PASSWORD (SMTP_USERNAME is set)"),
        (None, Some(_)) => bail!("missing variable SMTP_USERNAME (SMTP_PASSWORD is set)"),
    };
    if credentials.is_some() && tls == SmtpTls::None && !local {
        bail!(
            "SMTP_TLS=none towards {host}: the password would travel in clear text (use starttls or tls)"
        );
    }
    Ok(SmtpConfig { host, port, tls, credentials })
}

/// Hosts where plain-text SMTP is acceptable: this machine or the Mailpit container.
fn is_local_host(host: &str) -> bool {
    // Not `[::1]`: lettre resolves the host as written and rejects the brackets.
    matches!(host.to_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1" | "mailpit")
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = env_or("BIND_ADDR", "127.0.0.1:8080")
            .parse()
            .context("invalid BIND_ADDR")?;
        let mail = MailConfig::from_env()?;
        let telegram_bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty());
        if mail.transport == MailTransport::None && telegram_bot_token.is_none() {
            bail!("no alert channel: MAIL_TRANSPORT=none needs TELEGRAM_BOT_TOKEN");
        }
        Ok(Self {
            bind_addr,
            // External Postgres (`postgres://…`) or, if missing, the local SQLite file.
            database_url: std::env::var("DATABASE_URL")
                .ok()
                .map(|url| url.trim().to_owned())
                .filter(|url| !url.is_empty())
                .unwrap_or_else(|| "sqlite://data/ironheart.db".into()),
            public_base_url: env_or("PUBLIC_BASE_URL", "http://localhost:8080")
                .trim_end_matches('/')
                .to_owned(),
            google_client_id: required("GOOGLE_CLIENT_ID")?,
            google_client_secret: required("GOOGLE_CLIENT_SECRET")?,
            admin_email: required("ADMIN_EMAIL")?.trim().to_lowercase(),
            mail,
            secrets_key: secrets_key()?,
            secrets_key_version: env_or("SECRETS_KEY_VERSION", "1")
                .parse()
                .context("invalid SECRETS_KEY_VERSION")?,
            client_ip_header: std::env::var("CLIENT_IP_HEADER")
                .ok()
                .map(|h| h.trim().to_lowercase())
                .filter(|h| !h.is_empty()),
            telegram_bot_token,
        })
    }

    /// `Secure` cookie only when the service is exposed over HTTPS.
    pub fn secure_cookies(&self) -> bool {
        self.public_base_url.starts_with("https://")
    }
}

fn secrets_key() -> anyhow::Result<[u8; 32]> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let raw = required("SECRETS_KEY")?;
    let bytes = STANDARD.decode(raw.trim()).context("SECRETS_KEY is not base64")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("SECRETS_KEY must be 32 bytes (openssl rand -base64 32)"))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn required(key: &str) -> anyhow::Result<String> {
    std::env::var(key).with_context(|| format!("missing variable {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pairs: &[(&str, &str)]) -> anyhow::Result<MailTransport> {
        let vars = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        MailConfig::from_vars(&vars).map(|c| c.transport)
    }

    fn error(pairs: &[(&str, &str)]) -> String {
        format!("{:#}", parse(pairs).expect_err("expected an error"))
    }

    fn smtp(pairs: &[(&str, &str)]) -> SmtpConfig {
        match parse(pairs).unwrap() {
            MailTransport::Smtp(smtp) => smtp,
            other => panic!("expected SMTP, got {other:?}"),
        }
    }

    #[test]
    fn default_is_mailpit() {
        let config = MailConfig::from_vars(&HashMap::new()).unwrap();
        assert_eq!(config.from_email, "noreply@ironheart.local");
        assert_eq!(config.from_name, "IronHeart");
        assert_eq!(
            config.transport,
            MailTransport::Smtp(SmtpConfig {
                host: "localhost".into(),
                port: 1025,
                tls: SmtpTls::None,
                credentials: None,
            })
        );
    }

    #[test]
    fn http_providers() {
        assert_eq!(
            parse(&[("MAIL_TRANSPORT", "brevo"), ("BREVO_API_KEY", "k")]).unwrap(),
            MailTransport::Brevo { api_key: "k".into() }
        );
        assert_eq!(
            parse(&[("MAIL_TRANSPORT", "resend"), ("RESEND_API_KEY", "re_k")]).unwrap(),
            MailTransport::Resend { api_key: "re_k".into() }
        );
        assert_eq!(
            parse(&[("MAIL_TRANSPORT", "postmark"), ("POSTMARK_SERVER_TOKEN", "t")]).unwrap(),
            MailTransport::Postmark { server_token: "t".into(), message_stream: "outbound".into() }
        );
        assert_eq!(
            parse(&[
                ("MAIL_TRANSPORT", "mailgun"),
                ("MAILGUN_API_KEY", "k"),
                ("MAILGUN_DOMAIN", "mg.example.com"),
                ("MAILGUN_REGION", "EU"),
            ])
            .unwrap(),
            MailTransport::Mailgun {
                api_key: "k".into(),
                domain: "mg.example.com".into(),
                region: MailgunRegion::Eu,
            }
        );
        assert_eq!(
            parse(&[
                ("MAIL_TRANSPORT", "scaleway"),
                ("SCW_SECRET_KEY", "s"),
                ("SCW_PROJECT_ID", "p")
            ])
            .unwrap(),
            MailTransport::Scaleway {
                secret_key: "s".into(),
                project_id: "p".into(),
                region: "fr-par".into(),
            }
        );
    }

    #[test]
    fn missing_variables_are_named() {
        assert!(error(&[("MAIL_TRANSPORT", "brevo")]).contains("BREVO_API_KEY"));
        // An empty value counts as missing (`BREVO_API_KEY=` in .env).
        let err = error(&[("MAIL_TRANSPORT", "resend"), ("RESEND_API_KEY", " ")]);
        assert!(err.contains("RESEND_API_KEY"), "{err}");
        assert!(error(&[("MAIL_TRANSPORT", "postmark")]).contains("POSTMARK_SERVER_TOKEN"));
        let err = error(&[("MAIL_TRANSPORT", "mailgun"), ("MAILGUN_API_KEY", "k")]);
        assert!(err.contains("MAILGUN_DOMAIN"), "{err}");
        let err = error(&[("MAIL_TRANSPORT", "scaleway"), ("SCW_SECRET_KEY", "s")]);
        assert!(err.contains("SCW_PROJECT_ID"), "{err}");
        let err = error(&[
            ("MAIL_TRANSPORT", "mailgun"),
            ("MAILGUN_API_KEY", "k"),
            ("MAILGUN_DOMAIN", "d"),
            ("MAILGUN_REGION", "asia"),
        ]);
        assert!(err.contains("MAILGUN_REGION"), "{err}");
    }

    #[test]
    fn none_turns_email_off() {
        assert_eq!(parse(&[("MAIL_TRANSPORT", "none")]).unwrap(), MailTransport::None);
    }

    #[test]
    fn unknown_transport_lists_valid_ones() {
        let err = error(&[("MAIL_TRANSPORT", "ses")]);
        assert!(err.contains("ses") && err.contains(MAIL_TRANSPORTS), "{err}");
    }

    #[test]
    fn smtp_tls_and_default_ports() {
        let mailpit = smtp(&[("SMTP_HOST", "mailpit")]);
        assert_eq!((mailpit.tls, mailpit.port), (SmtpTls::None, 1025));
        let remote = smtp(&[("SMTP_HOST", "smtp.example.com")]);
        assert_eq!((remote.tls, remote.port), (SmtpTls::StartTls, 587));
        let implicit = smtp(&[("SMTP_HOST", "smtp.example.com"), ("SMTP_TLS", "tls")]);
        assert_eq!((implicit.tls, implicit.port), (SmtpTls::Tls, 465));
        let custom = smtp(&[
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_TLS", "starttls"),
            ("SMTP_PORT", "2525"),
        ]);
        assert_eq!((custom.tls, custom.port), (SmtpTls::StartTls, 2525));
        // Port 465 without SMTP_TLS is implicit TLS, not STARTTLS.
        let gmail = smtp(&[("SMTP_HOST", "smtp.gmail.com"), ("SMTP_PORT", "465")]);
        assert_eq!((gmail.tls, gmail.port), (SmtpTls::Tls, 465));
        // Brackets are not a host lettre can resolve: not treated as local.
        assert_eq!(smtp(&[("SMTP_HOST", "[::1]")]).tls, SmtpTls::StartTls);
        assert_eq!(smtp(&[("SMTP_HOST", "::1")]).tls, SmtpTls::None);
        assert!(error(&[("SMTP_TLS", "ssl")]).contains("SMTP_TLS"));
        assert!(error(&[("SMTP_PORT", "x")]).contains("SMTP_PORT"));
    }

    #[test]
    fn smtp_credentials() {
        let auth = smtp(&[
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_USERNAME", "user"),
            ("SMTP_PASSWORD", "secret"),
        ]);
        assert_eq!(auth.credentials, Some(("user".into(), "secret".into())));
        // The password is not trimmed; the other values are.
        let spaced = smtp(&[
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_USERNAME", " user "),
            ("SMTP_PASSWORD", " secret "),
        ]);
        assert_eq!(spaced.credentials, Some(("user".into(), " secret ".into())));
        assert!(error(&[("SMTP_USERNAME", "user"), ("SMTP_PASSWORD", "  ")]).contains("SMTP_PASSWORD"));
        assert!(error(&[("SMTP_USERNAME", "user")]).contains("SMTP_PASSWORD"));
        assert!(error(&[("SMTP_PASSWORD", "secret")]).contains("SMTP_USERNAME"));
    }

    #[test]
    fn smtp_refuses_clear_text_password() {
        let err = error(&[
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_TLS", "none"),
            ("SMTP_USERNAME", "user"),
            ("SMTP_PASSWORD", "secret"),
        ]);
        assert!(err.contains("clear text") && !err.contains("secret"), "{err}");
        // Towards a local host it is allowed (Mailpit, with authentication enabled).
        let local =
            smtp(&[("SMTP_HOST", "127.0.0.1"), ("SMTP_USERNAME", "u"), ("SMTP_PASSWORD", "p")]);
        assert_eq!(local.tls, SmtpTls::None);
        // Without credentials nothing secret travels.
        let open = smtp(&[("SMTP_HOST", "relay.internal"), ("SMTP_TLS", "none")]);
        assert_eq!(open.credentials, None);
    }
}
