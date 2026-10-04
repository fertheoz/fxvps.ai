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

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum MarginMode {
    /// Independent positions per fill, opposite positions may coexist.
    Hedging,
    /// One net position per symbol.
    Netting,
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
    /// Swap per lot per day in profit currency (signed, scaled 1e8).
    pub swap_long: Price,
    pub swap_short: Price,
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
        Quote {
            bid: Price::from_raw(self.bid.raw() - markup.raw()),
            ask: Price::from_raw(self.ask.raw() + markup.raw()),
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

#[cfg(test)]
mod tests;
