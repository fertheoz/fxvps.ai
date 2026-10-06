//! Pre-trade and account risk: symbol specs, account groups, margin
//! (including hedged margin), cross-currency conversion through a quote
//! book, ESMA leverage presets, margin-call / stop-out evaluation,
//! liquidation ordering and negative balance protection.

use money::{Currency, Money, MoneyError, Price, Qty, Rounding, SCALE};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Debug)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn sign(self) -> i64 {
        match self {
            Side::Buy => 1,
            Side::Sell => -1,
        }
    }
    pub fn opposite(self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

/// Conversions with the FIX-edge side type (`domain::Side`).
impl From<domain::Side> for Side {
    fn from(s: domain::Side) -> Side {
        match s {
            domain::Side::Buy => Side::Buy,
            domain::Side::Sell => Side::Sell,
        }
    }
}

impl From<Side> for domain::Side {
    fn from(s: Side) -> domain::Side {
        match s {
            Side::Buy => domain::Side::Buy,
            Side::Sell => domain::Side::Sell,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum MarginMode {
    /// Independent positions per fill, opposite positions may coexist.
    Hedging,
    /// One net position per symbol.
    Netting,
}

/// Which orders a routing rule applies to.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum OrderKindFilter {
    #[default]
    Any,
    Market,
    Pending,
}

/// One row of the routing rule table: the first enabled rule whose filters all
/// match an order decides its book and may override the group's markup,
/// slippage cap and partial-fill policy. Empty lists match everything.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RoutingRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub accounts: Vec<u64>,
    #[serde(default)]
    pub symbols: Vec<String>,
    /// Lots × 100 (centi-lots) to stay integral; `None` = no bound.
    #[serde(default)]
    pub min_centilots: Option<i64>,
    #[serde(default)]
    pub max_centilots: Option<i64>,
    #[serde(default)]
    pub kind: OrderKindFilter,
    /// UTC hour window `[from, to)`; wraps past midnight when from > to.
    #[serde(default)]
    pub hours_utc: Option<(u8, u8)>,
    /// Fixed book, or none to use `a_book_pct` / the group default.
    #[serde(default)]
    pub routing: Option<Routing>,
    /// Hybrid: this percentage of matching orders (by order id) goes A-book, the rest B-book.
    #[serde(default)]
    pub a_book_pct: Option<u8>,
    #[serde(default)]
    pub markup_points: Option<i64>,
    #[serde(default)]
    pub max_slippage_points: Option<i64>,
    #[serde(default)]
    pub partial_fill: Option<PartialFill>,
    /// Client toxicity window `[min, max]` (0..100, see [`FlowStats::toxicity`]).
    #[serde(default)]
    pub min_toxicity: Option<u8>,
    #[serde(default)]
    pub max_toxicity: Option<u8>,
}

impl RoutingRule {
    /// Does the rule apply to an order of `account` in `group` on `symbol`?
    pub fn matches(
        &self,
        group: &str,
        account: u64,
        symbol: &str,
        centilots: i64,
        pending: bool,
        hour_utc: u8,
        toxicity: u8,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        if self.min_toxicity.is_some_and(|m| toxicity < m)
            || self.max_toxicity.is_some_and(|m| toxicity > m)
        {
            return false;
        }
        if !self.groups.is_empty() && !self.groups.iter().any(|g| g == group) {
            return false;
        }
        if !self.accounts.is_empty() && !self.accounts.contains(&account) {
            return false;
        }
        if !self.symbols.is_empty() && !self.symbols.iter().any(|s| s == symbol) {
            return false;
        }
        if self.min_centilots.is_some_and(|m| centilots < m)
            || self.max_centilots.is_some_and(|m| centilots > m)
        {
            return false;
        }
        match self.kind {
            OrderKindFilter::Any => {}
            OrderKindFilter::Market if pending => return false,
            OrderKindFilter::Pending if !pending => return false,
            _ => {}
        }
        if let Some((from, to)) = self.hours_utc {
            let inside = if from <= to {
                hour_utc >= from && hour_utc < to
            } else {
                hour_utc >= from || hour_utc < to
            };
            if !inside {
                return false;
            }
        }
        true
    }

    /// Book for a matching order: fixed routing, else the hybrid split by order id, else `default`.
    pub fn book_for(&self, order_id: u64, default: Routing) -> Routing {
        if let Some(r) = self.routing {
            return r;
        }
        match self.a_book_pct {
            Some(pct) => {
                if (order_id % 100) < u64::from(pct) {
                    Routing::ABook
                } else {
                    Routing::BBook
                }
            }
            None => default,
        }
    }
}

/// Commission charged per fill (per side), overriding the symbol's per-lot commission.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum GroupCommission {
    /// Minor units of the group currency per lot.
    PerLot { minor: i64 },
    /// Minor units of the group currency per 1,000,000 of notional (1 bp = 10,000).
    PerMillion { minor: i64 },
}

/// What happens to the part of an A-book order the LP did not fill.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum PartialFill {
    /// Keep what filled, cancel the rest (IOC semantics).
    #[default]
    CancelRemainder,
    /// Send the remainder to the LP again, up to `max_attempts` LP orders in total.
    Retry { max_attempts: u32 },
    /// Fill-or-kill at the LP: all of it or nothing.
    AllOrNone,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Routing {
    /// STP to liquidity provider through the omnibus account.
    ABook,
    /// Internalized; broker book is the counterparty.
    BBook,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum AssetClass {
    MajorFx,
    MinorFx,
    Gold,
    MajorIndex,
    MinorIndex,
    Commodity,
    Equity,
    Crypto,
}

/// ESMA product-intervention leverage caps (retail clients).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum EsmaPreset {
    Retail,
    Professional,
}

impl EsmaPreset {
    pub fn max_leverage(self, class: AssetClass) -> Option<u32> {
        match self {
            EsmaPreset::Professional => None,
            EsmaPreset::Retail => Some(match class {
                AssetClass::MajorFx => 30,
                AssetClass::MinorFx | AssetClass::Gold | AssetClass::MajorIndex => 20,
                AssetClass::MinorIndex | AssetClass::Commodity => 10,
                AssetClass::Equity => 5,
                AssetClass::Crypto => 2,
            }),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct SymbolSpec {
    pub symbol: String,
    pub base: Currency,
    /// Profit (quote) currency.
    pub quote: Currency,
    pub margin_currency: Currency,
    /// Units of base per 1.0 lot.
    pub contract_size: i64,
    pub digits: u32,
    pub min_lot: Qty,
    pub lot_step: Qty,
    pub max_lot: Qty,
    /// Swap per lot per day (signed, scaled 1e8): profit-currency money per
    /// lot (`SwapMode::Money`) or points of the quote (`SwapMode::Points`).
    pub swap_long: Price,
    pub swap_short: Price,
    #[serde(default)]
    pub swap_mode: SwapMode,
    /// Weekday (0 = Sunday .. 6 = Saturday) whose rollover charges three days.
    #[serde(default = "default_triple_day")]
    pub triple_swap_day: u8,
    /// Commission per lot per side (in its own currency).
    pub commission_per_lot: Money,
    pub asset_class: AssetClass,
}

impl SymbolSpec {
    /// Standard FX pair spec, e.g. `SymbolSpec::fx("EURUSD", EUR, USD, 5)`.
    pub fn fx(symbol: &str, base: Currency, quote: Currency, digits: u32) -> SymbolSpec {
        SymbolSpec {
            symbol: symbol.into(),
            base,
            quote,
            margin_currency: base,
            contract_size: 100_000,
            digits,
            min_lot: Qty::from_raw(SCALE / 100),
            lot_step: Qty::from_raw(SCALE / 100),
            max_lot: Qty::from_units(100),
            swap_long: Price::ZERO,
            swap_short: Price::ZERO,
            swap_mode: SwapMode::Money,
            triple_swap_day: 3,
            commission_per_lot: Money::zero(Currency::USD),
            asset_class: AssetClass::MajorFx,
        }
    }

    /// Size of one point (10^-digits).
    pub fn point(&self) -> Price {
        Price::from_raw(SCALE / 10i64.pow(self.digits))
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct GroupConfig {
    pub name: String,
    pub currency: Currency,
    pub leverage: u32,
    pub margin_mode: MarginMode,
    /// Margin level (equity/margin, percent) at or below which a margin call fires.
    pub margin_call_pct: i64,
    /// Margin level (percent) at or below which positions are liquidated.
    pub stop_out_pct: i64,
    pub max_volume_per_order: Qty,
    /// Max absolute net lots per symbol per account.
    pub max_net_volume: Qty,
    /// `None` = all symbols allowed.
    pub allowed_symbols: Option<BTreeSet<String>>,
    /// Points added to ask / subtracted from bid for clients.
    pub markup_points: i64,
    /// Max distance (points) between requested and current price.
    pub price_tolerance_points: i64,
    /// Fraction (bps) of full margin charged per hedged leg (5000 = 50%).
    pub hedged_margin_bps: u32,
    pub esma: Option<EsmaPreset>,
    pub negative_balance_protection: bool,
    pub routing: Routing,
    #[serde(default)]
    pub partial_fill: PartialFill,
    /// Markup on the bid / ask side; `None` = `markup_points` on both.
    #[serde(default)]
    pub markup_bid_points: Option<i64>,
    #[serde(default)]
    pub markup_ask_points: Option<i64>,
    /// Per-symbol markup (both sides) that overrides the group values.
    #[serde(default)]
    pub symbol_markup_points: BTreeMap<String, i64>,
    /// A-book market orders go to the LP as IOC limits at requested ± this many
    /// points; the part the LP cannot fill inside the cap follows `partial_fill`.
    #[serde(default)]
    pub max_slippage_points: Option<i64>,
    /// `false`: a fill better than the requested price is given at the requested
    /// price and the difference stays with the broker (asymmetric slippage).
    #[serde(default = "default_true")]
    pub pass_price_improvement: bool,
    /// Group commission model; `None` = the symbol's per-lot commission.
    #[serde(default)]
    pub commission: Option<GroupCommission>,
    /// Swap scale in percent (100 = the symbol's swap, 0 = swap-free group).
    #[serde(default = "hundred_u32")]
    pub swap_multiplier_pct: u32,
}

fn hundred_u32() -> u32 {
    100
}

fn default_true() -> bool {
    true
}

impl GroupConfig {
    /// Reasonable retail defaults (ESMA retail, 1:30, 100%/50%).
    pub fn retail(name: &str, currency: Currency, routing: Routing) -> GroupConfig {
        GroupConfig {
            name: name.into(),
            currency,
            leverage: 30,
            margin_mode: MarginMode::Hedging,
            margin_call_pct: 100,
            stop_out_pct: 50,
            max_volume_per_order: Qty::from_units(50),
            max_net_volume: Qty::from_units(200),
            allowed_symbols: None,
            markup_points: 0,
            price_tolerance_points: 50,
            hedged_margin_bps: 5_000,
            esma: Some(EsmaPreset::Retail),
            negative_balance_protection: true,
            routing,
            partial_fill: PartialFill::default(),
            markup_bid_points: None,
            markup_ask_points: None,
            symbol_markup_points: BTreeMap::new(),
            max_slippage_points: None,
            pass_price_improvement: true,
            commission: None,
            swap_multiplier_pct: 100,
        }
    }

    /// Markup in points applied on `side` of `symbol`'s quote (bid for Sell,
    /// ask for Buy): per-symbol override, else the side value, else the group value.
    pub fn markup_points_for(&self, symbol: &str, side: Side) -> i64 {
        if let Some(p) = self.symbol_markup_points.get(symbol) {
            return *p;
        }
        match side {
            Side::Buy => self.markup_ask_points.unwrap_or(self.markup_points),
            Side::Sell => self.markup_bid_points.unwrap_or(self.markup_points),
        }
    }

    pub fn effective_leverage(&self, spec: &SymbolSpec) -> u32 {
        let cap = self.esma.and_then(|e| e.max_leverage(spec.asset_class));
        cap.map_or(self.leverage, |c| c.min(self.leverage)).max(1)
    }

    pub fn symbol_allowed(&self, symbol: &str) -> bool {
        self.allowed_symbols
            .as_ref()
            .is_none_or(|s| s.contains(symbol))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum RiskError {
    #[error("no quote to convert {0} -> {1}")]
    NoConversion(Currency, Currency),
    #[error("no quote for {0}")]
    NoQuote(String),
    #[error("money: {0}")]
    Money(String),
}

impl From<MoneyError> for RiskError {
    fn from(e: MoneyError) -> Self {
        RiskError::Money(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum RiskReject {
    #[error("symbol not allowed for group")]
    SymbolNotAllowed,
    #[error("invalid volume")]
    InvalidVolume,
    #[error("volume exceeds group limit")]
    VolumeLimit,
    #[error("net exposure limit exceeded")]
    ExposureLimit,
    #[error("requested price outside tolerance")]
    PriceOffQuote,
    #[error("insufficient free margin")]
    InsufficientMargin,
    #[error("risk error: {0}")]
    Error(RiskError),
}

impl From<RiskError> for RiskReject {
    fn from(e: RiskError) -> Self {
        RiskReject::Error(e)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct Quote {
    pub bid: Price,
    pub ask: Price,
}

impl Quote {
    pub fn mid(&self) -> Price {
        Price::from_raw(((self.bid.raw() as i128 + self.ask.raw() as i128) / 2) as i64)
    }
    /// Price at which a trade on `side` executes (buy at ask, sell at bid).
    pub fn for_side(&self, side: Side) -> Price {
        match side {
            Side::Buy => self.ask,
            Side::Sell => self.bid,
        }
    }
    /// Widens the quote by `points` on each side.
    pub fn with_markup(&self, markup: Price) -> Quote {
        self.with_markups(markup, markup)
    }
    /// Widens the quote by `bid` points below and `ask` points above.
    pub fn with_markups(&self, bid: Price, ask: Price) -> Quote {
        Quote {
            bid: Price::from_raw(self.bid.raw() - bid.raw()),
            ask: Price::from_raw(self.ask.raw() + ask.raw()),
        }
    }
}

/// Latest quotes by symbol; also used for cross-currency conversion.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct QuoteBook {
    quotes: BTreeMap<String, Quote>,
}

impl QuoteBook {
    pub fn new() -> QuoteBook {
        QuoteBook::default()
    }
    pub fn set(&mut self, symbol: &str, q: Quote) {
        self.quotes.insert(symbol.to_string(), q);
    }
    pub fn get(&self, symbol: &str) -> Option<Quote> {
        self.quotes.get(symbol).copied()
    }
    fn pair(&self, a: Currency, b: Currency) -> Option<Quote> {
        self.quotes.get(&format!("{a}{b}")).copied()
    }

    fn direct(&self, m: Money, to: Currency, mode: Rounding) -> Option<Result<Money, MoneyError>> {
        if m.currency == to {
            return Some(Ok(m));
        }
        if let Some(q) = self.pair(m.currency, to) {
            return Some(m.convert_mul(q.mid(), to, mode));
        }
        self.pair(to, m.currency)
            .map(|q| m.convert_div(q.mid(), to, mode))
    }

    /// Converts using mid prices: direct pair, inverse pair, or via USD.
    pub fn convert(&self, m: Money, to: Currency, mode: Rounding) -> Result<Money, RiskError> {
        if let Some(r) = self.direct(m, to, mode) {
            return Ok(r?);
        }
        let usd = Currency::USD;
        if m.currency != usd && to != usd {
            if let Some(Ok(mid)) = self.direct(m, usd, mode) {
                if let Some(r) = self.direct(mid, to, mode) {
                    return Ok(r?);
                }
            }
        }
        Err(RiskError::NoConversion(m.currency, to))
    }
}

/// Minimal position view used by risk calculations.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct PositionView {
    pub symbol: String,
    pub side: Side,
    pub volume: Qty,
    pub open_price: Price,
}

/// P&L in profit currency, scaled by 1e8.
pub fn pnl_scaled(side: Side, open: Price, close: Price, volume: Qty, contract_size: i64) -> i128 {
    let dp = (close.raw() as i128 - open.raw() as i128) * side.sign() as i128;
    dp * volume.raw() as i128 * contract_size as i128 / SCALE as i128
}

/// P&L of a (partial) position in the account currency.
pub fn pnl_money(
    spec: &SymbolSpec,
    side: Side,
    open: Price,
    close: Price,
    volume: Qty,
    account_ccy: Currency,
    quotes: &QuoteBook,
) -> Result<Money, RiskError> {
    let raw = pnl_scaled(side, open, close, volume, spec.contract_size);
    let m = Money::from_scaled(raw, spec.quote, Rounding::HalfEven)?;
    quotes.convert(m, account_ccy, Rounding::HalfEven)
}

/// Floating P&L of a position at the current (client) quote.
pub fn floating_pnl(
    spec: &SymbolSpec,
    pos: &PositionView,
    quote: Quote,
    account_ccy: Currency,
    quotes: &QuoteBook,
) -> Result<Money, RiskError> {
    let close = quote.for_side(pos.side.opposite());
    pnl_money(
        spec,
        pos.side,
        pos.open_price,
        close,
        pos.volume,
        account_ccy,
        quotes,
    )
}

/// Margin for one symbol given the total long and short lots.
pub fn symbol_margin(
    group: &GroupConfig,
    spec: &SymbolSpec,
    long: Qty,
    short: Qty,
    quotes: &QuoteBook,
) -> Result<Money, RiskError> {
    let (l, s) = (long.raw() as i128, short.raw() as i128);
    let hedged = l.min(s);
    let unhedged = (l - s).abs();
    let lots = match group.margin_mode {
        MarginMode::Netting => unhedged,
        MarginMode::Hedging => unhedged * 10_000 + 2 * hedged * group.hedged_margin_bps as i128,
    };
    let lots_div = match group.margin_mode {
        MarginMode::Netting => 1,
        MarginMode::Hedging => 10_000,
    };
    if lots == 0 {
        return Ok(Money::zero(group.currency));
    }
    let lev = group.effective_leverage(spec) as i128;
    // base units scaled 1e8
    let mut units = lots * spec.contract_size as i128 / lots_div;
    if spec.margin_currency != spec.base {
        let q = quotes
            .get(&spec.symbol)
            .ok_or_else(|| RiskError::NoQuote(spec.symbol.clone()))?;
        units = units * q.mid().raw() as i128 / SCALE as i128;
    }
    let m = Money::from_scaled(
        money::div_round(units, lev, Rounding::Ceiling)
            .ok_or(RiskError::Money("overflow".into()))?,
        spec.margin_currency,
        Rounding::Ceiling,
    )?;
    quotes.convert(m, group.currency, Rounding::Ceiling)
}

/// Total margin across all positions.
pub fn total_margin<'a>(
    group: &GroupConfig,
    specs: &BTreeMap<String, SymbolSpec>,
    positions: impl IntoIterator<Item = &'a PositionView>,
    quotes: &QuoteBook,
) -> Result<Money, RiskError> {
    let mut per: BTreeMap<&str, (Qty, Qty)> = BTreeMap::new();
    for p in positions {
        let e = per.entry(p.symbol.as_str()).or_default();
        match p.side {
            Side::Buy => e.0 = Qty::from_raw(e.0.raw() + p.volume.raw()),
            Side::Sell => e.1 = Qty::from_raw(e.1.raw() + p.volume.raw()),
        }
    }
    let mut total = Money::zero(group.currency);
    for (sym, (l, s)) in per {
        let spec = specs
            .get(sym)
            .ok_or_else(|| RiskError::NoQuote(sym.into()))?;
        total = total.checked_add(symbol_margin(group, spec, l, s, quotes)?)?;
    }
    Ok(total)
}

/// Snapshot of an account's risk figures (account currency).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct AccountRisk {
    pub balance: Money,
    pub floating: Money,
    pub equity: Money,
    pub margin: Money,
    pub free_margin: Money,
}

impl AccountRisk {
    pub fn new(balance: Money, floating: Money, margin: Money) -> Result<AccountRisk, RiskError> {
        let equity = balance.checked_add(floating)?;
        Ok(AccountRisk {
            balance,
            floating,
            equity,
            margin,
            free_margin: equity.checked_sub(margin)?,
        })
    }
    /// Margin level in hundredths of a percent; `None` when no margin is used.
    pub fn margin_level_x100(&self) -> Option<i128> {
        (self.margin.minor > 0).then(|| self.equity.minor * 10_000 / self.margin.minor)
    }
    fn at_or_below(&self, pct: i64) -> bool {
        self.margin.minor > 0 && self.equity.minor * 100 <= pct as i128 * self.margin.minor
    }
    pub fn is_margin_call(&self, g: &GroupConfig) -> bool {
        self.at_or_below(g.margin_call_pct)
    }
    pub fn is_stop_out(&self, g: &GroupConfig) -> bool {
        self.at_or_below(g.stop_out_pct)
    }
}

/// An order to check before acceptance.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OrderIntent<'a> {
    pub symbol: &'a str,
    pub side: Side,
    pub volume: Qty,
    /// Client-requested price for market orders (slippage guard).
    pub requested_price: Option<Price>,
    /// Price used for margin estimation (limit/stop price or market).
    pub price: Option<Price>,
}

/// Volume validity vs symbol spec and group.
pub fn check_volume(group: &GroupConfig, spec: &SymbolSpec, v: Qty) -> Result<(), RiskReject> {
    if v.raw() <= 0
        || v < spec.min_lot
        || v > spec.max_lot
        || v.raw() % spec.lot_step.raw().max(1) != 0
    {
        return Err(RiskReject::InvalidVolume);
    }
    if v > group.max_volume_per_order {
        return Err(RiskReject::VolumeLimit);
    }
    Ok(())
}

/// Full pre-trade check. `quote` is the client (marked-up) quote.
#[allow(clippy::too_many_arguments)]
pub fn pre_trade_check(
    group: &GroupConfig,
    specs: &BTreeMap<String, SymbolSpec>,
    balance: Money,
    positions: &[PositionView],
    order: &OrderIntent<'_>,
    quotes: &QuoteBook,
    client_quotes: &BTreeMap<String, Quote>,
    reduces_only: bool,
) -> Result<AccountRisk, RiskReject> {
    if !group.symbol_allowed(order.symbol) {
        return Err(RiskReject::SymbolNotAllowed);
    }
    let spec = specs
        .get(order.symbol)
        .ok_or(RiskReject::SymbolNotAllowed)?;
    check_volume(group, spec, order.volume)?;
    let q = *client_quotes
        .get(order.symbol)
        .ok_or_else(|| RiskError::NoQuote(order.symbol.into()))?;
    let market = q.for_side(order.side);
    if let Some(req) = order.requested_price {
        let tol = spec.point().raw() as i128 * group.price_tolerance_points as i128;
        if (req.raw() as i128 - market.raw() as i128).abs() > tol {
            return Err(RiskReject::PriceOffQuote);
        }
    }
    let net: i64 = positions
        .iter()
        .filter(|p| p.symbol == order.symbol)
        .map(|p| p.volume.raw() * p.side.sign())
        .sum::<i64>()
        + order.volume.raw() * order.side.sign();
    if !reduces_only && net.abs() > group.max_net_volume.raw() {
        return Err(RiskReject::ExposureLimit);
    }
    let mut floating = Money::zero(group.currency);
    for p in positions {
        let s = specs.get(&p.symbol).ok_or(RiskReject::SymbolNotAllowed)?;
        let pq = *client_quotes
            .get(&p.symbol)
            .ok_or_else(|| RiskError::NoQuote(p.symbol.clone()))?;
        floating = floating
            .checked_add(floating_pnl(s, p, pq, group.currency, quotes)?)
            .map_err(RiskError::from)?;
    }
    let current_margin = total_margin(group, specs, positions, quotes)?;
    let mut with = positions.to_vec();
    with.push(PositionView {
        symbol: order.symbol.into(),
        side: order.side,
        volume: order.volume,
        open_price: order.price.unwrap_or(market),
    });
    let new_margin = total_margin(group, specs, &with, quotes)?;
    let after = AccountRisk::new(balance, floating, new_margin)?;
    // Orders that do not increase margin are always allowed.
    if new_margin.minor > current_margin.minor && after.free_margin.is_negative() {
        return Err(RiskReject::InsufficientMargin);
    }
    Ok(after)
}

/// Liquidation order for stop-out: largest loss first (ties by id).
pub fn liquidation_order<Id: Ord + Copy>(items: &[(Id, Money)]) -> Vec<Id> {
    let mut v: Vec<_> = items.to_vec();
    v.sort_by(|a, b| a.1.minor.cmp(&b.1.minor).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(id, _)| id).collect()
}

/// Amount to credit back so a negative balance returns to zero.
pub fn negative_balance_compensation(group: &GroupConfig, balance: Money) -> Option<Money> {
    (group.negative_balance_protection && balance.is_negative()).then(|| balance.abs())
}

// ---------------------------------------------------------------------------
// Stage 7: client flow profile and B-book exposure / auto-hedge policy
// ---------------------------------------------------------------------------

/// Per-client flow statistics the engine keeps from fills and closing deals.
/// Feeds the toxicity score (rule-engine input and console profile).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FlowStats {
    /// Closing deals.
    pub trades: u64,
    /// Closing deals of positions held less than 60 s.
    pub short_holds: u64,
    pub wins: u64,
    pub hold_secs_sum: u64,
    /// Client realised P&L (minor units of the account currency).
    pub pnl_minor: i128,
    /// Our result on those deals (markup / B-book), minor units.
    pub broker_pnl_minor: i128,
    /// Fills with a requested price.
    pub fills: u64,
    /// Σ (requested − fill) × side / point: positive = the client got a
    /// better price than requested (latency arbitrage captures improvements).
    pub slip_gain_points_sum: i64,
}

impl FlowStats {
    pub const SHORT_HOLD_SECS: u64 = 60;
    /// Below this many closed trades the score is 0 (not enough evidence).
    pub const MIN_TRADES: u64 = 5;

    pub fn record_fill(&mut self, gain_points: i64) {
        self.fills += 1;
        self.slip_gain_points_sum = self.slip_gain_points_sum.saturating_add(gain_points);
    }

    pub fn record_close(&mut self, hold_secs: u64, pnl_minor: i128, broker_pnl_minor: i128) {
        self.trades += 1;
        if hold_secs < Self::SHORT_HOLD_SECS {
            self.short_holds += 1;
        }
        if pnl_minor > 0 {
            self.wins += 1;
        }
        self.hold_secs_sum = self.hold_secs_sum.saturating_add(hold_secs);
        self.pnl_minor += pnl_minor;
        self.broker_pnl_minor += broker_pnl_minor;
    }

    pub fn short_hold_ratio(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.short_holds as f64 / self.trades as f64
        }
    }

    pub fn win_rate(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.wins as f64 / self.trades as f64
        }
    }

    pub fn avg_hold_secs(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.hold_secs_sum as f64 / self.trades as f64
        }
    }

    pub fn avg_slip_gain_points(&self) -> f64 {
        if self.fills == 0 {
            0.0
        } else {
            self.slip_gain_points_sum as f64 / self.fills as f64
        }
    }

    /// 0..100 toxic-flow score: 45 % short holds (scalping), 30 % win rate
    /// above 50 %, 25 % captured price improvement (≥ 5 points = max).
    pub fn toxicity(&self) -> u8 {
        if self.trades < Self::MIN_TRADES {
            return 0;
        }
        let short = self.short_hold_ratio();
        let win = ((self.win_rate() - 0.5) * 2.0).clamp(0.0, 1.0);
        let gain = (self.avg_slip_gain_points() / 5.0).clamp(0.0, 1.0);
        (45.0 * short + 30.0 * win + 25.0 * gain)
            .round()
            .clamp(0.0, 100.0) as u8
    }
}

/// What happens when B-book exposure exceeds a limit.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum HedgeMode {
    /// New risk-increasing orders of the symbol go A-book while over the limit.
    #[default]
    SwitchToABook,
    /// Keep the client flow B-book and hedge the excess at the LP (omnibus
    /// hedge book, unwound when exposure falls back under `release_pct`).
    HedgeExcess,
}

/// B-book exposure limits and the automatic hedge (console: Risk → Hedge).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HedgePolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: HedgeMode,
    /// |net B-book lots| per symbol; `None` = no symbol limit.
    #[serde(default)]
    pub default_symbol_limit: Option<Qty>,
    #[serde(default)]
    pub symbol_limits: BTreeMap<String, Qty>,
    /// Σ |net B-book lots| over all symbols.
    #[serde(default)]
    pub total_limit: Option<Qty>,
    /// |net B-book lots| of one client on one symbol (SwitchToABook only).
    #[serde(default)]
    pub account_limit: Option<Qty>,
    /// HedgeExcess: share of the excess that is hedged.
    #[serde(default = "hundred")]
    pub hedge_ratio_pct: u8,
    /// HedgeExcess: the hedge is unwound once |net| ≤ limit × release %.
    #[serde(default = "eighty")]
    pub release_pct: u8,
}

fn hundred() -> u8 {
    100
}
fn eighty() -> u8 {
    80
}

impl Default for HedgePolicy {
    fn default() -> HedgePolicy {
        HedgePolicy {
            enabled: false,
            mode: HedgeMode::SwitchToABook,
            default_symbol_limit: None,
            symbol_limits: BTreeMap::new(),
            total_limit: None,
            account_limit: None,
            hedge_ratio_pct: 100,
            release_pct: 80,
        }
    }
}

impl HedgePolicy {
    pub fn symbol_limit(&self, symbol: &str) -> Option<Qty> {
        self.symbol_limits
            .get(symbol)
            .copied()
            .or(self.default_symbol_limit)
    }

    pub fn validate(&self) -> Result<(), String> {
        let pos = |q: Option<Qty>, what: &str| {
            if q.is_some_and(|q| q.raw() <= 0) {
                Err(format!("{what} must be positive"))
            } else {
                Ok(())
            }
        };
        pos(self.default_symbol_limit, "defaultSymbolLimit")?;
        pos(self.total_limit, "totalLimit")?;
        pos(self.account_limit, "accountLimit")?;
        if self.symbol_limits.len() > 500 {
            return Err("too many symbol limits".into());
        }
        for (s, q) in &self.symbol_limits {
            if s.is_empty() || s.len() > 16 {
                return Err("symbol name 1..16 characters".into());
            }
            pos(Some(*q), "symbol limit")?;
        }
        if self.hedge_ratio_pct == 0 || self.hedge_ratio_pct > 100 {
            return Err("hedgeRatioPct must be 1..100".into());
        }
        if self.release_pct > 100 {
            return Err("releasePct must be 0..100".into());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Stage 8: swap / rollover
// ---------------------------------------------------------------------------

/// How a symbol's swap rates are expressed.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum SwapMode {
    /// Profit-currency amount per lot per day.
    #[default]
    Money,
    /// Points of the quote per day (× point × contract size × lots).
    Points,
}

fn default_triple_day() -> u8 {
    3
}

/// When the daily rollover runs (console: Settings → Swap).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SwapConfig {
    #[serde(default = "yes_bool")]
    pub enabled: bool,
    /// UTC hour of the daily rollover (22 = 17:00 New York in summer).
    #[serde(default = "default_rollover_hour")]
    pub rollover_hour_utc: u8,
    /// No rollover on Saturday / Sunday (the triple day covers the weekend).
    #[serde(default = "yes_bool")]
    pub skip_weekend: bool,
}

fn yes_bool() -> bool {
    true
}
fn default_rollover_hour() -> u8 {
    22
}

impl Default for SwapConfig {
    fn default() -> SwapConfig {
        SwapConfig {
            enabled: true,
            rollover_hour_utc: 22,
            skip_weekend: true,
        }
    }
}

impl SwapConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.rollover_hour_utc > 23 {
            return Err("rolloverHourUtc must be 0..23".into());
        }
        Ok(())
    }
}

/// Weekday of a UNIX timestamp (ns), 0 = Sunday .. 6 = Saturday.
pub fn weekday_utc(ts_ns: u64) -> u8 {
    let day = ts_ns / 86_400_000_000_000;
    ((day + 4) % 7) as u8 // 1970-01-01 was a Thursday
}

/// Swap of one position for one day in the symbol's profit currency, scaled
/// 1e8 (negative = charged): the symbol rate × lots × the group's multiplier.
pub fn swap_scaled(spec: &SymbolSpec, side: Side, volume: Qty, multiplier_pct: u32) -> i128 {
    let rate = match side {
        Side::Buy => spec.swap_long,
        Side::Sell => spec.swap_short,
    }
    .raw() as i128;
    let per_lot = match spec.swap_mode {
        SwapMode::Money => rate,
        SwapMode::Points => {
            rate * spec.point().raw() as i128 / SCALE as i128 * spec.contract_size as i128
        }
    };
    per_lot * volume.raw() as i128 / SCALE as i128 * multiplier_pct as i128 / 100
}

#[cfg(test)]
mod tests;
