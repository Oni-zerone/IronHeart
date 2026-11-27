//! Brevo transactional API: <https://developers.brevo.com/reference/sendtransacemail>.

use serde::Serialize;

use super::{Email, Mailer};

pub const BASE_URL: &str = "https://api.brevo.com";

pub fn request(mailer: &Mailer, api_key: &str, email: &Email) -> reqwest::RequestBuilder {
    mailer
        .http
        .post(format!("{}/v3/smtp/email", mailer.base_url))
        .header("api-key", api_key)
        .header("accept", "application/json")
        .json(&payload(mailer, email))
}

fn payload<'a>(mailer: &'a Mailer, email: &'a Email) -> Payload<'a> {
    Payload {
        sender: Address { email: &mailer.from_email, name: Some(&mailer.from_name) },
        to: [Address { email: &email.to, name: None }],
        subject: &email.subject,
        html_content: &email.html,
        text_content: &email.text,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    sender: Address<'a>,
    to: [Address<'a>; 1],
    subject: &'a str,
    html_content: &'a str,
    text_content: &'a str,
}

#[derive(Serialize)]
struct Address<'a> {
    email: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

/// Cheapest authenticated read: checks the key and keeps it in use.
pub fn check(mailer: &Mailer, api_key: &str) -> reqwest::RequestBuilder {
    mailer
        .http
        .get(format!("{}/v3/account", mailer.base_url))
        .header("api-key", api_key)
        .header("accept", "application/json")
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::{config::MailTransport, mail::tests::*};

    #[test]
    fn payload_matches_api() {
        let mailer = mailer(MailTransport::Brevo { api_key: "k".into() }, "");
        assert_eq!(
            serde_json::to_value(payload(&mailer, &email())).unwrap(),
            serde_json::json!({
                "sender": { "email": "noreply@example.com", "name": "IronHeart" },
                "to": [{ "email": "a@example.com" }],
                "subject": "Subject",
                "htmlContent": "<p>Hello</p>",
                "textContent": "Hello",
            })
        );
    }

    #[tokio::test]
    async fn request_reaches_api() {
        let (url, received) = server(StatusCode::CREATED, "{}").await;
        let mailer = mailer(MailTransport::Brevo { api_key: "xkeysib-1".into() }, &url);
        mailer.send(&email()).await.unwrap();
        let received = received.lock().unwrap();
        assert_eq!((received.method.as_str(), received.path.as_str()), ("POST", "/v3/smtp/email"));
        assert_eq!(received.header("api-key"), Some("xkeysib-1"));
        assert_eq!(received.header("content-type"), Some("application/json"));
    }

    #[tokio::test]
    async fn key_check_reads_the_account() {
        let (url, received) = server(StatusCode::OK, "{}").await;
        let mailer = mailer(MailTransport::Brevo { api_key: "xkeysib-1".into() }, &url);
        assert_eq!(mailer.check_key().await, Some(crate::mail::KeyCheck::Valid));
        let received = received.lock().unwrap();
        assert_eq!((received.method.as_str(), received.path.as_str()), ("GET", "/v3/account"));
        assert_eq!(received.header("api-key"), Some("xkeysib-1"));
    }
}
