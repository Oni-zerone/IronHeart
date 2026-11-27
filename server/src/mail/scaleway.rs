//! Scaleway Transactional Email: <https://www.scaleway.com/en/developers/api/transactional-email/>.

use serde::Serialize;

use super::{Email, Mailer};

pub const BASE_URL: &str = "https://api.scaleway.com";

pub fn request(
    mailer: &Mailer,
    secret_key: &str,
    project_id: &str,
    region: &str,
    email: &Email,
) -> reqwest::RequestBuilder {
    mailer
        .http
        .post(format!("{}/transactional-email/v1alpha1/regions/{region}/emails", mailer.base_url))
        .header("x-auth-token", secret_key)
        .json(&payload(mailer, project_id, email))
}

fn payload<'a>(mailer: &'a Mailer, project_id: &'a str, email: &'a Email) -> Payload<'a> {
    Payload {
        from: Address { email: &mailer.from_email, name: Some(&mailer.from_name) },
        to: [Address { email: &email.to, name: None }],
        subject: &email.subject,
        text: &email.text,
        html: &email.html,
        project_id,
    }
}

#[derive(Serialize)]
struct Payload<'a> {
    from: Address<'a>,
    to: [Address<'a>; 1],
    subject: &'a str,
    text: &'a str,
    html: &'a str,
    project_id: &'a str,
}

#[derive(Serialize)]
struct Address<'a> {
    email: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

/// A send with only the project: 400 when the key is valid and may send in the
/// project, 401/403 otherwise. A key with only the permission to send
/// (`TransactionalEmailEmailApiCreate`) cannot read, so a read cannot be used.
pub fn check(
    mailer: &Mailer,
    secret_key: &str,
    project_id: &str,
    region: &str,
) -> reqwest::RequestBuilder {
    mailer
        .http
        .post(format!("{}/transactional-email/v1alpha1/regions/{region}/emails", mailer.base_url))
        .header("x-auth-token", secret_key)
        .json(&serde_json::json!({ "project_id": project_id }))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::{config::MailTransport, mail::tests::*};

    fn transport() -> MailTransport {
        MailTransport::Scaleway {
            secret_key: "scw-1".into(),
            project_id: "p-1".into(),
            region: "fr-par".into(),
        }
    }

    #[test]
    fn payload_matches_api() {
        let mailer = mailer(transport(), "");
        assert_eq!(
            serde_json::to_value(payload(&mailer, "p-1", &email())).unwrap(),
            serde_json::json!({
                "from": { "email": "noreply@example.com", "name": "IronHeart" },
                "to": [{ "email": "a@example.com" }],
                "subject": "Subject",
                "text": "Hello",
                "html": "<p>Hello</p>",
                "project_id": "p-1",
            })
        );
    }

    #[tokio::test]
    async fn request_reaches_api() {
        let (url, received) = server(StatusCode::OK, r#"{"emails":[]}"#).await;
        mailer(transport(), &url).send(&email()).await.unwrap();
        let received = received.lock().unwrap();
        assert_eq!(
            (received.method.as_str(), received.path.as_str()),
            ("POST", "/transactional-email/v1alpha1/regions/fr-par/emails")
        );
        assert_eq!(received.header("x-auth-token"), Some("scw-1"));
        assert_eq!(received.header("content-type"), Some("application/json"));
    }

    #[tokio::test]
    async fn key_check_sends_nothing() {
        let (url, received) = server(StatusCode::FORBIDDEN, r#"{"type":"permissions_denied"}"#).await;
        let transport = MailTransport::Scaleway {
            secret_key: "scw-1".into(),
            project_id: "p-1".into(),
            region: "fr-par".into(),
        };
        let mailer = mailer(transport, &url);
        assert!(matches!(mailer.check_key().await, Some(crate::mail::KeyCheck::Rejected(_))));
        let received = received.lock().unwrap();
        assert_eq!(
            (received.method.as_str(), received.path.as_str()),
            ("POST", "/transactional-email/v1alpha1/regions/fr-par/emails")
        );
        assert_eq!(received.header("x-auth-token"), Some("scw-1"));
        assert_eq!(received.body, r#"{"project_id":"p-1"}"#);
    }
}
