//! A-book routing over fix-gateway.
//!
//! [`FixLpRouter`] adapts the oms [`LpRouter`] to fix-gateway's order command
//! channel (lots -> units, core symbol -> LP symbol, `domain::Fixed` at the
//! FIX edge). [`run_bridge`] consumes fix-gateway events and feeds them to
//! the engine as journaled commands: quotes become `Command::Quote`,
//! executions `Command::LpFill` / `Command::LpReject`. Because every LP
//! execution is journaled, replaying the journal reproduces the same state
//! without talking to the LP.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use domain::{ExecType, Fixed, Order, OrderType, TimeInForce};
use fix_gateway::{GatewayEvent, OrderCommand};
use money::{Price, Qty};
use oms::{Command, LpOrderRequest, LpRouter};
use tokio::sync::{broadcast, mpsc};

use crate::api::{CoreEvent, GroupDepth};
use crate::lp_agg::{Aggregator, LpBook};

use crate::{EngineHandle, LpFeedback};

/// Core symbol (`EURUSD`) <-> LP symbol (`EUR/USD`) with the LP `OrderQty` per
/// engine lot (100 000 for an LP quoting units, 10 for LMAX 10 000-unit contracts).
#[derive(Clone, Debug, Default)]
pub struct SymbolMap {
    to_lp: BTreeMap<String, (String, i64)>,
    from_lp: BTreeMap<String, (String, i64)>,
}

impl SymbolMap {
    pub fn insert(&mut self, core: &str, lp: &str, contract_size: i64) {
        self.to_lp
            .insert(core.to_string(), (lp.to_string(), contract_size));
        self.from_lp
            .insert(lp.to_string(), (core.to_string(), contract_size));
    }
    pub fn to_lp(&self, core: &str) -> Option<(&str, i64)> {
        self.to_lp.get(core).map(|(s, c)| (s.as_str(), *c))
    }
    pub fn from_lp(&self, lp: &str) -> Option<(&str, i64)> {
        self.from_lp.get(lp).map(|(s, c)| (s.as_str(), *c))
    }
}

/// Lots -> LP `OrderQty` (`per_lot` = LP quantity of one lot).
pub fn lots_to_units(lots: Qty, contract_size: i64) -> Fixed {
    Fixed::from_raw(lots.raw().saturating_mul(contract_size))
}

/// LP `OrderQty` -> lots (truncating to the engine's 1e-8 lot resolution).
pub fn units_to_lots(units: Fixed, contract_size: i64) -> Qty {
    Qty::from_raw(units.raw() / contract_size.max(1))
}

/// Sends omnibus LP orders to fix-gateway (`cl_ord_id = <prefix><lp_order_id>`).
pub struct FixLpRouter {
    orders: mpsc::Sender<OrderCommand>,
    feedback: LpFeedback,
    symbols: Arc<SymbolMap>,
    prefix: String,
}

impl FixLpRouter {
    pub fn new(
        orders: mpsc::Sender<OrderCommand>,
        feedback: LpFeedback,
        symbols: Arc<SymbolMap>,
        prefix: impl Into<String>,
    ) -> FixLpRouter {
        FixLpRouter {
            orders,
            feedback,
            symbols,
            prefix: prefix.into(),
        }
    }

    fn reject(&self, lp_order_id: u64, reason: String) {
        tracing::warn!(lp_order_id, %reason, "LP order not sent");
        if let Ok(mut fb) = self.feedback.lock() {
            fb.push_back(Command::LpReject {
                lp_order_id,
                reason,
            });
        }
    }
}

impl LpRouter for FixLpRouter {
    fn send(&mut self, req: &LpOrderRequest) {
        let Some((lp_symbol, cs)) = self.symbols.to_lp(&req.symbol) else {
            return self.reject(req.lp_order_id, format!("no LP mapping for {}", req.symbol));
        };
        let order = lp_order(req, lp_symbol, cs, &self.prefix);
        if let Err(e) = self.orders.try_send(OrderCommand::Submit(order)) {
            self.reject(req.lp_order_id, format!("LP gateway unavailable: {e}"));
        }
    }

