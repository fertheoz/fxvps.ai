//! Outbound email. Production uses [`SmtpMailer`] (`IDENTITY_SMTP_URL`,
//! `IDENTITY_MAIL_FROM`); [`LogMailer`] is for development only and the binary
//! refuses it when a database is configured (see [`select_mailer`]) unless
//! `IDENTITY_DEV_MAILER=1` is set explicitly.

use std::sync::Mutex;

use async_trait::async_trait;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mail {
    pub to: String,
    pub subject: String,
    pub body: String,
    /// The actionable link (verification / reset), if any.
    pub link: Option<String>,
}

#[async_trait]
pub trait Mailer: Send + Sync + 'static {
    async fn send(&self, mail: Mail) -> Result<(), String>;
}

/// DEVELOPMENT: logs mail instead of sending it. With `file` set
/// (`IDENTITY_DEV_MAIL_FILE`), each message is also appended as a JSON line so
/// scripts and end-to-end tests can follow the emailed links.
#[derive(Default)]
pub struct LogMailer {
    pub file: Option<std::path::PathBuf>,
}

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, mail: Mail) -> Result<(), String> {
        if let Some(f) = &self.file {
            use std::io::Write;
            let line = serde_json::json!({
                "to": mail.to, "subject": mail.subject, "body": mail.body, "link": mail.link,
            });
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(f)
                .and_then(|mut h| writeln!(h, "{line}"))
                .map_err(|e| e.to_string())?;
        }
        // Recipient and subject only: links (verification / password reset) are
        // bearer secrets and must not reach log collectors.
        tracing::warn!(
            to = %mail.to,
            subject = %mail.subject,
            "DEV MAILER: mail not sent (link only in IDENTITY_DEV_MAIL_FILE, if set)"
        );
        Ok(())
    }
}

/// Captures mail in memory (tests).
#[derive(Default)]
pub struct MemoryMailer {
    pub sent: Mutex<Vec<Mail>>,
}

impl MemoryMailer {
    pub fn last_to(&self, to: &str) -> Option<Mail> {
        self.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .rev()
            .find(|m| m.to == to)
            .cloned()
    }

