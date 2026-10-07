//! NATS transport (feature `nats`): runs the FIX gateway in its own process
//! and lets the trading core talk to it over NATS, so a core restart (deploy)
//! never drops the LP sessions.
//!
//! Subjects for LP `<lp>` (prefix `fx.lp.<lp>`):
//! * `<prefix>.quotes`  — core NATS, every [`GatewayEvent::Quote`] (lossy is fine:
//!   the next quote supersedes it);
//! * `<prefix>.events`  — JetStream stream `FXLP` (10 min retention), every other
//!   event (executions, rejects, session up/down). A core that (re)starts replays
//!   the last [`REPLAY`] of them, so nothing is lost across a hand-over; the engine
//!   de-duplicates executions by exec id;
//! * `<prefix>.status`  — core NATS, the session table once a second;
//! * `<prefix>.orders`  — request/reply: JSON [`OrderCommand`] in, `ok` or an
//!   error text back. No responder / timeout → the core turns it into a
//!   `CommandRejected` event (the order is rejected, never silently lost).
//!
//! Payload: JSON (`Fixed` values as decimal strings).

use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_nats::jetstream;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{GatewayConfig, NatsConfig};
use crate::gateway::{GatewayEvent, OrderCommand, SessionKind, SessionStatus};

/// JetStream stream holding the non-quote events of every LP.
pub const STREAM: &str = "FXLP";
/// How far back a (re)starting core replays LP events.
pub const REPLAY: Duration = Duration::from_secs(120);
/// Order request timeout before the core rejects the order.
pub const ORDER_TIMEOUT: Duration = Duration::from_secs(3);
/// A session table older than this is reported as "gateway unreachable".
pub const STATUS_STALE: Duration = Duration::from_secs(5);
/// Interval of the last-quote re-publication (see `serve_gateway`).
pub const REPUBLISH: Duration = Duration::from_secs(5);

pub fn prefix(lp: &str) -> String {
    let token: String = lp
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    format!("fx.lp.{token}")
}