    fn replace(&mut self, req: &LpOrderRequest) {
        let Some((lp_symbol, cs)) = self.symbols.to_lp(&req.symbol) else {
            return;
        };
        let order = lp_order(req, lp_symbol, cs, &self.prefix);
        let orig = cl_ord_id(
            &self.prefix,
            req.lp_order_id,
            req.revision.saturating_sub(1),
        );
        if let Err(e) = self.orders.try_send(OrderCommand::Replace {
            orig_cl_ord_id: orig,
            order,
        }) {
            tracing::warn!(lp_order_id = req.lp_order_id, error = %e, "LP replace not sent");
        }
    }

    fn cancel(&mut self, req: &LpOrderRequest) {
        let Some((lp_symbol, _)) = self.symbols.to_lp(&req.symbol) else {
            return;
        };
        let cmd = lp_cancel(
            &self.prefix,
            req.lp_order_id,
            req.revision,
            lp_symbol,
            req.side,
        );
        if let Err(e) = self.orders.try_send(cmd) {
            tracing::warn!(lp_order_id = req.lp_order_id, error = %e, "LP cancel not sent");
        }
    }
}

/// ClOrdID of revision `rev` of LP order `id`: `LP-7`, then `LP-7-r1`, ...
/// (a cancel/replace needs a fresh ClOrdID; the chain is deterministic so a
/// restarted process can still address the resting order).
pub fn cl_ord_id(prefix: &str, id: u64, rev: u32) -> String {
    if rev == 0 {
        format!("{prefix}{id}")
    } else {
        format!("{prefix}{id}-r{rev}")
    }
}

/// Our LP order id from any revision's ClOrdID (`LP-7-r2` → 7).
pub fn parse_lp_id(prefix: &str, cl: &str) -> Option<u64> {
    let s = cl.strip_prefix(prefix)?;
    s.split('-').next()?.parse::<u64>().ok()
}

/// Builds the LP-side order of an omnibus request (lots -> LP quantity,
/// core -> LP symbol, IOC or FOK; resting = GTC limit).
fn lp_order(req: &LpOrderRequest, lp_symbol: &str, cs: i64, prefix: &str) -> Order {
    Order {
        cl_ord_id: cl_ord_id(prefix, req.lp_order_id, req.revision),
        symbol: lp_symbol.to_string(),
        side: req.side.into(),
        qty: lots_to_units(req.volume, cs),
        ord_type: if req.stop.is_some() {
            OrderType::Stop
        } else if req.limit.is_some() {
            OrderType::Limit
        } else {
            OrderType::Market
        },
        limit_price: req.limit.map(Fixed::from),
        stop_price: req.stop.map(Fixed::from),
        tif: if req.resting {
            TimeInForce::GoodTillCancel
        } else if req.all_or_none {
            TimeInForce::FillOrKill
        } else {
            TimeInForce::ImmediateOrCancel
        },
    }
}

/// Cancel of the revision currently live at the LP.
fn lp_cancel(prefix: &str, id: u64, rev: u32, lp_symbol: &str, side: risk::Side) -> OrderCommand {
    OrderCommand::Cancel {
        cl_ord_id: format!("{prefix}{id}-c{}", rev + 1),
        orig_cl_ord_id: cl_ord_id(prefix, id, rev),
        symbol: lp_symbol.to_string(),
        side: side.into(),
    }
}

/// One LP's order channel as seen by [`AggLpRouter`].
pub struct LpLink {
    pub name: String,
    pub orders: mpsc::Sender<OrderCommand>,
    pub symbols: Arc<SymbolMap>,
}

/// Multi-LP router (stage 6): asks the [`Aggregator`] for the candidate LPs
/// of an order (policy + mode), sends to the first whose gateway accepts the
/// command and journals the choice as `Command::LpRouted`. Fails over to the
/// next candidate when a gateway channel is closed or full; rejects when
/// nobody is eligible.
pub struct AggLpRouter {
    links: Vec<LpLink>,
    agg: Arc<Aggregator>,
    feedback: LpFeedback,
    prefix: String,
    /// LP of each resting order this process sent (after a restart the
    /// primary order-taking LP of the symbol stands in).
    resting: HashMap<u64, String>,
}

