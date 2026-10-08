//! MT5 plugin bridge: `/bridge`, protocol v1 (JSON text frames, field `t` =
//! type). One session = one institution (an MT5 server) trading through one
//! **institution account** (netting group). See `docs/10-mt5-plugin-kopru.md`
//! and `docs/11-mt5-plugin-plan.md`.
//!
//! Institutions come from `FXVPS_BRIDGE_FILE` (JSON list):
//! `[{"id":"kurum1","key_sha256":"<hex>","account":"K-1","orders_per_sec":200,"ips":[]}]`.
//! The plugin's key is only stored as its SHA-256.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use client_proto::{ErrorCode, OrderStatus};
use domain::{Fixed, Side};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;

use crate::hub::{AccountEvent, Hub, NewOrder};
use crate::PeerIp;
use core_engine::api::OrderKind;

pub const PROTOCOL: u64 = 1;
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// No frame from the plugin for this long = dead session (it pings every second).
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const HEARTBEAT_MS: u64 = 1_000;

#[derive(Clone, Debug, Deserialize)]
pub struct Institution {
    pub id: String,
    pub key_sha256: String,
    pub account: String,
    #[serde(default = "default_rate")]
    pub orders_per_sec: u32,
    /// Allowed source IPs (empty = any).
    #[serde(default)]
    pub ips: Vec<IpAddr>,
}

fn default_rate() -> u32 {
    100
}

/// Live view of one plugin session (console "Kurumlar").
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SessionStat {
    pub institution: String,
    pub server: String,
    pub plugin: String,
    pub ip: Option<String>,
    pub since_ms: u64,
    pub last_ms: u64,
    pub orders: u64,
    pub fills: u64,
    pub rejects: u64,
    pub reconcile_ok: Option<bool>,
    /// Order received -> final fill sent, ms (last 500 orders).
    pub fill_ms_p50: f64,
    pub fill_ms_p99: f64,
    #[serde(skip)]
    lat: std::collections::VecDeque<f64>,
}

impl SessionStat {
    fn push_latency(&mut self, ms: f64) {
        if self.lat.len() == 500 {
            self.lat.pop_front();
        }
        self.lat.push_back(ms);
        let mut v: Vec<f64> = self.lat.iter().copied().collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let at = |p: f64| v[((v.len() as f64 - 1.0) * p).round() as usize];
        self.fill_ms_p50 = (at(0.5) * 10.0).round() / 10.0;
        self.fill_ms_p99 = (at(0.99) * 10.0).round() / 10.0;
    }
}

