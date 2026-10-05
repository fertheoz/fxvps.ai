//! One WebSocket connection: handshake (Hello + Auth), subscriptions, conflated
//! quote delivery, account events, order commands.
//!
//! Outbound frames go through a bounded queue ([`Outbox`]) drained by a writer task.
//! A full queue means the client is not keeping up: the connection is closed with
//! [`close_code::SLOW_CONSUMER`] instead of buffering without bound.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use client_proto::{
    Ack, AuthOk, Body, CandleResponse, DealHistory, Decimal, Encoding, Envelope, ErrorCode,
    Heartbeat, Hello, OrderList, OrderType, Pong, QuoteBatch, Timeframe, PROTOCOL_VERSION,
};
use core_engine::api::{DealQuery, OrderKind, OrderModify, Protection};
use domain::Fixed;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::{interval, timeout, MissedTickBehavior};

use crate::auth::Claims;
use crate::conflate::Conflator;
use crate::hub::{AccountEvent, CmdError, Hub, NewOrder};

/// Application close codes (4000-4999 are reserved for applications by RFC 6455).
pub mod close_code {
    pub const NORMAL: u16 = 1000;
    pub const PROTOCOL: u16 = 4002;
    pub const UNAUTHENTICATED: u16 = 4001;
    pub const AUTH_TIMEOUT: u16 = 4003;
    pub const SLOW_CONSUMER: u16 = 4008;
    /// Connection refused by a policy limit (too many connections).
    pub const POLICY: u16 = 4009;
}

/// Instant at which a token with `exp` (unix seconds) stops being valid, given
/// the current unix time `now_s`.
pub fn token_deadline(exp: u64, now_s: u64) -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(exp.saturating_sub(now_s))
}

/// Outbound queue is full: the consumer is too slow.
#[derive(Debug, PartialEq, Eq)]
pub struct SlowConsumer;

/// Encodes envelopes and enqueues them without ever blocking.
pub struct Outbox {
    tx: mpsc::Sender<Message>,
    enc: Encoding,
    seq: u64,
}

impl Outbox {
    pub fn new(tx: mpsc::Sender<Message>, enc: Encoding) -> Self {
        Outbox { tx, enc, seq: 0 }
    }

    pub fn send(&mut self, body: Body) -> Result<(), SlowConsumer> {
        self.seq += 1;
        let env = Envelope::new(self.seq, body);
        let msg = match self.enc {
            Encoding::Protobuf => Message::Binary(env.to_protobuf().into()),
            Encoding::Json => Message::Text(env.to_json().into()),
        };
        self.tx.try_send(msg).map_err(|_| SlowConsumer)
    }

    pub fn error(
        &mut self,
        request_id: &str,
        code: ErrorCode,
        msg: &str,
    ) -> Result<(), SlowConsumer> {
        self.send(Body::Error(client_proto::Error {
            request_id: request_id.into(),
            code: code as i32,
            message: msg.into(),
        }))
    }
}

enum Inbound {
    Frame(Box<Envelope>, Encoding),
    Bad(String),
    Closed,
}

fn decode(msg: Message) -> Option<Inbound> {
    Some(match msg {
        Message::Binary(b) => match Envelope::from_protobuf(&b) {
            Ok(e) => Inbound::Frame(Box::new(e), Encoding::Protobuf),
            Err(e) => Inbound::Bad(e.to_string()),
        },
        Message::Text(t) => match Envelope::from_json(t.as_str()) {
            Ok(e) => Inbound::Frame(Box::new(e), Encoding::Json),
            Err(e) => Inbound::Bad(e.to_string()),
        },
        Message::Close(_) => Inbound::Closed,
        Message::Ping(_) | Message::Pong(_) => return None,
    })
}

fn close_msg(code: u16, reason: &str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.to_string().into(),
    }))
}

