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
pub mod metrics;

use std::sync::Arc;

use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

pub use config::ClientGatewayConfig;
pub use hub::Hub;

pub fn router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/healthz", get(|| async { "ok" }))
        .route("/metrics", get(metrics_handler))
        .with_state(hub)
}

async fn ws_handler(State(hub): State<Arc<Hub>>, ws: WebSocketUpgrade) -> impl IntoResponse {
    let max = hub.cfg.max_frame_bytes;
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| conn::run(hub, socket))
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

/// Serves on an already bound listener until the future is dropped.
pub async fn serve(hub: Arc<Hub>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(hub)).await
}
