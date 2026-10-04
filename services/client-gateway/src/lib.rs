//! Client-facing realtime gateway: WebSocket server speaking `client-proto`, with JWT
//! auth, per-connection subscriptions, quote conflation, backpressure, per-account
//! order rate limiting, candle history, `/healthz` and `/metrics`.

pub mod auth;
pub mod candles;
pub mod config;
pub mod conflate;
pub mod conn;
pub mod demo;
pub mod hub;
pub mod limits;
pub mod metrics;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts, State, WebSocketUpgrade};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

pub use config::ClientGatewayConfig;
pub use hub::Hub;

/// Public router: `/ws`, `/healthz`, and `/metrics` unless `metrics_listen`
/// moves it to a separate listener ([`metrics_router`]).
pub fn router(hub: Arc<Hub>) -> Router {
    let mut r = Router::new()
        .route("/ws", get(ws_handler))
        .route("/healthz", get(|| async { "ok" }));
    if hub.cfg.metrics_listen.is_none() {
        r = r.route("/metrics", get(metrics_handler));
    }
    r.with_state(hub)
}

/// `/metrics` (and `/healthz`) for the separate metrics listener.
pub fn metrics_router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(hub)
}

/// Peer IP when the server runs with connect info ([`serve`]).
pub struct PeerIp(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for PeerIp {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(p: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(PeerIp(
            p.extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip()),
        ))
    }
}

/// `Origin` check: empty allow list = any; a request without `Origin` (native
/// clients) is allowed.
pub fn origin_allowed(allowed: &[String], headers: &HeaderMap) -> bool {
    if allowed.is_empty() {
        return true;
    }
    match headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        None => true,
        Some(o) => allowed.iter().any(|a| a == o),
    }
}

async fn ws_handler(
    State(hub): State<Arc<Hub>>,
    PeerIp(ip): PeerIp,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&hub.cfg.allowed_origins, &headers) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let slot = match hub.conns.acquire(ip) {
        Ok(s) => s,
        Err(e) => {
            hub.metrics.connections_rejected.inc();
            tracing::debug!(?ip, ?e, "connection limit");
            return (StatusCode::TOO_MANY_REQUESTS, "too many connections").into_response();
        }
    };
    let max = hub.cfg.max_frame_bytes;
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| conn::run(hub, socket, slot))
}

async fn metrics_handler(State(hub): State<Arc<Hub>>) -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        hub.metrics.render(),
    )
}

/// Serves on an already bound listener until the future is dropped. With
/// `metrics_listen` set, `/metrics` is served there instead.
pub async fn serve(hub: Arc<Hub>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    if let Some(addr) = hub.cfg.metrics_listen.clone() {
        let ml = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(addr = %ml.local_addr()?, "metrics listening");
        let m = metrics_router(hub.clone());
        tokio::spawn(async move { axum::serve(ml, m).await });
    }
    axum::serve(
        listener,
        router(hub).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}