/// Runs a connection to completion. `slot` is the connection's reservation in
/// [`crate::limits::ConnLimits`], released when the connection ends.
pub async fn run(hub: Arc<Hub>, socket: WebSocket, mut slot: crate::limits::Slot) {
    hub.metrics.connections.inc();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Message>(hub.cfg.queue_capacity);
    // Close requested by the reader side; bypasses the (possibly full) queue.
    let (kill_tx, mut kill_rx) = oneshot::channel::<(u16, String)>();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                k = &mut kill_rx => {
                    if let Ok((code, reason)) = k {
                        // Flush what is already queued (bounded), then the close frame.
                        let _ = timeout(Duration::from_secs(2), async {
                            while let Ok(m) = rx.try_recv() {
                                sink.send(m).await?;
                            }
                            sink.send(close_msg(code, &reason)).await
                        })
                        .await;
                    }
                    break;
                }
                m = rx.recv() => match m {
                    Some(m) => {
                        let is_close = matches!(m, Message::Close(_));
                        if sink.send(m).await.is_err() || is_close {
                            break;
                        }
                    }
                    None => break,
                },
            }
        }
        let _ = sink.close().await;
    });

    let outcome = session(&hub, &mut stream, tx, &mut slot).await;
    drop(slot);
    if let Some((code, reason)) = outcome {
        if code == close_code::SLOW_CONSUMER {
            hub.metrics.slow_consumer_drops.inc();
            tracing::info!("closing slow consumer");
        }
        let _ = kill_tx.send((code, reason));
    }
    let _ = writer.await;
    hub.metrics.connections.dec();
}

type Stream = futures_util::stream::SplitStream<WebSocket>;

async fn next_inbound(stream: &mut Stream) -> Inbound {
    loop {
        match stream.next().await {
            Some(Ok(m)) => {
                if let Some(i) = decode(m) {
                    return i;
                }
            }
            Some(Err(_)) | None => return Inbound::Closed,
        }
    }
}

