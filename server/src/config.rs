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
