//! Polls a remote fix-gateway status endpoint (`FIX_STATUS_ADDR`, `GET /status`)
//! into an [`LpStatus`] table, for deployments where the gateway runs in another
//! process. On a failed poll the last known rows are kept but marked down.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use fix_gateway::SessionStatus;

use super::LpStatus;

/// Reason shown on rows while the gateway cannot be reached.
pub const UNREACHABLE: &str = "fix-gateway status endpoint unreachable";

/// One poll: replaces the table on success, marks every row down on failure.
pub async fn poll_once(client: &reqwest::Client, url: &str, table: &LpStatus) -> bool {
    let rows = async {
        client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json::<Vec<SessionStatus>>()
            .await
    }
    .await;
    let Ok(mut t) = table.write() else {
        return false;
    };
    match rows {
        Ok(rows) => {
            *t = rows;
            true
        }
        Err(e) => {
            tracing::debug!(error = %e, url, "LP status poll failed");
            for r in t.iter_mut() {
                r.logged_on = false;
                r.last_down_reason = Some(UNREACHABLE.into());
            }
            false
        }
    }
}

/// Spawns a poller every `every`; returns the shared table for [`super::AdminConfig`].
pub fn spawn(url: String, every: Duration) -> LpStatus {
    let table: LpStatus = Arc::new(RwLock::new(Vec::new()));
    let t = table.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_default();
        let mut tick = tokio::time::interval(every);
        loop {
            tick.tick().await;
            poll_once(&client, &url, &t).await;
        }
    });
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use fix_gateway::SessionKind;

    fn row(up: bool) -> SessionStatus {
        SessionStatus {
            kind: SessionKind::Trading,
            lp: "T".into(),
            sender_comp_id: "S".into(),
            target_comp_id: "T".into(),
            logged_on: up,
            since_ms: 1,
            last_down_reason: None,
            rejects: 2,
        }
    }

    #[tokio::test]
    async fn polls_remote_gateway_and_marks_down_when_gone() {
        let source = Arc::new(RwLock::new(vec![row(true)]));
        let addr = fix_gateway::status_http::spawn("127.0.0.1:0", source.clone())
            .await
            .unwrap();
        let client = reqwest::Client::new();
        let table: LpStatus = Arc::new(RwLock::new(Vec::new()));
        let url = format!("http://{addr}/status");
        assert!(poll_once(&client, &url, &table).await);
        assert_eq!(*table.read().unwrap(), vec![row(true)]);

        // Unknown path is a 404 -> failure keeps rows, marked down.
        assert!(!poll_once(&client, &format!("http://{addr}/nope"), &table).await);
        let t = table.read().unwrap();
        assert!(!t[0].logged_on);
        assert_eq!(t[0].last_down_reason.as_deref(), Some(UNREACHABLE));
    }
}
