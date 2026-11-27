//! Resend: <https://resend.com/docs/api-reference/emails/send-email>.

use serde::Serialize;

use super::{Email, Mailer};

pub const BASE_URL: &str = "https://api.resend.com";

pub fn request(mailer: &Mailer, api_key: &str, email: &Email) -> reqwest::RequestBuilder {
    mailer
        .http
        .post(format!("{}/emails", mailer.base_url))
        .bearer_auth(api_key)
        .json(&payload(mailer, email))
}

fn payload<'a>(mailer: &Mailer, email: &'a Email) -> Payload<'a> {
    Payload {
        from: mailer.sender(),
        to: [&email.to],
        subject: &email.subject,
        html: &email.html,
        text: &email.text,
    }
}

#[derive(Serialize)]
struct Payload<'a> {
    from: String,
    to: [&'a str; 1],
    subject: &'a str,
    html: &'a str,
    text: &'a str,
}

/// A send with an empty body: refused for the missing fields (4xx) when the
/// key is valid, 401/403 when it is not. Reads are refused to keys with
/// "Sending access" only, so a read cannot be used.
pub fn check(mailer: &Mailer, api_key: &str) -> reqwest::RequestBuilder {
    mailer
        .http
        .post(format!("{}/emails", mailer.base_url))
        .bearer_auth(api_key)
        .json(&serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::{config::MailTransport, mail::tests::*};

    #[test]
    fn payload_matches_api() {
        let mailer = mailer(MailTransport::Resend { api_key: "k".into() }, "");
        assert_eq!(
            serde_json::to_value(payload(&mailer, &email())).unwrap(),
            serde_json::json!({
                "from": "IronHeart <noreply@example.com>",
                "to": ["a@example.com"],
                "subject": "Subject",
                "html": "<p>Hello</p>",
                "text": "Hello",
            })
        );
    }

    #[tokio::test]
    async fn request_reaches_api() {
        let (url, received) = server(StatusCode::OK, r#"{"id":"1"}"#).await;
        let mailer = mailer(MailTransport::Resend { api_key: "re_1".into() }, &url);
        mailer.send(&email()).await.unwrap();
        let received = received.lock().unwrap();
        assert_eq!((received.method.as_str(), received.path.as_str()), ("POST", "/emails"));
        assert_eq!(received.header("authorization"), Some("Bearer re_1"));
        assert_eq!(received.header("content-type"), Some("application/json"));
    }

    #[tokio::test]
    async fn key_check_sends_nothing() {
        let (url, received) = server(StatusCode::UNPROCESSABLE_ENTITY, "{}").await;
        let mailer = mailer(MailTransport::Resend { api_key: "re_1".into() }, &url);
        assert_eq!(mailer.check_key().await, Some(crate::mail::KeyCheck::Valid));
        let received = received.lock().unwrap();
        assert_eq!((received.method.as_str(), received.path.as_str()), ("POST", "/emails"));
        assert_eq!(received.header("authorization"), Some("Bearer re_1"));
        assert_eq!(received.body, "{}");
    }
}
