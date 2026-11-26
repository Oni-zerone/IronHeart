//! Invio delle email: Brevo (API HTTP) in produzione, SMTP verso Mailpit in locale.
//!
//! Su DigitalOcean le porte SMTP in uscita sono bloccate, quindi in produzione
//! si usa solo l'API HTTP.

use anyhow::{Context, bail};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart},
};
use serde::Serialize;

use crate::config::{MailConfig, MailTransport};

const BREVO_URL: &str = "https://api.brevo.com/v3/smtp/email";

/// Una email verso un solo destinatario: con più destinatari si invia una
/// email per ciascuno, così nessuno vede gli indirizzi degli altri.
pub struct Email {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: String,
}

#[derive(Clone)]
pub struct Mailer {
    from_email: String,
    from_name: String,
    transport: Transport,
}

#[derive(Clone)]
enum Transport {
    Brevo { http: reqwest::Client, api_key: String },
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
}

impl Mailer {
    pub fn new(config: &MailConfig) -> anyhow::Result<Self> {
        let transport = match &config.transport {
            MailTransport::Brevo { api_key } => Transport::Brevo {
                http: reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()?,
                api_key: api_key.clone(),
            },
            MailTransport::Smtp { host, port } => Transport::Smtp(
                // Solo per Mailpit in locale: nessuna cifratura né autenticazione.
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
                    .port(*port)
                    .build(),
            ),
        };
        Ok(Self {
            from_email: config.from_email.clone(),
            from_name: config.from_name.clone(),
            transport,
        })
    }

    pub fn describe(&self) -> &'static str {
        match self.transport {
            Transport::Brevo { .. } => "Brevo",
            Transport::Smtp(_) => "SMTP",
        }
    }

    pub async fn send(&self, email: &Email) -> anyhow::Result<()> {
        match &self.transport {
            Transport::Brevo { http, api_key } => self.send_brevo(http, api_key, email).await,
            Transport::Smtp(smtp) => self.send_smtp(smtp, email).await,
        }
    }

    async fn send_brevo(
        &self,
        http: &reqwest::Client,
        api_key: &str,
        email: &Email,
    ) -> anyhow::Result<()> {
        let body = BrevoEmail {
            sender: BrevoAddress { email: &self.from_email, name: Some(&self.from_name) },
            to: [BrevoAddress { email: &email.to, name: None }],
            subject: &email.subject,
            html_content: &email.html,
            text_content: &email.text,
        };
        let response = http
            .post(BREVO_URL)
            .header("api-key", api_key)
            .header("accept", "application/json")
            .json(&body)
            .send()
            .await
            .context("Brevo non raggiungibile")?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!("Brevo ha risposto {status}: {detail}");
        }
        Ok(())
    }

    async fn send_smtp(
        &self,
        smtp: &AsyncSmtpTransport<Tokio1Executor>,
        email: &Email,
    ) -> anyhow::Result<()> {
        let from = Mailbox::new(Some(self.from_name.clone()), self.from_email.parse()?);
        let message = Message::builder()
            .from(from)
            .to(email.to.parse().context("destinatario non valido")?)
            .subject(&email.subject)
            .multipart(MultiPart::alternative_plain_html(
                email.text.clone(),
                email.html.clone(),
            ))?;
        smtp.send(message).await.context("invio SMTP non riuscito")?;
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrevoEmail<'a> {
    sender: BrevoAddress<'a>,
    to: [BrevoAddress<'a>; 1],
    subject: &'a str,
    html_content: &'a str,
    text_content: &'a str,
}

#[derive(Serialize)]
struct BrevoAddress<'a> {
    email: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brevo_payload_matches_api() {
        let body = BrevoEmail {
            sender: BrevoAddress { email: "noreply@example.com", name: Some("IronHeart") },
            to: [BrevoAddress { email: "a@example.com", name: None }],
            subject: "Oggetto",
            html_content: "<p>Ciao</p>",
            text_content: "Ciao",
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "sender": { "email": "noreply@example.com", "name": "IronHeart" },
                "to": [{ "email": "a@example.com" }],
                "subject": "Oggetto",
                "htmlContent": "<p>Ciao</p>",
                "textContent": "Ciao",
            })
        );
    }
}
