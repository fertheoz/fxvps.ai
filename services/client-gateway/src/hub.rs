//! Shared gateway state: quote/account fan-out, order routing, positions, candles.

use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use client_proto::{
    client_symbol, Decimal, ErrorCode, OrderUpdate, Position, PositionUpdate, TimeInForce,
};
use domain::{Execution, Fixed, Order, OrderType, Side};
use fix_gateway::{GatewayEvent, OrderCommand};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use tokio::sync::{broadcast, mpsc};

use crate::auth::Authenticator;
use crate::candles::{mid, CandleStore};
use crate::config::ClientGatewayConfig;
use crate::metrics::Metrics;

/// Top-of-book quote in client symbol form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientQuote {
    pub symbol: Arc<str>,
    pub bid: Option<Fixed>,
    pub ask: Option<Fixed>,
    pub bid_size: Option<Fixed>,
    pub ask_size: Option<Fixed>,
    pub ts_ns: u64,
}

impl ClientQuote {
    pub fn to_proto(&self) -> client_proto::Quote {
        client_proto::Quote {
            symbol: self.symbol.to_string(),
            bid: self.bid.map(Decimal::from),
            ask: self.ask.map(Decimal::from),
            bid_size: self.bid_size.map(Decimal::from),
            ask_size: self.ask_size.map(Decimal::from),
            ts_ns: self.ts_ns,
        }
    }
}

/// Per-account event fanned out to every connection authorized for `account_id`.
#[derive(Clone, Debug)]
pub enum AccountEvent {
    Order(OrderUpdate),
    Position(PositionUpdate),
}

impl AccountEvent {
    pub fn account_id(&self) -> &str {
        match self {
            AccountEvent::Order(o) => &o.account_id,
            AccountEvent::Position(p) => &p.account_id,
        }
    }
}

#[derive(Clone, Debug)]
struct OrderMeta {
    account_id: String,
    /// request_id of the originating PlaceOrder.
    client_request_id: String,
    /// The order as last sent (for modifications).
    order: Order,
}

