//! Managed configuration: the LP connection settings live in a JSON file owned by the
//! gateway (`FIX_CONFIG_FILE`, mode 0600) and are edited through the admin endpoints
//! (`GET/PUT /config`, see [`crate::status_http`]) instead of a hand-edited TOML.
//! Every successful update wakes the supervisor, which restarts the FIX sessions.
//!
//! Passwords are write-only: reads return `password: null` plus `password_set`, and an
//! update without a password keeps the stored one.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde_json::Value;
use tokio::sync::Notify;

use crate::config::{GatewayConfig, SessionEndpoint};

#[derive(Debug, thiserror::Error)]
pub enum ManagedError {
    #[error("invalid config: {0}")]
    Invalid(String),
    #[error("store config: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Managed {
    path: PathBuf,
    current: RwLock<Option<GatewayConfig>>,
    changed: Notify,
}

impl Managed {
    /// Opens `path`; a missing file means "not configured yet" (sessions stay idle).
    pub fn open(path: impl Into<PathBuf>) -> Result<Managed, ManagedError> {
        let path = path.into();
        let current = match std::fs::read(&path) {
            Ok(b) => Some(
                serde_json::from_slice::<GatewayConfig>(&b)
                    .map_err(|e| ManagedError::Invalid(format!("{}: {e}", path.display())))?,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Managed {
            path,
            current: RwLock::new(current),
            changed: Notify::new(),
        })
    }

    pub fn current(&self) -> Option<GatewayConfig> {
        self.current.read().ok().and_then(|c| c.clone())
    }

    /// Resolves when the next update has been stored.
    pub async fn changed(&self) {
        self.changed.notified().await
    }

    /// Current config with passwords removed (`null`) and `password_set` flags.
    pub fn redacted(&self) -> Value {
        match self.current() {
            None => Value::Null,
            Some(c) => redact(&c),
        }
    }

    /// Validates and stores `new`; empty passwords keep the stored ones.
    pub fn update(&self, mut new: GatewayConfig) -> Result<(), ManagedError> {
        let old = self.current();
        keep_password(&mut new.md, old.as_ref().map(|o| &o.md));
        keep_password(&mut new.trade, old.as_ref().map(|o| &o.trade));
        validate(&new)?;
        write_private(
            &self.path,
            &serde_json::to_vec_pretty(&new).map_err(std::io::Error::other)?,
        )?;
        if let Ok(mut c) = self.current.write() {
            *c = Some(new);
        }
        // Single supervisor: a stored permit also covers an update made while it restarts.
        self.changed.notify_one();
        Ok(())
    }
}

fn keep_password(ep: &mut SessionEndpoint, old: Option<&SessionEndpoint>) {
    if ep.password.as_deref().is_none_or(str::is_empty) {
        ep.password = old.and_then(|o| o.password.clone());
    }
}

/// Same shape as the stored config, passwords replaced by `password_set`.
pub fn redact(c: &GatewayConfig) -> Value {
    let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
    for (k, ep) in [("md", &c.md), ("trade", &c.trade)] {
        if let Some(o) = v.get_mut(k).and_then(Value::as_object_mut) {
            o.insert("password".into(), Value::Null);
            o.insert(
                "password_set".into(),
                Value::Bool(ep.password.as_deref().is_some_and(|p| !p.is_empty())),
            );
        }
    }
    v
}

fn validate(c: &GatewayConfig) -> Result<(), ManagedError> {
    let bad = |m: String| Err(ManagedError::Invalid(m));
    for (name, ep) in [("md", &c.md), ("trade", &c.trade)] {
        if ep
            .addr
            .rsplit_once(':')
            .is_none_or(|(h, p)| h.is_empty() || p.parse::<u16>().is_err())
        {
            return bad(format!("{name}.addr must be host:port"));
        }
        if ep.sender_comp_id.trim().is_empty() || ep.target_comp_id.trim().is_empty() {
            return bad(format!(
                "{name}: SenderCompID and TargetCompID are required"
            ));
        }
        if let Some(t) = &ep.tls {
            crate::tls::build(t, &ep.addr)
                .map_err(|e| ManagedError::Invalid(format!("{name}.tls: {e}")))?;
        }
    }
    if c.heartbeat_secs == 0 || c.heartbeat_secs > 300 {
        return bad("heartbeat_secs must be 1..=300".into());
    }
    if c.instruments.is_empty() {
        return bad("at least one instrument is required".into());
    }
    Ok(())
}

/// Atomic write (tmp + rename), owner-only permissions.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pw: Option<&str>) -> GatewayConfig {
        let mut c = GatewayConfig::from_toml(include_str!("../config/default.toml")).unwrap();
        c.md.password = pw.map(Into::into);
        c.trade.password = pw.map(Into::into);
        c
    }

    #[test]
    fn stores_redacts_and_keeps_password() {
        let dir = tempfile::tempdir().unwrap();
        let m = Managed::open(dir.path().join("lp.json")).unwrap();
        assert!(m.current().is_none());
        m.update(cfg(Some("s3cret"))).unwrap();
        let r = m.redacted();
        assert_eq!(r["md"]["password"], Value::Null);
        assert_eq!(r["md"]["password_set"], Value::Bool(true));
        assert!(!r.to_string().contains("s3cret"));

        // Update without password keeps it; reopening reads the file back.
        let mut c = cfg(None);
        c.md.sender_comp_id = "NEW".into();
        m.update(c).unwrap();
        let back = Managed::open(dir.path().join("lp.json"))
            .unwrap()
            .current()
            .unwrap();
        assert_eq!(back.md.sender_comp_id, "NEW");
        assert_eq!(back.md.password.as_deref(), Some("s3cret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("lp.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[test]
    fn rejects_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let m = Managed::open(dir.path().join("lp.json")).unwrap();
        let mut c = cfg(None);
        c.md.addr = "nohostport".into();
        assert!(m.update(c).is_err());
        let mut c = cfg(None);
        c.trade.target_comp_id = " ".into();
        assert!(m.update(c).is_err());
        assert!(m.current().is_none());
    }
}