    pub fn count(&self) -> usize {
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

#[async_trait]
impl Mailer for MemoryMailer {
    async fn send(&self, mail: Mail) -> Result<(), String> {
        self.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(mail);
        Ok(())
    }
}

/// Which mailer the binary should use.
#[derive(Debug, PartialEq, Eq)]
pub enum MailerKind {
    Smtp,
    Log,
}

/// Mailer policy (finding G4): SMTP when `IDENTITY_SMTP_URL` is set; the log
/// mailer only for development, i.e. without `DATABASE_URL` or with an explicit
/// `IDENTITY_DEV_MAILER=1`. A production configuration without SMTP is refused.
pub fn select_mailer(
    smtp_url: Option<&str>,
    database_url_set: bool,
    dev_mailer_flag: bool,
) -> Result<MailerKind, String> {
    if smtp_url.is_some() {
        return Ok(MailerKind::Smtp);
    }
    if !database_url_set || dev_mailer_flag {
        return Ok(MailerKind::Log);
    }
    Err(
        "DATABASE_URL is set but IDENTITY_SMTP_URL is not: refusing the development log \
         mailer (verification / reset links would only reach the logs). Configure SMTP \
         or set IDENTITY_DEV_MAILER=1 explicitly for a non-production database."
            .into(),
    )
}

/// Sends mail through any lettre transport (SMTP in production, a stub in tests).
pub struct SmtpMailer<T> {
    transport: T,
    from: lettre::message::Mailbox,
}

impl<T> SmtpMailer<T> {
    pub fn new(transport: T, from: &str) -> Result<Self, String> {
        Ok(SmtpMailer {
            transport,
            from: from
                .parse()
                .map_err(|e| format!("IDENTITY_MAIL_FROM: {e}"))?,
        })
    }
}

impl SmtpMailer<lettre::AsyncSmtpTransport<lettre::Tokio1Executor>> {
    /// From `IDENTITY_SMTP_URL` (`smtps://user:pass@host:465`,
    /// `smtp://host:587?tls=required`) and `IDENTITY_MAIL_FROM`.
    pub fn from_url(url: &str, from: &str) -> Result<Self, String> {
        let t = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::from_url(url)
            .map_err(|e| format!("IDENTITY_SMTP_URL: {e}"))?
            .timeout(Some(std::time::Duration::from_secs(20)))
            .build();
        SmtpMailer::new(t, from)
    }
}

/// Builds the RFC 5322 message for `mail`.
pub fn build_message(
    from: &lettre::message::Mailbox,
    mail: &Mail,
) -> Result<lettre::Message, String> {
    lettre::Message::builder()
        .from(from.clone())
        .to(mail.to.parse().map_err(|e| format!("recipient: {e}"))?)
        .subject(mail.subject.clone())
        .header(lettre::message::header::ContentType::TEXT_PLAIN)
        .body(mail.body.clone())
        .map_err(|e| e.to_string())
}

#[async_trait]
impl<T> Mailer for SmtpMailer<T>
where
    T: lettre::AsyncTransport + Send + Sync + 'static,
    T::Error: std::fmt::Display,
{
    async fn send(&self, mail: Mail) -> Result<(), String> {
        let msg = build_message(&self.from, &mail)?;
        self.transport.send(msg).await.map_err(|e| {
            tracing::error!(to = %mail.to, subject = %mail.subject, error = %e, "mail send failed");
            e.to_string()
        })?;
        tracing::info!(to = %mail.to, subject = %mail.subject, "mail sent");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lettre::transport::stub::AsyncStubTransport;

    #[test]
    fn log_mailer_only_in_dev() {
        assert_eq!(
            select_mailer(Some("smtp://x"), true, false),
            Ok(MailerKind::Smtp)
        );
        assert_eq!(select_mailer(None, false, false), Ok(MailerKind::Log));
        assert_eq!(select_mailer(None, true, true), Ok(MailerKind::Log));
        assert!(select_mailer(None, true, false).is_err());
    }

    fn mail() -> Mail {
        Mail {
            to: "user@example.com".into(),
            subject: "Reset your password".into(),
            body: "Open https://app/reset?t=secret".into(),
            link: Some("https://app/reset?t=secret".into()),
        }
    }

    #[tokio::test]
    async fn smtp_mailer_sends_through_transport() {
        let stub = AsyncStubTransport::new_ok();
        let m = SmtpMailer::new(stub.clone(), "fxvps <no-reply@fxvps.test>").unwrap();
        m.send(mail()).await.unwrap();
        let sent = stub.messages().await;
        assert_eq!(sent.len(), 1);
        let (env, raw) = &sent[0];
        assert_eq!(env.to()[0].to_string(), "user@example.com");
        assert_eq!(env.from().unwrap().to_string(), "no-reply@fxvps.test");
        assert!(raw.contains("Subject: Reset your password"));
        assert!(raw.contains("https://app/reset?t=secret"));
    }

    #[tokio::test]
    async fn smtp_errors_propagate_and_bad_input_is_rejected() {
        let m = SmtpMailer::new(AsyncStubTransport::new_error(), "no-reply@fxvps.test").unwrap();
        assert!(m.send(mail()).await.is_err());
        assert!(SmtpMailer::new(AsyncStubTransport::new_ok(), "not an address").is_err());
        let ok = SmtpMailer::new(AsyncStubTransport::new_ok(), "no-reply@fxvps.test").unwrap();
        let mut bad = mail();
        bad.to = "bad".into();
        assert!(ok.send(bad).await.is_err());
        assert!(SmtpMailer::from_url("smtps://u:p@smtp.example.com:465", "a@b.c").is_ok());
        assert!(SmtpMailer::from_url("ftp://x", "a@b.c").is_err());
    }
}
