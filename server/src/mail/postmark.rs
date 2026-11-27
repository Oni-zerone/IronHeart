//! Postmark: <https://postmarkapp.com/developer/api/email-api>.

use serde::Serialize;

use super::{Email, Mailer};

pub const BASE_URL: &str = "https://api.postmarkapp.com";

pub fn request(
    mailer: &Mailer,
    server_token: &str,
    message_stream: &str,
    email: &Email,
) -> reqwest::RequestBuilder {
    mailer
        .http
        .post(format!("{}/email", mailer.base_url))
        .header("x-postmark-server-token", server_token)
        .header("accept", "application/json")
        .json(&payload(mailer, message_stream, email))
}

fn payload<'a>(mailer: &Mailer, message_stream: &'a str, email: &'a Email) -> Payload<'a> {
    Payload {
        from: mailer.sender(),
        to: &email.to,
        subject: &email.subject,
        html_body: &email.html,
        text_body: &email.text,
        message_stream,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Payload<'a> {
    from: String,
    /// A string (comma-separated addresses), not an array.
    to: &'a str,
    subject: &'a str,
    html_body: &'a str,
    text_body: &'a str,
    message_stream: &'a str,
}

/// Reads the server the token belongs to: 200 with a valid Server token, 401
/// otherwise (<https://postmarkapp.com/developer/api/server-api>).
pub fn check(mailer: &Mailer, server_token: &str) -> reqwest::RequestBuilder {
    mailer
        .http
        .get(format!("{}/server", mailer.base_url))
        .header("x-postmark-server-token", server_token)
        .header("accept", "application/json")
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::{config::MailTransport, mail::tests::*};

    fn transport(token: &str) -> MailTransport {
        MailTransport::Postmark { server_token: token.into(), message_stream: "outbound".into() }
    }

    #[test]
    fn payload_matches_api() {
        let mailer = mailer(transport("t"), "");
        assert_eq!(
            serde_json::to_value(payload(&mailer, "outbound", &email())).unwrap(),
            serde_json::json!({
                "From": "IronHeart <noreply@example.com>",
                "To": "a@example.com",
                "Subject": "Subject",
                "HtmlBody": "<p>Hello</p>",
                "TextBody": "Hello",
                "MessageStream": "outbound",
            })
        );
    }

    #[tokio::test]
    async fn request_reaches_api() {
        let (url, received) = server(StatusCode::OK, r#"{"ErrorCode":0}"#).await;
        mailer(transport("pm-1"), &url).send(&email()).await.unwrap();
        let received = received.lock().unwrap();
        assert_eq!((received.method.as_str(), received.path.as_str()), ("POST", "/email"));
        assert_eq!(received.header("x-postmark-server-token"), Some("pm-1"));
        assert_eq!(received.header("content-type"), Some("application/json"));
    }

    #[tokio::test]
    async fn key_check_reads_the_server() {
        let (url, received) = server(StatusCode::UNAUTHORIZED, r#"{"ErrorCode":10}"#).await;
        let transport = MailTransport::Postmark {
            server_token: "pm-1".into(),
            message_stream: "outbound".into(),
        };
        let mailer = mailer(transport, &url);
        assert!(matches!(mailer.check_key().await, Some(crate::mail::KeyCheck::Rejected(_))));
        let received = received.lock().unwrap();
        assert_eq!((received.method.as_str(), received.path.as_str()), ("GET", "/server"));
        assert_eq!(received.header("x-postmark-server-token"), Some("pm-1"));
    }
}
