//! Mailgun: <https://documentation.mailgun.com/docs/mailgun/api-reference/send/mailgun/messages/post-v3--domain-name--messages>.

use super::{Email, Mailer};

pub const BASE_URL_US: &str = "https://api.mailgun.net";
pub const BASE_URL_EU: &str = "https://api.eu.mailgun.net";

pub fn request(
    mailer: &Mailer,
    api_key: &str,
    domain: &str,
    email: &Email,
) -> reqwest::RequestBuilder {
    mailer
        .http
        .post(format!("{}/v3/{domain}/messages", mailer.base_url))
        .basic_auth("api", Some(api_key))
        .form(&form(mailer, email))
}

/// Form fields (urlencoded: no attachments, so multipart is not needed).
fn form<'a>(mailer: &Mailer, email: &'a Email) -> [(&'static str, std::borrow::Cow<'a, str>); 5] {
    [
        ("from", mailer.sender().into()),
        ("to", email.to.as_str().into()),
        ("subject", email.subject.as_str().into()),
        ("text", email.text.as_str().into()),
        ("html", email.html.as_str().into()),
    ]
}

/// A send with no fields: 400 when the key is valid, 401 when it is not. Domain
/// sending keys may only send, so a read cannot be used.
pub fn check(mailer: &Mailer, api_key: &str, domain: &str) -> reqwest::RequestBuilder {
    mailer
        .http
        .post(format!("{}/v3/{domain}/messages", mailer.base_url))
        .basic_auth("api", Some(api_key))
        .form(&[] as &[(&str, &str)])
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::{
        config::{MailTransport, MailgunRegion},
        mail::tests::*,
    };

    fn transport(region: MailgunRegion) -> MailTransport {
        MailTransport::Mailgun {
            api_key: "key-1".into(),
            domain: "mg.example.com".into(),
            region,
        }
    }

    #[test]
    fn form_matches_api() {
        let mailer = mailer(transport(MailgunRegion::Us), "");
        let fields: Vec<(&str, String)> = form(&mailer, &email())
            .into_iter()
            .map(|(k, v)| (k, v.into_owned()))
            .collect();
        assert_eq!(
            fields,
            [
                ("from", "IronHeart <noreply@example.com>".to_owned()),
                ("to", "a@example.com".to_owned()),
                ("subject", "Subject".to_owned()),
                ("text", "Hello".to_owned()),
                ("html", "<p>Hello</p>".to_owned()),
            ]
        );
    }

    #[test]
    fn region_picks_base_url() {
        let eu = crate::config::MailConfig {
            from_email: "noreply@example.com".into(),
            from_name: "IronHeart".into(),
            transport: transport(MailgunRegion::Eu),
        };
        assert_eq!(Mailer::new(&eu).unwrap().base_url, BASE_URL_EU);
    }

    #[tokio::test]
    async fn request_reaches_api() {
        let (url, received) = server(StatusCode::OK, r#"{"id":"1"}"#).await;
        mailer(transport(MailgunRegion::Us), &url)
            .send(&email())
            .await
            .unwrap();
        let received = received.lock().unwrap();
        assert_eq!(
            (received.method.as_str(), received.path.as_str()),
            ("POST", "/v3/mg.example.com/messages")
        );
        // Basic base64("api:key-1").
        assert_eq!(received.header("authorization"), Some("Basic YXBpOmtleS0x"));
        assert_eq!(
            received.header("content-type"),
            Some("application/x-www-form-urlencoded")
        );
        assert!(
            received.body.contains("to=a%40example.com"),
            "{}",
            received.body
        );
        assert!(
            received.body.contains("html=%3Cp%3EHello%3C%2Fp%3E"),
            "{}",
            received.body
        );
    }

    #[tokio::test]
    async fn key_check_sends_nothing() {
        let (url, received) = server(StatusCode::BAD_REQUEST, "{}").await;
        let mailer = mailer(transport(MailgunRegion::Eu), &url);
        assert_eq!(mailer.check_key().await, Some(crate::mail::KeyCheck::Valid));
        let received = received.lock().unwrap();
        assert_eq!(
            (received.method.as_str(), received.path.as_str()),
            ("POST", "/v3/mg.example.com/messages")
        );
        assert!(
            received
                .header("authorization")
                .unwrap()
                .starts_with("Basic ")
        );
        assert_eq!(received.body, "");
    }
}
