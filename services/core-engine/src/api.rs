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

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};

use domain::Fixed;
use money::{Price, Qty};
use oms::{Command, Event, NewOrder, OrderType};
use tokio::sync::broadcast;

use crate::output;
use crate::EngineHandle;

pub use oms::OrderStatus;

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
}

/// Net position of an account in one symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionView {
    pub account: String,
    pub symbol: String,
    /// Signed units (positive = long); zero = flat.
    pub net_qty: Fixed,
    pub avg_price: Fixed,
    /// Floating P&L in account currency at the group's client quote.
    pub unrealized_pnl: Fixed,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreEvent {
    Quote(GroupQuote),
    Order(OrderView),
    Position(PositionView),
    Account(AccountView),
}

impl CoreEvent {
    pub fn account(&self) -> Option<&str> {
        match self {
            CoreEvent::Quote(_) => None,
            CoreEvent::Order(o) => Some(&o.account),
            CoreEvent::Position(p) => Some(&p.account),
            CoreEvent::Account(a) => Some(&a.account),
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
    /// Changes a pending order's quantity and/or limit price.
    fn modify_order(
        &self,
        account: &str,
        client_order_id: &str,
        qty: Option<Fixed>,
        limit_price: Option<Fixed>,
    ) -> CoreFuture<'_, OrderAck>;
    fn account_snapshot(&self, account: &str) -> CoreFuture<'_, AccountView>;
    fn positions(&self, account: &str) -> CoreFuture<'_, Vec<PositionView>>;
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
    pub fn number(&self, name: &str) -> Option<u64> {
        self.inner.read().ok()?.0.get(name).copied()
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
}

/// [`CoreApi`] over an in-process engine thread.
pub struct InProcessCore {
    engine: EngineHandle,
    events: broadcast::Sender<Arc<CoreEvent>>,
    names: AccountNames,
    symbols: BTreeMap<String, SymbolInfo>,
    groups: Vec<String>,
    account_groups: BTreeMap<String, String>,
    /// (account, client order id) -> engine client order id currently live
    /// (differs after modify, which is cancel + re-place).
    live: Mutex<HashMap<(String, String), String>>,
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
                let s: Vec<(String, i64)> = e
                    .symbols()
                    .map(|s| (s.symbol.clone(), s.contract_size))
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
                .map(|(s, contract_size)| (s, SymbolInfo { contract_size }))
                .collect(),
            groups,
            account_groups: accounts
                .into_iter()
                .map(|(no, g)| (names.name(no), g))
                .collect(),
            engine,
            events,
            names,
            live: Mutex::default(),
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

    fn lots(&self, symbol: &str, units: Fixed) -> CoreResult<Qty> {
        let info = self
            .symbols
            .get(symbol)
            .ok_or_else(|| CoreError::new(CoreErrorCode::UnknownSymbol, symbol))?;
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

    fn live_clid(&self, account: &str, client_order_id: &str) -> String {
        self.live
            .lock()
            .ok()
            .and_then(|m| {
                m.get(&(account.to_string(), client_order_id.to_string()))
                    .cloned()
            })
            .unwrap_or_else(|| client_order_id.to_string())
    }

    async fn place(&self, o: NewOrder) -> CoreResult<OrderAck> {
        let events = self
            .engine
            .command(Command::PlaceOrder(o))
            .await
            .map_err(unavailable)?;
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
            if req.kind == OrderKind::Limit {
                let px = req.limit_price.filter(|p| p.is_positive()).ok_or_else(|| {
                    CoreError::new(CoreErrorCode::BadRequest, "limit order needs limit_price")
                })?;
                o.order_type = OrderType::Limit;
                o.limit_price = Some(Price::from(px));
            }
            self.place(o).await
        })
    }

    fn cancel_order(&self, account: &str, client_order_id: &str) -> CoreFuture<'_, ()> {
        let account = account.to_string();
        let clid = self.live_clid(&account, client_order_id);
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
        qty: Option<Fixed>,
        limit_price: Option<Fixed>,
    ) -> CoreFuture<'_, OrderAck> {
        let account = account.to_string();
        let base = client_order_id.to_string();
        let cur = self.live_clid(&account, &base);
        Box::pin(async move {
            let no = self.account_no(&account)?;
            let c = cur.clone();
            let req = self
                .engine
                .read(move |e| {
                    e.order_by_client_id(no, &c)
                        .filter(|o| o.is_pending())
                        .map(|o| o.req.clone())
                })
                .await
                .map_err(unavailable)?
                .ok_or_else(|| {
                    CoreError::new(CoreErrorCode::UnknownOrder, "no pending order to modify")
                })?;
            let mut new = req.clone();
            if let Some(q) = qty.filter(|q| q.is_positive()) {
                new.volume = self.lots(&req.symbol, q)?;
            }
            if let Some(p) = limit_price.filter(|p| p.is_positive()) {
                new.limit_price = Some(Price::from(p));
            }
            let n = cur
                .rsplit_once('~')
                .and_then(|(_, n)| n.parse::<u64>().ok())
                .unwrap_or(0)
                + 1;
            new.client_order_id = format!("{base}~{n}");
            self.cancel_order(&account, &base).await?;
            let ack = self.place(new.clone()).await?;
            if let Ok(mut m) = self.live.lock() {
                m.insert((account, base), new.client_order_id);
            }
            Ok(ack)
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

    fn subscribe(&self) -> broadcast::Receiver<Arc<CoreEvent>> {
        self.events.subscribe()
    }
}
