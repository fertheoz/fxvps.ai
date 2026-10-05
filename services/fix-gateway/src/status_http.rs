//! Read-only HTTP status endpoint (`GET /status` -> JSON [`SessionStatus`] list) so a
//! core-engine in another process can show the LP session table. Plain HTTP/1.0 on
//! tokio (no web framework); bind to loopback or a private network only.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::warn;

use crate::gateway::SessionStatus;

/// Serves the table until the listener errors. Returns the bound address via `listener`.
pub async fn serve(listener: TcpListener, source: Arc<RwLock<Vec<SessionStatus>>>) {
    loop {
        let (mut sock, _) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                warn!(error = %e, "status accept failed");
                continue;
            }
        };
        let source = source.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let n =
                match tokio::time::timeout(std::time::Duration::from_secs(5), sock.read(&mut buf))
                    .await
                {
                    Ok(Ok(n)) => n,
                    _ => return,
                };
            let req = String::from_utf8_lossy(&buf[..n]);
            let (code, body) = if req.starts_with("GET /status ") {
                let rows = source.read().map(|t| t.clone()).unwrap_or_default();
                (
                    "200 OK",
                    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
                )
            } else {
                ("404 Not Found", "{}".to_string())
            };
            let resp = format!(
                "HTTP/1.0 {code}\r\ncontent-type: application/json\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.shutdown().await;
        });
    }
}

/// Binds `addr` and spawns [`serve`]; returns the bound address.
pub async fn spawn(
    addr: &str,
    source: Arc<RwLock<Vec<SessionStatus>>>,
) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    tokio::spawn(serve(listener, source));
    Ok(local)
}
