//! WebSocket load generator for `client-gateway`.
//!
//! Each simulated client connects to `/ws`, sends `Hello` + `Auth`, subscribes to a
//! symbol set and (optionally) places small market orders at a fixed rate, alternating
//! buy/sell so net exposure stays flat. Measured:
//! - **quote fan-out latency**: client receive time minus the quote's `ts_ns`
//!   (wall clock; meaningful only when client and server share a host/clock),
//! - **order ack / fill latency**: `PlaceOrder` send to `Ack` / first `FILLED` update,
//! - error counts by kind (connect, auth, protocol errors by `ErrorCode`, rejects).

pub mod stats;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use client_proto::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

pub use stats::{ClientStats, Hist, Report, Summary};

/// Load run parameters.
#[derive(Clone, Debug)]
pub struct LoadConfig {
    /// `ws://host:port/ws`.
    pub url: String,
    pub clients: usize,
    pub duration: Duration,
    /// Symbols each client subscribes to (client form, e.g. `EURUSD`).
    pub symbols: Vec<String>,
    /// Requested quote rate per connection (0 = server default).
    pub quote_hz: u32,
    /// Orders per second per client (0 disables orders).
    pub orders_per_sec: f64,
    /// Order quantity in base units.
    pub order_qty: i64,
    /// Bearer token per client: `tokens[i % len]`, account `accounts[i % len]`.
    pub tokens: Vec<String>,
    pub accounts: Vec<String>,
    /// Delay between successive client connects (ramp-up).
    pub ramp: Duration,
}

impl Default for LoadConfig {
    fn default() -> Self {
        LoadConfig {
            url: "ws://127.0.0.1:8080/ws".into(),
            clients: 10,
            duration: Duration::from_secs(10),
            symbols: vec!["EURUSD".into(), "GBPUSD".into(), "USDJPY".into()],
            quote_hz: 0,
            orders_per_sec: 0.2,
            order_qty: 1_000,
            tokens: Vec::new(),
            accounts: vec!["DEMO-1".into(), "DEMO-2".into(), "DEMO-3".into()],
            ramp: Duration::from_millis(5),
        }
    }
}

/// Mints dev HS256 tokens, one per account (`secret` = gateway's HS256 secret).
pub fn dev_tokens(secret: &str, accounts: &[String], ttl_secs: u64) -> Vec<String> {
    accounts
        .iter()
        .map(|a| client_gateway::auth::issue_hs256(secret.as_bytes(), "loadgen", &[a], ttl_secs))
        .collect()
}

/// Runs all clients to completion and aggregates their stats.
pub async fn run(cfg: LoadConfig) -> Report {
    let started = Instant::now();
    let deadline = started + cfg.duration;
    let mut handles = Vec::with_capacity(cfg.clients);
    for i in 0..cfg.clients {
        let c = cfg.clone();
        handles.push(tokio::spawn(async move { client(i, c, deadline).await }));
        if !cfg.ramp.is_zero() {
            tokio::time::sleep(cfg.ramp).await;
        }
    }
    let mut all = Vec::with_capacity(handles.len());
    for h in handles {
        all.push(h.await.unwrap_or_else(|_| {
            let mut s = ClientStats::default();
            s.error("task_panic");
            s
        }));
    }
    Report::from_clients(all, started.elapsed().as_secs_f64())
}

fn pick(v: &[String], i: usize) -> String {
    if v.is_empty() {
        String::new()
    } else {
        v[i % v.len()].clone()
    }
}

fn error_kind(code: i32) -> String {
    ErrorCode::try_from(code)
        .map(|c| format!("server_{}", c.as_str_name().to_ascii_lowercase()))
        .unwrap_or_else(|_| format!("server_code_{code}"))
}