/// Returns the close (code, reason) to send, or None if the peer went away.
async fn session(
    hub: &Hub,
    stream: &mut Stream,
    tx: mpsc::Sender<Message>,
    slot: &mut crate::limits::Slot,
) -> Option<(u16, String)> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(hub.cfg.auth_timeout_ms);
    let slow = || Some((close_code::SLOW_CONSUMER, "slow consumer".to_string()));

    // --- Hello ---
    let (hello, enc) = match tokio::time::timeout_at(deadline, next_inbound(stream)).await {
        Err(_) => return Some((close_code::AUTH_TIMEOUT, "hello timeout".into())),
        Ok(Inbound::Closed) => return None,
        Ok(Inbound::Bad(e)) => return Some((close_code::PROTOCOL, e)),
        Ok(Inbound::Frame(env, enc)) => match env.body {
            Some(Body::Hello(h)) => (h, enc),
            _ => return Some((close_code::PROTOCOL, "expected hello".into())),
        },
    };
    let mut out = Outbox::new(tx, enc);
    if hello.protocol_version != PROTOCOL_VERSION {
        let _ = out.error(
            "",
            ErrorCode::UnsupportedVersion,
            "unsupported protocol version",
        );
        return Some((close_code::PROTOCOL, "unsupported version".into()));
    }
    let max_hz = match hello.max_quote_hz {
        0 => hub.cfg.max_quote_hz,
        n => n.min(hub.cfg.max_quote_hz),
    };
    out.send(Body::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        client_name: "fxvps-client-gateway".into(),
        max_quote_hz: max_hz,
    }))
    .ok()?;

    // --- Auth ---
    let claims = match tokio::time::timeout_at(deadline, next_inbound(stream)).await {
        Err(_) => return Some((close_code::AUTH_TIMEOUT, "auth timeout".into())),
        Ok(Inbound::Closed) => return None,
        Ok(Inbound::Bad(e)) => return Some((close_code::PROTOCOL, e)),
        Ok(Inbound::Frame(env, _)) => match env.body {
            Some(Body::Auth(a)) => match hub.auth.verify(&a.token) {
                Ok(c) => c,
                Err(e) => {
                    hub.metrics.auth_failures.inc();
                    tracing::debug!(error = %e, "auth rejected");
                    let _ = out.error("", ErrorCode::Unauthenticated, "invalid token");
                    return Some((close_code::UNAUTHENTICATED, "unauthenticated".into()));
                }
            },
            _ => {
                hub.metrics.auth_failures.inc();
                let _ = out.error("", ErrorCode::Unauthenticated, "auth required");
                return Some((close_code::UNAUTHENTICATED, "auth required".into()));
            }
        },
    };
    if slot.bind_subject(&claims.sub).is_err() {
        hub.metrics.connections_rejected.inc();
        let _ = out.error(
            "",
            ErrorCode::RateLimited,
            "too many connections for this user",
        );
        return Some((close_code::POLICY, "too many connections".into()));
    }
    // The token is re-checked for the whole connection: at `exp` the session is
    // closed with UNAUTHENTICATED and the client reconnects with a fresh token.
    let expiry = token_deadline(claims.exp, domain::now_ns() / 1_000_000_000);
    let mut groups = HashSet::new();
    let mut infos = Vec::new();
    let mut snaps = Vec::new();
    for acc in &claims.accounts {
        if let Some(g) = hub.account_group(acc) {
            groups.insert(g);
        }
        if let Some((info, snap)) = hub.account_state(acc).await {
            infos.push(info);
            snaps.push(snap);
        }
    }
    if out
        .send(Body::AuthOk(AuthOk {
            subject: claims.sub.clone(),
            account_ids: claims.accounts.clone(),
            expires_at_s: claims.exp,
            accounts: infos,
        }))
        .is_err()
    {
        return slow();
    }
    for snap in snaps {
        if out.send(Body::AccountSnapshot(snap)).is_err() {
            return slow();
        }
    }

    let mut state = ConnState {
        claims,
        subs: HashSet::new(),
        groups,
        conflator: Conflator::default(),
    };
    let mut quotes = hub.subscribe_quotes();
    let mut accounts = hub.subscribe_accounts();
    let mut flush = interval(Duration::from_micros(1_000_000 / u64::from(max_hz.max(1))));
    flush.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut hb = interval(Duration::from_secs(hub.cfg.heartbeat_secs.max(1)));
    hb.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let expired = tokio::time::sleep_until(expiry);
    tokio::pin!(expired);
    loop {
        tokio::select! {
            () = &mut expired => {
                hub.metrics.sessions_expired.inc();
                let _ = out.error("", ErrorCode::Unauthenticated, "token expired");
                return Some((close_code::UNAUTHENTICATED, "token expired".into()));
            }
            inbound = next_inbound(stream) => match inbound {
                Inbound::Closed => return None,
                Inbound::Bad(e) => {
                    if out.error("", ErrorCode::BadRequest, &e).is_err() { return slow(); }
                }
                Inbound::Frame(env, _) => {
                    hub.metrics.frames_in.inc();
                    if let Some(body) = env.body {
                        match handle(hub, &mut state, &mut out, body).await {
                            Ok(()) => {}
                            Err(SlowConsumer) => return slow(),
                        }
                    }
                }
            },
            q = quotes.recv() => match q {
                Ok(m) => {
                    let visible = m.group.as_deref().is_none_or(|g| state.groups.contains(g));
                    if visible && state.subs.contains(&*m.quote.symbol) {
                        state.conflator.push(Arc::new(m.quote.clone()))
                    }
                }
                // Latest-wins semantics: missing intermediate ticks is fine.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    return Some((close_code::NORMAL, "shutting down".into()));
                }
            },
            ev = accounts.recv() => match ev {
                Ok(ev) => if state.claims.may_access(ev.account_id()) {
                    let body = match &*ev {
                        AccountEvent::Order(o) => Body::OrderUpdate((**o).clone()),
                        AccountEvent::Position(p) => Body::PositionUpdate(p.clone()),
                        AccountEvent::Account(a) => Body::AccountSnapshot(a.clone()),
                        AccountEvent::Deal(d) => Body::DealUpdate(d.clone()),
                    };
                    if out.send(body).is_err() { return slow(); }
                },
                // Account events must not be lost silently: force a resync.
                Err(broadcast::error::RecvError::Lagged(_)) => return slow(),
                Err(broadcast::error::RecvError::Closed) => {
                    return Some((close_code::NORMAL, "shutting down".into()));
                }
            },
            _ = flush.tick() => {
                if !state.conflator.is_empty() {
                    hub.metrics.quotes_conflated.inc_by(std::mem::take(&mut state.conflator.conflated));
                    let quotes: Vec<_> = state.conflator.take().iter().map(|q| q.to_proto()).collect();
                    let n = quotes.len() as u64;
                    if out.send(Body::QuoteBatch(QuoteBatch { quotes })).is_err() {
                        return slow();
                    }
                    hub.metrics.quote_batches_sent.inc();
                    hub.metrics.quotes_sent.inc_by(n);
                }
            },
            _ = hb.tick() => {
                if out.send(Body::Heartbeat(Heartbeat { ts_ns: domain::now_ns() })).is_err() {
                    return slow();
                }
            },
        }
    }
}

