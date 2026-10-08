//! Shared gateway state: quote/account fan-out, order routing through the
//! core engine ([`CoreApi`]), candles.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use client_proto::{
    client_symbol, AccountInfo, AccountSnapshot, DealUpdate, Decimal, ErrorCode, OrderUpdate,
    Position, PositionUpdate, TimeInForce,
};
use core_engine::api::{
    AccountView, CoreApi, CoreError, CoreErrorCode, CoreEvent, DealEntry, DealPage, DealQuery,
    DealView, MarginMode, OrderKind, OrderModify, OrderOrigin, OrderStatus, OrderView,
    PlaceOrderRequest, PositionView, Protection,
};
use domain::{Fixed, Side};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use tokio::sync::broadcast;

use crate::auth::{Authenticator, Claims};
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

/// Quote on the internal fan-out channel. `group: None` = visible to every
/// connection; otherwise only to connections holding an account of that
/// group (marked-up prices differ per group).
#[derive(Clone, Debug)]
pub struct QuoteMsg {
    pub group: Option<Arc<str>>,
    pub quote: ClientQuote,
}

/// LP depth on the internal fan-out channel; `group` as for [`QuoteMsg`].
#[derive(Clone, Debug)]
pub struct DepthMsg {
    pub group: Option<Arc<str>>,
    pub depth: Arc<client_proto::Depth>,
}

/// Per-account event fanned out to every connection authorized for `account_id`.
#[derive(Clone, Debug)]
pub enum AccountEvent {
    Order(Box<OrderUpdate>),
    Position(PositionUpdate),
    Account(AccountSnapshot),
    Deal(DealUpdate),
}

impl AccountEvent {
    pub fn account_id(&self) -> &str {
        match self {
            AccountEvent::Order(o) => &o.account_id,
            AccountEvent::Position(p) => &p.account_id,
            AccountEvent::Account(a) => &a.account_id,
            AccountEvent::Deal(d) => &d.account_id,
        }
    }
}

/// Latest quote per (group, client symbol).
type LastQuotes = HashMap<(Option<Arc<str>>, Arc<str>), Arc<ClientQuote>>;

pub struct Hub {
    pub cfg: ClientGatewayConfig,
    pub auth: Authenticator,
    pub metrics: Metrics,
    pub conns: crate::limits::ConnLimits,
    quotes: broadcast::Sender<Arc<QuoteMsg>>,
    depths: broadcast::Sender<Arc<DepthMsg>>,
    accounts: broadcast::Sender<Arc<AccountEvent>>,
    candles: Mutex<CandleStore>,
    /// Per-account client preferences (opaque JSON) and the file they persist to.
    prefs: Mutex<HashMap<String, String>>,
    prefs_path: Mutex<Option<std::path::PathBuf>>,
    /// Latest quote per (group, symbol): sent on subscribe so a new connection shows
    /// every price at once instead of waiting for each instrument's next tick.
    last_quotes: Mutex<LastQuotes>,
    /// Client symbol -> internal symbol.
    symbols: BTreeMap<String, String>,
    /// Client symbol -> instrument spec (for `SymbolList`).
    specs: BTreeMap<String, client_proto::Instrument>,
    core: Option<Arc<dyn CoreApi>>,
    /// Group whose quotes feed the candle store (mid is markup-neutral).
    candle_group: Option<String>,
    limiter: DefaultKeyedRateLimiter<String>,
    /// REST requests per API key / login session ([`Hub::allow_rest`]).
    rest_limiter: DefaultKeyedRateLimiter<String>,
    /// REST checks since start; every few thousand, idle keys are dropped
    /// (session ids keep coming, unlike accounts).
    rest_checks: AtomicU64,
}

/// Command validation / routing failure, mapped to a protocol `Error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdError(pub ErrorCode, pub String);

impl CmdError {
    fn new(code: ErrorCode, msg: impl Into<String>) -> Self {
        CmdError(code, msg.into())
    }

    /// The engine already holds an order with this client order id (account
    /// scoped): a retried request, answered from the stored order.
    pub fn is_duplicate_order(&self) -> bool {
        self.0 == ErrorCode::BadRequest && self.1.contains("duplicate client order id")
    }
}

impl From<CoreError> for CmdError {
    fn from(e: CoreError) -> Self {
        let code = match e.code {
            CoreErrorCode::BadRequest => ErrorCode::BadRequest,
            CoreErrorCode::UnknownAccount => ErrorCode::Forbidden,
            CoreErrorCode::UnknownSymbol => ErrorCode::UnknownSymbol,
            CoreErrorCode::UnknownOrder => ErrorCode::UnknownOrder,
            CoreErrorCode::InsufficientMargin => ErrorCode::InsufficientMargin,
            CoreErrorCode::Rejected => ErrorCode::OrderRejected,
            CoreErrorCode::Unavailable => ErrorCode::Unavailable,
            CoreErrorCode::Internal => ErrorCode::Internal,
        };
        CmdError(code, e.message)
    }
}

