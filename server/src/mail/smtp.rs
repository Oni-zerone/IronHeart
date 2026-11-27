//! Standard SMTP: Mailpit locally, any other provider in production
//! (Brevo SMTP, Amazon SES, Gmail, a server of your own…).

use std::time::Duration;

use anyhow::Context;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart},
    transport::smtp::{
        authentication::Credentials,
        client::{Tls, TlsParameters},
    },
};

use super::{Email, KeyCheck, Mailer};
use crate::config::{SmtpConfig, SmtpTls};

/// TLS with rustls and the webpki roots, the same stack as `reqwest`.
pub fn transport(config: &SmtpConfig) -> anyhow::Result<AsyncSmtpTransport<Tokio1Executor>> {
    let tls = match config.tls {
        // Plain text: the configuration allows it only towards a local host
        // when there are credentials.
        SmtpTls::None => Tls::None,
        SmtpTls::StartTls => Tls::Required(TlsParameters::new(config.host.clone())?),
        SmtpTls::Tls => Tls::Wrapper(TlsParameters::new(config.host.clone())?),
    };
    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
        .port(config.port)
        .tls(tls)
        .timeout(Some(Duration::from_secs(15)));
    if let Some((user, password)) = &config.credentials {
        builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
    }
    Ok(builder.build())
}

/// Connects and logs in, then quits. A permanent (5xx) answer, such as a
/// refused login, cannot be fixed by retrying: alerts would not go out.
pub async fn check(transport: &AsyncSmtpTransport<Tokio1Executor>) -> KeyCheck {
    match transport.test_connection().await {
        Ok(true) => KeyCheck::Valid,
        Ok(false) => KeyCheck::Unreachable("SMTP server did not answer".into()),
        Err(err) if err.is_permanent() => KeyCheck::Rejected(format!("SMTP server refused: {err}")),
        Err(err) => KeyCheck::Unreachable(format!("SMTP unreachable: {err}")),
    }
}

pub async fn send(
    mailer: &Mailer,
    transport: &AsyncSmtpTransport<Tokio1Executor>,
    email: &Email,
) -> anyhow::Result<()> {
    let from = Mailbox::new(Some(mailer.from_name.clone()), mailer.from_email.parse()?);
    let message = Message::builder()
        .from(from)
        .to(email.to.parse().context("invalid recipient")?)
        .subject(&email.subject)
        .multipart(MultiPart::alternative_plain_html(email.text.clone(), email.html.clone()))?;
    transport.send(message).await.context("SMTP send failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tls_mode_builds() {
        for tls in [SmtpTls::None, SmtpTls::StartTls, SmtpTls::Tls] {
            let config = SmtpConfig {
                host: "smtp.example.com".into(),
                port: 587,
                tls,
                credentials: Some(("user".into(), "secret".into())),
            };
            transport(&config).unwrap();
        }
    }
}
