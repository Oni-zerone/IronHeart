use std::net::SocketAddr;

use anyhow::Context;

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
    /// `x-forwarded-for` (CapRover) o `cf-connecting-ip` (Cloudflare).
    pub client_ip_header: Option<String>,
}

pub struct MailConfig {
    pub from_email: String,
    pub from_name: String,
    pub transport: MailTransport,
}

pub enum MailTransport {
    /// Production: Brevo transactional API.
    Brevo { api_key: String },
    /// Development: unencrypted SMTP to Mailpit.
    Smtp { host: String, port: u16 },
}

impl MailConfig {
    fn from_env() -> anyhow::Result<Self> {
        let transport = match env_or("MAIL_TRANSPORT", "smtp").as_str() {
            "brevo" => MailTransport::Brevo { api_key: required("BREVO_API_KEY")? },
            "smtp" => MailTransport::Smtp {
                host: env_or("SMTP_HOST", "localhost"),
                port: env_or("SMTP_PORT", "1025").parse().context("invalid SMTP_PORT")?,
            },
            other => anyhow::bail!("unknown MAIL_TRANSPORT: {other} (brevo or smtp)"),
        };
        Ok(Self {
            from_email: env_or("MAIL_FROM_EMAIL", "noreply@ironheart.local"),
            from_name: env_or("MAIL_FROM_NAME", "IronHeart"),
            transport,
        })
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = env_or("BIND_ADDR", "127.0.0.1:8080")
            .parse()
            .context("invalid BIND_ADDR")?;
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
            mail: MailConfig::from_env()?,
            secrets_key: secrets_key()?,
            secrets_key_version: env_or("SECRETS_KEY_VERSION", "1")
                .parse()
                .context("invalid SECRETS_KEY_VERSION")?,
            client_ip_header: std::env::var("CLIENT_IP_HEADER")
                .ok()
                .map(|h| h.trim().to_lowercase())
                .filter(|h| !h.is_empty()),
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