#[derive(Debug, Default)]
pub struct Bridge {
    institutions: std::sync::RwLock<HashMap<String, Institution>>,
    path: Option<std::path::PathBuf>,
    sessions: std::sync::Mutex<HashMap<u64, SessionStat>>,
    next: std::sync::atomic::AtomicU64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Bridge {
    pub fn new(list: Vec<Institution>) -> Bridge {
        let b = Bridge::default();
        b.set(list);
        b
    }

    fn set(&self, list: Vec<Institution>) {
        if let Ok(mut m) = self.institutions.write() {
            *m = list.into_iter().map(|i| (i.id.clone(), i)).collect();
        }
    }

    fn read_file(path: &std::path::Path) -> std::io::Result<Vec<Institution>> {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    pub fn load(path: &std::path::Path) -> std::io::Result<Bridge> {
        let mut b = Bridge::new(Bridge::read_file(path)?);
        b.path = Some(path.to_path_buf());
        Ok(b)
    }

    /// `FXVPS_BRIDGE_FILE`; `None` = bridge disabled. A missing file is an
    /// empty list: the console creates it with the first institution.
    pub fn from_env() -> Option<Bridge> {
        let p = std::env::var("FXVPS_BRIDGE_FILE")
            .ok()
            .filter(|p| !p.trim().is_empty())?;
        match Bridge::load(std::path::Path::new(&p)) {
            Ok(b) => {
                tracing::info!(institutions = b.count(), "MT5 bridge enabled");
                Some(b)
            }
            Err(e) => {
                tracing::error!(%p, %e, "MT5 bridge file unreadable; bridge disabled");
                None
            }
        }
    }

    pub fn count(&self) -> usize {
        self.institutions.read().map(|m| m.len()).unwrap_or(0)
    }

    /// Reloads the institution file on change and writes the live session
    /// table next to it (`durum.json`) every two seconds.
    pub fn spawn_tasks(self: &Arc<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let me = self.clone();
        tokio::spawn(async move {
            let status = path.with_file_name("durum.json");
            let mut seen = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            loop {
                tick.tick().await;
                let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                if mtime != seen {
                    match Bridge::read_file(&path) {
                        Ok(list) => {
                            let n = list.len();
                            me.set(list);
                            seen = mtime;
                            tracing::info!(institutions = n, "bridge institutions reloaded");
                        }
                        Err(e) => {
                            tracing::error!(%e, "bridge institutions unreadable; keeping the old list")
                        }
                    }
                }
                let sessions: Vec<SessionStat> = me
                    .sessions
                    .lock()
                    .map(|m| m.values().cloned().collect())
                    .unwrap_or_default();
                if let Some(dir) = status.parent().filter(|d| d.exists()) {
                    let body = json!({"at": now_ms(), "sessions": sessions}).to_string();
                    let tmp = dir.join("durum.json.tmp");
                    if std::fs::write(&tmp, body).is_ok() {
                        let _ = std::fs::rename(&tmp, &status);
                    }
                }
            }
        });
    }

    fn authenticate(&self, id: &str, key: &str, ip: Option<IpAddr>) -> Option<Institution> {
        let m = self.institutions.read().ok()?;
        let inst = m.get(id)?;
        let digest = hex(&Sha256::digest(key.as_bytes()));
        if !eq_ct(&digest, &inst.key_sha256.to_ascii_lowercase()) {
            return None;
        }
        if !inst.ips.is_empty() && !ip.is_some_and(|ip| inst.ips.contains(&ip)) {
            return None;
        }
        Some(inst.clone())
    }

    fn open_session(&self, s: SessionStat) -> u64 {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut m) = self.sessions.lock() {
            m.insert(id, s);
        }
        id
    }

    fn stat(&self, id: u64, f: impl FnOnce(&mut SessionStat)) {
        if let Ok(mut m) = self.sessions.lock() {
            if let Some(s) = m.get_mut(&id) {
                f(s);
            }
        }
    }

    fn close_session(&self, id: u64) {
        if let Ok(mut m) = self.sessions.lock() {
            m.remove(&id);
        }
    }
}

/// Removes the session from the live table however the session ends.
struct SessionGuard(Arc<Bridge>, u64);
impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.0.close_session(self.1);
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn eq_ct(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

pub async fn handler(
    State(hub): State<Arc<Hub>>,
    Extension(bridge): Extension<Arc<Bridge>>,
    PeerIp(ip): PeerIp,
    ws: WebSocketUpgrade,
) -> Response {
    if hub.core().is_none() {
        return (StatusCode::SERVICE_UNAVAILABLE, "no core").into_response();
    }
    // same connection limits as the terminal: a hello flood cannot pile up sockets
    let slot = match hub.conns.acquire(ip) {
        Ok(s) => s,
        Err(e) => {
            hub.metrics.connections_rejected.inc();
            tracing::debug!(?ip, ?e, "bridge connection limit");
            return (StatusCode::TOO_MANY_REQUESTS, "too many connections").into_response();
        }
    };
    ws.max_message_size(64 * 1024)
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            run(hub, bridge, socket, ip).await
        })
}

/// Per-symbol data the session needs to convert lots <-> base units.
#[derive(Clone)]
struct Sym {
    contract: Fixed,
}

/// Order of this session awaiting fills.
struct Track {
    filled: Fixed,
    done: bool,
    at: Instant,
}

struct Session {
    hub: Arc<Hub>,
    inst: Institution,
    syms: HashMap<String, Sym>,
    tracks: HashMap<String, Track>,
    bucket: f64,
    last_refill: Instant,
}

