//! TCP acceptors for the MD and trading sessions.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fix_codec::{
    Body, InstrumentRef, MarketDataSnapshot, MdEntry, MdEntryType, SubscriptionRequestType,
};
use fix_session::{
    run_session, MemoryStore, Role, Session, SessionCommand, SessionConfig, SessionEvent,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{EndpointConfig, SimConfig};
use crate::engine::{rej, Engine};
use crate::market::Market;

/// Fault-injection scenario for tests and console demos (process-wide).
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scenario {
    /// Percentage of new orders rejected (0..100).
    #[serde(default)]
    pub reject_pct: u8,
    /// Added delay before each execution report (ms).
    #[serde(default)]
    pub latency_ms: u64,
}

static SCENARIO: std::sync::OnceLock<Mutex<Scenario>> = std::sync::OnceLock::new();

pub fn scenario() -> Scenario {
    SCENARIO
        .get_or_init(|| Mutex::new(Scenario::default()))
        .lock()
        .map(|s| *s)
        .unwrap_or_default()
}

pub fn set_scenario(s: Scenario) {
    if let Ok(mut g) = SCENARIO
        .get_or_init(|| Mutex::new(Scenario::default()))
        .lock()
    {
        *g = s;
    }
}

type SharedMarket = Arc<Mutex<Market>>;

/// Running simulator.
pub struct SimHandle {
    pub md_addr: SocketAddr,
    pub trade_addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    market: SharedMarket,
    ticks: broadcast::Sender<()>,
}

impl SimHandle {
    /// Control hook: jumps the instrument's mid price (symbol such as
    /// `EUR/USD`, or security id) and publishes the new book right away.
    /// Used to simulate price shocks (e.g. stop-out scenarios).
    pub fn set_mid(&self, symbol: &str, mid: domain::Price) -> bool {
        let ok = self
            .market
            .lock()
            .map(|mut m| m.set_mid(symbol, mid))
            .unwrap_or(false);
        if ok {
            let _ = self.ticks.send(());
        }
        ok
    }

    /// Current mid price of an instrument.
    pub fn mid(&self, symbol: &str) -> Option<domain::Price> {
        self.market.lock().ok().and_then(|m| m.mid(symbol))
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        for t in self.tasks {
            let _ = t.await;
        }
    }
}

/// Binds both listeners and starts the price feed.
pub async fn start(cfg: SimConfig) -> std::io::Result<SimHandle> {
    let md_listener = TcpListener::bind(&cfg.md.listen).await?;
    let trade_listener = TcpListener::bind(&cfg.trade.listen).await?;
    let md_addr = md_listener.local_addr()?;
    let trade_addr = trade_listener.local_addr()?;
    info!(%md_addr, %trade_addr, "lp-simulator listening");

    let market: SharedMarket = Arc::new(Mutex::new(Market::new(&cfg)));
    let (ticks_tx, _) = broadcast::channel::<()>(64);
    let (shutdown, shutdown_rx) = watch::channel(false);
    let cfg = Arc::new(cfg);

    let mut tasks = Vec::new();
    {
        let market = market.clone();
        let ticks_tx = ticks_tx.clone();
        let mut sd = shutdown_rx.clone();
        let every = Duration::from_millis(cfg.tick_interval_ms.max(1));
        tasks.push(tokio::spawn(async move {
            let mut iv = tokio::time::interval(every);
            loop {
                tokio::select! {
                    _ = iv.tick() => {
                        if let Ok(mut m) = market.lock() {
                            m.step();
                        }
                        let _ = ticks_tx.send(());
                    }
                    _ = sd.changed() => break,
                }
            }
        }));
    }
    tasks.push(tokio::spawn(accept_loop(
        md_listener,
        cfg.md.clone(),
        shutdown_rx.clone(),
        {
            let market = market.clone();
            let cfg = cfg.clone();
            let ticks_tx = ticks_tx.clone();
            move |io, ep, sd| {
                tokio::spawn(md_connection(
                    io,
                    ep,
                    market.clone(),
                    cfg.clone(),
                    ticks_tx.subscribe(),
                    sd,
                ));
            }
        },
    )));
    tasks.push(tokio::spawn(accept_loop(
        trade_listener,
        cfg.trade.clone(),
        shutdown_rx,
        {
            let market = market.clone();
            move |io, ep, sd| {
                tokio::spawn(trade_connection(io, ep, market.clone(), sd));
            }
        },
    )));
    if let Some(addr) = cfg.control_listen.clone() {
        let listener = TcpListener::bind(&addr).await?;
        info!(control = %listener.local_addr()?, "lp-simulator control API");
        let st = ControlState {
            market: market.clone(),
            ticks: ticks_tx.clone(),
        };
        let app = axum::Router::new()
            .route("/state", axum::routing::get(control_state))
            .route("/shock", axum::routing::post(control_shock))
            .route("/scenario", axum::routing::post(control_scenario))
            .with_state(st);
        let mut sd = shutdown_rx.clone();
        tasks.push(tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = sd.changed().await;
                })
                .await;
        }));
    }
    Ok(SimHandle {
        md_addr,
        trade_addr,
        shutdown,
        tasks,
        market,
        ticks: ticks_tx,
    })
}