impl AggLpRouter {
    pub fn new(
        links: Vec<LpLink>,
        agg: Arc<Aggregator>,
        feedback: LpFeedback,
        prefix: impl Into<String>,
    ) -> AggLpRouter {
        AggLpRouter {
            links,
            agg,
            feedback,
            prefix: prefix.into(),
            resting: HashMap::new(),
        }
    }

    /// The link a resting order lives on: remembered from the send, else
    /// (after a restart) the primary order-taking LP of the symbol.
    fn resting_link(&self, id: u64, symbol: &str) -> Option<&LpLink> {
        let name = self
            .resting
            .get(&id)
            .cloned()
            .or_else(|| self.agg.resting_lp(symbol))?;
        self.links.iter().find(|l| l.name == name)
    }

    fn push(&self, cmd: Command) {
        if let Ok(mut fb) = self.feedback.lock() {
            fb.push_back(cmd);
        }
    }
}

impl LpRouter for AggLpRouter {
    fn send(&mut self, req: &LpOrderRequest) {
        // deal-moment book (parça 15): what we saw when we decided
        self.agg.snapshot_for(req.lp_order_id, &req.symbol);
        // A resting order goes to the primary order-taking LP of the symbol
        // even before it has quoted (a fresh process): it rests there.
        let cands = if req.resting {
            self.agg.resting_lp(&req.symbol).into_iter().collect()
        } else {
            self.agg.choose(&req.symbol, req.side, req.volume)
        };
        if cands.is_empty() {
            tracing::warn!(lp_order_id = req.lp_order_id, symbol = %req.symbol, "no eligible LP");
            return self.push(Command::LpReject {
                lp_order_id: req.lp_order_id,
                reason: format!("no eligible LP for {}", req.symbol),
            });
        }
        let mut last = String::new();
        for lp in &cands {
            let Some(link) = self.links.iter().find(|l| &l.name == lp) else {
                continue;
            };
            let Some((lp_symbol, cs)) = link.symbols.to_lp(&req.symbol) else {
                last = format!("{lp}: no LP mapping for {}", req.symbol);
                continue;
            };
            let order = lp_order(req, lp_symbol, cs, &self.prefix);
            match link.orders.try_send(OrderCommand::Submit(order)) {
                Ok(()) => {
                    if req.resting {
                        self.resting.insert(req.lp_order_id, lp.clone());
                    }
                    return self.push(Command::LpRouted {
                        lp_order_id: req.lp_order_id,
                        lp: lp.clone(),
                    });
                }
                Err(e) => {
                    tracing::warn!(lp, lp_order_id = req.lp_order_id, error = %e, "LP gateway unavailable, failing over");
                    last = format!("{lp}: LP gateway unavailable: {e}");
                }
            }
        }
        tracing::warn!(lp_order_id = req.lp_order_id, %last, "LP order not sent");
        self.push(Command::LpReject {
            lp_order_id: req.lp_order_id,
            reason: if last.is_empty() {
                "no LP link".into()
            } else {
                last
            },
        });
    }

    fn replace(&mut self, req: &LpOrderRequest) {
        let Some(link) = self.resting_link(req.lp_order_id, &req.symbol) else {
            return tracing::warn!(lp_order_id = req.lp_order_id, "LP replace: no link");
        };
        let Some((lp_symbol, cs)) = link.symbols.to_lp(&req.symbol) else {
            return;
        };
        let order = lp_order(req, lp_symbol, cs, &self.prefix);
        let orig = cl_ord_id(
            &self.prefix,
            req.lp_order_id,
            req.revision.saturating_sub(1),
        );
        if let Err(e) = link.orders.try_send(OrderCommand::Replace {
            orig_cl_ord_id: orig,
            order,
        }) {
            tracing::warn!(lp_order_id = req.lp_order_id, error = %e, "LP replace not sent");
        }
    }

