//! Shared gateway state: quote/account fan-out, order routing through the
//! core engine ([`CoreApi`]), candles.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

use client_proto::{
    client_symbol, AccountSnapshot, Decimal, ErrorCode, OrderUpdate, Position, PositionUpdate,
    TimeInForce,
};
use core_engine::api::{
    AccountView, CoreApi, CoreError, CoreErrorCode, CoreEvent, OrderKind, OrderStatus, OrderView,
    PlaceOrderRequest, PositionView,
};
use domain::{Fixed, OrderType, Side};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use tokio::sync::broadcast;

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

/// Quote on the internal fan-out channel. `group: None` = visible to every
/// connection; otherwise only to connections holding an account of that
/// group (marked-up prices differ per group).
#[derive(Clone, Debug)]
pub struct QuoteMsg {
    pub group: Option<Arc<str>>,
    pub quote: ClientQuote,
}

/// Per-account event fanned out to every connection authorized for `account_id`.
#[derive(Clone, Debug)]
pub enum AccountEvent {
    Order(OrderUpdate),
    Position(PositionUpdate),
    Account(AccountSnapshot),
}

impl AccountEvent {
    pub fn account_id(&self) -> &str {
        match self {
            AccountEvent::Order(o) => &o.account_id,
            AccountEvent::Position(p) => &p.account_id,
            AccountEvent::Account(a) => &a.account_id,
        }
    }
}

pub struct Hub {
    pub cfg: ClientGatewayConfig,
    pub auth: Authenticator,
    pub metrics: Metrics,
    quotes: broadcast::Sender<Arc<QuoteMsg>>,
    accounts: broadcast::Sender<Arc<AccountEvent>>,
    candles: Mutex<CandleStore>,
    /// Client symbol -> internal symbol.
    symbols: BTreeMap<String, String>,
    /// Client symbol -> instrument spec (for `SymbolList`).
    specs: BTreeMap<String, client_proto::Instrument>,
    core: Option<Arc<dyn CoreApi>>,
    /// Group whose quotes feed the candle store (mid is markup-neutral).
    candle_group: Option<String>,
    limiter: DefaultKeyedRateLimiter<String>,
}

/// Command validation / routing failure, mapped to a protocol `Error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdError(pub ErrorCode, pub String);

impl CmdError {
    fn new(code: ErrorCode, msg: impl Into<String>) -> Self {
        CmdError(code, msg.into())
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
    pub ord_type: OrderType,
    pub qty: Fixed,
    pub limit_price: Option<Fixed>,
    pub tif: Option<TimeInForce>,
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
    }
}

pub fn position(p: &PositionView) -> Position {
    Position {
        symbol: p.symbol.clone(),
        net_qty: Some(p.net_qty.into()),
        avg_price: Some(p.avg_price.into()),
        unrealized_pnl: Some(p.unrealized_pnl.into()),
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
        let (accounts, _) = broadcast::channel(4096);
        let rate = NonZeroU32::new(cfg.orders_per_second).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new(cfg.order_burst).unwrap_or(NonZeroU32::MIN);
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
            limiter: RateLimiter::keyed(Quota::per_second(rate).allow_burst(burst)),
            symbols,
            candle_group: core.as_ref().and_then(|c| c.groups().into_iter().next()),
            core,
            quotes,
            accounts,
            metrics: Metrics::default(),
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
        let _ = self.quotes.send(Arc::new(QuoteMsg { group, quote: q }));
    }

    /// Ingests a client-form quote visible to every connection (tests,
    /// alternative feeds).
    pub fn publish_client_quote(&self, q: ClientQuote) {
        self.publish(None, q, true);
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

    /// Group of an account (quote visibility), if a core is attached.
    pub fn account_group(&self, account_id: &str) -> Option<String> {
        self.core.as_ref()?.account_group(account_id)
    }

    /// Current snapshot (balance, equity, margin, positions) from the core.
    pub async fn account_snapshot(&self, account_id: &str) -> Option<AccountSnapshot> {
        let core = self.core.as_ref()?;
        core.account_snapshot(account_id)
            .await
            .ok()
            .map(|a| account_snapshot(&a))
    }

    pub async fn place_order(&self, o: NewOrder) -> Result<(), CmdError> {
        if !self.symbols.contains_key(&o.symbol) {
            return Err(CmdError::new(ErrorCode::UnknownSymbol, o.symbol.clone()));
        }
        if !o.qty.is_positive() {
            return Err(CmdError::new(ErrorCode::BadRequest, "qty must be > 0"));
        }
        let kind = match o.ord_type {
            OrderType::Market => OrderKind::Market,
            OrderType::Limit => {
                if !o.limit_price.is_some_and(|p| p.is_positive()) {
                    return Err(CmdError::new(
                        ErrorCode::BadRequest,
                        "limit order needs limit_price",
                    ));
                }
                OrderKind::Limit
            }
        };
        // Market orders execute immediately (IOC at the LP); limit orders
        // rest in the core until triggered (GTC). Other TIFs are not
        // supported yet.
        if matches!(o.tif, Some(TimeInForce::Fok))
            || (kind == OrderKind::Limit && matches!(o.tif, Some(TimeInForce::Ioc)))
        {
            return Err(CmdError::new(
                ErrorCode::BadRequest,
                "unsupported time in force",
            ));
        }
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        self.metrics.orders_received.inc();
        core.place_order(PlaceOrderRequest {
            account: o.account_id,
            client_order_id: o.request_id,
            symbol: o.symbol,
            side: o.side,
            kind,
            qty: o.qty,
            limit_price: o.limit_price,
        })
        .await?;
        Ok(())
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
        qty: Option<Fixed>,
        limit_price: Option<Fixed>,
    ) -> Result<(), CmdError> {
        let core = self.core.as_ref().ok_or_else(unavailable)?;
        self.metrics.orders_received.inc();
        core.modify_order(account_id, target, qty, limit_price)
            .await?;
        Ok(())
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
            CoreEvent::Order(o) => AccountEvent::Order(order_update(o)),
            CoreEvent::Position(p) => AccountEvent::Position(PositionUpdate {
                account_id: p.account.clone(),
                position: Some(position(p)),
            }),
            CoreEvent::Account(a) => AccountEvent::Account(account_snapshot(a)),
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

    #[tokio::test]
    async fn orders_without_trading_path_are_unavailable() {
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
