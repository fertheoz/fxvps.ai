//! Small HTTP/1.0 endpoint on tokio (no web framework); bind to loopback or a private
//! network only.
//!
//! * `GET /status` — JSON [`SessionStatus`] list (no auth; read by core-engine).
//! * `GET /config`, `PUT /config` — managed LP configuration ([`Managed`]), only when an
//!   admin token is configured (`FIX_ADMIN_TOKEN`); `Authorization: Bearer <token>`.
//!   Reads never contain passwords.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

use crate::config::GatewayConfig;
use crate::gateway::SessionStatus;
use crate::managed::Managed;

const MAX_HEAD: usize = 8 * 1024;
const MAX_BODY: usize = 256 * 1024;

/// Admin side of the endpoint.
#[derive(Clone)]
pub struct Admin {
    pub token: String,
    pub managed: Arc<Managed>,
}

struct Request {
    method: String,
    path: String,
    bearer: Option<String>,
    body: Vec<u8>,
}

async fn read_request(sock: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::with_capacity(1024);
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
        let mut chunk = [0u8; 1024];
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let (mut len, mut bearer) = (0usize, None);
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => len = v.parse().ok()?,
            "authorization" => bearer = v.strip_prefix("Bearer ").map(str::to_string),
            _ => {}
        }
    }
    if len > MAX_BODY {
        return None;
    }
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let mut chunk = vec![0u8; len - body.len()];
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Some(Request {
        method,
        path,
        bearer,
        body,
    })
}

fn eq_ct(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn error(code: &'static str, msg: impl Into<String>) -> (&'static str, String) {
    (code, serde_json::json!({ "error": msg.into() }).to_string())
}

fn route(
    req: &Request,
    source: &RwLock<Vec<SessionStatus>>,
    admin: Option<&Admin>,
) -> (&'static str, String) {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/status") => {
            let rows = source.read().map(|t| t.clone()).unwrap_or_default();
            (
                "200 OK",
                serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
            )
        }
        (m, "/config") => {
            let Some(admin) = admin else {
                return error("404 Not Found", "config endpoint disabled");
            };
            if !req
                .bearer
                .as_deref()
                .is_some_and(|t| eq_ct(t, &admin.token))
            {
                return error("401 Unauthorized", "bad or missing admin token");
            }
            match m {
                "GET" => ("200 OK", admin.managed.redacted().to_string()),
                "PUT" => match serde_json::from_slice::<GatewayConfig>(&req.body) {
                    Err(e) => error("400 Bad Request", format!("config: {e}")),
                    Ok(c) => match admin.managed.update(c) {
                        Ok(()) => {
                            info!("LP config updated via admin endpoint; restarting sessions");
                            ("200 OK", admin.managed.redacted().to_string())
                        }
                        Err(e) => error("400 Bad Request", e.to_string()),
                    },
                },
                _ => error("405 Method Not Allowed", "use GET or PUT"),
            }
        }
        _ => error("404 Not Found", "not found"),
    }
}

/// Serves until the listener errors.
pub async fn serve(
    listener: TcpListener,
    source: Arc<RwLock<Vec<SessionStatus>>>,
    admin: Option<Admin>,
) {
    loop {
        let (mut sock, _) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                warn!(error = %e, "status accept failed");
                continue;
            }
        };
        let source = source.clone();
        let admin = admin.clone();
        tokio::spawn(async move {
            let Ok(Some(req)) =
                tokio::time::timeout(Duration::from_secs(5), read_request(&mut sock)).await
            else {
                return;
            };
            let (code, body) = route(&req, &source, admin.as_ref());
            let resp = format!(
                "HTTP/1.0 {code}\r\ncontent-type: application/json\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.shutdown().await;
        });
    }
}

/// Binds `addr` and spawns [`serve`] without admin endpoints; returns the bound address.
pub async fn spawn(
    addr: &str,
    source: Arc<RwLock<Vec<SessionStatus>>>,
) -> std::io::Result<SocketAddr> {
    spawn_with_admin(addr, source, None).await
}

/// Binds `addr` and spawns [`serve`]; returns the bound address.
pub async fn spawn_with_admin(
    addr: &str,
    source: Arc<RwLock<Vec<SessionStatus>>>,
    admin: Option<Admin>,
) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    tokio::spawn(serve(listener, source, admin));
    Ok(local)
}
