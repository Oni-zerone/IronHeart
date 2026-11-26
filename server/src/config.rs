use std::net::SocketAddr;

use anyhow::Context;

pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    /// Indirizzo da cui il browser raggiunge il servizio, senza `/` finale.
    /// In sviluppo con Vite è `http://localhost:5173`.
    pub public_base_url: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub mail: MailConfig,
}

pub struct MailConfig {
    pub from_email: String,
    pub from_name: String,
    pub transport: MailTransport,
}

pub enum MailTransport {
    /// Produzione: API transazionale di Brevo.
    Brevo { api_key: String },
    /// Sviluppo: SMTP senza cifratura verso Mailpit.
    Smtp { host: String, port: u16 },
}

impl MailConfig {
    fn from_env() -> anyhow::Result<Self> {
        let transport = match env_or("MAIL_TRANSPORT", "smtp").as_str() {
            "brevo" => MailTransport::Brevo { api_key: required("BREVO_API_KEY")? },
            "smtp" => MailTransport::Smtp {
                host: env_or("SMTP_HOST", "localhost"),
                port: env_or("SMTP_PORT", "1025").parse().context("SMTP_PORT non valido")?,
            },
            other => anyhow::bail!("MAIL_TRANSPORT sconosciuto: {other} (brevo o smtp)"),
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
            .context("BIND_ADDR non valido")?;
        Ok(Self {
            bind_addr,
            database_url: env_or("DATABASE_URL", "sqlite://data/ironheart.db"),
            public_base_url: env_or("PUBLIC_BASE_URL", "http://localhost:8080")
                .trim_end_matches('/')
                .to_owned(),
            google_client_id: required("GOOGLE_CLIENT_ID")?,
            google_client_secret: required("GOOGLE_CLIENT_SECRET")?,
            mail: MailConfig::from_env()?,
        })
    }

    /// Cookie `Secure` solo quando il servizio è esposto in HTTPS.
    pub fn secure_cookies(&self) -> bool {
        self.public_base_url.starts_with("https://")
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn required(key: &str) -> anyhow::Result<String> {
    std::env::var(key).with_context(|| format!("manca la variabile {key}"))
}
