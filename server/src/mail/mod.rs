//! Email sending: one HTTP API per provider, or standard SMTP.
//!
//! The HTTP APIs also work where outgoing SMTP ports are blocked (some
//! DigitalOcean accounts); SMTP is the fallback for every other provider and
//! reaches Mailpit locally. No SDKs: one `reqwest` call or `lettre`.

mod brevo;
mod mailgun;
mod postmark;
mod resend;
mod scaleway;
mod smtp;

use std::time::Duration;

use anyhow::{Context, bail};
use lettre::{AsyncSmtpTransport, Tokio1Executor};

use crate::config::{MailConfig, MailTransport, MailgunRegion};

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
    http: reqwest::Client,
    /// Provider API origin; overridden only in tests.
    base_url: String,
    transport: Transport,
}

#[derive(Clone)]
enum Transport {
    Brevo { api_key: String },
    Resend { api_key: String },
    Postmark { server_token: String, message_stream: String },
    Mailgun { api_key: String, domain: String },
    Scaleway { secret_key: String, project_id: String, region: String },
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
}

impl Mailer {
    pub fn new(config: &MailConfig) -> anyhow::Result<Self> {
        let (base_url, transport) = match &config.transport {
            MailTransport::Brevo { api_key } => {
                (brevo::BASE_URL, Transport::Brevo { api_key: api_key.clone() })
            }
            MailTransport::Resend { api_key } => {
                (resend::BASE_URL, Transport::Resend { api_key: api_key.clone() })
            }
            MailTransport::Postmark { server_token, message_stream } => (
                postmark::BASE_URL,
                Transport::Postmark {
                    server_token: server_token.clone(),
                    message_stream: message_stream.clone(),
                },
            ),
            MailTransport::Mailgun { api_key, domain, region } => (
                match region {
                    MailgunRegion::Us => mailgun::BASE_URL_US,
                    MailgunRegion::Eu => mailgun::BASE_URL_EU,
                },
                Transport::Mailgun { api_key: api_key.clone(), domain: domain.clone() },
            ),
            MailTransport::Scaleway { secret_key, project_id, region } => (
                scaleway::BASE_URL,
                Transport::Scaleway {
                    secret_key: secret_key.clone(),
                    project_id: project_id.clone(),
                    region: region.clone(),
                },
            ),
            MailTransport::Smtp(smtp) => ("", Transport::Smtp(smtp::transport(smtp)?)),
        };
        Ok(Self {
            from_email: config.from_email.clone(),
            from_name: config.from_name.clone(),
            http: reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?,
            base_url: base_url.to_owned(),
            transport,
        })
    }

    /// Provider name for the log, never the credentials.
    pub fn describe(&self) -> &'static str {
        match self.transport {
            Transport::Brevo { .. } => "Brevo",
            Transport::Resend { .. } => "Resend",
            Transport::Postmark { .. } => "Postmark",
            Transport::Mailgun { .. } => "Mailgun",
            Transport::Scaleway { .. } => "Scaleway",
            Transport::Smtp(_) => "SMTP",
        }
    }

    pub async fn send(&self, email: &Email) -> anyhow::Result<()> {
        let request = match &self.transport {
            Transport::Brevo { api_key } => brevo::request(self, api_key, email),
            Transport::Resend { api_key } => resend::request(self, api_key, email),
            Transport::Postmark { server_token, message_stream } => {
                postmark::request(self, server_token, message_stream, email)
            }
            Transport::Mailgun { api_key, domain } => {
                mailgun::request(self, api_key, domain, email)
            }
            Transport::Scaleway { secret_key, project_id, region } => {
                scaleway::request(self, secret_key, project_id, region, email)
            }
            Transport::Smtp(transport) => return smtp::send(self, transport, email).await,
        };
        check(self.describe(), request.send().await).await
    }

    /// Sender as `Name <email>`, for the APIs that take a single string.
    fn sender(&self) -> String {
        // `Mailbox` quotes the name when needed; an invalid address is left to the provider.
        match self.from_email.parse() {
            Ok(address) => {
                lettre::message::Mailbox::new(Some(self.from_name.clone()), address).to_string()
            }
            Err(_) => format!("{} <{}>", self.from_name, self.from_email),
        }
    }

    /// Same mailer towards a test server.
    #[cfg(test)]
    fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.to_owned();
        self
    }
}