/// Validated order parameters from a PlaceOrder.
pub struct NewOrder {
    pub request_id: String,
    pub account_id: String,
    pub symbol: String,
    pub side: Side,
    pub ord_type: OrderKind,
    pub qty: Fixed,
    pub limit_price: Option<Fixed>,
    pub tif: Option<TimeInForce>,
    pub stop_price: Option<Fixed>,
    pub sl: Option<Fixed>,
    pub tp: Option<Fixed>,
    pub trailing_distance: Option<Fixed>,
    pub oco_group: Option<u64>,
    pub expire_at_ns: Option<u64>,
    /// Market orders: client slippage tolerance in points.
    pub max_deviation_points: Option<u32>,
}

impl NewOrder {
    /// Market order with no extras (tests, simple callers).
    pub fn market(
        request_id: &str,
        account_id: &str,
        symbol: &str,
        side: Side,
        qty: Fixed,
    ) -> Self {
        NewOrder {
            request_id: request_id.into(),
            account_id: account_id.into(),
            symbol: symbol.into(),
            side,
            ord_type: OrderKind::Market,
            qty,
            limit_price: None,
            tif: None,
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

fn order_type(k: OrderKind) -> client_proto::OrderType {
    use client_proto::OrderType as P;
    match k {
        OrderKind::Market => P::Market,
        OrderKind::Limit => P::Limit,
        OrderKind::Stop => P::Stop,
        OrderKind::StopLimit => P::StopLimit,
    }
}

fn margin_mode(m: MarginMode) -> client_proto::MarginMode {
    match m {
        MarginMode::Netting => client_proto::MarginMode::Netting,
        MarginMode::Hedging => client_proto::MarginMode::Hedging,
    }
}

fn dec(f: Option<Fixed>) -> Option<Decimal> {
    f.map(Decimal::from)
}

fn id_str(id: Option<u64>) -> String {
    id.map(|i| i.to_string()).unwrap_or_default()
}

fn status(s: OrderStatus) -> client_proto::OrderStatus {
    use client_proto::OrderStatus as P;
    match s {
        OrderStatus::New | OrderStatus::Accepted => P::New,
        OrderStatus::PartiallyFilled => P::PartiallyFilled,
        OrderStatus::Filled => P::Filled,
        OrderStatus::Cancelled => P::Canceled,
        OrderStatus::Rejected => P::Rejected,
        OrderStatus::Expired => P::Expired,
    }
}

pub fn order_update(o: &OrderView) -> OrderUpdate {
    OrderUpdate {
        account_id: o.account.clone(),
        client_request_id: o.client_order_id.clone(),
        order_id: o.order_id.to_string(),
        symbol: o.symbol.clone(),
        side: client_proto::Side::from_domain(o.side) as i32,
        status: status(o.status) as i32,
        filled_qty: Some(o.filled_qty.into()),
        leaves_qty: Some(
            if o.status.is_terminal() {
                Fixed::ZERO
            } else {
                o.qty - o.filled_qty
            }
            .into(),
        ),
        avg_price: o.avg_price.map(Decimal::from),
        last_qty: o.last_qty.map(Decimal::from),
        last_price: o.last_price.map(Decimal::from),
        text: o.reason.clone().unwrap_or_default(),
        ts_ns: o.ts_ns,
        order_type: order_type(o.kind) as i32,
        qty: Some(o.qty.into()),
        limit_price: dec(o.limit_price),
        stop_price: dec(o.stop_price),
        sl: dec(o.sl),
        tp: dec(o.tp),
        trailing_distance: dec(o.trailing_distance),
        oco_group: o.oco_group.unwrap_or(0),
        expire_at_ns: o.expire_at_ns.unwrap_or(0),
        position_id: id_str(o.position_id),
        stop_triggered: o.stop_triggered,
        close_position_id: id_str(o.close_position_id),
        created_ns: o.created_ns,
    }
}

pub fn position(p: &PositionView) -> Position {
    let qty = if p.net_qty.raw() < 0 {
        Fixed::ZERO - p.net_qty
    } else {
        p.net_qty
    };
    Position {
        symbol: p.symbol.clone(),
        net_qty: Some(p.net_qty.into()),
        avg_price: Some(p.avg_price.into()),
        unrealized_pnl: Some(p.unrealized_pnl.into()),
        position_id: p.position_id.to_string(),
        side: client_proto::Side::from_domain(p.side) as i32,
        qty: Some(qty.into()),
        sl: dec(p.sl),
        tp: dec(p.tp),
        trailing_distance: dec(p.trailing_distance),
        open_time_ns: p.open_ts_ns,
    }
}

pub fn deal(d: &DealView) -> client_proto::Deal {
    use client_proto::{DealEntry as E, DealReason as R};
    client_proto::Deal {
        deal_id: d.deal_id.to_string(),
        order_id: d.order_id.to_string(),
        client_request_id: d.client_order_id.clone(),
        position_id: d.position_id.to_string(),
        symbol: d.symbol.clone(),
        side: client_proto::Side::from_domain(d.side) as i32,
        entry: match d.entry {
            DealEntry::In => E::In,
            DealEntry::Out => E::Out,
        } as i32,
        qty: Some(d.qty.into()),
        price: Some(d.price.into()),
        realized_pnl: Some(d.pnl.into()),
        commission: Some(d.commission.into()),
        ts_ns: d.ts_ns,
        reason: match d.reason {
            OrderOrigin::Client => R::Client,
            OrderOrigin::StopLoss => R::StopLoss,
            OrderOrigin::TakeProfit => R::TakeProfit,
            OrderOrigin::StopOut => R::StopOut,
            OrderOrigin::Copy => R::Client,
        } as i32,
    }
}

pub fn account_info(a: &AccountView) -> AccountInfo {
    AccountInfo {
        account_id: a.account.clone(),
        currency: a.currency.clone(),
        margin_mode: margin_mode(a.margin_mode) as i32,
        leverage: a.leverage,
        group: a.group.clone(),
    }
}

pub fn account_snapshot(a: &AccountView) -> AccountSnapshot {
    AccountSnapshot {
        account_id: a.account.clone(),
        currency: a.currency.clone(),
        balance: Some(a.balance.into()),
        equity: Some(a.equity.into()),
        margin_used: Some(a.margin.into()),
        positions: a.positions.iter().map(position).collect(),
        free_margin: Some(a.free_margin.into()),
        margin_level: a.margin_level_pct.map(Decimal::from),
        margin_mode: margin_mode(a.margin_mode) as i32,
        leverage: a.leverage,
    }
}

fn unavailable() -> CmdError {
    CmdError::new(ErrorCode::Unavailable, "trading unavailable")
}

impl Hub {
    /// `symbols`: internal instrument symbols (e.g. `EUR/USD`); without
    /// `core` order commands answer `UNAVAILABLE`.
    pub fn new(
        cfg: ClientGatewayConfig,
        auth: Authenticator,
        symbols: impl IntoIterator<Item = String>,
        core: Option<Arc<dyn CoreApi>>,
    ) -> Arc<Self> {
        let (quotes, _) = broadcast::channel(4096);
        let (depths, _) = broadcast::channel(4096);
        let (accounts, _) = broadcast::channel(4096);
        let rate = NonZeroU32::new(cfg.orders_per_second).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new(cfg.order_burst).unwrap_or(NonZeroU32::MIN);
        let rest_rate = NonZeroU32::new(cfg.rest_requests_per_second).unwrap_or(NonZeroU32::MIN);
        let rest_burst = NonZeroU32::new(cfg.rest_burst).unwrap_or(NonZeroU32::MIN);
        let symbols: BTreeMap<String, String> = symbols
            .into_iter()
            .map(|s| (client_symbol(&s), s))
            .collect();
        let specs = symbols
            .iter()
            .map(|(c, i)| (c.clone(), instrument_spec(i, None, None)))
            .collect();
        Arc::new(Hub {
            specs,
            candles: Mutex::new(CandleStore::new(cfg.candle_capacity)),
            last_quotes: Mutex::new(HashMap::new()),
            prefs: Mutex::new(HashMap::new()),
            prefs_path: Mutex::new(None),
            limiter: RateLimiter::keyed(Quota::per_second(rate).allow_burst(burst)),
            rest_limiter: RateLimiter::keyed(Quota::per_second(rest_rate).allow_burst(rest_burst)),
            rest_checks: AtomicU64::new(0),
            symbols,
            candle_group: core.as_ref().and_then(|c| c.groups().into_iter().next()),
            core,
            quotes,
            depths,
            accounts,
            metrics: Metrics::default(),
            conns: crate::limits::ConnLimits::new(
                cfg.max_connections,
                cfg.max_connections_per_ip,
                cfg.max_connections_per_subject,
            ),
            auth,
            cfg,
        })
    }

    /// Like [`Hub::new`], but with full instrument specs (tick size, qty step) so
    /// `SymbolList` can describe them.
    pub fn with_instruments(
        cfg: ClientGatewayConfig,
        auth: Authenticator,
        instruments: &[domain::Instrument],
        core: Option<Arc<dyn CoreApi>>,
    ) -> Arc<Self> {
        let mut hub = Hub::new(
            cfg,
            auth,
            instruments.iter().map(|i| i.symbol.clone()),
            core,
        );
        if let Some(h) = Arc::get_mut(&mut hub) {
            for i in instruments {
                h.specs.insert(
                    client_symbol(&i.symbol),
                    instrument_spec(&i.symbol, Some(i.tick_size), Some(i.qty_step)),
                );
            }
        }
        hub
    }

    /// Instrument specs of every served symbol, sorted by client symbol.
    pub fn instruments(&self) -> Vec<client_proto::Instrument> {
        self.specs.values().cloned().collect()
    }

    pub fn core(&self) -> Option<&Arc<dyn CoreApi>> {
        self.core.as_ref()
    }

    pub fn subscribe_quotes(&self) -> broadcast::Receiver<Arc<QuoteMsg>> {
        self.quotes.subscribe()
    }

    pub fn subscribe_depths(&self) -> broadcast::Receiver<Arc<DepthMsg>> {
        self.depths.subscribe()
    }

    pub fn subscribe_accounts(&self) -> broadcast::Receiver<Arc<AccountEvent>> {
        self.accounts.subscribe()
    }

    pub fn is_known_symbol(&self, client_sym: &str) -> bool {
        self.symbols.contains_key(client_sym)
    }

    fn publish(&self, group: Option<Arc<str>>, q: ClientQuote, candles: bool) {
        if candles {
            if let (Some(b), Some(a)) = (q.bid, q.ask) {
                if let Ok(mut c) = self.candles.lock() {
                    c.on_price(&q.symbol, mid(b, a), q.ts_ns);
                }
            }
        }
        if let Ok(mut last) = self.last_quotes.lock() {
            last.insert((group.clone(), q.symbol.clone()), Arc::new(q.clone()));
        }
        let _ = self.quotes.send(Arc::new(QuoteMsg { group, quote: q }));
    }

    /// Latest known quotes of `symbols` as a connection with `groups` sees them
    /// (group streams after the common one, so the marked-up price wins).
    pub fn last_quotes(
        &self,
        symbols: &[String],
        groups: &HashSet<String>,
    ) -> Vec<Arc<ClientQuote>> {
        let Ok(last) = self.last_quotes.lock() else {
            return Vec::new();
        };
        let mut out: Vec<_> = last
            .iter()
            .filter(|((g, s), _)| {
                g.as_deref().is_none_or(|g| groups.contains(g))
                    && symbols.iter().any(|x| **x == **s)
            })
            .map(|((g, _), q)| (g.is_some(), q.clone()))
            .collect();
        out.sort_by_key(|(grouped, _)| *grouped);
        out.into_iter().map(|(_, q)| q).collect()
    }

    /// Ingests a client-form quote visible to every connection (tests,
    /// alternative feeds).
    pub fn publish_client_quote(&self, q: ClientQuote) {
        self.publish(None, q, true);
    }

    /// Loads the preferences file (if any) and persists every later change to it.
    pub fn use_prefs_file(&self, path: std::path::PathBuf) -> std::io::Result<usize> {
        let n = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let map: HashMap<String, String> =
                    serde_json::from_str(&text).map_err(std::io::Error::other)?;
                let n = map.len();
                if let Ok(mut p) = self.prefs.lock() {
                    *p = map;
                }
                n
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e),
        };
        if let Ok(mut p) = self.prefs_path.lock() {
            *p = Some(path);
        }
        Ok(n)
    }

    /// Stored preferences of an account (empty when none).
    pub fn prefs(&self, account_id: &str) -> String {
        self.prefs
            .lock()
            .ok()
            .and_then(|p| p.get(account_id).cloned())
            .unwrap_or_default()
    }

    /// Replaces an account's preferences and writes the file (temp + rename).
    pub fn set_prefs(&self, account_id: &str, json: String) -> std::io::Result<()> {
        let snapshot = {
            let mut p = self
                .prefs
                .lock()
                .map_err(|_| std::io::Error::other("prefs lock"))?;
            if json.is_empty() {
                p.remove(account_id);
            } else {
                p.insert(account_id.to_string(), json);
            }
            serde_json::to_vec(&*p).map_err(std::io::Error::other)?
        };
        let path = self.prefs_path.lock().ok().and_then(|p| p.clone());
        if let Some(path) = path {
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, snapshot)?;
            std::fs::rename(tmp, path)?;
        }
        Ok(())
    }

