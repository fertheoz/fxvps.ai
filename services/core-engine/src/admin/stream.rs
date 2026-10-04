//! `GET /v1/stream`: server-sent events carrying invalidation hints
//! (`event: invalidate`, `data: {"topics": ["listClients", ...]}`); topic
//! names are `AdminApi` method names so the UI can invalidate its queries.
//! `EventSource` cannot send headers, so the token may be passed as
//! `?access_token=`.

use super::auth::Actor;
use super::{AdminCtx, ApiError};
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Deserialize)]
pub struct StreamQuery {
    access_token: Option<String>,
}

pub async fn stream(
    State(ctx): State<AdminCtx>,
    Query(q): Query<StreamQuery>,
    req: Request,
) -> Response {
    let actor: Result<Actor, ApiError> = match q.access_token {
        Some(t) => ctx
            .auth
            .verify(&t)
            .map_err(|e| ApiError::unauthorized(format!("invalid token: {}", e.0))),
        None => {
            let (mut parts, _) = req.into_parts();
            Actor::from_request_parts(&mut parts, &ctx).await
        }
    };
    if let Err(e) = actor {
        return e.into_response();
    }
    let hello = tokio_stream::once(Ok::<Event, Infallible>(
        Event::default()
            .event("hello")
            .data(json!({ "topics": [] }).to_string()),
    ));
    let rx = BroadcastStream::new(ctx.live.subscribe()).map(|m| {
        let data = m.unwrap_or_else(|_| json!({ "topics": ["*"] }).to_string());
        Ok::<Event, Infallible>(Event::default().event("invalidate").data(data))
    });
    Sse::new(hello.chain(rx))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// Watches the engine sequence number and publishes engine topics when it
/// moves (client orders, fills, quotes). Ends when the engine is gone.
pub fn spawn_ticker(ctx: AdminCtx, interval_ms: u64) {
    tokio::spawn(async move {
        let mut last: Option<u64> = None;
        let mut iv = tokio::time::interval(Duration::from_millis(interval_ms.max(50)));
        loop {
            iv.tick().await;
            let Ok(Value::Number(n)) = ctx.engine.query(|e| json!(e.seq())).await else {
                break;
            };
            let seq = n.as_u64();
            if last.is_some() && seq != last && ctx.live.receiver_count() > 0 {
                ctx.notify(&super::routes::LIVE_ENGINE_TOPICS);
            }
            last = seq;
        }
    });
}