    fn cancel(&mut self, req: &LpOrderRequest) {
        let Some(link) = self.resting_link(req.lp_order_id, &req.symbol) else {
            return tracing::warn!(lp_order_id = req.lp_order_id, "LP cancel: no link");
        };
        let Some((lp_symbol, _)) = link.symbols.to_lp(&req.symbol) else {
            return;
        };
        let cmd = lp_cancel(
            &self.prefix,
            req.lp_order_id,
            req.revision,
            lp_symbol,
            req.side,
        );
        if let Err(e) = link.orders.try_send(cmd) {
            tracing::warn!(lp_order_id = req.lp_order_id, error = %e, "LP cancel not sent");
        }
        self.resting.remove(&req.lp_order_id);
    }
}

/// Turns one fix-gateway event into an engine command (if relevant).
pub fn to_command(ev: &GatewayEvent, symbols: &SymbolMap, prefix: &str) -> Option<Command> {
    let lp_id = |cl: &str| parse_lp_id(prefix, cl);
    match ev {
        GatewayEvent::Quote(q) => {
            let (sym, _) = symbols.from_lp(&q.symbol)?;
            let bid = q.best_bid()?.price;
            let ask = q.best_ask()?.price;
            Some(Command::Quote {
                symbol: sym.to_string(),
                bid: Price::from(bid),
                ask: Price::from(ask),
            })
        }
        GatewayEvent::Execution(x) => {
            let lp_order_id = lp_id(x.cl_ord_id.as_deref()?)?;
            match x.exec_type {
                ExecType::Trade => {
                    let (_, cs) = symbols.from_lp(&x.symbol)?;
                    let (q, px) = (x.last_qty?, x.last_px?);
                    Some(Command::LpFill {
                        lp_order_id,
                        // scoped by our order id: LP exec ids need not be
                        // unique across sessions (the simulator restarts them)
                        exec_id: format!("{lp_order_id}:{}", x.exec_id),
                        volume: units_to_lots(q, cs),
                        price: Price::from(px),
                    })
                }
                ExecType::Rejected | ExecType::Canceled | ExecType::Expired => {
                    Some(Command::LpReject {
                        lp_order_id,
                        reason: x
                            .text
                            .clone()
                            .unwrap_or_else(|| format!("{:?}", x.exec_type)),
                    })
                }
                _ => None,
            }
        }
        GatewayEvent::CommandRejected { cl_ord_id, reason } => Some(Command::LpReject {
            lp_order_id: lp_id(cl_ord_id)?,
            reason: reason.clone(),
        }),
        _ => None,
    }
}

/// Feeds fix-gateway quotes and executions into the engine until the event
/// channel closes or the engine stops.
/// Group markups the bridge applies to LP depth, refreshed from the engine
/// now and then (markups change rarely; the top of book itself is journaled
/// through `Command::Quote` and priced by the engine).
#[derive(Default)]
struct Markups {
    /// group configs (markups are per side and per symbol)
    groups: Vec<risk::GroupConfig>,
    /// symbol -> point (raw)
    points: BTreeMap<String, i64>,
    at: Option<Instant>,
    /// Level count last logged per symbol: one line per change (LMAX accounts may
    /// be top-of-book only; this shows what the feed actually delivers).
    logged: BTreeMap<String, usize>,
}

impl Markups {
    const TTL: Duration = Duration::from_secs(5);

    async fn refresh(&mut self, engine: &EngineHandle) {
        if self.at.is_some_and(|t| t.elapsed() < Self::TTL) {
            return;
        }
        if let Ok((groups, points)) = engine
            .read(|e| {
                let g: Vec<risk::GroupConfig> = e.groups().cloned().collect();
                let p: BTreeMap<String, i64> = e
                    .symbols()
                    .map(|s| (s.symbol.clone(), s.point().raw()))
                    .collect();
                (g, p)
            })
            .await
        {
            self.groups = groups;
            self.points = points;
        }
        self.at = Some(Instant::now());
    }