#[derive(Default)]
struct OrderBook {
    /// cl_ord_id (ours, towards fix-gateway) -> meta.
    by_cl_ord_id: HashMap<String, OrderMeta>,
    /// (account, PlaceOrder request_id) -> current live cl_ord_id.
    live: HashMap<(String, String), String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pos {
    pub net: Fixed,
    pub avg: Fixed,
}

impl Pos {
    /// Applies a signed fill (positive = buy).
    pub fn apply(&mut self, dq: Fixed, px: Fixed) {
        let new_net = self.net + dq;
        let same_dir = self.net.is_zero() || (self.net.raw() > 0) == (dq.raw() > 0);
        if same_dir {
            let a = abs(self.net);
            let b = abs(dq);
            let num = i128::from(self.avg.raw()) * i128::from(a.raw())
                + i128::from(px.raw()) * i128::from(b.raw());
            let den = i128::from(a.raw()) + i128::from(b.raw());
            if den != 0 {
                self.avg = Fixed::from_raw((num / den) as i64);
            }
        } else if new_net.is_zero() {
            self.avg = Fixed::ZERO;
        } else if (new_net.raw() > 0) != (self.net.raw() > 0) {
            // Flipped through zero: remainder opened at the fill price.
            self.avg = px;
        }
        self.net = new_net;
    }
}

fn abs(f: Fixed) -> Fixed {
    if f.raw() < 0 {
        -f
    } else {
        f
    }
}

pub struct Hub {
    pub cfg: ClientGatewayConfig,
    pub auth: Authenticator,
    pub metrics: Metrics,
    quotes: broadcast::Sender<Arc<ClientQuote>>,
    accounts: broadcast::Sender<Arc<AccountEvent>>,
    candles: Mutex<CandleStore>,
    /// Client symbol -> internal (LP) symbol.
    symbols: BTreeMap<String, String>,
    orders_tx: Option<mpsc::Sender<OrderCommand>>,
    orders: Mutex<OrderBook>,
    positions: Mutex<HashMap<String, BTreeMap<String, Pos>>>,
    limiter: DefaultKeyedRateLimiter<String>,
    next_id: AtomicU64,
    id_prefix: String,
}

/// Command validation / routing failure, mapped to a protocol `Error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdError(pub ErrorCode, pub String);

impl CmdError {
    fn new(code: ErrorCode, msg: impl Into<String>) -> Self {
        CmdError(code, msg.into())
    }
}

/// Validated order parameters from a PlaceOrder.
pub struct NewOrder {
    pub request_id: String,
    pub account_id: String,
    pub symbol: String,
    pub side: Side,
    pub ord_type: OrderType,
    pub qty: Fixed,
    pub limit_price: Option<Fixed>,
    pub tif: Option<TimeInForce>,
}

impl Hub {
    /// `symbols`: internal instrument symbols (e.g. `EUR/USD`) tradable through
    /// `orders_tx`; without `orders_tx` order commands answer `UNAVAILABLE`.
    pub fn new(
        cfg: ClientGatewayConfig,
        auth: Authenticator,
        symbols: impl IntoIterator<Item = String>,
        orders_tx: Option<mpsc::Sender<OrderCommand>>,
    ) -> Arc<Self> {
        let (quotes, _) = broadcast::channel(4096);
        let (accounts, _) = broadcast::channel(4096);
        let rate = NonZeroU32::new(cfg.orders_per_second).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new(cfg.order_burst).unwrap_or(NonZeroU32::MIN);
        Arc::new(Hub {
            candles: Mutex::new(CandleStore::new(cfg.candle_capacity)),
            limiter: RateLimiter::keyed(Quota::per_second(rate).allow_burst(burst)),
            symbols: symbols
                .into_iter()
                .map(|s| (client_symbol(&s), s))
                .collect(),
            orders_tx,
            orders: Mutex::default(),
            positions: Mutex::default(),
            quotes,
            accounts,
            metrics: Metrics::default(),
            next_id: AtomicU64::new(1),
            id_prefix: format!("CG{}-", domain::now_ns() / 1_000_000 % 100_000_000),
            auth,
            cfg,
        })
    }

    pub fn subscribe_quotes(&self) -> broadcast::Receiver<Arc<ClientQuote>> {
        self.quotes.subscribe()
    }

    pub fn subscribe_accounts(&self) -> broadcast::Receiver<Arc<AccountEvent>> {
        self.accounts.subscribe()
    }

    pub fn is_known_symbol(&self, client_sym: &str) -> bool {
        self.symbols.contains_key(client_sym)
    }

    /// Ingests a normalized LP quote: candles + fan-out.
    pub fn publish_quote(&self, q: &domain::Quote) {
        let cq = ClientQuote {
            symbol: client_symbol(&q.symbol).into(),
            bid: q.best_bid().map(|l| l.price),
            ask: q.best_ask().map(|l| l.price),
            bid_size: q.best_bid().map(|l| l.qty),
            ask_size: q.best_ask().map(|l| l.qty),
            ts_ns: q.ts_recv_ns,
        };
        if let (Some(b), Some(a)) = (cq.bid, cq.ask) {
            if let Ok(mut c) = self.candles.lock() {
                c.on_price(&cq.symbol, mid(b, a), cq.ts_ns);
            }
        }
        let _ = self.quotes.send(Arc::new(cq));
    }

    /// Ingests a client-form quote directly (tests, alternative feeds).
    pub fn publish_client_quote(&self, q: ClientQuote) {
        if let (Some(b), Some(a)) = (q.bid, q.ask) {
            if let Ok(mut c) = self.candles.lock() {
                c.on_price(&q.symbol, mid(b, a), q.ts_ns);
            }
        }
        let _ = self.quotes.send(Arc::new(q));
    }

    pub fn candles(
        &self,
        symbol: &str,
        tf: client_proto::Timeframe,
        from: u64,
        to: u64,
        limit: usize,
    ) -> Vec<client_proto::Candle> {
        self.candles
            .lock()
            .map(|c| c.query(symbol, tf, from, to, limit))
            .unwrap_or_default()
            .into_iter()
            .map(|b| b.to_proto())
            .collect()
    }