#[derive(Clone)]
struct ControlState {
    market: SharedMarket,
    ticks: broadcast::Sender<()>,
}

async fn control_state(
    axum::extract::State(st): axum::extract::State<ControlState>,
) -> axum::Json<serde_json::Value> {
    let mids: Vec<serde_json::Value> = st
        .market
        .lock()
        .map(|m| {
            m.security_ids()
                .map(|id| serde_json::json!({ "securityId": id, "mid": m.mid(id).map(|p| p.to_string()) }))
                .collect()
        })
        .unwrap_or_default();
    axum::Json(serde_json::json!({ "scenario": scenario(), "instruments": mids }))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShockReq {
    /// Symbol (`EUR/USD`) or security id.
    symbol: String,
    /// Relative move in percent (e.g. -1.5); clamped to ±20 %.
    pct: f64,
}

async fn control_shock(
    axum::extract::State(st): axum::extract::State<ControlState>,
    axum::Json(req): axum::Json<ShockReq>,
) -> Result<axum::Json<serde_json::Value>, axum::http::StatusCode> {
    let pct = req.pct.clamp(-20.0, 20.0);
    let (old, new) = {
        let mut m = st
            .market
            .lock()
            .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
        let old = m
            .mid(&req.symbol)
            .ok_or(axum::http::StatusCode::NOT_FOUND)?;
        let new =
            domain::Price::from_raw(((old.raw() as f64) * (1.0 + pct / 100.0)).round() as i64);
        if !m.set_mid(&req.symbol, new) {
            return Err(axum::http::StatusCode::NOT_FOUND);
        }
        (old, new)
    };
    let _ = st.ticks.send(());
    info!(symbol = %req.symbol, pct, %old, %new, "scenario: price shock");
    Ok(axum::Json(
        serde_json::json!({ "symbol": req.symbol, "oldMid": old.to_string(), "newMid": new.to_string() }),
    ))
}

async fn control_scenario(axum::Json(s): axum::Json<Scenario>) -> axum::Json<Scenario> {
    let s = Scenario {
        reject_pct: s.reject_pct.min(100),
        latency_ms: s.latency_ms.min(30_000),
    };
    set_scenario(s);
    info!(?s, "scenario updated");
    axum::Json(s)
}

async fn accept_loop<F>(
    listener: TcpListener,
    ep: EndpointConfig,
    mut shutdown: watch::Receiver<bool>,
    on_conn: F,
) where
    F: Fn(TcpStream, EndpointConfig, watch::Receiver<bool>),
{
    loop {
        tokio::select! {
            r = listener.accept() => match r {
                Ok((io, peer)) => {
                    info!(%peer, comp = %ep.comp_id, "accepted connection");
                    let _ = io.set_nodelay(true);
                    on_conn(io, ep.clone(), shutdown.clone());
                }
                Err(e) => warn!(error = %e, "accept failed"),
            },
            _ = shutdown.changed() => break,
        }
    }
}

fn session_for(ep: &EndpointConfig) -> Session<MemoryStore> {
    let mut c = SessionConfig::new(Role::Acceptor, &ep.comp_id, &ep.client_comp_id);
    c.username = ep.username.clone();
    c.password = ep.password.clone();
    Session::new(c, MemoryStore::new(), std::time::Instant::now())
}

type Channels = (
    mpsc::Sender<SessionCommand>,
    mpsc::Receiver<SessionEvent>,
    JoinHandle<()>,
);

fn spawn_session(io: TcpStream, ep: &EndpointConfig) -> Channels {
    let (cmd_tx, cmd_rx) = mpsc::channel(1024);
    let (ev_tx, ev_rx) = mpsc::channel(1024);
    let session = session_for(ep);
    let h = tokio::spawn(async move {
        run_session(io, session, cmd_rx, ev_tx).await;
    });
    (cmd_tx, ev_rx, h)
}

fn snapshot(market: &SharedMarket, cfg: &SimConfig, security_id: &str, req: &str) -> Option<Body> {
    let m = market.lock().ok()?;
    let book = m.book(security_id)?;
    let mut entries = Vec::with_capacity(book.bids.len() + book.asks.len());
    entries.extend(book.bids.iter().map(|l| MdEntry {
        entry_type: MdEntryType::Bid,
        price: l.price,
        size: l.qty,
    }));
    entries.extend(book.asks.iter().map(|l| MdEntry {
        entry_type: MdEntryType::Offer,
        price: l.price,
        size: l.qty,
    }));
    Some(Body::MarketDataSnapshot(MarketDataSnapshot {
        md_req_id: Some(req.to_owned()),
        instrument: InstrumentRef {
            security_id: security_id.to_owned(),
            security_id_source: cfg.security_id_source.clone(),
        },
        entries,
    }))
}

async fn md_connection(
    io: TcpStream,
    ep: EndpointConfig,
    market: SharedMarket,
    cfg: Arc<SimConfig>,
    mut ticks: broadcast::Receiver<()>,
    mut shutdown: watch::Receiver<bool>,
) {
    let (cmd, mut events, handle) = spawn_session(io, &ep);
    // (security_id, md_req_id)
    let mut subs: HashSet<(String, String)> = HashSet::new();
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Some(SessionEvent::App(m)) => {
                    let Body::MarketDataRequest(req) = m.body else { continue };
                    for i in &req.instruments {
                        let known = market
                            .lock()
                            .map(|m| m.book(&i.security_id).is_some())
                            .unwrap_or(false);
                        if !known {
                            // MarketDataRequestReject (Y), UnknownSymbol.
                            let rej = Body::Unknown {
                                msg_type: "Y".into(),
                                fields: vec![(262, req.md_req_id.clone().into_bytes()), (281, b"0".to_vec())],
                            };
                            let _ = cmd.send(SessionCommand::Send(rej)).await;
                            continue;
                        }
                        let key = (i.security_id.clone(), req.md_req_id.clone());
                        match req.subscription_type {
                            SubscriptionRequestType::Unsubscribe => {
                                subs.retain(|(s, _)| s != &i.security_id);
                            }
                            t => {
                                if let Some(w) = snapshot(&market, &cfg, &i.security_id, &req.md_req_id) {
                                    let _ = cmd.send(SessionCommand::Send(w)).await;
                                }
                                if t == SubscriptionRequestType::Subscribe {
                                    subs.insert(key);
                                }
                            }
                        }
                    }
                }
                Some(SessionEvent::Disconnected(r)) => {
                    info!(reason = %r, "MD session closed");
                    break;
                }
                Some(_) => {}
                None => break,
            },
            t = ticks.recv() => {
                if matches!(t, Err(broadcast::error::RecvError::Closed)) {
                    break;
                }
                for (sid, req) in &subs {
                    if let Some(w) = snapshot(&market, &cfg, sid, req) {
                        if cmd.try_send(SessionCommand::Send(w)).is_err() {
                            warn!("MD client slow, dropping snapshot");
                        }
                    }
                }
            }
            _ = shutdown.changed() => {
                let _ = cmd.send(SessionCommand::Logout(Some("simulator shutdown".into()))).await;
                break;
            }
        }
    }
    drop(cmd);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