struct ConnState {
    claims: Claims,
    subs: HashSet<String>,
    /// Groups of the authorized accounts (marked-up quote streams).
    groups: HashSet<String>,
    conflator: Conflator,
}

/// Optional wire decimal -> Fixed; present but not representable = error.
fn fixed(d: Option<Decimal>, what: &str) -> Result<Option<Fixed>, CmdError> {
    d.map(|d| {
        d.to_fixed()
            .ok_or_else(|| CmdError(ErrorCode::BadRequest, format!("{what}: invalid decimal")))
    })
    .transpose()
}

fn reply(out: &mut Outbox, request_id: &str, r: Result<(), CmdError>) -> Result<(), SlowConsumer> {
    match r {
        Ok(()) => out.send(Body::Ack(Ack {
            request_id: request_id.into(),
        })),
        Err(CmdError(code, msg)) => out.error(request_id, code, &msg),
    }
}

fn check_account(hub: &Hub, st: &ConnState, account_id: &str) -> Result<(), CmdError> {
    if !st.claims.may_access(account_id) {
        return Err(CmdError(
            ErrorCode::Forbidden,
            "account not authorized".into(),
        ));
    }
    if !hub.allow_order(account_id) {
        return Err(CmdError(
            ErrorCode::RateLimited,
            "order rate limit exceeded".into(),
        ));
    }
    Ok(())
}

