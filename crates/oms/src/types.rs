//! Engine types: orders, positions, accounts, commands and events.

use ledger::AccountId;
use money::{Money, Price, Qty};
pub use risk::{
    FlowStats, GroupConfig, HedgeMode, HedgePolicy, MarginMode, PartialFill, Routing, RoutingRule,
    Side, SwapConfig, SwapMode, SymbolSpec, TradingCalendar, TradingSession,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type AccountNo = u64;
pub type OrderId = u64;
pub type PositionId = u64;
pub type LpOrderId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    StopLimit,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum OrderStatus {
    New,
    Accepted,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Expired,
}

impl OrderStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            OrderStatus::Filled
                | OrderStatus::Cancelled
                | OrderStatus::Rejected
                | OrderStatus::Expired
        )
    }

    /// Legal lifecycle transitions.
    pub fn can_transition(self, to: OrderStatus) -> bool {
        use OrderStatus::*;
        matches!(
            (self, to),
            (New, Accepted)
                | (New, Rejected)
                | (Accepted, PartiallyFilled)
                | (Accepted, Filled)
                | (Accepted, Cancelled)
                | (Accepted, Expired)
                | (Accepted, Rejected)
                | (PartiallyFilled, PartiallyFilled)
                | (PartiallyFilled, Filled)
                | (PartiallyFilled, Cancelled)
        )
    }
}

/// Client request to place an order.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct NewOrder {
    pub account: AccountNo,
    /// Idempotency key, unique per account.
    pub client_order_id: String,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub volume: Qty,
    pub limit_price: Option<Price>,
    pub stop_price: Option<Price>,
    /// Client-seen price for market orders (slippage tolerance check).
    pub requested_price: Option<Price>,
    pub sl: Option<Price>,
    pub tp: Option<Price>,
    /// Trailing stop distance in points.
    pub trailing_points: Option<i64>,
    pub oco_group: Option<u64>,
    /// Expiry timestamp (ns) for pending orders.
    pub expire_at: Option<u64>,
    /// Where the order came from and the client's IP (rule conditions;
    /// journaled with the order).
    #[serde(default)]
    pub platform: risk::Platform,
    #[serde(default)]
    pub ip: Option<String>,
    /// Market orders: client slippage tolerance in points (MT5 deviation);
    /// the tighter of this and the group cap / circuit breaker applies.
    #[serde(default)]
    pub max_deviation_points: Option<i64>,
}

