//! Session supervisors: connect, log on, normalize, reconnect.

use std::sync::Arc;
use std::time::Duration;

use domain::{Execution, Order, Quote, Side};
use fix_codec::{
    now_timestamp, Body, MarketDataRequest, MdEntryType, OrderCancelReplaceRequest,
    OrderCancelRequest, SubscriptionRequestType,
};
use fix_session::{
    run_session, FileStore, MemoryStore, MessageStore, Role, Session, SessionCommand,
    SessionConfig, SessionEvent,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{GatewayConfig, SessionEndpoint};
use crate::normalize::{self, Books};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionKind {
    MarketData,
    Trading,
}

/// Normalized events published on the internal channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayEvent {
    SessionUp {
        session: SessionKind,
    },
    SessionDown {
        session: SessionKind,
        reason: String,
    },
    Quote(Quote),
    Execution(Execution),
    /// LP rejected a cancel or replace (OrderCancelReject 9).
    CancelRejected {
        cl_ord_id: String,
        orig_cl_ord_id: String,
        text: Option<String>,
    },
    /// The gateway could not forward a command (session down, unknown symbol, ...).
    CommandRejected {
        cl_ord_id: String,
        reason: String,
    },
    /// Session-level Reject from the LP.
    SessionReject {
        session: SessionKind,
        ref_seq_num: u64,
        text: Option<String>,
    },
}

/// Commands accepted from internal components (OMS, tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderCommand {
    Submit(Order),
    Cancel {
        cl_ord_id: String,
        orig_cl_ord_id: String,
        symbol: String,
        side: Side,
    },
    Replace {
        orig_cl_ord_id: String,
        order: Order,
    },
}