    /// Restores the candle history saved by [`Hub::save_candles`]; returns the bar count.
    pub fn load_candles(&self, path: &std::path::Path) -> std::io::Result<usize> {
        let text = std::fs::read_to_string(path)?;
        Ok(self
            .candles
            .lock()
            .map(|mut c| c.restore(&text))
            .unwrap_or(0))
    }

    /// Writes the candle history to `path` (temp file + rename).
    pub fn save_candles(&self, path: &std::path::Path) -> std::io::Result<()> {
        let text = self.candles.lock().map(|c| c.dump()).unwrap_or_default();
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
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

    /// Per-account order rate limit. `true` = allowed.
    pub fn allow_order(&self, account_id: &str) -> bool {
        let ok = self.limiter.check_key(&account_id.to_string()).is_ok();
        if !ok {
            self.metrics.orders_rate_limited.inc();
        }
        ok
    }

    /// The one gate every order command passes, WebSocket and REST alike:
    /// trading scope of the token (read-only API keys never trade), the
    /// account claim, then the per-account order rate limit.
    pub fn authorize_trade(&self, claims: &Claims, account_id: &str) -> Result<(), CmdError> {
        if !claims.may_trade() {
            return Err(CmdError::new(ErrorCode::Forbidden, "read-only API key"));
        }
        if !claims.may_access(account_id) {
            return Err(CmdError::new(
                ErrorCode::Forbidden,
                "account not authorized",
            ));
        }
        if !self.allow_order(account_id) {
            return Err(CmdError::new(
                ErrorCode::RateLimited,
                "order rate limit exceeded",
            ));
        }
        Ok(())
    }

    /// REST request rate limit per API key (identity `sid` = `apikey:<id>`)
    /// or, for interactive tokens, per session / subject. `true` = allowed.
    pub fn allow_rest(&self, claims: &Claims) -> bool {
        if self.rest_checks.fetch_add(1, Ordering::Relaxed) % 4096 == 4095 {
            self.rest_limiter.retain_recent();
            self.rest_limiter.shrink_to_fit();
        }
        let key = claims.sid.as_deref().unwrap_or(&claims.sub).to_string();
        let ok = self.rest_limiter.check_key(&key).is_ok();
        if !ok {
            self.metrics.rest_rate_limited.inc();
        }
        ok
    }

    /// First order of `account_id` matching `hit`: working orders first, then
    /// the recent history (newest first).
    pub async fn find_order(
        &self,
        account_id: &str,
        hit: impl Fn(&OrderUpdate) -> bool,
    ) -> Result<Option<OrderUpdate>, CmdError> {
        if let Some(o) = self.orders(account_id).await?.into_iter().find(&hit) {
            return Ok(Some(o));
        }
        Ok(self.order_history(account_id).await?.into_iter().find(hit))
    }

    /// Group of an account (quote visibility), if a core is attached.
    pub fn account_group(&self, account_id: &str) -> Option<String> {
        self.core.as_ref()?.account_group(account_id)
    }

    /// Current snapshot (balance, equity, margin, positions) from the core.
    pub async fn account_snapshot(&self, account_id: &str) -> Option<AccountSnapshot> {
        self.account_state(account_id).await.map(|(_, s)| s)
    }

    /// Account parameters and snapshot from the core.
    pub async fn account_state(&self, account_id: &str) -> Option<(AccountInfo, AccountSnapshot)> {
        let core = self.core.as_ref()?;
        core.account_snapshot(account_id)
            .await
            .ok()
            .map(|a| (account_info(&a), account_snapshot(&a)))
    }

    /// Validates and places an order; returns the engine order id.
    pub async fn place_order(&self, o: NewOrder) -> Result<u64, CmdError> {
        if !self.symbols.contains_key(&o.symbol) {
            return Err(CmdError::new(ErrorCode::UnknownSymbol, o.symbol.clone()));
        }
        if !o.qty.is_positive() {
            return Err(CmdError::new(ErrorCode::BadRequest, "qty must be > 0"));
        }
        let kind = o.ord_type;
        let positive = |p: Option<Fixed>| p.is_some_and(|p| p.is_positive());
        if matches!(kind, OrderKind::Limit | OrderKind::StopLimit) && !positive(o.limit_price) {
            return Err(CmdError::new(ErrorCode::BadRequest, "limit_price required"));
        }
        if matches!(kind, OrderKind::Stop | OrderKind::StopLimit) && !positive(o.stop_price) {
            return Err(CmdError::new(ErrorCode::BadRequest, "stop_price required"));
        }
        // Market orders execute immediately (IOC at the LP); pending orders
        // rest in the core until triggered: GTC, or GTD with expire_at_ns.
        let pending = kind != OrderKind::Market;
        if matches!(o.tif, Some(TimeInForce::Fok))
            || (pending && matches!(o.tif, Some(TimeInForce::Ioc)))
            || (!pending && matches!(o.tif, Some(TimeInForce::Gtd)))
        {
            return Err(CmdError::new(
                ErrorCode::BadRequest,
                "unsupported time in force",
            ));
        }
        if matches!(o.tif, Some(TimeInForce::Gtd)) && o.expire_at_ns.is_none() {
            return Err(CmdError::new(
                ErrorCode::BadRequest,
                "GTD needs expire_at_ns",
            ));
        }
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        self.metrics.orders_received.inc();
        let ack = core
            .place_order(PlaceOrderRequest {
                account: o.account_id,
                client_order_id: o.request_id,
                symbol: o.symbol,
                side: o.side,
                kind,
                qty: o.qty,
                limit_price: o.limit_price,
                stop_price: o.stop_price,
                sl: o.sl,
                tp: o.tp,
                trailing_distance: o.trailing_distance,
                oco_group: o.oco_group,
                expire_at_ns: o.expire_at_ns,
                max_deviation_points: o.max_deviation_points,
            })
            .await?;
        Ok(ack.order_id)
    }

    pub async fn cancel_order(&self, account_id: &str, target: &str) -> Result<(), CmdError> {
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        self.metrics.orders_received.inc();
        Ok(core.cancel_order(account_id, target).await?)
    }

    pub async fn modify_order(
        &self,
        account_id: &str,
        target: &str,
        change: OrderModify,
    ) -> Result<(), CmdError> {
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        self.metrics.orders_received.inc();
        core.modify_order(account_id, target, change).await?;
        Ok(())
    }

    pub async fn modify_position(
        &self,
        account_id: &str,
        position_id: &str,
        protection: Protection,
    ) -> Result<(), CmdError> {
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        let pid = parse_position_id(position_id)?;
        self.metrics.orders_received.inc();
        Ok(core.modify_position(account_id, pid, protection).await?)
    }

    /// Closes `qty` (all if `None`) of a position; returns the closing order id.
    pub async fn close_position(
        &self,
        account_id: &str,
        position_id: &str,
        qty: Option<Fixed>,
        request_id: &str,
    ) -> Result<u64, CmdError> {
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        let pid = parse_position_id(position_id)?;
        if qty.is_some_and(|q| !q.is_positive()) {
            return Err(CmdError::new(ErrorCode::BadRequest, "qty must be > 0"));
        }
        self.metrics.orders_received.inc();
        let ack = core
            .close_position(account_id, pid, qty, request_id)
            .await?;
        Ok(ack.order_id)
    }

    /// Working orders of an account (OrderUpdate shape).
    pub async fn orders(&self, account_id: &str) -> Result<Vec<OrderUpdate>, CmdError> {
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        Ok(core
            .orders(account_id)
            .await?
            .iter()
            .map(order_update)
            .collect())
    }

    /// Finished orders of an account (OrderUpdate shape), newest first.
    pub async fn order_history(&self, account_id: &str) -> Result<Vec<OrderUpdate>, CmdError> {
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        Ok(core
            .order_history(account_id)
            .await?
            .iter()
            .map(order_update)
            .collect())
    }

    /// One page of deal history.
    pub async fn deals(&self, account_id: &str, q: DealQuery) -> Result<DealPage, CmdError> {
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        Ok(core.deals(account_id, q).await?)
    }

    fn on_core_event(&self, ev: &CoreEvent) {
        let ae = match ev {
            CoreEvent::Quote(q) => {
                let candles = self.candle_group.as_deref() == Some(q.group.as_str());
                return self.publish(
                    Some(q.group.as_str().into()),
                    ClientQuote {
                        symbol: q.symbol.as_str().into(),
                        bid: Some(q.bid),
                        ask: Some(q.ask),
                        bid_size: None,
                        ask_size: None,
                        ts_ns: q.ts_ns,
                    },
                    candles,
                );
            }
            CoreEvent::Depth(d) => {
                let levels = |ls: &[(Fixed, Fixed)]| {
                    ls.iter()
                        .map(|(p, q)| client_proto::DepthLevel {
                            price: Some(Decimal::from(*p)),
                            qty: Some(Decimal::from(*q)),
                        })
                        .collect()
                };
                let _ = self.depths.send(Arc::new(DepthMsg {
                    group: Some(d.group.as_str().into()),
                    depth: Arc::new(client_proto::Depth {
                        symbol: d.symbol.clone(),
                        bids: levels(&d.bids),
                        asks: levels(&d.asks),
                        ts_ns: d.ts_ns,
                    }),
                }));
                return;
            }
            CoreEvent::Order(o) => AccountEvent::Order(Box::new(order_update(o))),
            CoreEvent::Position(p) => AccountEvent::Position(PositionUpdate {
                account_id: p.account.clone(),
                position: Some(position(p)),
            }),
            CoreEvent::Account(a) => AccountEvent::Account(account_snapshot(a)),
            CoreEvent::Deal(d) => AccountEvent::Deal(DealUpdate {
                account_id: d.account.clone(),
                deal: Some(deal(d)),
            }),
        };
        let _ = self.accounts.send(Arc::new(ae));
    }

    /// Consumes core events (marked-up quotes, order / position / account
    /// updates) until the channel closes.
    pub async fn run_core_bridge(self: Arc<Self>, mut rx: broadcast::Receiver<Arc<CoreEvent>>) {
        loop {
            match rx.recv().await {
                Ok(ev) => self.on_core_event(&ev),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "bridge lagged behind core events")
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

fn parse_position_id(s: &str) -> Result<u64, CmdError> {
    s.parse()
        .map_err(|_| CmdError::new(ErrorCode::UnknownOrder, "unknown position"))
}

fn decimals(f: Fixed) -> u32 {
    let mut raw = f.raw();
    if raw == 0 {
        return 0;
    }
    let mut d = domain::fixed::SCALE_DIGITS;
    while d > 0 && raw % 10 == 0 {
        raw /= 10;
        d -= 1;
    }
    d
}

/// Builds a client instrument spec from an internal symbol (`EUR/USD` or `EURUSD`).
fn instrument_spec(
    internal: &str,
    tick_size: Option<Fixed>,
    qty_step: Option<Fixed>,
) -> client_proto::Instrument {
    let sym = client_symbol(internal);
    let (base, quote) = match internal.split_once('/') {
        Some((b, q)) => (b.to_string(), q.to_string()),
        None if sym.len() == 6 => (sym[..3].to_string(), sym[3..].to_string()),
        None => (sym.clone(), String::new()),
    };
    // FX convention: 1 lot = 100,000 units of base. Metals/others are configured
    // per venue later; the default keeps the field present for clients.
    let contract = match base.as_str() {
        "XAU" => Fixed::from_int(100),
        "XAG" => Fixed::from_int(5_000),
        _ if base.len() == 3 && quote.len() == 3 => Fixed::from_int(100_000),
        _ => Fixed::from_int(1),
    };
    client_proto::Instrument {
        symbol: sym,
        digits: tick_size.map(decimals).unwrap_or(0),
        tick_size: tick_size.map(Decimal::from),
        qty_step: qty_step.map(Decimal::from),
        contract_size: Some(contract.into()),
        base,
        quote,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(s: &str) -> Fixed {
        s.parse().unwrap()
    }

    #[test]
    fn instrument_specs() {
        let inst = domain::Instrument {
            symbol: "USD/JPY".into(),
            security_id: "4004".into(),
            tick_size: px("0.001"),
            qty_step: px("1000"),
            contract_size: 1,
        };
        let hub = Hub::with_instruments(
            ClientGatewayConfig::default(),
            Authenticator::hs256(b"k"),
            &[inst],
            None,
        );
        let l = hub.instruments();
        assert_eq!(l.len(), 1);
        let i = &l[0];
        assert_eq!(
            (i.symbol.as_str(), i.base.as_str(), i.quote.as_str()),
            ("USDJPY", "USD", "JPY")
        );
        assert_eq!(i.digits, 3);
        assert_eq!(i.tick_size, Some(px("0.001").into()));
        assert_eq!(i.qty_step, Some(px("1000").into()));
        assert_eq!(i.contract_size, Some(px("100000").into()));
        // Plain constructor: specs without tick data.
        let h2 = Hub::new(
            ClientGatewayConfig::default(),
            Authenticator::hs256(b"k"),
            vec!["EUR/USD".to_string()],
            None,
        );
        assert_eq!(h2.instruments()[0].tick_size, None);
        assert_eq!(decimals(px("0.00001")), 5);
        assert_eq!(decimals(px("1")), 0);
    }

    #[test]
    fn last_quotes_snapshot_for_new_subscribers() {
        let hub = Hub::new(
            ClientGatewayConfig::default(),
            Authenticator::hs256(b"k"),
            Vec::new(),
            None,
        );
        let q = |symbol: &str, bid: i64| ClientQuote {
            symbol: symbol.into(),
            bid: Some(Fixed::from_raw(bid)),
            ask: Some(Fixed::from_raw(bid + 1)),
            bid_size: None,
            ask_size: None,
            ts_ns: 1,
        };
        hub.publish_client_quote(q("EURUSD", 1));
        hub.publish_client_quote(q("EURUSD", 2));
        hub.publish_client_quote(q("GBPUSD", 5));
        hub.publish(Some("vip".into()), q("EURUSD", 9), false);
        let subs = ["EURUSD".to_string()];
        let common = hub.last_quotes(&subs, &HashSet::new());
        assert_eq!(common.len(), 1);
        assert_eq!(common[0].bid, Some(Fixed::from_raw(2)));
        // A member of the group gets its own (marked-up) price last, so it wins.
        let vip = hub.last_quotes(&subs, &HashSet::from(["vip".to_string()]));
        assert_eq!(vip.last().unwrap().bid, Some(Fixed::from_raw(9)));
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

    fn claims(scope: Option<&str>, sid: Option<&str>) -> Claims {
        Claims {
            sub: "u".into(),
            exp: 0,
            accounts: vec!["A1".into()],
            roles: vec![],
            amr: vec![],
            scope: scope.map(str::to_string),
            sid: sid.map(str::to_string),
        }
    }

    #[test]
    fn trade_gate_checks_scope_then_account_then_rate() {
        let cfg = ClientGatewayConfig {
            orders_per_second: 1,
            order_burst: 1,
            ..Default::default()
        };
        let hub = Hub::new(cfg, Authenticator::hs256(b"k"), Vec::new(), None);
        let read = claims(Some("read"), None);
        assert_eq!(
            hub.authorize_trade(&read, "A1"),
            Err(CmdError::new(ErrorCode::Forbidden, "read-only API key"))
        );
        let trade = claims(Some("trade"), None);
        assert_eq!(
            hub.authorize_trade(&trade, "B2"),
            Err(CmdError::new(
                ErrorCode::Forbidden,
                "account not authorized"
            ))
        );
        // The refusals above did not spend the account's order budget.
        assert_eq!(hub.authorize_trade(&trade, "A1"), Ok(()));
        assert_eq!(
            hub.authorize_trade(&claims(None, None), "A1")
                .unwrap_err()
                .0,
            ErrorCode::RateLimited
        );
    }

    #[test]
    fn rest_rate_limit_per_api_key() {
        let cfg = ClientGatewayConfig {
            rest_requests_per_second: 1,
            rest_burst: 2,
            ..Default::default()
        };
        let hub = Hub::new(cfg, Authenticator::hs256(b"k"), Vec::new(), None);
        let k1 = claims(Some("read"), Some("apikey:k1"));
        let k2 = claims(Some("read"), Some("apikey:k2"));
        let login = claims(None, None);
        assert_eq!((0..5).filter(|_| hub.allow_rest(&k1)).count(), 2);
        // Another key of the same user, and the user's own login, are separate.
        assert!(hub.allow_rest(&k2));
        assert!(hub.allow_rest(&login));
        assert_eq!(hub.metrics.rest_rate_limited.get(), 3);
    }

    #[test]
    fn duplicate_order_errors_are_recognised() {
        assert!(CmdError::from(CoreError::new(
            CoreErrorCode::BadRequest,
            "duplicate client order id"
        ))
        .is_duplicate_order());
        assert!(
            !CmdError::new(ErrorCode::OrderRejected, "duplicate client order id")
                .is_duplicate_order()
        );
        assert!(!CmdError::new(ErrorCode::BadRequest, "qty must be > 0").is_duplicate_order());
    }

    #[tokio::test]
    async fn orders_without_trading_path_are_unavailable() {
        let hub = Hub::new(
            ClientGatewayConfig::default(),
            Authenticator::hs256(b"k"),
            vec!["EUR/USD".to_string()],
            None,
        );
        let o = |sym: &str| NewOrder::market("r", "A", sym, Side::Buy, px("1000"));
        assert_eq!(
            hub.place_order(o("XXXYYY")).await.unwrap_err().0,
            ErrorCode::UnknownSymbol
        );
        assert_eq!(
            hub.place_order(o("EURUSD")).await.unwrap_err().0,
            ErrorCode::Unavailable
        );
        assert_eq!(
            hub.cancel_order("A", "nope").await.unwrap_err().0,
            ErrorCode::Unavailable
        );
    }
}
