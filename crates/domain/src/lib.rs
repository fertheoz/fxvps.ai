//! Venue-agnostic domain types shared by gateways, OMS and simulators.
//!
//! All monetary values use [`Fixed`] (i64, 8 decimals). No `f64` for money.

pub mod fixed;

pub use fixed::{Fixed, FixedParseError, Price, Qty};

use serde::{Deserialize, Serialize};

/// Tradable instrument as configured for a given liquidity provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instrument {
    /// Internal symbol, e.g. `EUR/USD`.
    pub symbol: String,
    /// LP-side identifier sent in `SecurityID(48)`, e.g. `4001` for LMAX EUR/USD.
    pub security_id: String,
    /// Minimum price increment.
    pub tick_size: Price,
    /// Minimum quantity increment (contract/lot step at the LP).
    #[serde(default = "default_qty_step")]
    pub qty_step: Qty,
    /// Units of the base currency per 1 `OrderQty(38)` at the LP: `1` when the LP
    /// quotes quantities in units (lp-simulator), `10000` for LMAX FX contracts.
    #[serde(default = "default_contract_size")]
    pub contract_size: i64,
}

fn default_qty_step() -> Qty {
    Fixed::from_parts(1, 2)
}

fn default_contract_size() -> i64 {
    1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeInForce {
    Day,
    GoodTillCancel,
    ImmediateOrCancel,
    FillOrKill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecType {
    New,
    Trade,
    Canceled,
    Replaced,
    Rejected,
    Expired,
    OrderStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Canceled,
    Replaced,
    Rejected,
    Expired,
}

impl OrderStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            OrderStatus::Filled
                | OrderStatus::Canceled
                | OrderStatus::Rejected
                | OrderStatus::Expired
        )
    }
}

/// One side of top-of-book.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level {
    pub price: Price,
    pub qty: Qty,
}

/// Normalized quote (top of book plus optional depth) from one LP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    pub lp: String,
    pub symbol: String,
    /// Bids, best first.
    pub bids: Vec<Level>,
    /// Asks, best first.
    pub asks: Vec<Level>,
    /// Gateway receive time, nanoseconds since UNIX epoch.
    pub ts_recv_ns: u64,
}

impl Quote {
    pub fn best_bid(&self) -> Option<Level> {
        self.bids.first().copied()
    }
    pub fn best_ask(&self) -> Option<Level> {
        self.asks.first().copied()
    }
}

/// Order as requested by an internal component (OMS, risk, tests).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    /// Client order id; idempotency key towards the LP.
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: Side,
    pub qty: Qty,
    pub ord_type: OrderType,
    pub limit_price: Option<Price>,
    pub tif: TimeInForce,
}

/// Normalized execution report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    pub lp: String,
    pub symbol: String,
    pub order_id: String,
    pub cl_ord_id: Option<String>,
    pub orig_cl_ord_id: Option<String>,
    pub exec_id: String,
    pub exec_type: ExecType,
    pub status: OrderStatus,
    pub side: Side,
    pub last_qty: Option<Qty>,
    pub last_px: Option<Price>,
    pub cum_qty: Qty,
    pub leaves_qty: Qty,
    pub avg_px: Option<Price>,
    pub text: Option<String>,
    pub ts_recv_ns: u64,
}

/// Current wall clock in nanoseconds since UNIX epoch.
pub fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