impl NewOrder {
    pub fn market(
        account: AccountNo,
        clid: &str,
        symbol: &str,
        side: Side,
        volume: Qty,
    ) -> NewOrder {
        NewOrder {
            account,
            client_order_id: clid.into(),
            symbol: symbol.into(),
            side,
            order_type: OrderType::Market,
            volume,
            limit_price: None,
            stop_price: None,
            requested_price: None,
            sl: None,
            tp: None,
            trailing_points: None,
            oco_group: None,
            expire_at: None,
            platform: risk::Platform::Unknown,
            ip: None,
            max_deviation_points: None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct Order {
    pub id: OrderId,
    pub req: NewOrder,
    pub status: OrderStatus,
    pub filled: Qty,
    pub avg_price: Price,
    pub routing: Routing,
    /// Set for orders that close (part of) an existing position.
    pub close_position: Option<PositionId>,
    /// Position opened/increased by this order (hedging mode).
    pub position: Option<PositionId>,
    /// Stop-limit order whose stop has triggered.
    pub stop_triggered: bool,
    /// Order is executing (routed to LP / being filled).
    pub working: bool,
    pub reject_reason: Option<String>,
    pub created_ts: u64,
    /// Who created the order (client or a server-side close).
    #[serde(default)]
    pub origin: OrderOrigin,
    /// Client price at which the last LP limit attempt went unfilled: the order
    /// re-arms only when the market improves on it (no IOC storm on one tick).
    #[serde(default)]
    pub rearm_px: Option<Price>,
    /// LP orders this order has been part of (retries and re-arms send again).
    #[serde(default)]
    pub lp_attempts: u32,
    /// Routing rule that decided this order (see `RoutingRule`), with its overrides.
    #[serde(default)]
    pub rule: Option<String>,
    #[serde(default)]
    pub markup_override: Option<i64>,
    #[serde(default)]
    pub max_slippage_override: Option<i64>,
    #[serde(default)]
    pub partial_fill_override: Option<PartialFill>,
    /// Copy trading: (provider account, provider position) this order mirrors.
    #[serde(default)]
    pub copy_from: Option<(AccountNo, PositionId)>,
    /// Pending limit entry resting at the LP (`GroupConfig::lp_resting`).
    #[serde(default)]
    pub lp_resting: Option<LpOrderId>,
    /// LP fills of a market order's retry chain held back until the chain is
    /// over: the client then gets one fill at their VWAP (tek kalem).
    #[serde(default)]
    pub chain_fills: Vec<LpExec>,
    /// Last look: held until this instant (ns), then judged and executed.
    #[serde(default)]
    pub held_until: Option<u64>,
}

/// Origin of an order (deal reason in the history).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum OrderOrigin {
    #[default]
    Client,
    StopLoss,
    TakeProfit,
    StopOut,
    /// Opened / closed by copy trading on behalf of a follower.
    Copy,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum DealEntry {
    /// Opened / increased a position.
    In,
    /// Reduced / closed a position.
    Out,
}

/// One execution against a position (history record).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct Deal {
    pub id: u64,
    pub order_id: OrderId,
    pub account: AccountNo,
    pub position_id: PositionId,
    pub symbol: String,
    pub side: Side,
    pub entry: DealEntry,
    pub volume: Qty,
    pub price: Price,
    /// Realized P&L (zero for `In`), account currency.
    pub pnl: Money,
    /// Commission charged on this deal (negative = cost), account currency.
    pub commission: Money,
    pub ts: u64,
    pub reason: OrderOrigin,
    /// Price of this execution at the LP (A-book only).
    #[serde(default)]
    pub lp_price: Option<Price>,
    /// Broker result of a closing deal, minor units of the account currency: the
    /// markup on the A-book, the opposite of the client P&L on the B-book.
    #[serde(default)]
    pub broker_pnl: i128,
    /// Our own result at the LP for a closing A-book deal (minor units).
    #[serde(default)]
    pub lp_pnl: i128,
    /// Swap released with this closing deal (minor units, negative = cost).
    #[serde(default)]
    pub swap: i128,
    /// Part of `swap` that is the swap-free admin fee (broker revenue on
    /// either book; the rest of an A-book swap passes through to the LP).
    #[serde(default)]
    pub swap_fee: i128,
}

/// Kind of an engine-made balance move that is neither a deal nor an admin
/// cash operation.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum CashMoveKind {
    /// Copy trading performance fee: debit on the follower.
    CopyFee,
    /// The same fee credited to the strategy provider.
    CopyFeeIncome,
    /// Negative balance protection: the broker covered a negative balance.
    NegativeBalanceCompensation,
}

/// Balance move booked by the engine itself (statement history).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct CashMove {
    pub account: AccountNo,
    pub kind: CashMoveKind,
    /// Signed amount in the account currency (negative = debit).
    pub amount: Money,
    /// The other account of a copy fee (provider / follower).
    #[serde(default)]
    pub counterparty: Option<AccountNo>,
    pub ts: u64,
}

/// New parameters of a pending order (`Command::ModifyOrder`); every field
/// is the full new value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct OrderChange {
    pub volume: Qty,
    pub limit_price: Option<Price>,
    pub stop_price: Option<Price>,
    pub sl: Option<Price>,
    pub tp: Option<Price>,
    pub trailing_points: Option<i64>,
    pub expire_at: Option<u64>,
}

impl Order {
    pub fn remaining(&self) -> Qty {
        Qty::from_raw(self.req.volume.raw() - self.filled.raw())
    }
    /// What still has to go to the LP: the remainder less the chain fills
    /// already in hand (not yet applied to the client).
    pub fn lp_open(&self) -> Qty {
        let held: i64 = self.chain_fills.iter().map(|f| f.volume.raw()).sum();
        Qty::from_raw(self.req.volume.raw() - self.filled.raw() - held)
    }
    /// Waiting for its trigger price (also a limit that came back from the LP
    /// partly filled: the remainder keeps waiting).
    pub fn is_pending(&self) -> bool {
        self.req.order_type != OrderType::Market
            && !self.working
            && matches!(
                self.status,
                OrderStatus::Accepted | OrderStatus::PartiallyFilled
            )
    }

