//! A-book routing over fix-gateway.
//!
//! [`FixLpRouter`] adapts the oms [`LpRouter`] to fix-gateway's order command
//! channel (lots -> units, core symbol -> LP symbol, `domain::Fixed` at the
//! FIX edge). [`run_bridge`] consumes fix-gateway events and feeds them to
//! the engine as journaled commands: quotes become `Command::Quote`,
//! executions `Command::LpFill` / `Command::LpReject`. Because every LP
//! execution is journaled, replaying the journal reproduces the same state
//! without talking to the LP.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use domain::{ExecType, Fixed, Order, OrderType, TimeInForce};
use fix_gateway::{GatewayEvent, OrderCommand};
use money::{Price, Qty};
use oms::{Command, LpOrderRequest, LpRouter};
use tokio::sync::{broadcast, mpsc};

use crate::api::{CoreEvent, GroupDepth};

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
        let order = Order {
            cl_ord_id: format!("{}{}", self.prefix, req.lp_order_id),
            symbol: lp_symbol.to_string(),
            side: req.side.into(),
            qty: lots_to_units(req.volume, cs),
            ord_type: if req.limit.is_some() {
                OrderType::Limit
            } else {
                OrderType::Market
            },
            limit_price: req.limit.map(Fixed::from),
            tif: if req.all_or_none {
                TimeInForce::FillOrKill
            } else {
                TimeInForce::ImmediateOrCancel
            },
        };
        if let Err(e) = self.orders.try_send(OrderCommand::Submit(order)) {
            self.reject(req.lp_order_id, format!("LP gateway unavailable: {e}"));
        }
    }
}

/// Turns one fix-gateway event into an engine command (if relevant).
pub fn to_command(ev: &GatewayEvent, symbols: &SymbolMap, prefix: &str) -> Option<Command> {
    let lp_id = |cl: &str| cl.strip_prefix(prefix).and_then(|s| s.parse::<u64>().ok());
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

    /// One `GroupDepth` per group for a raw LP book (empty books are skipped).
    /// LP sizes arrive in LP contracts; they go out in lots (`contracts_per_lot`).
    fn depths(
        &self,
        symbol: &str,
        contracts_per_lot: i64,
        q: &domain::Quote,
        ts_ns: u64,
    ) -> Vec<GroupDepth> {
        if q.bids.is_empty() && q.asks.is_empty() {
            return Vec::new();
        }
        let point = self.points.get(symbol).copied().unwrap_or(0);
        self.groups
            .iter()
            .map(|g| {
                let m_bid = point * g.markup_points_for(symbol, risk::Side::Sell);
                let m_ask = point * g.markup_points_for(symbol, risk::Side::Buy);
                let level = |l: &domain::Level, m: i64| {
                    (
                        Fixed::from_raw(l.price.raw() + m),
                        Fixed::from_raw(units_to_lots(l.qty, contracts_per_lot).raw()),
                    )
                };
                GroupDepth {
                    group: g.name.clone(),
                    symbol: symbol.to_string(),
                    bids: q.bids.iter().map(|l| level(l, -m_bid)).collect(),
                    asks: q.asks.iter().map(|l| level(l, m_ask)).collect(),
                    ts_ns,
                }
            })
            .collect()
    }
}

pub async fn run_bridge(
    engine: EngineHandle,
    mut rx: broadcast::Receiver<GatewayEvent>,
    symbols: Arc<SymbolMap>,
    prefix: String,
    events: broadcast::Sender<Arc<CoreEvent>>,
) {
    let mut markups = Markups::default();
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(skipped = n, "core bridge lagged behind fix-gateway");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        if let Some(cmd) = to_command(&ev, &symbols, &prefix) {
            if engine.command(cmd).await.is_err() {
                break; // engine stopped
            }
            if let GatewayEvent::Quote(q) = &ev {
                if let Some((sym, cs)) = symbols.from_lp(&q.symbol) {
                    markups.refresh(&engine).await;
                    let depths = markups.depths(sym, cs, q, domain::now_ns());
                    let levels = q.bids.len().max(q.asks.len());
                    if markups.logged.get(sym) != Some(&levels) {
                        markups.logged.insert(sym.to_string(), levels);
                        tracing::info!(
                            symbol = sym,
                            bids = q.bids.len(),
                            asks = q.asks.len(),
                            top_bid_lots = ?units_to_lots(q.bids.first().map_or(Fixed::ZERO, |l| l.qty), cs),
                            groups = depths.len(),
                            "lp book levels"
                        );
                    }
                    for d in depths {
                        let _ = events.send(Arc::new(CoreEvent::Depth(d)));
                    }
                }
            }
        } else if let GatewayEvent::SessionUp { .. } | GatewayEvent::SessionDown { .. } = ev {
            tracing::info!(event = ?ev, "fix-gateway session event");
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
        });
        assert!(matches!(
            fb.lock().unwrap().pop_front(),
            Some(Command::LpReject { lp_order_id: 8, .. })
        ));
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
