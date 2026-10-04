//! `GET /v1/stream`: server-sent events carrying invalidation hints
//! (`event: invalidate`, `data: {"topics": ["listClients", ...]}`); topic
//! names are `AdminApi` method names so the UI can invalidate its queries.
//! `EventSource` cannot send headers, so instead of putting the bearer token in
//! the URL (it would end up in proxy / access logs) the client first calls
//! `POST /v1/stream/ticket` with its `Authorization` header and opens
//! `/v1/stream?ticket=<id>`: the ticket is random, single-use and expires after
//! [`TICKET_TTL`]. An `Authorization` header (fetch-based SSE) works as well.

use super::auth::Actor;
use super::{AdminCtx, ApiError};
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::time::{Duration, Instant};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

/// Lifetime of a stream ticket.
pub const TICKET_TTL: Duration = Duration::from_secs(30);

/// Single-use stream tickets: id -> (actor, expiry).
#[derive(Default)]
pub struct Tickets(std::sync::Mutex<HashMap<String, (Actor, Instant)>>);

impl Tickets {
    pub fn issue(&self, actor: Actor) -> String {
        use rand::RngCore;
        let mut b = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut b);
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let now = Instant::now();
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.retain(|_, (_, exp)| *exp > now);
        m.insert(id.clone(), (actor, now + TICKET_TTL));
        id
    }

    /// Consumes a ticket (single use); `None` when unknown or expired.
    pub fn redeem(&self, id: &str) -> Option<Actor> {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let (actor, exp) = m.remove(id)?;
        (exp > Instant::now()).then_some(actor)
    }
}

/// `POST /v1/stream/ticket` -> `{ "ticket", "expiresInMs" }`.
pub async fn ticket(State(ctx): State<AdminCtx>, actor: Actor) -> Response {
    let t = ctx.tickets.issue(actor);
    axum::Json(json!({ "ticket": t, "expiresInMs": TICKET_TTL.as_millis() as u64 })).into_response()
}

#[derive(Deserialize)]
pub struct StreamQuery {
    ticket: Option<String>,
}

pub async fn stream(
    State(ctx): State<AdminCtx>,
    Query(q): Query<StreamQuery>,
    req: Request,
) -> Response {
    let actor: Result<Actor, ApiError> = match q.ticket {
        Some(t) => ctx
            .tickets
            .redeem(&t)
            .ok_or_else(|| ApiError::unauthorized("invalid or expired stream ticket")),
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
