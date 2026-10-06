//! The boundary between client-facing services and the core engine.
//!
//! [`CoreApi`] is what client-gateway talks to: place / cancel / modify
//! orders, account snapshots, positions and a stream of [`CoreEvent`]s
//! (marked-up quotes per group plus order, position and account updates).
//! Quantities on this boundary are in base-currency units (as on the client
//! wire, e.g. `100000` = 1 standard lot); the engine works in lots.
//!
//! [`InProcessCore`] implements it on top of an [`EngineHandle`]. A
//! NATS-based implementation (request/reply for commands, subjects per
//! account for events) is the planned follow-up for running core-engine as a
//! separate process.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use domain::Fixed;
use money::{Price, Qty};
use oms::{Command, Event, NewOrder, OrderChange, OrderType};
use tokio::sync::broadcast;

use crate::output;
use crate::EngineHandle;

pub use oms::{DealEntry, MarginMode, OrderOrigin, OrderStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreErrorCode {
    BadRequest,
    UnknownAccount,
    UnknownSymbol,
    UnknownOrder,
    /// Pre-trade margin check failed.
    InsufficientMargin,
    /// Any other risk / validation rejection by the engine.
    Rejected,
    Unavailable,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct CoreError {
    pub code: CoreErrorCode,
    pub message: String,
}

impl CoreError {
    pub fn new(code: CoreErrorCode, message: impl Into<String>) -> CoreError {
        CoreError {
            code,
            message: message.into(),
        }
    }
}

/// Maps an engine rejection reason to an error code.
pub fn classify_reject(reason: &str) -> CoreErrorCode {
    if reason.contains("insufficient free margin") {
        CoreErrorCode::InsufficientMargin
    } else if reason.contains("unknown symbol") {
        CoreErrorCode::UnknownSymbol
    } else if reason.contains("unknown account") {
        CoreErrorCode::UnknownAccount
    } else {
        CoreErrorCode::Rejected
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderKind {
    Market,
    Limit,
    Stop,
    StopLimit,
}

impl From<OrderType> for OrderKind {
    fn from(t: OrderType) -> Self {
        match t {
            OrderType::Market => OrderKind::Market,
            OrderType::Limit => OrderKind::Limit,
            OrderType::Stop => OrderKind::Stop,
            OrderType::StopLimit => OrderKind::StopLimit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceOrderRequest {
    pub account: String,
    /// Idempotency key, unique per account.
    pub client_order_id: String,
    /// Core (client) symbol, e.g. `EURUSD`.
    pub symbol: String,
    pub side: domain::Side,
    pub kind: OrderKind,
    /// Units of base currency.
    pub qty: Fixed,
    pub limit_price: Option<Fixed>,
    /// Trigger price (stop / stop-limit).
    pub stop_price: Option<Fixed>,
    pub sl: Option<Fixed>,
    pub tp: Option<Fixed>,
    /// Trailing stop distance in price units.
    pub trailing_distance: Option<Fixed>,
    /// One-cancels-other group (per account).
    pub oco_group: Option<u64>,
    /// Pending order expiry (ns since epoch).
    pub expire_at_ns: Option<u64>,
    /// Market orders: client slippage tolerance in points (MT5 deviation).
    pub max_deviation_points: Option<u32>,
}

impl PlaceOrderRequest {
    /// Plain market order; set the other fields as needed.
    pub fn market(
        account: &str,
        client_order_id: &str,
        symbol: &str,
        side: domain::Side,
        qty: Fixed,
    ) -> PlaceOrderRequest {
        PlaceOrderRequest {
            account: account.into(),
            client_order_id: client_order_id.into(),
            symbol: symbol.into(),
            side,
            kind: OrderKind::Market,
            qty,
            limit_price: None,
            stop_price: None,
            sl: None,
            tp: None,
            trailing_distance: None,
            oco_group: None,
            expire_at_ns: None,
            max_deviation_points: None,
        }
    }
}

/// Changes to a pending order; `None` keeps the current value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderModify {
    pub qty: Option<Fixed>,
    pub limit_price: Option<Fixed>,
    pub stop_price: Option<Fixed>,
    pub sl: Option<Fixed>,
    pub tp: Option<Fixed>,
    pub trailing_distance: Option<Fixed>,
    pub expire_at_ns: Option<u64>,
    /// Remove the expiry (GTC).
    pub clear_expiry: bool,
    /// `sl` / `tp` / `trailing_distance` replace the current values (`None`
    /// removes) instead of keeping them.
    pub replace_protection: bool,
}

/// Protective levels of a position (full replacement; `None` = off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Protection {
    pub sl: Option<Fixed>,
    pub tp: Option<Fixed>,
    pub trailing_distance: Option<Fixed>,
}

/// One execution against a position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DealView {
    pub account: String,
    pub deal_id: u64,
    pub order_id: u64,
    pub client_order_id: String,
    pub position_id: u64,
    pub symbol: String,
    pub side: domain::Side,
    pub entry: DealEntry,
    /// Units of base currency.
    pub qty: Fixed,
    pub price: Fixed,
    /// Realized P&L, account currency.
    pub pnl: Fixed,
    /// Commission (negative = cost), account currency.
    pub commission: Fixed,
    pub ts_ns: u64,
    pub reason: OrderOrigin,
}

/// Deal history query (oldest first).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DealQuery {
    /// Inclusive bounds (ns); 0 = unbounded.
    pub from_ns: u64,
    pub to_ns: u64,
    /// Return deals with id > `after` (paging cursor; 0 = from the start).
    pub after: u64,
    /// Page size (at least 1).
    pub limit: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DealPage {
    pub deals: Vec<DealView>,
    /// Cursor for the next page (`None` = last page).
    pub next: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderAck {
    pub order_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderView {
    pub account: String,
    /// Client order id as given by the client (modify suffixes stripped).
    pub client_order_id: String,
    pub order_id: u64,
    pub symbol: String,
    pub side: domain::Side,
    pub status: OrderStatus,
    pub qty: Fixed,
    pub filled_qty: Fixed,
    pub avg_price: Option<Fixed>,
    pub last_qty: Option<Fixed>,
    pub last_price: Option<Fixed>,
    pub reason: Option<String>,
    pub ts_ns: u64,
    pub kind: OrderKind,
    pub limit_price: Option<Fixed>,
    pub stop_price: Option<Fixed>,
    pub sl: Option<Fixed>,
    pub tp: Option<Fixed>,
    pub trailing_distance: Option<Fixed>,
    pub oco_group: Option<u64>,
    pub expire_at_ns: Option<u64>,
    /// Position opened / increased by the order.
    pub position_id: Option<u64>,
    /// Position the order closes.
    pub close_position_id: Option<u64>,
    pub stop_triggered: bool,
    pub created_ns: u64,
}

/// One open position (hedging accounts can hold several per symbol).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionView {
    pub account: String,
    pub position_id: u64,
    pub symbol: String,
    pub side: domain::Side,
    /// Signed units (positive = long); zero = closed.
    pub net_qty: Fixed,
    pub avg_price: Fixed,
    /// Floating P&L in account currency at the group's client quote.
    pub unrealized_pnl: Fixed,
    pub sl: Option<Fixed>,
    pub tp: Option<Fixed>,
    pub trailing_distance: Option<Fixed>,
    pub open_ts_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountView {
    pub account: String,
    pub group: String,
    pub currency: String,
    pub balance: Fixed,
    pub equity: Fixed,
    pub margin: Fixed,
    pub free_margin: Fixed,
    /// Equity / margin in percent; `None` without margin.
    pub margin_level_pct: Option<Fixed>,
    pub margin_call: bool,
    pub margin_mode: MarginMode,
    pub leverage: u32,
    pub positions: Vec<PositionView>,
}

/// Client quote of a symbol for one group (markup applied).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupQuote {
    pub group: String,
    pub symbol: String,
    pub bid: Fixed,
    pub ask: Fixed,
    pub ts_ns: u64,
}

/// LP depth of a symbol as one group sees it (markup applied to every level).
/// Quantities are lots, best level first. Not journaled: a replay
/// has no depth until the next LP tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupDepth {
    pub group: String,
    pub symbol: String,
    pub bids: Vec<(Fixed, Fixed)>,
    pub asks: Vec<(Fixed, Fixed)>,
    pub ts_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreEvent {
    Quote(GroupQuote),
    Depth(GroupDepth),
    Order(OrderView),
    Position(PositionView),
    Account(AccountView),
    Deal(DealView),
}

impl CoreEvent {
    pub fn account(&self) -> Option<&str> {
        match self {
            CoreEvent::Quote(_) | CoreEvent::Depth(_) => None,
            CoreEvent::Order(o) => Some(&o.account),
            CoreEvent::Position(p) => Some(&p.account),
            CoreEvent::Account(a) => Some(&a.account),
            CoreEvent::Deal(d) => Some(&d.account),
        }
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
pub type CoreFuture<'a, T> = Pin<Box<dyn Future<Output = CoreResult<T>> + Send + 'a>>;

/// In-process API boundary of the core engine.
pub trait CoreApi: Send + Sync {
    /// Tradable core symbols (e.g. `EURUSD`).
    fn symbols(&self) -> Vec<String>;
    /// Group names (quotes are published per group).
    fn groups(&self) -> Vec<String>;
    /// Group of an account (one group per account), if known.
    fn account_group(&self, account: &str) -> Option<String>;
    fn place_order(&self, req: PlaceOrderRequest) -> CoreFuture<'_, OrderAck>;
    fn cancel_order(&self, account: &str, client_order_id: &str) -> CoreFuture<'_, ()>;
    /// Changes a pending order in place (it keeps its ids).
    fn modify_order(
        &self,
        account: &str,
        client_order_id: &str,
        change: OrderModify,
    ) -> CoreFuture<'_, OrderAck>;
    /// Replaces SL / TP / trailing distance of an open position.
    fn modify_position(
        &self,
        account: &str,
        position_id: u64,
        protection: Protection,
    ) -> CoreFuture<'_, ()>;
    /// Closes `qty` units (all if `None`) of a position at market; the
    /// closing order uses `client_order_id`.
    fn close_position(
        &self,
        account: &str,
        position_id: u64,
        qty: Option<Fixed>,
        client_order_id: &str,
    ) -> CoreFuture<'_, OrderAck>;
    fn account_snapshot(&self, account: &str) -> CoreFuture<'_, AccountView>;
    fn positions(&self, account: &str) -> CoreFuture<'_, Vec<PositionView>>;
    /// Pending (working) orders of an account.
    fn orders(&self, account: &str) -> CoreFuture<'_, Vec<OrderView>>;
    /// Finished orders of an account (filled, cancelled, rejected, expired), newest first.
    fn order_history(&self, account: &str) -> CoreFuture<'_, Vec<OrderView>>;
    /// Deal history page of an account.
    fn deals(&self, account: &str, query: DealQuery) -> CoreFuture<'_, DealPage>;
    /// All core events; consumers filter by account / group.
    fn subscribe(&self) -> broadcast::Receiver<Arc<CoreEvent>>;
}

type NameMaps = (BTreeMap<String, u64>, BTreeMap<u64, String>);

/// External account id <-> engine account number.
#[derive(Clone, Default, Debug)]
pub struct AccountNames {
    inner: Arc<RwLock<NameMaps>>,
}

impl AccountNames {
    pub fn insert(&self, name: &str, no: u64) {
        if let Ok(mut g) = self.inner.write() {
            g.0.insert(name.to_string(), no);
            g.1.insert(no, name.to_string());
        }
    }
    /// Engine number of an external account id: a registered name, else a
    /// plain account number (accounts opened from the back office are linked
    /// in identity by their number; the token's `accounts` claim is signed).
    pub fn number(&self, name: &str) -> Option<u64> {
        let named = self.inner.read().ok()?.0.get(name).copied();
        named.or_else(|| {
            (!name.is_empty() && name.len() <= 12 && name.bytes().all(|b| b.is_ascii_digit()))
                .then(|| name.parse().ok())
                .flatten()
        })
    }
    /// Name of an engine account (falls back to the number).
    pub fn name(&self, no: u64) -> String {
        self.inner
            .read()
            .ok()
            .and_then(|g| g.1.get(&no).cloned())
            .unwrap_or_else(|| no.to_string())
    }
}

#[derive(Clone, Debug)]
struct SymbolInfo {
    contract_size: i64,
    /// Price point (raw 1e-8 units).
    point: i64,
}

/// [`CoreApi`] over an in-process engine thread.
pub struct InProcessCore {
    engine: EngineHandle,
    events: broadcast::Sender<Arc<CoreEvent>>,
    names: AccountNames,
    symbols: BTreeMap<String, SymbolInfo>,
    groups: Vec<String>,
    account_groups: BTreeMap<String, String>,
}

fn unavailable(e: String) -> CoreError {
    CoreError::new(CoreErrorCode::Unavailable, e)
}

impl InProcessCore {
    /// Caches symbols, groups and account groups from the engine.
    pub async fn new(
        engine: EngineHandle,
        events: broadcast::Sender<Arc<CoreEvent>>,
        names: AccountNames,
    ) -> CoreResult<InProcessCore> {
        let (symbols, groups, accounts) = engine
            .read(|e| {
                let s: Vec<(String, i64, i64)> = e
                    .symbols()
                    .map(|s| (s.symbol.clone(), s.contract_size, s.point().raw()))
                    .collect();
                let g: Vec<String> = e.groups().map(|g| g.name.clone()).collect();
                let a: Vec<(u64, String)> = e.accounts().map(|a| (a.id, a.group.clone())).collect();
                (s, g, a)
            })
            .await
            .map_err(unavailable)?;
        Ok(InProcessCore {
            symbols: symbols
                .into_iter()
                .map(|(s, contract_size, point)| {
                    (
                        s,
                        SymbolInfo {
                            contract_size,
                            point,
                        },
                    )
                })
                .collect(),
            groups,
            account_groups: accounts
                .into_iter()
                .map(|(no, g)| (names.name(no), g))
                .collect(),
            engine,
            events,
            names,
        })
    }

    pub fn engine(&self) -> &EngineHandle {
        &self.engine
    }

    fn account_no(&self, account: &str) -> CoreResult<u64> {
        self.names
            .number(account)
            .ok_or_else(|| CoreError::new(CoreErrorCode::UnknownAccount, account))
    }

    fn info(&self, symbol: &str) -> CoreResult<&SymbolInfo> {
        self.symbols
            .get(symbol)
            .ok_or_else(|| CoreError::new(CoreErrorCode::UnknownSymbol, symbol))
    }

    /// Price distance -> whole points (rounded, at least 1).
    fn points(&self, symbol: &str, distance: Option<Fixed>) -> CoreResult<Option<i64>> {
        let Some(d) = distance else {
            return Ok(None);
        };
        if !d.is_positive() {
            return Err(CoreError::new(
                CoreErrorCode::BadRequest,
                "trailing distance must be > 0",
            ));
        }
        let pt = self.info(symbol)?.point.max(1);
        Ok(Some(((d.raw() + pt / 2) / pt).max(1)))
    }

    fn lots(&self, symbol: &str, units: Fixed) -> CoreResult<Qty> {
        let info = self.info(symbol)?;
        if !units.is_positive() {
            return Err(CoreError::new(CoreErrorCode::BadRequest, "qty must be > 0"));
        }
        if units.raw() % info.contract_size != 0 {
            return Err(CoreError::new(
                CoreErrorCode::BadRequest,
                "qty is not a whole number of lot units",
            ));
        }
        Ok(Qty::from_raw(units.raw() / info.contract_size))
    }

    async fn place(&self, o: NewOrder) -> CoreResult<OrderAck> {
        self.order_command(Command::PlaceOrder(o)).await
    }

    /// Runs a command that creates an order; maps rejections.
    async fn order_command(&self, cmd: Command) -> CoreResult<OrderAck> {
        let events = self.engine.command(cmd).await.map_err(unavailable)?;
        for ev in &events {
            match ev {
                Event::OrderRejected { reason, .. } | Event::CommandRejected { reason } => {
                    return Err(CoreError::new(classify_reject(reason), reason.clone()))
                }
                Event::DuplicateOrder { .. } => {
                    return Err(CoreError::new(
                        CoreErrorCode::BadRequest,
                        "duplicate client order id",
                    ))
                }
                _ => {}
            }
        }
        events
            .iter()
            .find_map(|e| match e {
                Event::OrderAccepted { order_id } => Some(OrderAck {
                    order_id: *order_id,
                }),
                _ => None,
            })
            .ok_or_else(|| CoreError::new(CoreErrorCode::Internal, "order not accepted"))
    }
}

/// Optional protective price (must be > 0 when given).
fn protective(p: Option<Fixed>) -> CoreResult<Option<Price>> {
    match p {
        None => Ok(None),
        Some(p) if p.is_positive() => Ok(Some(Price::from(p))),
        Some(_) => Err(CoreError::new(
            CoreErrorCode::BadRequest,
            "SL / TP must be > 0",
        )),
    }
}

impl InProcessCore {
    async fn position_symbol(&self, no: u64, position_id: u64) -> CoreResult<String> {
        self.engine
            .read(move |e| {
                e.position(position_id)
                    .filter(|p| p.account == no)
                    .map(|p| p.symbol.clone())
            })
            .await
            .map_err(unavailable)?
            .ok_or_else(|| CoreError::new(CoreErrorCode::UnknownOrder, "unknown position"))
    }
}

impl CoreApi for InProcessCore {
    fn symbols(&self) -> Vec<String> {
        self.symbols.keys().cloned().collect()
    }

    fn groups(&self) -> Vec<String> {
        self.groups.clone()
    }

    fn account_group(&self, account: &str) -> Option<String> {
        self.account_groups.get(account).cloned()
    }

    fn place_order(&self, req: PlaceOrderRequest) -> CoreFuture<'_, OrderAck> {
        Box::pin(async move {
            let no = self.account_no(&req.account)?;
            let volume = self.lots(&req.symbol, req.qty)?;
            if req.client_order_id.contains('~') {
                return Err(CoreError::new(
                    CoreErrorCode::BadRequest,
                    "client order id must not contain '~'",
                ));
            }
            let mut o = NewOrder::market(
                no,
                &req.client_order_id,
                &req.symbol,
                req.side.into(),
                volume,
            );
            let price = |p: Option<Fixed>, what: &str| {
                p.filter(|p| p.is_positive())
                    .map(Price::from)
                    .ok_or_else(|| {
                        CoreError::new(CoreErrorCode::BadRequest, format!("{what} required"))
                    })
            };
            o.order_type = match req.kind {
                OrderKind::Market => OrderType::Market,
                OrderKind::Limit => OrderType::Limit,
                OrderKind::Stop => OrderType::Stop,
                OrderKind::StopLimit => OrderType::StopLimit,
            };
            if matches!(req.kind, OrderKind::Limit | OrderKind::StopLimit) {
                o.limit_price = Some(price(req.limit_price, "limit_price")?);
            }
            if matches!(req.kind, OrderKind::Stop | OrderKind::StopLimit) {
                o.stop_price = Some(price(req.stop_price, "stop_price")?);
            }
            o.sl = protective(req.sl)?;
            o.tp = protective(req.tp)?;
            o.trailing_points = self.points(&req.symbol, req.trailing_distance)?;
            if req.kind == OrderKind::Market {
                o.max_deviation_points = req.max_deviation_points.map(i64::from);
            }
            if req.kind != OrderKind::Market {
                o.oco_group = req.oco_group.filter(|g| *g != 0);
                o.expire_at = req.expire_at_ns.filter(|t| *t != 0);
            }
            self.place(o).await
        })
    }

    fn cancel_order(&self, account: &str, client_order_id: &str) -> CoreFuture<'_, ()> {
        let account = account.to_string();
        let clid = client_order_id.to_string();
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let c = clid.clone();
            let id = self
                .engine
                .read(move |e| e.order_by_client_id(no, &c).map(|o| o.id))
                .await
                .map_err(unavailable)?
                .ok_or_else(|| CoreError::new(CoreErrorCode::UnknownOrder, clid.clone()))?;
            let events = self
                .engine
                .command(Command::CancelOrder {
                    account: no,
                    order_id: id,
                })
                .await
                .map_err(unavailable)?;
            for ev in events {
                if let Event::CommandRejected { reason } = ev {
                    return Err(CoreError::new(CoreErrorCode::Rejected, reason));
                }
            }
            Ok(())
        })
    }

    fn modify_order(
        &self,
        account: &str,
        client_order_id: &str,
        change: OrderModify,
    ) -> CoreFuture<'_, OrderAck> {
        let account = account.to_string();
        let clid = client_order_id.to_string();
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let c = clid.clone();
            let (id, req) = self
                .engine
                .read(move |e| {
                    e.order_by_client_id(no, &c)
                        .filter(|o| o.is_pending())
                        .map(|o| (o.id, o.req.clone()))
                })
                .await
                .map_err(unavailable)?
                .ok_or_else(|| {
                    CoreError::new(CoreErrorCode::UnknownOrder, "no pending order to modify")
                })?;
            let mut c = OrderChange {
                volume: req.volume,
                limit_price: req.limit_price,
                stop_price: req.stop_price,
                sl: req.sl,
                tp: req.tp,
                trailing_points: req.trailing_points,
                expire_at: req.expire_at,
            };
            if let Some(q) = change.qty {
                c.volume = self.lots(&req.symbol, q)?;
            }
            let pos = |p: Fixed, what: &str| {
                p.is_positive().then(|| Price::from(p)).ok_or_else(|| {
                    CoreError::new(CoreErrorCode::BadRequest, format!("{what} must be > 0"))
                })
            };
            if let Some(p) = change.limit_price {
                if c.limit_price.is_none() {
                    return Err(CoreError::new(
                        CoreErrorCode::BadRequest,
                        "order has no limit price",
                    ));
                }
                c.limit_price = Some(pos(p, "limit_price")?);
            }
            if let Some(p) = change.stop_price {
                if c.stop_price.is_none() {
                    return Err(CoreError::new(
                        CoreErrorCode::BadRequest,
                        "order has no stop price",
                    ));
                }
                c.stop_price = Some(pos(p, "stop_price")?);
            }
            if change.replace_protection {
                c.sl = protective(change.sl)?;
                c.tp = protective(change.tp)?;
                c.trailing_points = self.points(&req.symbol, change.trailing_distance)?;
            } else {
                if change.sl.is_some() {
                    c.sl = protective(change.sl)?;
                }
                if change.tp.is_some() {
                    c.tp = protective(change.tp)?;
                }
                if change.trailing_distance.is_some() {
                    c.trailing_points = self.points(&req.symbol, change.trailing_distance)?;
                }
            }
            if change.clear_expiry {
                c.expire_at = None;
            } else if let Some(t) = change.expire_at_ns.filter(|t| *t != 0) {
                c.expire_at = Some(t);
            }
            let events = self
                .engine
                .command(Command::ModifyOrder {
                    account: no,
                    order_id: id,
                    change: c,
                })
                .await
                .map_err(unavailable)?;
            for ev in events {
                if let Event::CommandRejected { reason } = ev {
                    return Err(CoreError::new(classify_reject(&reason), reason));
                }
            }
            Ok(OrderAck { order_id: id })
        })
    }

    fn modify_position(
        &self,
        account: &str,
        position_id: u64,
        protection: Protection,
    ) -> CoreFuture<'_, ()> {
        let account = account.to_string();
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let symbol = self.position_symbol(no, position_id).await?;
            let events = self
                .engine
                .command(Command::ModifyPosition {
                    account: no,
                    position_id,
                    sl: protective(protection.sl)?,
                    tp: protective(protection.tp)?,
                    trailing_points: self.points(&symbol, protection.trailing_distance)?,
                })
                .await
                .map_err(unavailable)?;
            for ev in events {
                if let Event::CommandRejected { reason } = ev {
                    return Err(CoreError::new(classify_reject(&reason), reason));
                }
            }
            Ok(())
        })
    }

    fn close_position(
        &self,
        account: &str,
        position_id: u64,
        qty: Option<Fixed>,
        client_order_id: &str,
    ) -> CoreFuture<'_, OrderAck> {
        let account = account.to_string();
        let clid = client_order_id.to_string();
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let symbol = self.position_symbol(no, position_id).await?;
            let volume = qty.map(|q| self.lots(&symbol, q)).transpose()?;
            if clid.is_empty() || clid.contains('~') {
                return Err(CoreError::new(
                    CoreErrorCode::BadRequest,
                    "invalid client order id",
                ));
            }
            self.order_command(Command::ClosePosition {
                account: no,
                position_id,
                volume,
                client_order_id: clid,
            })
            .await
        })
    }

    fn account_snapshot(&self, account: &str) -> CoreFuture<'_, AccountView> {
        let account = account.to_string();
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let names = self.names.clone();
            self.engine
                .read(move |e| output::account_view(e, no, &names))
                .await
                .map_err(unavailable)?
                .ok_or_else(|| CoreError::new(CoreErrorCode::UnknownAccount, account))
        })
    }

    fn positions(&self, account: &str) -> CoreFuture<'_, Vec<PositionView>> {
        let account = account.to_string();
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let names = self.names.clone();
            self.engine
                .read(move |e| output::positions_view(e, no, &names))
                .await
                .map_err(unavailable)
        })
    }

    fn orders(&self, account: &str) -> CoreFuture<'_, Vec<OrderView>> {
        let account = account.to_string();
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let names = self.names.clone();
            self.engine
                .read(move |e| output::pending_orders_view(e, no, &names))
                .await
                .map_err(unavailable)
        })
    }

    fn order_history(&self, account: &str) -> CoreFuture<'_, Vec<OrderView>> {
        let account = account.to_string();
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let names = self.names.clone();
            self.engine
                .read(move |e| output::order_history_view(e, no, &names, 500))
                .await
                .map_err(unavailable)
        })
    }

    fn deals(&self, account: &str, query: DealQuery) -> CoreFuture<'_, DealPage> {
        let account = account.to_string();
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let names = self.names.clone();
            self.engine
                .read(move |e| output::deal_page(e, no, query, &names))
                .await
                .map_err(unavailable)
        })
    }

    fn subscribe(&self) -> broadcast::Receiver<Arc<CoreEvent>> {
        self.events.subscribe()
    }
}