/// Longest part of an error response kept in the log.
const ERROR_BODY_MAX: usize = 300;

/// Success on 2xx; otherwise an error with the status and the (truncated) response body.
async fn check(
    provider: &str,
    response: Result<reqwest::Response, reqwest::Error>,
) -> anyhow::Result<()> {
    let response = response.with_context(|| format!("{provider} unreachable"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let mut detail = response.text().await.unwrap_or_default();
    if detail.len() > ERROR_BODY_MAX {
        let mut end = ERROR_BODY_MAX;
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
        detail.push('…');
    }
    bail!("{provider} answered {status}: {detail}");
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{body::Bytes, extract::Request, http::StatusCode};

    use super::*;
    use crate::config::MailConfig;

    /// What the test server received.
    #[derive(Debug, Default, Clone)]
    pub(super) struct Received {
        pub method: String,
        pub path: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Received {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
        }
    }

    /// Server on 127.0.0.1 that records the request and answers `status` with `body`.
    pub(super) async fn server(
        status: StatusCode,
        body: &'static str,
    ) -> (String, Arc<Mutex<Received>>) {
        let received = Arc::new(Mutex::new(Received::default()));
        let store = received.clone();
        let app = axum::Router::new().fallback(move |request: Request| {
            let store = store.clone();
            async move {
                let (parts, bytes) = request.into_parts();
                let bytes: Bytes = axum::body::to_bytes(bytes, 1 << 20).await.unwrap();
                *store.lock().unwrap() = Received {
                    method: parts.method.to_string(),
                    path: parts.uri.to_string(),
                    headers: parts
                        .headers
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_owned()))
                        .collect(),
                    body: String::from_utf8(bytes.to_vec()).unwrap(),
                };
                (status, body)
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, received)
    }

    pub(super) fn mailer(transport: MailTransport, base_url: &str) -> Mailer {
        let config = MailConfig {
            from_email: "noreply@example.com".into(),
            from_name: "IronHeart".into(),
            transport,
        };
        Mailer::new(&config).unwrap().with_base_url(base_url)
    }

    pub(super) fn email() -> Email {
        Email {
            to: "a@example.com".into(),
            subject: "Subject".into(),
            text: "Hello".into(),
            html: "<p>Hello</p>".into(),
        }
    }

    #[test]
    fn sender_has_name() {
        let mailer = mailer(MailTransport::Resend { api_key: "k".into() }, "");
        assert_eq!(mailer.sender(), "IronHeart <noreply@example.com>");
    }

    #[tokio::test]
    async fn error_response_reports_status_and_truncated_body() {
        let long = "x".repeat(1000).leak();
        let (url, _) = server(StatusCode::UNPROCESSABLE_ENTITY, long).await;
        let mailer = mailer(MailTransport::Resend { api_key: "k".into() }, &url);
        let err = format!("{:#}", mailer.send(&email()).await.unwrap_err());
        assert!(err.starts_with("Resend answered 422"), "{err}");
        assert!(err.len() < 400, "body not truncated: {} bytes", err.len());
    }

    #[tokio::test]
    async fn unreachable_provider_is_an_error() {
        let mailer = mailer(MailTransport::Brevo { api_key: "k".into() }, "http://127.0.0.1:9");
        let err = format!("{:#}", mailer.send(&email()).await.unwrap_err());
        assert!(err.starts_with("Brevo unreachable"), "{err}");
    }
}