    pub fn positions(&self, account_id: &str) -> Vec<Position> {
        self.positions
            .lock()
            .map(|p| {
                p.get(account_id)
                    .map(|m| {
                        m.iter()
                            .map(|(s, p)| Position {
                                symbol: s.clone(),
                                net_qty: Some(p.net.into()),
                                avg_price: Some(p.avg.into()),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    /// Per-account order rate limit. `true` = allowed.
    pub fn allow_order(&self, account_id: &str) -> bool {
        let ok = self.limiter.check_key(&account_id.to_string()).is_ok();
        if !ok {
            self.metrics.orders_rate_limited.inc();
        }
        ok
    }

    fn new_cl_ord_id(&self) -> String {
        format!(
            "{}{}",
            self.id_prefix,
            self.next_id.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn send_cmd(&self, cmd: OrderCommand) -> Result<(), CmdError> {
        let tx = self
            .orders_tx
            .as_ref()
            .ok_or_else(|| CmdError::new(ErrorCode::Unavailable, "trading unavailable"))?;
        tx.try_send(cmd)
            .map_err(|_| CmdError::new(ErrorCode::Unavailable, "order path busy or down"))
    }

    pub fn place_order(&self, o: NewOrder) -> Result<(), CmdError> {
        let internal = self
            .symbols
            .get(&o.symbol)
            .ok_or_else(|| CmdError::new(ErrorCode::UnknownSymbol, o.symbol.clone()))?;
        if !o.qty.is_positive() {
            return Err(CmdError::new(ErrorCode::BadRequest, "qty must be > 0"));
        }
        if o.ord_type == OrderType::Limit && !o.limit_price.is_some_and(|p| p.is_positive()) {
            return Err(CmdError::new(
                ErrorCode::BadRequest,
                "limit order needs limit_price",
            ));
        }
        let tif = match (o.tif, o.ord_type) {
            (Some(TimeInForce::Day), _) => domain::TimeInForce::Day,
            (Some(TimeInForce::Gtc), _) => domain::TimeInForce::GoodTillCancel,
            (Some(TimeInForce::Fok), _) => domain::TimeInForce::FillOrKill,
            (Some(TimeInForce::Ioc), _) | (_, OrderType::Market) => {
                domain::TimeInForce::ImmediateOrCancel
            }
            _ => domain::TimeInForce::GoodTillCancel,
        };
        let cl_ord_id = self.new_cl_ord_id();
        let order = Order {
            cl_ord_id: cl_ord_id.clone(),
            symbol: internal.clone(),
            side: o.side,
            qty: o.qty,
            ord_type: o.ord_type,
            limit_price: if o.ord_type == OrderType::Limit {
                o.limit_price
            } else {
                None
            },
            tif,
        };
        {
            let mut book = self.orders.lock().map_err(|_| poisoned())?;
            let key = (o.account_id.clone(), o.request_id.clone());
            if book.live.contains_key(&key) {
                return Err(CmdError::new(ErrorCode::BadRequest, "duplicate request_id"));
            }
            book.live.insert(key, cl_ord_id.clone());
            book.by_cl_ord_id.insert(
                cl_ord_id,
                OrderMeta {
                    account_id: o.account_id,
                    client_request_id: o.request_id,
                    order: order.clone(),
                },
            );
        }
        self.metrics.orders_received.inc();
        self.send_cmd(OrderCommand::Submit(order))
    }

    fn live_meta(&self, account_id: &str, target: &str) -> Result<(String, OrderMeta), CmdError> {
        let book = self.orders.lock().map_err(|_| poisoned())?;
        let cur = book
            .live
            .get(&(account_id.to_string(), target.to_string()))
            .ok_or_else(|| CmdError::new(ErrorCode::UnknownOrder, target.to_string()))?;
        let meta = book.by_cl_ord_id.get(cur).cloned().ok_or_else(poisoned)?;
        Ok((cur.clone(), meta))
    }

    pub fn cancel_order(&self, account_id: &str, target: &str) -> Result<(), CmdError> {
        let (orig, meta) = self.live_meta(account_id, target)?;
        let cl_ord_id = self.new_cl_ord_id();
        self.orders
            .lock()
            .map_err(|_| poisoned())?
            .by_cl_ord_id
            .insert(cl_ord_id.clone(), meta.clone());
        self.metrics.orders_received.inc();
        self.send_cmd(OrderCommand::Cancel {
            cl_ord_id,
            orig_cl_ord_id: orig,
            symbol: meta.order.symbol,
            side: meta.order.side,
        })
    }

    pub fn modify_order(
        &self,
        account_id: &str,
        target: &str,
        qty: Option<Fixed>,
        limit_price: Option<Fixed>,
    ) -> Result<(), CmdError> {
        let (orig, meta) = self.live_meta(account_id, target)?;
        let mut order = meta.order.clone();
        order.cl_ord_id = self.new_cl_ord_id();
        if let Some(q) = qty.filter(|q| q.is_positive()) {
            order.qty = q;
        }
        if let Some(p) = limit_price.filter(|p| p.is_positive()) {
            order.limit_price = Some(p);
        }
        self.orders
            .lock()
            .map_err(|_| poisoned())?
            .by_cl_ord_id
            .insert(
                order.cl_ord_id.clone(),
                OrderMeta {
                    order: order.clone(),
                    ..meta
                },
            );
        self.metrics.orders_received.inc();
        self.send_cmd(OrderCommand::Replace {
            orig_cl_ord_id: orig,
            order,
        })
    }

    /// Maps an LP execution to an OrderUpdate (+ PositionUpdate on fills).
    pub fn on_execution(&self, x: &Execution) {
        let Some(id) = x.cl_ord_id.as_deref() else {
            return;
        };
        let meta = {
            let Ok(mut book) = self.orders.lock() else {
                return;
            };
            let Some(meta) = book.by_cl_ord_id.get(id).cloned() else {
                return;
            };
            if x.exec_type == domain::ExecType::Replaced {
                book.live.insert(
                    (meta.account_id.clone(), meta.client_request_id.clone()),
                    id.to_string(),
                );
            }
            meta
        };
        let sym = client_symbol(&x.symbol);
        let upd = OrderUpdate {
            account_id: meta.account_id.clone(),
            client_request_id: meta.client_request_id.clone(),
            order_id: x.order_id.clone(),
            symbol: sym.clone(),
            side: client_proto::Side::from_domain(x.side) as i32,
            status: client_proto::OrderStatus::from_domain(x.status) as i32,
            filled_qty: Some(x.cum_qty.into()),
            leaves_qty: Some(x.leaves_qty.into()),
            avg_price: x.avg_px.map(Decimal::from),
            last_qty: x.last_qty.map(Decimal::from),
            last_price: x.last_px.map(Decimal::from),
            text: x.text.clone().unwrap_or_default(),
            ts_ns: x.ts_recv_ns,
        };
        let _ = self.accounts.send(Arc::new(AccountEvent::Order(upd)));
        if let (Some(q), Some(px)) = (x.last_qty, x.last_px) {
            if q.is_positive() {
                let dq = if x.side == Side::Buy { q } else { -q };
                let pos = {
                    let Ok(mut all) = self.positions.lock() else {
                        return;
                    };
                    let p = all
                        .entry(meta.account_id.clone())
                        .or_default()
                        .entry(sym.clone())
                        .or_default();
                    p.apply(dq, px);
                    *p
                };
                let _ = self
                    .accounts
                    .send(Arc::new(AccountEvent::Position(PositionUpdate {
                        account_id: meta.account_id,
                        position: Some(Position {
                            symbol: sym,
                            net_qty: Some(pos.net.into()),
                            avg_price: Some(pos.avg.into()),
                        }),
                    })));
            }
        }
    }

    fn on_command_rejected(&self, cl_ord_id: &str, reason: &str) {
        let meta = self
            .orders
            .lock()
            .ok()
            .and_then(|b| b.by_cl_ord_id.get(cl_ord_id).cloned());
        if let Some(meta) = meta {
            let upd = OrderUpdate {
                account_id: meta.account_id,
                client_request_id: meta.client_request_id,
                symbol: client_symbol(&meta.order.symbol),
                side: client_proto::Side::from_domain(meta.order.side) as i32,
                status: client_proto::OrderStatus::Rejected as i32,
                text: reason.to_string(),
                ts_ns: domain::now_ns(),
                ..Default::default()
            };
            let _ = self.accounts.send(Arc::new(AccountEvent::Order(upd)));
        }
    }

    /// Consumes fix-gateway events until the channel closes.
    pub async fn run_bridge(self: Arc<Self>, mut rx: broadcast::Receiver<GatewayEvent>) {
        loop {
            match rx.recv().await {
                Ok(GatewayEvent::Quote(q)) => self.publish_quote(&q),
                Ok(GatewayEvent::Execution(x)) => self.on_execution(&x),
                Ok(GatewayEvent::CommandRejected { cl_ord_id, reason }) => {
                    self.on_command_rejected(&cl_ord_id, &reason)
                }
                Ok(GatewayEvent::CancelRejected {
                    cl_ord_id, text, ..
                }) => self.on_command_rejected(
                    &cl_ord_id,
                    text.as_deref().unwrap_or("cancel/replace rejected"),
                ),
                Ok(ev) => tracing::info!(event = ?ev, "fix-gateway event"),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "bridge lagged behind fix-gateway")
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

fn poisoned() -> CmdError {
    CmdError::new(ErrorCode::Internal, "internal state error")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(s: &str) -> Fixed {
        s.parse().unwrap()
    }

    #[test]
    fn position_netting() {
        let mut p = Pos::default();
        p.apply(px("100"), px("1.1"));
        p.apply(px("100"), px("1.2"));
        assert_eq!(
            p,
            Pos {
                net: px("200"),
                avg: px("1.15")
            }
        );
        p.apply(px("-50"), px("1.3"));
        assert_eq!(
            p,
            Pos {
                net: px("150"),
                avg: px("1.15")
            }
        );
        p.apply(px("-200"), px("1.0"));
        assert_eq!(
            p,
            Pos {
                net: px("-50"),
                avg: px("1.0")
            }
        );
        p.apply(px("50"), px("1.0"));
        assert_eq!(p, Pos::default());
    }

    #[test]
    fn rate_limit_per_account() {
        let cfg = ClientGatewayConfig {
            orders_per_second: 1,
            order_burst: 3,
            ..Default::default()
        };
        let hub = Hub::new(cfg, Authenticator::hs256(b"k"), Vec::new(), None);
        let allowed = (0..10).filter(|_| hub.allow_order("A1")).count();
        assert_eq!(allowed, 3);
        // Independent bucket per account.
        assert!(hub.allow_order("A2"));
        assert_eq!(hub.metrics.orders_rate_limited.get(), 7);
    }

    #[test]
    fn orders_without_trading_path_are_unavailable() {
        let hub = Hub::new(
            ClientGatewayConfig::default(),
            Authenticator::hs256(b"k"),
            vec!["EUR/USD".to_string()],
            None,
        );
        let o = |sym: &str| NewOrder {
            request_id: "r".into(),
            account_id: "A".into(),
            symbol: sym.into(),
            side: Side::Buy,
            ord_type: OrderType::Market,
            qty: px("1000"),
            limit_price: None,
            tif: None,
        };
        assert_eq!(
            hub.place_order(o("XXXYYY")).unwrap_err().0,
            ErrorCode::UnknownSymbol
        );
        assert_eq!(
            hub.place_order(o("EURUSD")).unwrap_err().0,
            ErrorCode::Unavailable
        );
        assert_eq!(
            hub.cancel_order("A", "nope").unwrap_err().0,
            ErrorCode::UnknownOrder
        );
    }
}