async fn client(idx: usize, cfg: LoadConfig, deadline: Instant) -> ClientStats {
    let mut st = ClientStats::default();
    let Ok(Ok((mut ws, _))) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(&cfg.url),
    )
    .await
    else {
        st.error("connect");
        return st;
    };
    let mut seq = 0u64;
    let mut frame = |body: Body| {
        seq += 1;
        Message::Binary(Envelope::new(seq, body).to_protobuf().into())
    };
    let token = pick(&cfg.tokens, idx);
    let account = pick(&cfg.accounts, idx);
    let hello = frame(Body::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        client_name: format!("loadgen-{idx}"),
        max_quote_hz: cfg.quote_hz,
    }));
    let auth = frame(Body::Auth(Auth { token }));
    if ws.send(hello).await.is_err() || ws.send(auth).await.is_err() {
        st.error("send");
        return st;
    }
    let mut authed = false;
    let mut subscribed = false;
    let mut pending: HashMap<String, (Instant, bool)> = HashMap::new();
    let mut n_orders = 0u64;
    let period =
        (cfg.orders_per_sec > 0.0).then(|| Duration::from_secs_f64(1.0 / cfg.orders_per_sec));
    // Stagger first orders across clients so they do not all fire at once.
    let offset = period
        .map(|p| p.mul_f64((idx % 97) as f64 / 97.0))
        .unwrap_or_default();
    let mut tick = tokio::time::interval_at(
        tokio::time::Instant::now() + offset + period.unwrap_or(Duration::from_secs(3600)),
        period.unwrap_or(Duration::from_secs(3600)),
    );
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let end = tokio::time::Instant::from_std(deadline);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(end) => break,
            _ = tick.tick(), if authed && period.is_some() => {
                n_orders += 1;
                let id = format!("lg-{idx}-{n_orders}");
                let side = if n_orders % 2 == 1 { Side::Buy } else { Side::Sell };
                let sym = pick(&cfg.symbols, idx);
                let msg = frame(Body::PlaceOrder(PlaceOrder {
                    request_id: id.clone(),
                    account_id: account.clone(),
                    symbol: sym,
                    side: side as i32,
                    order_type: OrderType::Market as i32,
                    qty: Some(Decimal { value: cfg.order_qty, scale: 0 }),
                    limit_price: None,
                    tif: TimeInForce::Ioc as i32,
                }));
                pending.insert(id, (Instant::now(), false));
                st.orders_sent += 1;
                if ws.send(msg).await.is_err() {
                    st.error("send");
                    break;
                }
            }
            m = ws.next() => {
                let env = match m {
                    Some(Ok(Message::Binary(b))) => Envelope::from_protobuf(&b).ok(),
                    Some(Ok(Message::Text(t))) => Envelope::from_json(&t).ok(),
                    Some(Ok(Message::Close(_))) | None => { st.error("closed_by_server"); break; }
                    Some(Ok(_)) => continue,
                    Some(Err(_)) => { st.error("ws_error"); break; }
                };
                let Some(body) = env.and_then(|e| e.body) else { st.error("decode"); continue; };
                match body {
                    Body::AuthOk(_) => {
                        authed = true;
                        st.connected = true;
                        if !subscribed && !cfg.symbols.is_empty() {
                            subscribed = true;
                            let sub = frame(Body::Subscribe(Subscribe {
                                request_id: format!("sub-{idx}"),
                                symbols: cfg.symbols.clone(),
                            }));
                            if ws.send(sub).await.is_err() { st.error("send"); break; }
                        }
                    }
                    Body::QuoteBatch(b) => {
                        let now = domain::now_ns();
                        for q in b.quotes {
                            st.quotes += 1;
                            if q.ts_ns > 0 && now >= q.ts_ns {
                                st.quote_latency.record_us((now - q.ts_ns) / 1_000);
                            }
                        }
                    }
                    Body::Ack(a) => {
                        if let Some((t, acked)) = pending.get_mut(&a.request_id) {
                            if !*acked {
                                *acked = true;
                                st.acks += 1;
                                st.ack_latency.record_us(t.elapsed().as_micros() as u64);
                            }
                        }
                    }
                    Body::OrderUpdate(u) => {
                        let s = OrderStatus::try_from(u.status).unwrap_or(OrderStatus::Unspecified);
                        if matches!(s, OrderStatus::Filled) {
                            if let Some((t, _)) = pending.remove(&u.client_request_id) {
                                st.fills += 1;
                                st.fill_latency.record_us(t.elapsed().as_micros() as u64);
                            }
                        } else if matches!(s, OrderStatus::Rejected | OrderStatus::Canceled | OrderStatus::Expired)
                            && pending.remove(&u.client_request_id).is_some()
                        {
                            st.rejects += 1;
                        }
                    }
                    Body::Error(e) => {
                        if !authed {
                            st.error("auth");
                            break;
                        }
                        if pending.remove(&e.request_id).is_some() {
                            st.rejects += 1;
                        }
                        st.error(error_kind(e.code));
                    }
                    _ => {}
                }
            }
        }
    }
    let unanswered = pending
        .values()
        .filter(|(t, _)| t.elapsed() > Duration::from_secs(5))
        .count();
    for _ in 0..unanswered {
        st.error("order_no_final_state_5s");
    }
    let _ = ws.close(None).await;
    st
}