async fn run(hub: Arc<Hub>, bridge: Arc<Bridge>, socket: WebSocket, ip: Option<IpAddr>) {
    let (mut tx, mut rx) = socket.split();
    // --- hello ------------------------------------------------------------
    let hello = match tokio::time::timeout(HELLO_TIMEOUT, rx.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<Value>(&t).ok(),
        _ => None,
    };
    let Some(hello) = hello.filter(|h| h["t"] == "hello") else {
        let _ = send(
            &mut tx,
            json!({"t":"error","code":"bad_request","text":"hello expected"}),
        )
        .await;
        return;
    };
    if hello["v"].as_u64() != Some(PROTOCOL) {
        let _ = send(
            &mut tx,
            json!({"t":"error","code":"version","text":format!("protocol v{PROTOCOL} required")}),
        )
        .await;
        return;
    }
    let id = hello["institution"].as_str().unwrap_or_default();
    let key = hello["key"].as_str().unwrap_or_default();
    let Some(inst) = bridge.authenticate(id, key, ip) else {
        tracing::warn!(institution = id, ?ip, "bridge authentication failed");
        let _ = send(
            &mut tx,
            json!({"t":"error","code":"auth","text":"authentication failed"}),
        )
        .await;
        return;
    };
    tracing::info!(
        institution = %inst.id,
        server = hello["server"].as_str().unwrap_or_default(),
        plugin = hello["plugin"].as_str().unwrap_or_default(),
        ?ip,
        "bridge session up"
    );

    let sid = bridge.open_session(SessionStat {
        institution: inst.id.clone(),
        server: hello["server"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(64)
            .collect(),
        plugin: hello["plugin"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(32)
            .collect(),
        ip: ip.map(|i| i.to_string()),
        since_ms: now_ms(),
        last_ms: now_ms(),
        ..Default::default()
    });
    let _guard = SessionGuard(bridge.clone(), sid);
    // Subscribe before the welcome so nothing is missed.
    let mut quotes = hub.subscribe_quotes();
    let mut accounts = hub.subscribe_accounts();
    let groups: HashSet<String> = hub.account_group(&inst.account).into_iter().collect();

    let mut syms = HashMap::new();
    let mut list = Vec::new();
    for i in hub.instruments() {
        let Some(contract) = i
            .contract_size
            .and_then(|d| d.to_fixed())
            .filter(|c| c.is_positive())
        else {
            continue;
        };
        list.push(json!({
            "symbol": i.symbol,
            "digits": i.digits,
            "contract_size": contract.to_string(),
            "tick_size": i.tick_size.and_then(|d| d.to_fixed()).map(|f| f.to_string()),
        }));
        syms.insert(i.symbol.clone(), Sym { contract });
    }
    let welcome = json!({
        "t": "welcome", "v": PROTOCOL, "account": inst.account,
        "heartbeat_ms": HEARTBEAT_MS, "symbols": list,
    });
    if send(&mut tx, welcome).await.is_err() {
        return;
    }
    let names: Vec<String> = syms.keys().cloned().collect();
    for q in hub.last_quotes(&names, &groups) {
        if send(&mut tx, quote_json(&q)).await.is_err() {
            return;
        }
    }

    let rate = inst.orders_per_sec.max(1) as f64;
    let mut s = Session {
        hub: hub.clone(),
        inst,
        syms,
        tracks: HashMap::new(),
        bucket: rate,
        last_refill: Instant::now(),
    };
    let mut last_seen = Instant::now();
    let mut idle = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            m = rx.next() => {
                last_seen = Instant::now();
                match m {
                    Some(Ok(Message::Text(t))) => {
                        let outs = s.on_frame(&t).await;
                        let is_order = serde_json::from_str::<Value>(&t).is_ok_and(|v| v["t"] == "order");
                        bridge.stat(sid, |st| {
                            st.last_ms = now_ms();
                            if is_order {
                                st.orders += 1;
                            }
                            for o in &outs {
                                match o["t"].as_str() {
                                    Some("reject") => st.rejects += 1,
                                    Some("fill") if o["done"] == true => st.fills += 1,
                                    Some("reconcile_result") => st.reconcile_ok = o["ok"].as_bool(),
                                    _ => {}
                                }
                            }
                        });
                        for out in outs {
                            if send(&mut tx, out).await.is_err() { break; }
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {}
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                }
            }
            q = quotes.recv() => match q {
                Ok(m) => {
                    let visible = m.group.as_deref().is_none_or(|g| groups.contains(g));
                    if visible && s.syms.contains_key(&*m.quote.symbol)
                        && send(&mut tx, quote_json(&m.quote)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            e = accounts.recv() => match e {
                Ok(ev) => {
                    if let AccountEvent::Order(u) = &*ev {
                        if u.account_id == s.inst.account {
                            if let Some(out) = s.on_order_update(u) {
                                let lat = s.tracks.get(&u.client_request_id).map(|t| t.at.elapsed().as_secs_f64() * 1e3);
                                bridge.stat(sid, |st| match out["t"].as_str() {
                                    Some("fill") if out["done"] == true => {
                                        st.fills += 1;
                                        if let Some(ms) = lat {
                                            st.push_latency(ms);
                                        }
                                    }
                                    Some("reject") => st.rejects += 1,
                                    _ => {}
                                });
                                if send(&mut tx, out).await.is_err() { break; }
                            }
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    // events were dropped: a fill for a tracked order may be among
                    // them, so re-read every open order from the engine's record
                    tracing::warn!(institution = %s.inst.id, n, "bridge lagged on account events; resyncing");
                    let open: Vec<String> = s.tracks.iter().filter(|(_, t)| !t.done).map(|(id, _)| id.clone()).collect();
                    let mut outs = Vec::new();
                    for id in open {
                        if let Some(t) = s.tracks.get_mut(&id) {
                            t.done = true; // replay() reports the final state or re-opens it
                        }
                        outs.extend(s.replay(&id).await);
                    }
                    bridge.stat(sid, |st| {
                        for o in &outs {
                            match o["t"].as_str() {
                                Some("fill") if o["done"] == true => st.fills += 1,
                                Some("reject") => st.rejects += 1,
                                _ => {}
                            }
                        }
                    });
                    for out in outs {
                        if send(&mut tx, out).await.is_err() {
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = idle.tick() => {
                if last_seen.elapsed() > IDLE_TIMEOUT {
                    tracing::warn!(institution = %s.inst.id, "bridge session idle, closing");
                    break;
                }
            }
        }
    }
    tracing::info!(institution = %s.inst.id, "bridge session closed");
}

async fn send<S>(tx: &mut S, v: Value) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    tx.send(Message::Text(v.to_string().into()))
        .await
        .map_err(|_| ())
}

fn quote_json(q: &crate::hub::ClientQuote) -> Value {
    json!({
        "t": "quote", "s": &*q.symbol,
        "b": q.bid.map(|f| f.to_string()), "a": q.ask.map(|f| f.to_string()),
        "ts": q.ts_ns,
    })
}

fn reject(id: &str, code: &str, text: impl Into<String>) -> Value {
    json!({"t":"reject","id":id,"code":code,"text":text.into()})
}

/// Maps an engine reason to a protocol reject code.
fn reject_code(code: ErrorCode, text: &str) -> &'static str {
    let t = text.to_ascii_lowercase();
    match code {
        ErrorCode::RateLimited => "rate",
        ErrorCode::InsufficientMargin => "margin",
        ErrorCode::Unavailable => "session",
        ErrorCode::UnknownSymbol | ErrorCode::BadRequest => "bad_request",
        _ if t.contains("margin") => "margin",
        _ if t.contains("price") || t.contains("slippage") || t.contains("quote") => "price",
        _ if t.contains("lp") || t.contains("liquidity") => "liquidity",
        _ if t.contains("closed") || t.contains("session") => "closed",
        _ => "rejected",
    }
}

fn status_code(text: &str) -> &'static str {
    reject_code(ErrorCode::OrderRejected, text)
}

impl Session {
    fn admit(&mut self) -> bool {
        let rate = self.inst.orders_per_sec.max(1) as f64;
        let now = Instant::now();
        self.bucket =
            (self.bucket + now.duration_since(self.last_refill).as_secs_f64() * rate).min(rate);
        self.last_refill = now;
        if self.bucket >= 1.0 {
            self.bucket -= 1.0;
            true
        } else {
            false
        }
    }

    async fn on_frame(&mut self, text: &str) -> Vec<Value> {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return vec![json!({"t":"error","code":"bad_request","text":"invalid JSON"})];
        };
        match v["t"].as_str().unwrap_or_default() {
            "ping" => vec![json!({"t":"pong","ts":v["ts"]})],
            "order" => self.on_order(&v).await,
            "reconcile" => vec![self.on_reconcile(&v).await],
            other => vec![
                json!({"t":"error","code":"bad_request","text":format!("unknown type {other:?}")}),
            ],
        }
    }

    async fn on_order(&mut self, v: &Value) -> Vec<Value> {
        let id = v["id"].as_str().unwrap_or_default().to_string();
        if id.is_empty() || id.len() > 64 || id.contains('~') {
            return vec![reject(
                &id,
                "bad_request",
                "id required (1-64 chars, no '~')",
            )];
        }
        // Re-sent id (plugin reconnect / retry): report the engine's state.
        if let Some(t) = self.tracks.get(&id) {
            if t.done {
                return self.replay(&id).await;
            }
            return Vec::new(); // still working: fills will follow
        }
        let symbol = v["symbol"].as_str().unwrap_or_default().to_string();
        let Some(sym) = self.syms.get(&symbol).cloned() else {
            return vec![reject(
                &id,
                "bad_request",
                format!("unknown symbol {symbol}"),
            )];
        };
        let side = match v["side"].as_str() {
            Some("buy") => Side::Buy,
            Some("sell") => Side::Sell,
            _ => return vec![reject(&id, "bad_request", "side must be buy or sell")],
        };
        let Some(lots) = dec(&v["lots"]).filter(|l| l.is_positive()) else {
            return vec![reject(&id, "bad_request", "lots required")];
        };
        let Some(units) = lots.checked_mul(sym.contract) else {
            return vec![reject(&id, "bad_request", "lots out of range")];
        };
        if !self.admit() {
            return vec![reject(
                &id,
                "rate",
                format!("over {} orders/s", self.inst.orders_per_sec),
            )];
        }
        let mut o = NewOrder::market(&id, &self.inst.account, &symbol, side, units);
        match v["kind"].as_str().unwrap_or("market") {
            "market" => {
                o.max_deviation_points = v["deviation"]
                    .as_u64()
                    .filter(|d| *d > 0)
                    .map(|d| d.min(u32::MAX as u64) as u32);
            }
            "limit" => {
                let Some(p) = dec(&v["price"]).filter(|p| p.is_positive()) else {
                    return vec![reject(&id, "bad_request", "limit price required")];
                };
                o.ord_type = OrderKind::Limit;
                o.limit_price = Some(p);
            }
            k => {
                return vec![reject(
                    &id,
                    "bad_request",
                    format!("unsupported kind {k:?}"),
                )]
            }
        }
        self.tracks.insert(
            id.clone(),
            Track {
                filled: Fixed::from_int(0),
                done: false,
                at: Instant::now(),
            },
        );
        match self.hub.place_order(o).await {
            Ok(_) => Vec::new(),
            Err(e) if e.1.contains("duplicate") => {
                // Placed in an earlier session: answer from the engine's record.
                if let Some(t) = self.tracks.get_mut(&id) {
                    t.done = true;
                }
                self.replay(&id).await
            }
            Err(e) => {
                if let Some(t) = self.tracks.get_mut(&id) {
                    t.done = true;
                }
                vec![reject(&id, reject_code(e.0, &e.1), e.1)]
            }
        }
    }

    /// Final state of an order already known to the engine.
    async fn replay(&mut self, id: &str) -> Vec<Value> {
        let acc = self.inst.account.clone();
        let mut all = self.hub.orders(&acc).await.unwrap_or_default();
        all.extend(self.hub.order_history(&acc).await.unwrap_or_default());
        let Some(u) = all.into_iter().rev().find(|u| u.client_request_id == id) else {
            return vec![reject(id, "rejected", "order not found")];
        };
        let contract = self.syms.get(&u.symbol).map(|s| s.contract);
        let filled = u
            .filled_qty
            .and_then(|d| d.to_fixed())
            .unwrap_or(Fixed::from_int(0));
        let st = OrderStatus::try_from(u.status).unwrap_or(OrderStatus::Unspecified);
        if filled.is_zero()
            && matches!(
                st,
                OrderStatus::Rejected | OrderStatus::Canceled | OrderStatus::Expired
            )
        {
            return vec![reject(id, status_code(&u.text), u.text.clone())];
        }
        if !terminal(st) {
            if let Some(t) = self.tracks.get_mut(id) {
                t.done = false;
            }
            return Vec::new();
        }
        let avg = u.avg_price.and_then(|d| d.to_fixed());
        vec![json!({
            "t":"fill","id":id,"lots":"0","price":avg.map(|f| f.to_string()),
            "filled": lots_of(filled, contract), "avg": avg.map(|f| f.to_string()), "done": true,
        })]
    }

    fn on_order_update(&mut self, u: &client_proto::OrderUpdate) -> Option<Value> {
        let id = u.client_request_id.as_str();
        let contract = self.syms.get(&u.symbol).map(|s| s.contract);
        let track = self.tracks.get_mut(id)?;
        if track.done {
            return None;
        }
        let st = OrderStatus::try_from(u.status).unwrap_or(OrderStatus::Unspecified);
        let filled = u
            .filled_qty
            .and_then(|d| d.to_fixed())
            .unwrap_or(Fixed::from_int(0));
        let fin = terminal(st);
        if filled.is_zero() {
            if fin {
                track.done = true;
                return Some(reject(id, status_code(&u.text), u.text.clone()));
            }
            return None;
        }
        let part = filled
            .checked_sub(track.filled)
            .unwrap_or(Fixed::from_int(0));
        if !part.is_positive() && !fin {
            return None;
        }
        track.filled = filled;
        track.done = fin;
        let avg = u
            .avg_price
            .and_then(|d| d.to_fixed())
            .map(|f| f.to_string());
        let price = u
            .last_price
            .and_then(|d| d.to_fixed())
            .map(|f| f.to_string())
            .or(avg.clone());
        Some(json!({
            "t":"fill","id":id,"lots":lots_of(part, contract),"price":price,
            "filled":lots_of(filled, contract),"avg":avg,"done":fin,
        }))
    }

    async fn on_reconcile(&mut self, v: &Value) -> Value {
        let Some(snap) = self.hub.account_snapshot(&self.inst.account).await else {
            return json!({"t":"reconcile_result","ok":false,"text":"account unavailable"});
        };
        let mut ours: HashMap<String, Fixed> = HashMap::new();
        for p in &snap.positions {
            let q = p
                .net_qty
                .and_then(|d| d.to_fixed())
                .unwrap_or(Fixed::from_int(0));
            let contract = self.syms.get(&p.symbol).map(|s| s.contract);
            let lots = contract
                .and_then(|c| q.checked_div(c))
                .unwrap_or(Fixed::from_int(0));
            let e = ours.entry(p.symbol.clone()).or_insert(Fixed::from_int(0));
            *e = *e + lots;
        }
        let theirs: HashMap<String, Fixed> = v["net"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, x)| Some((k.clone(), dec(x)?)))
                    .collect()
            })
            .unwrap_or_default();
        let mut diff = serde_json::Map::new();
        for k in ours.keys().chain(theirs.keys()).collect::<HashSet<_>>() {
            let a = ours.get(k).copied().unwrap_or(Fixed::from_int(0));
            let b = theirs.get(k).copied().unwrap_or(Fixed::from_int(0));
            if a != b {
                diff.insert(k.clone(), json!((b - a).to_string()));
            }
        }
        let ours: serde_json::Map<String, Value> = ours
            .into_iter()
            .filter(|(_, v)| !v.is_zero())
            .map(|(k, v)| (k, json!(v.to_string())))
            .collect();
        if !diff.is_empty() {
            tracing::warn!(institution = %self.inst.id, ?diff, "bridge reconciliation mismatch");
        }
        json!({"t":"reconcile_result","ok":diff.is_empty(),"ours":ours,"diff":diff})
    }
}

fn terminal(st: OrderStatus) -> bool {
    matches!(
        st,
        OrderStatus::Filled | OrderStatus::Canceled | OrderStatus::Rejected | OrderStatus::Expired
    )
}

fn dec(v: &Value) -> Option<Fixed> {
    match v {
        Value::String(s) => s.trim().parse().ok(),
        Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

fn lots_of(units: Fixed, contract: Option<Fixed>) -> String {
    contract
        .and_then(|c| units.checked_div(c))
        .unwrap_or(units)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticates_by_key_digest_and_ip() {
        let key = "s3cret";
        let b = Bridge::new(vec![Institution {
            id: "k1".into(),
            key_sha256: hex(&Sha256::digest(key.as_bytes())),
            account: "K-1".into(),
            orders_per_sec: 10,
            ips: vec!["10.0.0.1".parse().unwrap()],
        }]);
        assert!(b
            .authenticate("k1", key, Some("10.0.0.1".parse().unwrap()))
            .is_some());
        assert!(b
            .authenticate("k1", "wrong", Some("10.0.0.1".parse().unwrap()))
            .is_none());
        assert!(b
            .authenticate("k1", key, Some("10.0.0.2".parse().unwrap()))
            .is_none());
        assert!(b.authenticate("k2", key, None).is_none());
    }

    #[test]
    fn reject_codes() {
        assert_eq!(reject_code(ErrorCode::InsufficientMargin, ""), "margin");
        assert_eq!(
            reject_code(ErrorCode::OrderRejected, "LP: no liquidity"),
            "liquidity"
        );
        assert_eq!(
            reject_code(ErrorCode::OrderRejected, "price moved beyond slippage"),
            "price"
        );
        assert_eq!(
            lots_of(Fixed::from_int(30_000), Some(Fixed::from_int(100_000))),
            "0.3"
        );
    }
}
