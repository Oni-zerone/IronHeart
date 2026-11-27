//! Email sending: Brevo (HTTP API) in production, SMTP to Mailpit locally.
//!
//! On DigitalOcean outgoing SMTP ports are blocked, so in production only
//! the HTTP API is used.

use anyhow::{Context, bail};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart},
};
use serde::Serialize;

use crate::config::{MailConfig, MailTransport};

const BREVO_URL: &str = "https://api.brevo.com/v3/smtp/email";
/// Cheapest authenticated call: used to check (and keep alive) the API key.
const BREVO_ACCOUNT_URL: &str = "https://api.brevo.com/v3/account";

/// Outcome of a check of the Brevo API key.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyCheck {
    Valid,
    /// Brevo refused the key (401/403): expired, revoked or wrong.
    Rejected(String),
    /// Brevo unreachable or answering with another error: retry later.
    Unreachable(String),
}

impl KeyCheck {
    fn from_response(status: reqwest::StatusCode, body: &str) -> Self {
        let detail = || {
            let body: String = body.chars().take(200).collect();
            format!("Brevo answered {status}: {body}")
        };
        if status.is_success() {
            Self::Valid
        } else if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            Self::Rejected(detail())
        } else {
            Self::Unreachable(detail())
        }
    }
}

/// An email to a single recipient: with several recipients one email is sent
/// to each, so nobody sees the other addresses.
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
                // Only for local Mailpit: no encryption or authentication.
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

    /// Checks the Brevo API key with an authenticated read. `None` with SMTP,
    /// which has no key. Brevo deactivates keys that go unused for 90 days, so
    /// this call also keeps the key alive when no alert has been sent for a while.
    pub async fn check_key(&self) -> Option<KeyCheck> {
        let Transport::Brevo { http, api_key } = &self.transport else { return None };
        let response = http
            .get(BREVO_ACCOUNT_URL)
            .header("api-key", api_key)
            .header("accept", "application/json")
            .send()
            .await;
        Some(match response {
            Ok(response) => {
                let status = response.status();
                let body = if status.is_success() { String::new() } else { response.text().await.unwrap_or_default() };
                KeyCheck::from_response(status, &body)
            }
            Err(err) => KeyCheck::Unreachable(format!("Brevo unreachable: {err}")),
        })
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
            .context("Brevo unreachable")?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!("Brevo answered {status}: {detail}");
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
            .to(email.to.parse().context("invalid recipient")?)
            .subject(&email.subject)
            .multipart(MultiPart::alternative_plain_html(
                email.text.clone(),
                email.html.clone(),
            ))?;
        smtp.send(message).await.context("SMTP send failed")?;
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
    fn key_check_classifies_responses() {
        use reqwest::StatusCode;
        assert_eq!(KeyCheck::from_response(StatusCode::OK, ""), KeyCheck::Valid);
        assert!(matches!(
            KeyCheck::from_response(StatusCode::UNAUTHORIZED, r#"{"code":"unauthorized"}"#),
            KeyCheck::Rejected(detail) if detail.contains("401") && detail.contains("unauthorized")
        ));
        assert!(matches!(KeyCheck::from_response(StatusCode::FORBIDDEN, ""), KeyCheck::Rejected(_)));
        assert!(matches!(KeyCheck::from_response(StatusCode::BAD_GATEWAY, ""), KeyCheck::Unreachable(_)));
        assert!(matches!(KeyCheck::from_response(StatusCode::TOO_MANY_REQUESTS, ""), KeyCheck::Unreachable(_)));
    }

    #[test]
    fn brevo_payload_matches_api() {
        let body = BrevoEmail {
            sender: BrevoAddress { email: "noreply@example.com", name: Some("IronHeart") },
            to: [BrevoAddress { email: "a@example.com", name: None }],
            subject: "Subject",
            html_content: "<p>Hello</p>",
            text_content: "Hello",
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "sender": { "email": "noreply@example.com", "name": "IronHeart" },
                "to": [{ "email": "a@example.com" }],
                "subject": "Subject",
                "htmlContent": "<p>Hello</p>",
                "textContent": "Hello",
            })
        );
    }
}
