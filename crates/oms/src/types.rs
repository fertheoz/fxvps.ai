//! Engine types: orders, positions, accounts, commands and events.

use ledger::AccountId;
use money::{Money, Price, Qty};
pub use risk::{GroupConfig, MarginMode, Routing, Side, SymbolSpec};
use serde::{Deserialize, Serialize};

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
}

impl Order {
    pub fn remaining(&self) -> Qty {
        Qty::from_raw(self.req.volume.raw() - self.filled.raw())
    }
    pub fn is_pending(&self) -> bool {
        self.req.order_type != OrderType::Market
            && !self.working
            && matches!(self.status, OrderStatus::Accepted)
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
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Command {
    AddSymbol(SymbolSpec),
    SetGroup(GroupConfig),
    OpenAccount {
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
    /// Sends aggregated A-book orders (when aggregation is enabled).
    FlushLp,
    /// Daily rollover: charge/credit swaps.
    Rollover,
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
    OrderFilled {
        order_id: OrderId,
        volume: Qty,
        price: Price,
        status: OrderStatus,
    },
    OrderCancelled {
        order_id: OrderId,
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
}