async fn handle(
    hub: &Hub,
    st: &mut ConnState,
    out: &mut Outbox,
    body: Body,
) -> Result<(), SlowConsumer> {
    match body {
        Body::Subscribe(s) => {
            let unknown: Vec<_> = s
                .symbols
                .iter()
                .filter(|x| !hub.is_known_symbol(x))
                .cloned()
                .collect();
            if !unknown.is_empty() {
                return out.error(&s.request_id, ErrorCode::UnknownSymbol, &unknown.join(","));
            }
            // Current prices first: the flush tick sends them with the next batch.
            for q in hub.last_quotes(&s.symbols, &st.groups) {
                st.conflator.push(q);
            }
            st.subs.extend(s.symbols);
            out.send(Body::Ack(Ack {
                request_id: s.request_id,
            }))
        }
        Body::Unsubscribe(u) => {
            for sym in &u.symbols {
                st.subs.remove(sym);
                st.conflator.remove(sym);
            }
            out.send(Body::Ack(Ack {
                request_id: u.request_id,
            }))
        }
        Body::CandleRequest(c) => {
            let tf = Timeframe::try_from(c.timeframe).unwrap_or(Timeframe::Unspecified);
            if tf == Timeframe::Unspecified {
                return out.error(&c.request_id, ErrorCode::BadRequest, "timeframe required");
            }
            if !hub.is_known_symbol(&c.symbol) {
                return out.error(&c.request_id, ErrorCode::UnknownSymbol, &c.symbol);
            }
            let limit = match c.limit {
                0 => 500,
                n => n.min(5_000) as usize,
            };
            let candles = hub.candles(&c.symbol, tf, c.from_ns, c.to_ns, limit);
            out.send(Body::CandleResponse(CandleResponse {
                request_id: c.request_id,
                symbol: c.symbol,
                timeframe: tf as i32,
                candles,
            }))
        }
        Body::PlaceOrder(p) => {
            let r = check_account(hub, st, &p.account_id).and_then(|()| {
                let side = client_proto::Side::try_from(p.side)
                    .ok()
                    .and_then(|s| s.to_domain())
                    .ok_or_else(|| CmdError(ErrorCode::BadRequest, "side required".into()))?;
                let ord_type = match OrderType::try_from(p.order_type) {
                    Ok(OrderType::Market) => OrderKind::Market,
                    Ok(OrderType::Limit) => OrderKind::Limit,
                    Ok(OrderType::Stop) => OrderKind::Stop,
                    Ok(OrderType::StopLimit) => OrderKind::StopLimit,
                    _ => {
                        return Err(CmdError(
                            ErrorCode::BadRequest,
                            "order_type required".into(),
                        ))
                    }
                };
                let qty = p
                    .qty
                    .and_then(|d| d.to_fixed())
                    .ok_or_else(|| CmdError(ErrorCode::BadRequest, "qty required".into()))?;
                Ok(NewOrder {
                    request_id: p.request_id.clone(),
                    account_id: p.account_id.clone(),
                    symbol: p.symbol.clone(),
                    side,
                    ord_type,
                    qty,
                    limit_price: fixed(p.limit_price, "limit_price")?,
                    tif: client_proto::TimeInForce::try_from(p.tif)
                        .ok()
                        .filter(|t| *t != client_proto::TimeInForce::Unspecified),
                    stop_price: fixed(p.stop_price, "stop_price")?,
                    sl: fixed(p.sl, "sl")?,
                    tp: fixed(p.tp, "tp")?,
                    trailing_distance: fixed(p.trailing_distance, "trailing_distance")?,
                    oco_group: (p.oco_group != 0).then_some(p.oco_group),
                    expire_at_ns: (p.expire_at_ns != 0).then_some(p.expire_at_ns),
                })
            });
            let r = match r {
                Ok(o) => hub.place_order(o).await,
                Err(e) => Err(e),
            };
            reply(out, &p.request_id, r)
        }
        Body::CancelOrder(c) => {
            let r = match check_account(hub, st, &c.account_id) {
                Ok(()) => hub.cancel_order(&c.account_id, &c.target_request_id).await,
                Err(e) => Err(e),
            };
            reply(out, &c.request_id, r)
        }
        Body::ModifyOrder(m) => {
            let change = || -> Result<OrderModify, CmdError> {
                Ok(OrderModify {
                    qty: fixed(m.qty, "qty")?,
                    limit_price: fixed(m.limit_price, "limit_price")?,
                    stop_price: fixed(m.stop_price, "stop_price")?,
                    sl: fixed(m.sl, "sl")?,
                    tp: fixed(m.tp, "tp")?,
                    trailing_distance: fixed(m.trailing_distance, "trailing_distance")?,
                    expire_at_ns: (m.expire_at_ns != 0).then_some(m.expire_at_ns),
                    clear_expiry: m.clear_expiry,
                    replace_protection: m.replace_protection,
                })
            };
            let r = match check_account(hub, st, &m.account_id).and_then(|()| change()) {
                Ok(c) => {
                    hub.modify_order(&m.account_id, &m.target_request_id, c)
                        .await
                }
                Err(e) => Err(e),
            };
            reply(out, &m.request_id, r)
        }
        Body::ModifyPosition(m) => {
            let prot = || -> Result<Protection, CmdError> {
                Ok(Protection {
                    sl: fixed(m.sl, "sl")?,
                    tp: fixed(m.tp, "tp")?,
                    trailing_distance: fixed(m.trailing_distance, "trailing_distance")?,
                })
            };
            let r = match check_account(hub, st, &m.account_id).and_then(|()| prot()) {
                Ok(p) => hub.modify_position(&m.account_id, &m.position_id, p).await,
                Err(e) => Err(e),
            };
            reply(out, &m.request_id, r)
        }
        Body::ClosePosition(c) => {
            let r = match check_account(hub, st, &c.account_id).and_then(|()| fixed(c.qty, "qty")) {
                Ok(q) => {
                    hub.close_position(&c.account_id, &c.position_id, q, &c.request_id)
                        .await
                }
                Err(e) => Err(e),
            };
            reply(out, &c.request_id, r)
        }
        Body::OrderListRequest(r) => {
            if !st.claims.may_access(&r.account_id) {
                return out.error(
                    &r.request_id,
                    ErrorCode::Forbidden,
                    "account not authorized",
                );
            }
            match hub.orders(&r.account_id).await {
                Ok(orders) => out.send(Body::OrderList(OrderList {
                    request_id: r.request_id,
                    account_id: r.account_id,
                    orders,
                })),
                Err(CmdError(code, msg)) => out.error(&r.request_id, code, &msg),
            }
        }
        Body::DealHistoryRequest(r) => {
            if !st.claims.may_access(&r.account_id) {
                return out.error(
                    &r.request_id,
                    ErrorCode::Forbidden,
                    "account not authorized",
                );
            }
            let after = match r.cursor.as_str() {
                "" => Ok(0),
                c => c.parse::<u64>(),
            };
            let Ok(after) = after else {
                return out.error(&r.request_id, ErrorCode::BadRequest, "bad cursor");
            };
            let q = DealQuery {
                from_ns: r.from_ns,
                to_ns: r.to_ns,
                after,
                limit: match r.limit {
                    0 => 500,
                    n => n.min(5_000) as usize,
                },
            };
            match hub.deals(&r.account_id, q).await {
                Ok(page) => out.send(Body::DealHistory(DealHistory {
                    request_id: r.request_id,
                    account_id: r.account_id,
                    deals: page.deals.iter().map(crate::hub::deal).collect(),
                    next_cursor: page.next.map(|n| n.to_string()).unwrap_or_default(),
                })),
                Err(CmdError(code, msg)) => out.error(&r.request_id, code, &msg),
            }
        }
        Body::SymbolListRequest(r) => out.send(Body::SymbolList(client_proto::SymbolList {
            request_id: r.request_id,
            instruments: hub.instruments(),
        })),
        Body::Ping(p) => out.send(Body::Pong(Pong {
            nonce: p.nonce,
            ts_ns: domain::now_ns(),
        })),
        Body::Heartbeat(_) | Body::Pong(_) => Ok(()),
        _ => out.error("", ErrorCode::BadRequest, "unexpected message"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_deadline_is_relative_to_exp() {
        let now = tokio::time::Instant::now();
        let d = token_deadline(1_100, 1_000);
        assert!(d >= now + Duration::from_secs(100) && d < now + Duration::from_secs(101));
        // already expired: immediately
        assert!(token_deadline(900, 1_000) <= tokio::time::Instant::now());
    }

    #[test]
    fn full_queue_is_slow_consumer() {
        let (tx, mut rx) = mpsc::channel(2);
        let mut out = Outbox::new(tx, Encoding::Protobuf);
        let hb = || Body::Heartbeat(Heartbeat { ts_ns: 1 });
        assert!(out.send(hb()).is_ok());
        assert!(out.send(hb()).is_ok());
        assert_eq!(out.send(hb()), Err(SlowConsumer));
        // Draining frees capacity again (bounded, not latched).
        let _ = rx.try_recv();
        assert!(out.send(hb()).is_ok());
    }
}