    /// The order's limit price, if it has a limit leg (limit, stop-limit).
    pub fn limit_leg(&self) -> Option<Price> {
        match self.req.order_type {
            OrderType::Limit | OrderType::StopLimit => self.req.limit_price,
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct Position {
    pub id: PositionId,
    pub account: AccountNo,
    pub symbol: String,
    pub side: Side,
    pub volume: Qty,
    /// Volume with an outstanding close order.
    pub closing: Qty,
    /// Client open price (incl. markup).
    pub open_price: Price,
    /// LP open price (A-book) or same as `open_price` (B-book).
    pub lp_open_price: Price,
    pub sl: Option<Price>,
    pub tp: Option<Price>,
    pub trailing_points: Option<i64>,
    pub routing: Routing,
    pub opened_ts: u64,
    /// LP-resting order standing for the TP (`GroupConfig::lp_resting`).
    #[serde(default)]
    pub lp_tp: Option<LpOrderId>,
    /// LP-resting STOP order standing for the SL.
    #[serde(default)]
    pub lp_sl: Option<LpOrderId>,
    /// Accumulated swap (minor units of the account currency, negative = charged).
    #[serde(default)]
    pub swap_minor: i128,
    /// Part of `swap_minor` that is the swap-free admin fee: broker revenue on
    /// either book, never settled with the LP.
    #[serde(default)]
    pub swap_fee_minor: i128,
    /// Copy trading: (provider account, provider position) this position mirrors.
    #[serde(default)]
    pub copy_from: Option<(AccountNo, PositionId)>,
}

/// A follower copying a strategy provider (copy trading / MAM).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct CopySubscription {
    pub follower: AccountNo,
    pub provider: AccountNo,
    /// Follower volume = provider volume x ratio_bps / 10 000 (rounded down to the lot step).
    pub ratio_bps: u32,
    /// Stop copying and close the copies when equity falls this many percent
    /// below the equity at subscription (0 = off).
    pub equity_stop_pct: u32,
    /// Performance fee on new copy profit above the high-water mark (bps).
    pub perf_fee_bps: u32,
    pub since_ts: u64,
    /// Follower equity at subscription (minor units).
    pub start_equity: i128,
    /// Realized copy result incl. commission and swap (minor units).
    pub realized: i128,
    /// High-water mark of `realized` already charged (minor units).
    pub hwm: i128,
    /// Fees paid to the provider so far (minor units).
    pub fees_paid: i128,
    /// Provider position -> follower lots already sent (raw). The unfilled part
    /// of a copy open that ends rejected or cancelled is taken back off.
    pub copied: BTreeMap<PositionId, i64>,
    /// Provider position -> back-off after copy orders that did not fill.
    #[serde(default)]
    pub retry: BTreeMap<PositionId, CopyRetry>,
    pub active: bool,
    #[serde(default)]
    pub stopped_reason: Option<String>,
    /// Stopped with close: the copies are closed (also opens still in flight
    /// at the stop, once they fill) until none is left.
    #[serde(default)]
    pub closing: bool,
}

/// Back-off of the copy orders of one provider position. Opens and closes
/// back off separately: a failing open never holds back mirroring the
/// provider's reduce or close.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub struct CopyRetry {
    /// Copy opens in a row that ended without filling.
    pub fails: u32,
    /// No copy open for the position before this time (ns).
    pub next_ts: u64,
    /// Copy closes in a row that ended without filling.
    #[serde(default)]
    pub close_fails: u32,
    /// No copy close for the position before this time (ns).
    #[serde(default)]
    pub close_next_ts: u64,
}

impl CopyRetry {
    pub fn is_clear(&self) -> bool {
        self.fails == 0 && self.close_fails == 0
    }
}

impl Position {
    pub fn view(&self) -> risk::PositionView {
        risk::PositionView {
            symbol: self.symbol.clone(),
            side: self.side,
            volume: self.volume,
            open_price: self.open_price,
        }
    }
    pub fn free_volume(&self) -> Qty {
        Qty::from_raw(self.volume.raw() - self.closing.raw())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct Account {
    pub id: AccountNo,
    pub group: String,
    pub ledger_id: AccountId,
    pub margin_call: bool,
}

/// An order sent to the LP for the omnibus account, with the client orders
/// it is allocated to.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct LpOrder {
    pub id: LpOrderId,
    pub symbol: String,
    pub side: Side,
    pub volume: Qty,
    pub filled: Qty,
    pub children: Vec<OrderId>,
    pub done: bool,
    /// Executions reported by the LP (back office execution log).
    #[serde(default)]
    pub fills: Vec<LpExec>,
    #[serde(default)]
    pub created_ts: u64,
    #[serde(default)]
    pub reject_reason: Option<String>,
    /// Limit sent to the LP (IOC); `None` = market order.
    #[serde(default)]
    pub limit: Option<Price>,
    /// Raw LP quote at the moment the order went out: the reference for the
    /// slippage the LP inflicted (fill vs. this), as opposed to the client's.
    #[serde(default)]
    pub sent_bid: Option<Price>,
    #[serde(default)]
    pub sent_ask: Option<Price>,
    /// LP that took the order (`Command::LpRouted`); `None` before the
    /// router answered or in single-LP journals written before stage 6.
    #[serde(default)]
    pub lp: Option<String>,
    /// Broker hedge of B-book excess (no client children; see `HedgePolicy`).
    #[serde(default)]
    pub hedge: bool,
    /// Rests at the LP as a GTC limit (`GroupConfig::lp_resting`): a
    /// position's TP (`position` set, child created on the fill) or a pending
    /// limit entry (`children` = the client order).
    #[serde(default)]
    pub resting: bool,
    #[serde(default)]
    pub position: Option<PositionId>,
    /// Cancel/replace count; the LP-side ClOrdID carries it.
    #[serde(default)]
    pub revision: u32,
    /// Stop trigger of a resting STOP order (SL / stop entry at the LP).
    #[serde(default)]
    pub stop: Option<Price>,
    /// Last cancel/replace (ns): price-only replaces are throttled.
    #[serde(default)]
    pub replaced_ts: u64,
}

/// One LP execution report applied to an [`LpOrder`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct LpExec {
    pub exec_id: String,
    pub volume: Qty,
    pub price: Price,
    pub ts: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[allow(clippy::large_enum_variant)] // SetGroup carries the whole group config; replayed, not hot
pub enum Command {
    AddSymbol(SymbolSpec),
    SetGroup(GroupConfig),
    /// Replaces the routing rule table (evaluated top to bottom).
    SetRules(Vec<RoutingRule>),
    OpenAccount {
        account: AccountNo,
        group: String,
    },
    /// Moves an account to another group (same currency; no open positions or
    /// working orders, since the margin mode and routing may change).
    SetAccountGroup {
        account: AccountNo,
        group: String,
    },
    Deposit {
        account: AccountNo,
        amount: Money,
        key: String,
    },
    Withdraw {
        account: AccountNo,
        amount: Money,
        key: String,
    },
    Quote {
        symbol: String,
        bid: Price,
        ask: Price,
    },
    PlaceOrder(NewOrder),
    CancelOrder {
        account: AccountNo,
        order_id: OrderId,
    },
    /// Changes a pending order in place (keeps its id).
    ModifyOrder {
        account: AccountNo,
        order_id: OrderId,
        change: OrderChange,
    },
    ModifyPosition {
        account: AccountNo,
        position_id: PositionId,
        sl: Option<Price>,
        tp: Option<Price>,
        trailing_points: Option<i64>,
    },
    ClosePosition {
        account: AccountNo,
        position_id: PositionId,
        volume: Option<Qty>,
        client_order_id: String,
    },
    LpFill {
        lp_order_id: LpOrderId,
        exec_id: String,
        volume: Qty,
        price: Price,
    },
    LpReject {
        lp_order_id: LpOrderId,
        reason: String,
    },
    /// The router handed the omnibus order to this LP (journaled so a
    /// replay reproduces the per-LP reports without the router).
    LpRouted {
        lp_order_id: LpOrderId,
        lp: String,
    },
    /// B-book exposure limits / auto-hedge policy (stage 7).
    SetHedge(HedgePolicy),
    /// Rollover schedule (stage 8).
    SetSwapConfig(SwapConfig),
    /// Holiday calendar (stage 13).
    SetCalendar(TradingCalendar),
    /// High-impact calendar events (ns) for the rules' news window.
    SetNewsTimes(Vec<u64>),
    /// Sends aggregated A-book orders (when aggregation is enabled).
    FlushLp,
    /// Daily rollover: charge/credit swaps.
    Rollover,
    /// Follower starts copying a provider (hedging follower, same currency).
    CopySubscribe {
        follower: AccountNo,
        provider: AccountNo,
        ratio_bps: u32,
        equity_stop_pct: u32,
        perf_fee_bps: u32,
    },
    /// Stops copying; `close` also closes the open copies.
    CopyUnsubscribe {
        follower: AccountNo,
        provider: AccountNo,
        close: bool,
    },
    /// Charges performance fees (above the high-water mark) to the provider's followers.
    CopySettle {
        provider: AccountNo,
    },
    /// Time advance only (expiry processing).
    Tick,
}

/// Journaled input: everything the engine needs to be deterministic.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct Envelope {
    pub seq: u64,
    pub ts: u64,
    pub cmd: Command,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Event {
    CommandRejected {
        reason: String,
    },
    AccountOpened {
        account: AccountNo,
    },
    AccountGroupChanged {
        account: AccountNo,
        group: String,
    },
    BalanceChanged {
        account: AccountNo,
        balance: Money,
    },
    OrderAccepted {
        order_id: OrderId,
    },
    OrderRejected {
        order_id: OrderId,
        reason: String,
    },
    DuplicateOrder {
        order_id: OrderId,
    },
    OrderTriggered {
        order_id: OrderId,
    },
    /// Last look: the order is held until `until` (ns), then judged.
    OrderHeld {
        order_id: OrderId,
        until: u64,
    },
    /// An LP-resting order was placed (`active`) or cancelled/rejected/
    /// filled away (`!active`).
    LpRestingChanged {
        lp_order_id: LpOrderId,
        active: bool,
    },
    OrderFilled {
        order_id: OrderId,
        volume: Qty,
        price: Price,
        status: OrderStatus,
    },
    OrderCancelled {
        order_id: OrderId,
    },
    OrderModified {
        order_id: OrderId,
    },
    DealAdded {
        deal_id: u64,
    },
    OrderExpired {
        order_id: OrderId,
    },
    LpOrderSent {
        lp_order_id: LpOrderId,
        volume: Qty,
    },
    PositionOpened {
        position_id: PositionId,
    },
    PositionModified {
        position_id: PositionId,
        sl: Option<Price>,
        tp: Option<Price>,
    },
    PositionClosed {
        position_id: PositionId,
        volume: Qty,
        price: Price,
        pnl: Money,
        remaining: Qty,
    },
    MarginCall {
        account: AccountNo,
    },
    StopOut {
        account: AccountNo,
        position_id: PositionId,
    },
    NegativeBalanceCompensated {
        account: AccountNo,
        amount: Money,
    },
    CopyChanged {
        follower: AccountNo,
        provider: AccountNo,
        active: bool,
    },
    CopyFee {
        follower: AccountNo,
        provider: AccountNo,
        amount: Money,
    },
    /// Daily rollover ran (`positions` charged) or was skipped (same day / weekend / disabled).
    Rollover {
        applied: bool,
        positions: u32,
        reason: String,
    },
}

/// EWMA volatility state of one symbol, integers only (replay-safe): mid
/// returns sampled at most once a minute, in 1e-8 units; `var` is the
/// EWMA (λ = 0.94) of their squares, 1e-16 units.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub struct VolState {
    pub last_mid: i64,
    pub last_ts: u64,
    pub var: i128,
    pub samples: u32,
}

impl VolState {
    pub const SAMPLE_NS: u64 = 60_000_000_000;

    /// Feeds a mid price at `now`; a new sample only after a minute.
    pub fn observe(&mut self, mid: i64, now: u64) {
        if self.last_ts == 0 || mid <= 0 {
            self.last_mid = mid;
            self.last_ts = now;
            return;
        }
        if now.saturating_sub(self.last_ts) < Self::SAMPLE_NS || self.last_mid <= 0 {
            return;
        }
        let r = ((mid as i128 - self.last_mid as i128) * 100_000_000) / self.last_mid as i128;
        self.var = (self.var * 94 + r * r * 6) / 100;
        self.samples += 1;
        self.last_mid = mid;
        self.last_ts = now;
    }

    /// Daily volatility (fraction) from per-minute samples: σ_min × √1440.
    pub fn daily_sigma(&self) -> f64 {
        if self.samples < 5 {
            return 0.0;
        }
        (self.var as f64 * 1440.0).sqrt() / 1e8
    }
}