    /// One `GroupDepth` per group for an aggregated book (prices in core
    /// units, sizes in lots; empty books are skipped).
    fn depths(&self, symbol: &str, book: &LpBook, ts_ns: u64) -> Vec<GroupDepth> {
        if book.bids.is_empty() && book.asks.is_empty() {
            return Vec::new();
        }
        let point = self.points.get(symbol).copied().unwrap_or(0);
        self.groups
            .iter()
            .map(|g| {
                let m_bid = point * g.markup_points_for(symbol, risk::Side::Sell);
                let m_ask = point * g.markup_points_for(symbol, risk::Side::Buy);
                let level = |l: &(Price, Qty), m: i64| {
                    (Fixed::from_raw(l.0.raw() + m), Fixed::from_raw(l.1.raw()))
                };
                GroupDepth {
                    group: g.name.clone(),
                    symbol: symbol.to_string(),
                    bids: book.bids.iter().map(|l| level(l, -m_bid)).collect(),
                    asks: book.asks.iter().map(|l| level(l, m_ask)).collect(),
                    ts_ns,
                }
            })
            .collect()
    }
}

/// Feeds one LP's fix-gateway events into the aggregator and the engine
/// until the event channel closes or the engine stops: quotes update the
/// aggregated book (journaled as `Command::Quote` when the best bid/ask
/// changes) and the merged depth goes to clients; executions become
/// `Command::LpFill` / `Command::LpReject`.
pub async fn run_bridge(
    engine: EngineHandle,
    mut rx: broadcast::Receiver<GatewayEvent>,
    symbols: Arc<SymbolMap>,
    prefix: String,
    events: broadcast::Sender<Arc<CoreEvent>>,
    lp: String,
    agg: Arc<Aggregator>,
    ticks: Option<Arc<crate::ticks::TickStore>>,
) {
    let mut markups = Markups::default();
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(lp, skipped = n, "core bridge lagged behind fix-gateway");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        if let GatewayEvent::Quote(q) = &ev {
            let Some((sym, cs)) = symbols.from_lp(&q.symbol) else {
                continue;
            };
            let lots = |l: &domain::Level| (Price::from(l.price), units_to_lots(l.qty, cs));
            let book = LpBook {
                bids: q.bids.iter().map(lots).collect(),
                asks: q.asks.iter().map(lots).collect(),
                ts_ns: q.ts_recv_ns,
            };
            let top = (
                book.bids.first().map_or(0, |l| l.1.raw()),
                book.asks.first().map_or(0, |l| l.1.raw()),
            );
            if let Some((bid, ask)) = agg.update(&lp, sym, book) {
                if let Some(t) = &ticks {
                    t.record(sym, q.ts_recv_ns, bid.raw(), ask.raw(), top.0, top.1);
                }
                let cmd = Command::Quote {
                    symbol: sym.to_string(),
                    bid,
                    ask,
                };
                if engine.command(cmd).await.is_err() {
                    break; // engine stopped
                }
            }
            let levels = q.bids.len().max(q.asks.len());
            if markups.logged.get(sym) != Some(&levels) {
                markups.logged.insert(sym.to_string(), levels);
                tracing::info!(
                    lp,
                    symbol = sym,
                    bids = q.bids.len(),
                    asks = q.asks.len(),
                    top_bid_lots = ?units_to_lots(q.bids.first().map_or(Fixed::ZERO, |l| l.qty), cs),
                    "lp book levels"
                );
            }
            markups.refresh(&engine).await;
            for d in markups.depths(sym, &agg.merged(sym), domain::now_ns()) {
                let _ = events.send(Arc::new(CoreEvent::Depth(d)));
            }
        } else if let Some(cmd) = to_command(&ev, &symbols, &prefix) {
            if engine.command(cmd).await.is_err() {
                break; // engine stopped
            }
        } else if let GatewayEvent::SessionStats {
            session: fix_gateway::SessionKind::MarketData,
            latency_ms,
            ..
        } = &ev
        {
            agg.set_latency(&lp, *latency_ms);
        } else if let GatewayEvent::SessionUp { .. } | GatewayEvent::SessionDown { .. } = ev {
            tracing::info!(lp, event = ?ev, "fix-gateway session event");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Execution, Level, OrderStatus, Quote, Side};
    use money::{px, qty};

    fn map() -> SymbolMap {
        let mut m = SymbolMap::default();
        m.insert("EURUSD", "EUR/USD", 100_000);
        m
    }

    #[test]
    fn lmax_contracts() {
        // LMAX FX: 1 contract = 10 000 units -> 10 contracts per lot.
        assert_eq!(lots_to_units(qty("1"), 10), Fixed::from_int(10));
        assert_eq!(lots_to_units(qty("0.01"), 10), "0.1".parse().unwrap());
        assert_eq!(units_to_lots("2.5".parse().unwrap(), 10), qty("0.25"));
    }

    #[test]
    fn resting_requests_become_gtc_and_revisions_map_back() {
        assert_eq!(cl_ord_id("LP-", 7, 0), "LP-7");
        assert_eq!(cl_ord_id("LP-", 7, 2), "LP-7-r2");
        assert_eq!(parse_lp_id("LP-", "LP-7"), Some(7));
        assert_eq!(parse_lp_id("LP-", "LP-7-r2"), Some(7));
        assert_eq!(parse_lp_id("LP-", "LP-7-c3"), Some(7));
        assert_eq!(parse_lp_id("LP-", "AUD-7"), None);
        let (tx, mut rx) = mpsc::channel(4);
        let fb = LpFeedback::default();
        let mut r = FixLpRouter::new(tx, fb.clone(), Arc::new(map()), "LP-");
        let req = LpOrderRequest {
            lp_order_id: 11,
            symbol: "EURUSD".into(),
            side: risk::Side::Sell,
            volume: qty("1"),
            limit: Some(px("1.10105")),
            all_or_none: false,
            resting: true,
            revision: 0,
            stop: None,
        };
        r.send(&req);
        match rx.try_recv().unwrap() {
            OrderCommand::Submit(o) => {
                assert_eq!(o.cl_ord_id, "LP-11");
                assert_eq!(o.tif, TimeInForce::GoodTillCancel);
                assert_eq!(o.ord_type, OrderType::Limit);
            }
            c => panic!("{c:?}"),
        }
        let mut rep = req.clone();
        rep.revision = 1;
        rep.limit = Some(px("1.10205"));
        r.replace(&rep);
        match rx.try_recv().unwrap() {
            OrderCommand::Replace {
                orig_cl_ord_id,
                order,
            } => {
                assert_eq!(orig_cl_ord_id, "LP-11");
                assert_eq!(order.cl_ord_id, "LP-11-r1");
                assert_eq!(order.tif, TimeInForce::GoodTillCancel);
            }
            c => panic!("{c:?}"),
        }
        // a stop (SL at the LP) goes out as OrdType 3 with StopPx, GTC
        let mut st = req.clone();
        st.lp_order_id = 12;
        st.limit = None;
        st.stop = Some(px("1.09905"));
        r.send(&st);
        match rx.try_recv().unwrap() {
            OrderCommand::Submit(o) => {
                assert_eq!(o.ord_type, OrderType::Stop);
                assert_eq!(o.stop_price, Some(Fixed::from(px("1.09905"))));
                assert_eq!(o.limit_price, None);
                assert_eq!(o.tif, TimeInForce::GoodTillCancel);
            }
            c => panic!("{c:?}"),
        }
        let mut can = rep.clone();
        can.revision = 1;
        r.cancel(&can);
        match rx.try_recv().unwrap() {
            OrderCommand::Cancel {
                orig_cl_ord_id,
                cl_ord_id,
                ..
            } => {
                assert_eq!(orig_cl_ord_id, "LP-11-r1");
                assert_eq!(cl_ord_id, "LP-11-c2");
            }
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn router_converts_lots_and_symbols() {
        let (tx, mut rx) = mpsc::channel(4);
        let fb = LpFeedback::default();
        let mut r = FixLpRouter::new(tx, fb.clone(), Arc::new(map()), "LP-");
        r.send(&LpOrderRequest {
            lp_order_id: 7,
            symbol: "EURUSD".into(),
            side: risk::Side::Sell,
            volume: qty("0.25"),
            limit: None,
            all_or_none: false,
            resting: false,
            revision: 0,
            stop: None,
        });
        match rx.try_recv().unwrap() {
            OrderCommand::Submit(o) => {
                assert_eq!(o.cl_ord_id, "LP-7");
                assert_eq!(o.symbol, "EUR/USD");
                assert_eq!(o.side, Side::Sell);
                assert_eq!(o.qty, Fixed::from_int(25_000));
            }
            c => panic!("{c:?}"),
        }
        r.send(&LpOrderRequest {
            lp_order_id: 8,
            symbol: "XAUUSD".into(),
            side: risk::Side::Buy,
            volume: qty("1"),
            limit: None,
            all_or_none: false,
            resting: false,
            revision: 0,
            stop: None,
        });
        assert!(matches!(
            fb.lock().unwrap().pop_front(),
            Some(Command::LpReject { lp_order_id: 8, .. })
        ));
    }

    #[test]
    fn agg_router_fails_over_and_journals_lp() {
        let (tx_a, rx_a) = mpsc::channel(4);
        drop(rx_a); // first LP's gateway is gone
        let (tx_b, mut rx_b) = mpsc::channel(4);
        let agg = Arc::new(Aggregator::default());
        let book = |b: &str, a: &str| LpBook {
            bids: vec![(px(b), qty("10"))],
            asks: vec![(px(a), qty("10"))],
            ts_ns: 1,
        };
        agg.update("LMAX", "EURUSD", book("1.1", "1.1001"));
        agg.update("SIM", "EURUSD", book("1.1", "1.1002"));
        let fb = LpFeedback::default();
        let m = Arc::new(map());
        let links = vec![
            LpLink {
                name: "LMAX".into(),
                orders: tx_a,
                symbols: m.clone(),
            },
            LpLink {
                name: "SIM".into(),
                orders: tx_b,
                symbols: m,
            },
        ];
        let mut r = AggLpRouter::new(links, agg, fb.clone(), "LP-");
        r.send(&LpOrderRequest {
            lp_order_id: 9,
            symbol: "EURUSD".into(),
            side: risk::Side::Buy,
            volume: qty("1"),
            limit: None,
            all_or_none: false,
            resting: false,
            revision: 0,
            stop: None,
        });
        assert!(
            matches!(rx_b.try_recv().unwrap(), OrderCommand::Submit(o) if o.cl_ord_id == "LP-9")
        );
        assert_eq!(
            fb.lock().unwrap().pop_front(),
            Some(Command::LpRouted {
                lp_order_id: 9,
                lp: "SIM".into()
            })
        );
    }

    #[test]
    fn executions_and_quotes_become_commands() {
        let m = map();
        let q = GatewayEvent::Quote(Quote {
            lp: "SIM".into(),
            symbol: "EUR/USD".into(),
            bids: vec![Level {
                price: "1.1".parse().unwrap(),
                qty: Fixed::from_int(1),
            }],
            asks: vec![Level {
                price: "1.1002".parse().unwrap(),
                qty: Fixed::from_int(1),
            }],
            ts_recv_ns: 0,
        });
        assert_eq!(
            to_command(&q, &m, "LP-"),
            Some(Command::Quote {
                symbol: "EURUSD".into(),
                bid: px("1.1"),
                ask: px("1.1002")
            })
        );
        let x = Execution {
            lp: "SIM".into(),
            symbol: "EUR/USD".into(),
            order_id: "1".into(),
            cl_ord_id: Some("LP-9".into()),
            orig_cl_ord_id: None,
            exec_id: "E1".into(),
            exec_type: ExecType::Trade,
            status: OrderStatus::Filled,
            side: Side::Buy,
            last_qty: Some(Fixed::from_int(50_000)),
            last_px: Some("1.1002".parse().unwrap()),
            cum_qty: Fixed::from_int(50_000),
            leaves_qty: Fixed::ZERO,
            avg_px: None,
            text: None,
            ts_recv_ns: 0,
        };
        assert_eq!(
            to_command(&GatewayEvent::Execution(x.clone()), &m, "LP-"),
            Some(Command::LpFill {
                lp_order_id: 9,
                exec_id: "9:E1".into(),
                volume: qty("0.5"),
                price: px("1.1002")
            })
        );
        let foreign = Execution {
            cl_ord_id: Some("CG-1".into()),
            ..x
        };
        assert_eq!(
            to_command(&GatewayEvent::Execution(foreign), &m, "LP-"),
            None
        );
    }
}
