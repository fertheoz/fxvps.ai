//! Outbound email. Production deployments plug an SMTP/API mailer into [`Mailer`];
//! the default [`LogMailer`] writes the message (including links) to the log and is
//! for development only.

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
        tracing::warn!(
            to = %mail.to,
            subject = %mail.subject,
            link = mail.link.as_deref().unwrap_or(""),
            "DEV MAILER (not sent): {}",
            mail.body
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