impl OrderCommand {
    fn cl_ord_id(&self) -> &str {
        match self {
            OrderCommand::Submit(o) | OrderCommand::Replace { order: o, .. } => &o.cl_ord_id,
            OrderCommand::Cancel { cl_ord_id, .. } => cl_ord_id,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("message store: {0}")]
    Store(#[from] std::io::Error),
    #[error("tls: {0}")]
    Tls(#[from] crate::tls::TlsError),
}

pub struct GatewayHandle {
    events: broadcast::Sender<GatewayEvent>,
    status: Arc<std::sync::RwLock<Vec<SessionStatus>>>,
    orders: mpsc::Sender<OrderCommand>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl GatewayHandle {
    /// New receiver for normalized events (slow receivers lag, see tokio broadcast).
    pub fn subscribe(&self) -> broadcast::Receiver<GatewayEvent> {
        self.events.subscribe()
    }

    /// Current state of the MD and trading sessions (for admin views).
    pub fn status(&self) -> Vec<SessionStatus> {
        self.status.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Shared, cheap-to-clone view of [`GatewayHandle::status`].
    pub fn status_source(&self) -> Arc<std::sync::RwLock<Vec<SessionStatus>>> {
        self.status.clone()
    }

    /// Sender for order commands.
    pub fn orders(&self) -> mpsc::Sender<OrderCommand> {
        self.orders.clone()
    }

    /// Logs out both sessions and waits for the supervisors to stop.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        for t in self.tasks {
            let _ = t.await;
        }
    }
}

/// Snapshot of one FIX session as seen by the gateway.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub kind: SessionKind,
    /// `GatewayConfig.lp` of the owning gateway (empty in older status files).
    #[serde(default)]
    pub lp: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub logged_on: bool,
    /// Unix ms of the last up/down transition (0 = never connected).
    pub since_ms: u64,
    pub last_down_reason: Option<String>,
    /// Session-level Rejects received since start.
    pub rejects: u64,
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Folds one gateway event into the status table.
pub fn apply_status(table: &mut [SessionStatus], ev: &GatewayEvent) {
    let (kind, up, reason, reject) = match ev {
        GatewayEvent::SessionUp { session } => (*session, Some(true), None, false),
        GatewayEvent::SessionDown { session, reason } => {
            (*session, Some(false), Some(reason.clone()), false)
        }
        GatewayEvent::SessionReject { session, .. } => (*session, None, None, true),
        _ => return,
    };
    let Some(r) = table.iter_mut().find(|r| r.kind == kind) else {
        return;
    };
    if let Some(up) = up {
        r.logged_on = up;
        r.since_ms = unix_ms();
        if reason.is_some() {
            r.last_down_reason = reason;
        }
    }
    if reject {
        r.rejects += 1;
    }
}

type Store = Box<dyn MessageStore>;

fn store_for(cfg: &GatewayConfig, name: &str) -> Result<Store, GatewayError> {
    Ok(match &cfg.store_dir {
        Some(dir) => Box::new(FileStore::open(dir.join(name))?),
        None => Box::new(MemoryStore::new()),
    })
}

fn session_for(cfg: &GatewayConfig, ep: &SessionEndpoint, store: Store) -> Session<Store> {
    let mut c = SessionConfig::new(Role::Initiator, &ep.sender_comp_id, &ep.target_comp_id);
    c.heartbeat_interval = Duration::from_secs(cfg.heartbeat_secs.max(1));
    c.reset_on_logon = ep.reset_on_logon;
    c.username = ep.username.clone();
    c.password = ep.password.clone();
    Session::new(c, store, std::time::Instant::now())
}

/// Starts MD and trading supervisors. Event channel capacity: 4096.
pub fn start(cfg: GatewayConfig) -> Result<GatewayHandle, GatewayError> {
    let (events, _) = broadcast::channel(4096);
    let (orders, orders_rx) = mpsc::channel(1024);
    let (shutdown, sd) = watch::channel(false);
    let tls_for = |ep: &SessionEndpoint| -> Result<Option<crate::tls::Tls>, GatewayError> {
        Ok(match &ep.tls {
            Some(t) => Some(crate::tls::build(t, &ep.addr)?),
            None => None,
        })
    };
    let md_tls = tls_for(&cfg.md)?;
    let trade_tls = tls_for(&cfg.trade)?;
    let md_session = session_for(&cfg, &cfg.md, store_for(&cfg, "md")?);
    let trade_session = session_for(&cfg, &cfg.trade, store_for(&cfg, "trade")?);
    let status = Arc::new(std::sync::RwLock::new(
        [
            (SessionKind::MarketData, &cfg.md),
            (SessionKind::Trading, &cfg.trade),
        ]
        .into_iter()
        .map(|(kind, ep)| SessionStatus {
            kind,
            lp: cfg.lp.clone(),
            sender_comp_id: ep.sender_comp_id.clone(),
            target_comp_id: ep.target_comp_id.clone(),
            logged_on: false,
            since_ms: 0,
            last_down_reason: None,
            rejects: 0,
        })
        .collect::<Vec<_>>(),
    ));
    let cfg = Arc::new(cfg);
    let mut status_rx = events.subscribe();
    let mut status_sd = sd.clone();
    let status_w = status.clone();
    let tracker = tokio::spawn(async move {
        loop {
            tokio::select! {
                ev = status_rx.recv() => match ev {
                    Ok(ev) => {
                        if let Ok(mut t) = status_w.write() {
                            apply_status(&mut t, &ev);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                },
                _ = status_sd.changed() => break,
            }
        }
    });
    let tasks = vec![
        tracker,
        tokio::spawn(md_task(
            cfg.clone(),
            md_tls,
            md_session,
            events.clone(),
            sd.clone(),
        )),
        tokio::spawn(trade_task(
            cfg,
            trade_tls,
            trade_session,
            events.clone(),
            orders_rx,
            sd,
        )),
    ];
    Ok(GatewayHandle {
        events,
        status,
        orders,
        shutdown,
        tasks,
    })
}

/// Byte stream to the LP: plain TCP or TLS.
trait Io: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Io for T {}

async fn open(addr: &str, tls: Option<&crate::tls::Tls>) -> std::io::Result<Box<dyn Io>> {
    Ok(match tls {
        Some(t) => {
            Box::new(fix_session::tls::connect_tls(addr, &t.server_name, t.config.clone()).await?)
        }
        None => {
            let s = TcpStream::connect(addr).await?;
            s.set_nodelay(true)?;
            Box::new(s)
        }
    })
}

/// Connects (TCP, then TLS when configured) or returns `None` on shutdown.
async fn connect(
    addr: &str,
    tls: Option<&crate::tls::Tls>,
    sd: &mut watch::Receiver<bool>,
) -> Option<std::io::Result<Box<dyn Io>>> {
    tokio::select! {
        r = open(addr, tls) => Some(r),
        _ = sd.changed() => None,
    }
}

/// Reconnect pacing and the logon-failure circuit breaker: a connection that
/// ends before Logon succeeds counts as a failed logon (wrong credentials lock
/// LP accounts), and after `max_logon_failures` in a row the session stops
/// until the config is saved again. Delays double up to 60 s.
#[derive(Default)]
struct Retry {
    failures: u32,
    logon_failures: u32,
}

impl Retry {
    fn ok(&mut self) {
        *self = Retry::default();
    }
    /// Records a connection that ended; true when the session must halt.
    fn ended(&mut self, logged_on: bool, max: u32) -> bool {
        if logged_on {
            self.ok();
            return false;
        }
        self.failures += 1;
        self.logon_failures += 1;
        max > 0 && self.logon_failures >= max
    }
    fn connect_failed(&mut self) {
        self.failures += 1;
    }
    fn delay(&self, base_ms: u64) -> Duration {
        let ms = base_ms.saturating_mul(1u64 << self.failures.min(6));
        Duration::from_millis(ms.min(60_000))
    }
}

/// Sleeps; returns false on shutdown.
async fn backoff(cfg: &GatewayConfig, retry: &Retry, sd: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(retry.delay(cfg.reconnect_delay_ms)) => !*sd.borrow(),
        _ = sd.changed() => false,
    }
}

fn halted(events: &broadcast::Sender<GatewayEvent>, session: SessionKind, n: u32) {
    down(
        events,
        session,
        format!(
            "halted after {n} failed logons in a row (check username/password); \
             no further attempts until the LP settings are saved again"
        ),
    );
}

async fn graceful_logout(
    cmd: &mpsc::Sender<SessionCommand>,
    ev: &mut mpsc::Receiver<SessionEvent>,
) {
    let _ = cmd
        .send(SessionCommand::Logout(Some("gateway shutdown".into())))
        .await;
    let _ = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(e) = ev.recv().await {
            if matches!(e, SessionEvent::Disconnected(_)) {
                break;
            }
        }
    })
    .await;
}

fn down(events: &broadcast::Sender<GatewayEvent>, session: SessionKind, reason: String) {
    warn!(?session, %reason, "session down");
    let _ = events.send(GatewayEvent::SessionDown { session, reason });
}

async fn md_task(
    cfg: Arc<GatewayConfig>,
    tls: Option<crate::tls::Tls>,
    session: Session<Store>,
    events: broadcast::Sender<GatewayEvent>,
    mut sd: watch::Receiver<bool>,
) {
    let kind = SessionKind::MarketData;
    let mut session = Some(session);
    let mut req_counter = 0u64;
    let mut retry = Retry::default();
    while let Some(s) = session.take() {
        let io = match connect(&cfg.md.addr, tls.as_ref(), &mut sd).await {
            None => break,
            Some(Ok(io)) => io,
            Some(Err(e)) => {
                down(&events, kind, format!("connect {}: {e}", cfg.md.addr));
                session = Some(s);
                retry.connect_failed();
                if !backoff(&cfg, &retry, &mut sd).await {
                    break;
                }
                continue;
            }
        };
        let (cmd, cmd_rx) = mpsc::channel(64);
        let (ev_tx, mut ev) = mpsc::channel(4096);
        let handle = tokio::spawn(run_session(io, s, cmd_rx, ev_tx));
        let mut books = Books::default();
        let mut stop = false;
        let mut up = false;
        loop {
            tokio::select! {
                e = ev.recv() => match e {
                    Some(SessionEvent::LoggedOn) => {
                        info!("MD session logged on, subscribing");
                        up = true;
                        let _ = events.send(GatewayEvent::SessionUp { session: kind });
                        req_counter += 1;
                        let req = Body::MarketDataRequest(MarketDataRequest {
                            md_req_id: format!("md-{req_counter}"),
                            subscription_type: SubscriptionRequestType::Subscribe,
                            market_depth: cfg.market_depth,
                            md_update_type: Some(0),
                            entry_types: vec![MdEntryType::Bid, MdEntryType::Offer],
                            instruments: cfg
                                .instruments
                                .iter()
                                .filter_map(|i| normalize::instrument_ref(&cfg, &i.symbol))
                                .collect(),
                        });
                        let _ = cmd.send(SessionCommand::Send(req)).await;
                    }
                    Some(SessionEvent::App(m)) => match &m.body {
                        Body::MarketDataSnapshot(w) => {
                            if let Some(q) = books.snapshot(&cfg, w) {
                                let _ = events.send(GatewayEvent::Quote(q));
                            }
                        }
                        Body::MarketDataIncremental(x) => {
                            for q in books.incremental(&cfg, x) {
                                let _ = events.send(GatewayEvent::Quote(q));
                            }
                        }
                        other => warn!(msg_type = other.msg_type(), "unexpected message on MD session"),
                    },
                    Some(SessionEvent::PeerReject(r)) => {
                        let _ = events.send(GatewayEvent::SessionReject { session: kind, ref_seq_num: r.ref_seq_num, text: r.text });
                    }
                    Some(SessionEvent::Disconnected(r)) => {
                        down(&events, kind, r);
                        break;
                    }
                    None => break,
                },
                _ = sd.changed() => {
                    graceful_logout(&cmd, &mut ev).await;
                    stop = true;
                    break;
                }
            }
        }
        drop(cmd);
        session = handle.await.ok();
        if stop {
            break;
        }
        if retry.ended(up, cfg.max_logon_failures) {
            halted(&events, kind, retry.logon_failures);
            break;
        }
        if !backoff(&cfg, &retry, &mut sd).await {
            break;
        }
    }
}

fn to_body(cfg: &GatewayConfig, c: &OrderCommand) -> Result<Body, String> {
    let unknown = |s: &str| format!("unknown symbol {s}");
    Ok(match c {
        OrderCommand::Submit(o) => Body::NewOrderSingle(
            normalize::new_order_single(cfg, o).ok_or_else(|| unknown(&o.symbol))?,
        ),
        OrderCommand::Cancel {
            cl_ord_id,
            orig_cl_ord_id,
            symbol,
            side,
        } => Body::OrderCancelRequest(OrderCancelRequest {
            orig_cl_ord_id: orig_cl_ord_id.clone(),
            cl_ord_id: cl_ord_id.clone(),
            instrument: normalize::instrument_ref(cfg, symbol).ok_or_else(|| unknown(symbol))?,
            side: *side,
            transact_time: now_timestamp(),
            order_qty: None,
        }),
        OrderCommand::Replace {
            orig_cl_ord_id,
            order,
        } => Body::OrderCancelReplaceRequest(OrderCancelReplaceRequest {
            orig_cl_ord_id: orig_cl_ord_id.clone(),
            cl_ord_id: order.cl_ord_id.clone(),
            instrument: normalize::instrument_ref(cfg, &order.symbol)
                .ok_or_else(|| unknown(&order.symbol))?,
            side: order.side,
            transact_time: now_timestamp(),
            order_qty: order.qty,
            ord_type: order.ord_type,
            price: order.limit_price,
            time_in_force: Some(order.tif),
        }),
    })
}

fn reject_cmd(events: &broadcast::Sender<GatewayEvent>, c: &OrderCommand, reason: String) {
    warn!(cl_ord_id = c.cl_ord_id(), %reason, "order command rejected by gateway");
    let _ = events.send(GatewayEvent::CommandRejected {
        cl_ord_id: c.cl_ord_id().to_owned(),
        reason,
    });
}

async fn trade_task(
    cfg: Arc<GatewayConfig>,
    tls: Option<crate::tls::Tls>,
    session: Session<Store>,
    events: broadcast::Sender<GatewayEvent>,
    mut orders: mpsc::Receiver<OrderCommand>,
    mut sd: watch::Receiver<bool>,
) {
    let kind = SessionKind::Trading;
    let mut session = Some(session);
    let mut orders_open = true;
    let mut retry = Retry::default();
    while let Some(s) = session.take() {
        let io = match connect(&cfg.trade.addr, tls.as_ref(), &mut sd).await {
            None => break,
            Some(Ok(io)) => io,
            Some(Err(e)) => {
                down(&events, kind, format!("connect {}: {e}", cfg.trade.addr));
                session = Some(s);
                // Fail fast instead of queueing orders while disconnected.
                while let Ok(c) = orders.try_recv() {
                    reject_cmd(&events, &c, "trading session down".into());
                }
                retry.connect_failed();
                if !backoff(&cfg, &retry, &mut sd).await {
                    break;
                }
                continue;
            }
        };
        let (cmd, cmd_rx) = mpsc::channel(1024);
        let (ev_tx, mut ev) = mpsc::channel(4096);
        let handle = tokio::spawn(run_session(io, s, cmd_rx, ev_tx));
        let mut up = false;
        let mut stop = false;
        loop {
            tokio::select! {
                e = ev.recv() => match e {
                    Some(SessionEvent::LoggedOn) => {
                        info!("trading session logged on");
                        up = true;
                        let _ = events.send(GatewayEvent::SessionUp { session: kind });
                    }
                    Some(SessionEvent::App(m)) => match &m.body {
                        Body::ExecutionReport(er) => {
                            let _ = events.send(GatewayEvent::Execution(normalize::execution(&cfg, er)));
                        }
                        Body::OrderCancelReject(r) => {
                            let _ = events.send(GatewayEvent::CancelRejected {
                                cl_ord_id: r.cl_ord_id.clone(),
                                orig_cl_ord_id: r.orig_cl_ord_id.clone(),
                                text: r.text.clone(),
                            });
                        }
                        other => warn!(msg_type = other.msg_type(), "unexpected message on trading session"),
                    },
                    Some(SessionEvent::PeerReject(r)) => {
                        let _ = events.send(GatewayEvent::SessionReject { session: kind, ref_seq_num: r.ref_seq_num, text: r.text });
                    }
                    Some(SessionEvent::Disconnected(r)) => {
                        down(&events, kind, r);
                        break;
                    }
                    None => break,
                },
                c = orders.recv(), if orders_open => match c {
                    None => orders_open = false,
                    Some(c) if !up => reject_cmd(&events, &c, "trading session not logged on".into()),
                    Some(c) => match to_body(&cfg, &c) {
                        Ok(body) => {
                            if cmd.send(SessionCommand::Send(body)).await.is_err() {
                                reject_cmd(&events, &c, "trading session closed".into());
                            }
                        }
                        Err(reason) => reject_cmd(&events, &c, reason),
                    },
                },
                _ = sd.changed() => {
                    graceful_logout(&cmd, &mut ev).await;
                    stop = true;
                    break;
                }
            }
        }
        drop(cmd);
        session = handle.await.ok();
        if stop {
            break;
        }
        if retry.ended(up, cfg.max_logon_failures) {
            halted(&events, kind, retry.logon_failures);
            break;
        }
        if !backoff(&cfg, &retry, &mut sd).await {
            break;
        }
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    fn table() -> Vec<SessionStatus> {
        [SessionKind::MarketData, SessionKind::Trading]
            .into_iter()
            .map(|kind| SessionStatus {
                kind,
                lp: "T".into(),
                sender_comp_id: "S".into(),
                target_comp_id: "T".into(),
                logged_on: false,
                since_ms: 0,
                last_down_reason: None,
                rejects: 0,
            })
            .collect()
    }

    #[test]
    fn tracks_up_down_and_rejects_per_session() {
        let mut t = table();
        apply_status(
            &mut t,
            &GatewayEvent::SessionUp {
                session: SessionKind::Trading,
            },
        );
        assert!(t[1].logged_on && !t[0].logged_on);
        assert!(t[1].since_ms > 0);
        apply_status(
            &mut t,
            &GatewayEvent::SessionReject {
                session: SessionKind::Trading,
                ref_seq_num: 7,
                text: None,
            },
        );
        apply_status(
            &mut t,
            &GatewayEvent::SessionDown {
                session: SessionKind::Trading,
                reason: "eof".into(),
            },
        );
        assert!(!t[1].logged_on);
        assert_eq!(t[1].rejects, 1);
        assert_eq!(t[1].last_down_reason.as_deref(), Some("eof"));
        assert_eq!(t[0].rejects, 0);
    }
}

#[cfg(test)]
mod retry_tests {
    use super::Retry;
    use std::time::Duration;

    #[test]
    fn halts_after_failed_logons_and_backs_off() {
        let mut r = Retry::default();
        assert!(!r.ended(false, 3));
        assert!(!r.ended(false, 3));
        assert!(r.ended(false, 3), "third failed logon halts");
        // Delays double, capped at 60 s.
        assert_eq!(r.delay(1000), Duration::from_secs(8));
        r.failures = 20;
        assert_eq!(r.delay(1000), Duration::from_secs(60));
        // A successful logon resets everything; connect errors never halt.
        assert!(!r.ended(true, 3));
        assert_eq!((r.failures, r.logon_failures), (0, 0));
        for _ in 0..10 {
            r.connect_failed();
        }
        assert_eq!(r.logon_failures, 0);
        assert!(!Retry::default().ended(false, 0), "0 = never halt");
    }
}