async fn trade_connection(
    io: TcpStream,
    ep: EndpointConfig,
    market: SharedMarket,
    mut shutdown: watch::Receiver<bool>,
) {
    let (cmd, mut events, handle) = spawn_session(io, &ep);
    let mut engine = Engine::new();
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Some(SessionEvent::App(m)) => {
                    let replies = match &m.body {
                        Body::NewOrderSingle(nos) => {
                            let sc = scenario();
                            if sc.latency_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(sc.latency_ms.min(30_000))).await;
                            }
                            if sc.reject_pct > 0 && (rand::random::<u8>() % 100) < sc.reject_pct.min(100) {
                                engine.reject(nos, rej::OTHER, "scenario: injected reject")
                            } else {
                                let book = market.lock().ok().and_then(|m| m.book(&nos.instrument.security_id).cloned());
                                engine.new_order(nos, book.as_ref())
                            }
                        }
                        Body::OrderCancelRequest(r) => vec![engine.cancel(r)],
                        Body::OrderCancelReplaceRequest(r) => vec![engine.replace(r)],
                        other => {
                            warn!(msg_type = other.msg_type(), "unsupported message on trading session");
                            Vec::new()
                        }
                    };
                    for r in replies {
                        let _ = cmd.send(SessionCommand::Send(r)).await;
                    }
                }
                Some(SessionEvent::Disconnected(r)) => {
                    info!(reason = %r, "trading session closed");
                    break;
                }
                Some(_) => {}
                None => break,
            },
            _ = shutdown.changed() => {
                let _ = cmd.send(SessionCommand::Logout(Some("simulator shutdown".into()))).await;
                break;
            }
        }
    }
    drop(cmd);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}