async fn ensure_stream(js: &jetstream::Context) -> Result<(), String> {
    js.get_or_create_stream(jetstream::stream::Config {
        name: STREAM.into(),
        subjects: vec!["fx.lp.*.events".into()],
        max_age: Duration::from_secs(600),
        storage: jetstream::stream::StorageType::File,
        ..Default::default()
    })
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Gateway side: publishes the gateway's events and status and serves order
/// requests until `events` closes. `orders` is the gateway's command channel.
pub async fn serve_gateway(
    url: &str,
    lp: &str,
    mut events: broadcast::Receiver<GatewayEvent>,
    orders: mpsc::Sender<OrderCommand>,
    status: Arc<RwLock<Vec<SessionStatus>>>,
) -> Result<(), String> {
    let client = async_nats::connect(url).await.map_err(|e| e.to_string())?;
    let js = jetstream::new(client.clone());
    ensure_stream(&js).await?;
    let pre = prefix(lp);
    info!(lp, prefix = %pre, "NATS gateway bridge up");

    // order requests
    let mut sub = client
        .subscribe(format!("{pre}.orders"))
        .await
        .map_err(|e| e.to_string())?;
    let req_client = client.clone();
    let order_task: JoinHandle<()> = tokio::spawn(async move {
        use futures_util::StreamExt;
        while let Some(msg) = sub.next().await {
            let reply = match serde_json::from_slice::<OrderCommand>(&msg.payload) {
                Ok(cmd) => match orders.send(cmd).await {
                    Ok(()) => "ok".to_string(),
                    Err(_) => "gateway stopping".to_string(),
                },
                Err(e) => format!("bad order command: {e}"),
            };
            if let Some(to) = msg.reply {
                let _ = req_client.publish(to, reply.into()).await;
            }
        }
    });

    // status heartbeat
    let st_client = client.clone();
    let st_subject = format!("{pre}.status");
    let status_task: JoinHandle<()> = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let rows = status.read().map(|t| t.clone()).unwrap_or_default();
            if let Ok(p) = serde_json::to_vec(&rows) {
                let _ = st_client.publish(st_subject.clone(), p.into()).await;
            }
        }
    });

    let quotes = format!("{pre}.quotes");
    let evs = format!("{pre}.events");
    // Last quote per symbol, re-published every REPUBLISH: a core that just
    // took over (blue/green) or a symbol that rarely ticks (EUR/ILS) still
    // gets its current price within seconds instead of at the next tick.
    let mut last: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();
    let mut republish = tokio::time::interval(REPUBLISH);
    republish.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        let next = tokio::select! {
            ev = events.recv() => ev,
            _ = republish.tick() => {
                for p in last.values() {
                    if let Err(e) = client.publish(quotes.clone(), p.clone().into()).await {
                        warn!(error = %e, "NATS quote re-publish failed");
                        break;
                    }
                }
                continue;
            }
        };
        match next {
            Ok(ev) => {
                let Ok(payload) = serde_json::to_vec(&ev) else {
                    continue;
                };
                if matches!(ev, GatewayEvent::SessionStats { .. }) {
                    continue; // carried by the status heartbeat
                }
                if let GatewayEvent::Quote(q) = &ev {
                    last.insert(q.symbol.clone(), payload.clone());
                    if let Err(e) = client.publish(quotes.clone(), payload.into()).await {
                        warn!(error = %e, "NATS quote publish failed");
                    }
                } else {
                    // await the JetStream ack: executions must be stored
                    match js.publish(evs.clone(), payload.into()).await {
                        Ok(ack) => {
                            if let Err(e) = ack.await {
                                warn!(error = %e, "JetStream did not store an LP event");
                            }
                        }
                        Err(e) => warn!(error = %e, "JetStream publish failed"),
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => warn!(n, "NATS bridge lagged"),
            Err(broadcast::error::RecvError::Closed) => break Ok(()),
        }
    };
    order_task.abort();
    status_task.abort();
    result
}

/// Core side of one remote LP: same surface as an in-process gateway
/// (event broadcast, order channel, session table).
pub struct RemoteGateway {
    pub lp: String,
    events: broadcast::Sender<GatewayEvent>,
    orders: mpsc::Sender<OrderCommand>,
    status: Arc<RwLock<Vec<SessionStatus>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl RemoteGateway {
    /// Connects to the gateway of `cfg.lp` over NATS. Events of the last
    /// [`REPLAY`] are delivered first (hand-over safety).
    pub async fn connect(url: &str, cfg: &GatewayConfig) -> Result<RemoteGateway, String> {
        let client = async_nats::connect(url).await.map_err(|e| e.to_string())?;
        let js = jetstream::new(client.clone());
        ensure_stream(&js).await?;
        let pre = prefix(&cfg.lp);
        let (events, _) = broadcast::channel(65_536);
        let (orders, mut orders_rx) = mpsc::channel::<OrderCommand>(1024);
        let status = Arc::new(RwLock::new(
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
                last_down_reason: Some("waiting for the gateway".into()),
                rejects: 0,
                in_seq: 0,
                last_msg_ms: 0,
            })
            .collect::<Vec<_>>(),
        ));
        let mut tasks = Vec::new();

        // quotes
        let mut qsub = client
            .subscribe(format!("{pre}.quotes"))
            .await
            .map_err(|e| e.to_string())?;
        let tx = events.clone();
        tasks.push(tokio::spawn(async move {
            use futures_util::StreamExt;
            while let Some(m) = qsub.next().await {
                if let Ok(ev) = serde_json::from_slice::<GatewayEvent>(&m.payload) {
                    let _ = tx.send(ev);
                }
            }
        }));

        // durable events: replay the recent past, then follow
        let start = time::OffsetDateTime::now_utc() - REPLAY;
        let consumer = js
            .create_consumer_on_stream(
                jetstream::consumer::pull::Config {
                    filter_subject: format!("{pre}.events"),
                    deliver_policy: jetstream::consumer::DeliverPolicy::ByStartTime {
                        start_time: start,
                    },
                    ack_policy: jetstream::consumer::AckPolicy::None,
                    inactive_threshold: Duration::from_secs(60),
                    ..Default::default()
                },
                STREAM,
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut stream = consumer.messages().await.map_err(|e| e.to_string())?;
        let tx = events.clone();
        let lp = cfg.lp.clone();
        tasks.push(tokio::spawn(async move {
            use futures_util::StreamExt;
            while let Some(m) = stream.next().await {
                match m {
                    Ok(m) => {
                        if let Ok(ev) = serde_json::from_slice::<GatewayEvent>(&m.payload) {
                            let _ = tx.send(ev);
                        }
                    }
                    Err(e) => warn!(lp, error = %e, "LP event stream error"),
                }
            }
            warn!(lp, "LP event stream ended");
        }));

        // status
        let mut ssub = client
            .subscribe(format!("{pre}.status"))
            .await
            .map_err(|e| e.to_string())?;
        let table = status.clone();
        let last_seen = Arc::new(std::sync::Mutex::new(None::<std::time::Instant>));
        let seen = last_seen.clone();
        tasks.push(tokio::spawn(async move {
            use futures_util::StreamExt;
            while let Some(m) = ssub.next().await {
                if let Ok(rows) = serde_json::from_slice::<Vec<SessionStatus>>(&m.payload) {
                    if let Ok(mut t) = table.write() {
                        *t = rows;
                    }
                    if let Ok(mut s) = seen.lock() {
                        *s = Some(std::time::Instant::now());
                    }
                }
            }
        }));
        let table = status.clone();
        tasks.push(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                let stale = last_seen
                    .lock()
                    .map(|s| s.is_none_or(|t| t.elapsed() > STATUS_STALE))
                    .unwrap_or(true);
                if stale {
                    if let Ok(mut t) = table.write() {
                        for r in t.iter_mut() {
                            r.logged_on = false;
                            r.last_down_reason = Some("gateway process unreachable".into());
                        }
                    }
                }
            }
        }));

        // orders
        let tx = events.clone();
        let subject = format!("{pre}.orders");
        tasks.push(tokio::spawn(async move {
            while let Some(cmd) = orders_rx.recv().await {
                let reason = match serde_json::to_vec(&cmd) {
                    Err(e) => Some(format!("serialize: {e}")),
                    Ok(p) => match tokio::time::timeout(
                        ORDER_TIMEOUT,
                        client.request(subject.clone(), p.into()),
                    )
                    .await
                    {
                        Err(_) => Some("LP gateway timeout".to_string()),
                        Ok(Err(e)) => Some(format!("LP gateway unavailable: {e}")),
                        Ok(Ok(r)) if r.payload.as_ref() == b"ok" => None,
                        Ok(Ok(r)) => Some(String::from_utf8_lossy(&r.payload).into_owned()),
                    },
                };
                if let Some(reason) = reason {
                    crate::gateway::reject_cmd(&tx, &cmd, reason);
                }
            }
        }));
        info!(lp = %cfg.lp, prefix = %pre, "remote LP gateway attached over NATS");
        Ok(RemoteGateway {
            lp: cfg.lp.clone(),
            events,
            orders,
            status,
            tasks,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GatewayEvent> {
        self.events.subscribe()
    }
    pub fn orders(&self) -> mpsc::Sender<OrderCommand> {
        self.orders.clone()
    }
    pub fn status_source(&self) -> Arc<RwLock<Vec<SessionStatus>>> {
        self.status.clone()
    }
    /// Stops the local tasks; the remote gateway and its LP sessions keep running.
    pub fn shutdown(self) {
        for t in self.tasks {
            t.abort();
        }
    }
}

/// Legacy helper kept for configs with `[nats]`: publishes events under
/// `cfg.subject_prefix` (no orders / status). Superseded by [`serve_gateway`].
pub async fn run_publisher(
    cfg: NatsConfig,
    mut rx: broadcast::Receiver<GatewayEvent>,
) -> Result<(), async_nats::ConnectError> {
    let client = async_nats::connect(&cfg.url).await?;
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let Ok(payload) = serde_json::to_vec(&ev) else {
                    continue;
                };
                let subject = match &ev {
                    GatewayEvent::Quote(_) => format!("{}.quotes", cfg.subject_prefix),
                    _ => format!("{}.events", cfg.subject_prefix),
                };
                if let Err(e) = client.publish(subject, payload.into()).await {
                    warn!(error = %e, "NATS publish failed");
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => warn!(n, "NATS publisher lagged"),
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_is_subject_safe() {
        assert_eq!(prefix("LMAX"), "fx.lp.LMAX");
        assert_eq!(prefix("SIM 2.x"), "fx.lp.SIM2x");
    }

    #[test]
    fn events_round_trip_as_json() {
        let ev = GatewayEvent::CommandRejected {
            cl_ord_id: "LP-1".into(),
            reason: "x".into(),
        };
        let j = serde_json::to_vec(&ev).unwrap();
        assert_eq!(serde_json::from_slice::<GatewayEvent>(&j).unwrap(), ev);
        let st = GatewayEvent::SessionStats {
            session: SessionKind::Trading,
            in_seq: 5,
            last_msg_ms: 9,
        };
        let j = serde_json::to_vec(&st).unwrap();
        assert_eq!(serde_json::from_slice::<GatewayEvent>(&j).unwrap(), st);
    }
}
