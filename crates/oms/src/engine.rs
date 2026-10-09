//! The single-writer OMS/risk/ledger state machine.

use crate::alloc::{allocate, AllocationMode};
use crate::router::{LpOrderRequest, LpRouter, NullRouter};
use crate::types::*;
use ledger::{AccountId, AccountKind, Ledger, LedgerSnapshot, Posting, TxnKind, TxnRequest};
use money::{Money, Price, Qty, Rounding, SCALE};
use risk::{
    AccountRisk, GroupCommission, OrderIntent, PartialFill, Quote, QuoteBook, RiskError,
    RoutingRule,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// External bank / PSP account.
pub const BANK: AccountId = AccountId(1);
/// External LP counterparty (P&L and swaps settled by the LP).
pub const LP_COUNTERPARTY: AccountId = AccountId(2);
/// Omnibus master account at the LP.
pub const OMNIBUS: AccountId = AccountId(10);
/// Broker book (revenue, B-book counterparty).
pub const BROKER_BOOK: AccountId = AccountId(11);
const CLIENT_LEDGER_BASE: u64 = 1_000;

/// Fill guard for A-book market orders without a slippage cap: the LP may not
/// fill more than this far (basis points) past the client's requested price.
pub const SLIPPAGE_GUARD_BPS: i64 = 50;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub struct EngineConfig {
    pub allocation: AllocationMode,
    /// Queue A-book market orders and send one LP order per (symbol, side)
    /// on `Command::FlushLp`.
    pub aggregate_a_book: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
struct State {
    /// High-impact calendar events (ns, sorted) for the rules' news window.
    #[serde(default)]
    news_times: Vec<u64>,
    config: EngineConfig,
    symbols: BTreeMap<String, SymbolSpec>,
    groups: BTreeMap<String, GroupConfig>,
    accounts: BTreeMap<AccountNo, Account>,
    orders: BTreeMap<OrderId, Order>,
    /// "account:client_order_id" -> order id
    client_ids: BTreeMap<String, OrderId>,
    positions: BTreeMap<PositionId, Position>,
    lp_orders: BTreeMap<LpOrderId, LpOrder>,
    lp_exec_ids: BTreeSet<String>,
    /// Net LP position per symbol held by the omnibus account (raw lots).
    omnibus_net: BTreeMap<String, i64>,
    pending_lp: Vec<OrderId>,
    /// Index of possibly-pending orders (pruned lazily).
    pending_ids: BTreeSet<OrderId>,
    quotes: QuoteBook,
    next_id: u64,
    next_exec: u64,
    seq: u64,
    now: u64,
    /// Deal history (append only; deal id = index + 1).
    #[serde(default)]
    deals: Vec<Deal>,
    /// Routing rule table, evaluated top to bottom at order entry.
    #[serde(default)]
    rules: Vec<RoutingRule>,
    /// B-book exposure limits / auto-hedge (stage 7).
    #[serde(default)]
    hedge: HedgePolicy,
    /// Broker hedge book at the LP per symbol: signed raw lots, average price
    /// (raw), realised result (minor units of the symbol's quote currency).
    #[serde(default)]
    hedge_net: BTreeMap<String, i64>,
    #[serde(default)]
    hedge_pending: BTreeMap<String, i64>,
    #[serde(default)]
    hedge_avg: BTreeMap<String, i64>,
    #[serde(default)]
    hedge_realized: BTreeMap<String, i128>,
    /// Per-client flow profile (toxicity input).
    #[serde(default)]
    flow: BTreeMap<AccountNo, FlowStats>,
    /// Rollover schedule (stage 8) and the UTC day it last ran (0 = never).
    #[serde(default)]
    swap: SwapConfig,
    /// Holiday calendar (stage 13).
    #[serde(default)]
    calendar: TradingCalendar,
    #[serde(default)]
    last_rollover_day: u64,
    /// Swap share handed from `reduce_position` to the closing deal (transient).
    #[serde(default)]
    pending_deal_swap: i128,
    /// Swap-free fee part of `pending_deal_swap` (transient).
    #[serde(default)]
    pending_deal_swap_fee: i128,
    /// Copy trading subscriptions (follower, provider).
    #[serde(default)]
    copy_subs: BTreeMap<(AccountNo, AccountNo), CopySubscription>,
    /// Copy source of the order being placed (transient).
    #[serde(default)]
    pending_copy: Option<(AccountNo, PositionId)>,
    #[serde(default)]
    copy_seq: u64,
    /// Fills waiting for their markout horizons (oldest first, bounded).
    #[serde(default)]
    markouts: std::collections::VecDeque<MarkoutSample>,
    /// Balance moves the engine books on its own (copy fees, negative
    /// balance compensation), oldest first: statement history.
    #[serde(default)]
    cash_moves: Vec<CashMove>,
}

/// A client fill whose later mid moves feed `FlowStats` markout.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
struct MarkoutSample {
    account: AccountNo,
    symbol: String,
    sign: i64,
    /// Raw mid (no spread, no markup) when the fill happened: the reference
    /// the later mid moves are measured from. 0 = restored from a snapshot
    /// taken before the mid was stored; dropped unmeasured.
    #[serde(default)]
    mid: i64,
    /// Not read: kept (as the mid) so the previous image, which requires it
    /// and measures from it, can still restore our snapshots on a rollback.
    #[serde(default)]
    price: i64,
    ts: u64,
    /// Horizons already measured (bit per `FlowStats::MARKOUT_SECS`).
    done: u8,
}

const MARKOUT_MAX_PENDING: usize = 20_000;

/// Serializable engine snapshot.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct EngineSnapshot {
    state: State,
    pub ledger: LedgerSnapshot,
}

impl EngineSnapshot {
    pub fn seq(&self) -> u64 {
        self.state.seq
    }
}

pub struct Engine {
    st: State,
    ledger: Ledger,
    router: Box<dyn LpRouter>,
    events: Vec<Event>,
}

type R<T> = Result<T, String>;

/// Minimum gap between two price-only cancel/replaces of one resting order.
const REPLACE_MIN_NS: u64 = 1_000_000_000;

fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

impl Engine {
    pub fn new(config: EngineConfig, router: Box<dyn LpRouter>) -> Engine {
        let mut ledger = Ledger::new();
        let setup = [
            (BANK, AccountKind::External, None, "bank"),
            (
                LP_COUNTERPARTY,
                AccountKind::External,
                None,
                "lp-counterparty",
            ),
            (OMNIBUS, AccountKind::LpOmnibus, None, "lp-omnibus"),
            (
                BROKER_BOOK,
                AccountKind::BrokerBook,
                Some(OMNIBUS),
                "broker-book",
            ),
        ];
        for (id, k, p, n) in setup {
            ledger.open_account(id, k, p, n).expect("fresh ledger");
        }
        Engine {
            st: State {
                config,
                next_id: 1,
                next_exec: 1,
                ..Default::default()
            },
            ledger,
            router,
            events: Vec::new(),
        }
    }

    pub fn set_router(&mut self, router: Box<dyn LpRouter>) {
        self.router = router;
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// Engine clock (ns since epoch; last journaled timestamp).
    pub fn now_ns(&self) -> u64 {
        self.st.now
    }

    pub fn seq(&self) -> u64 {
        self.st.seq
    }
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }
    pub fn order(&self, id: OrderId) -> Option<&Order> {
        self.st.orders.get(&id)
    }
    pub fn orders(&self) -> impl Iterator<Item = &Order> {
        self.st.orders.values()
    }
    pub fn position(&self, id: PositionId) -> Option<&Position> {
        self.st.positions.get(&id)
    }
    pub fn account(&self, id: AccountNo) -> Option<&Account> {
        self.st.accounts.get(&id)
    }
    pub fn accounts(&self) -> impl Iterator<Item = &Account> {
        self.st.accounts.values()
    }
    pub fn positions_of(&self, account: AccountNo) -> Vec<&Position> {
        self.st
            .positions
            .values()
            .filter(|p| p.account == account)
            .collect()
    }
    /// Every open position in id order: one pass for whole-book reads
    /// (`positions_of` per account would be accounts x positions).
    pub fn positions(&self) -> impl Iterator<Item = &Position> {
        self.st.positions.values()
    }
    pub fn symbol_spec(&self, symbol: &str) -> Option<&SymbolSpec> {
        self.st.symbols.get(symbol)
    }
    pub fn symbols(&self) -> impl Iterator<Item = &SymbolSpec> {
        self.st.symbols.values()
    }
    pub fn group(&self, name: &str) -> Option<&GroupConfig> {
        self.st.groups.get(name)
    }
    pub fn groups(&self) -> impl Iterator<Item = &GroupConfig> {
        self.st.groups.values()
    }
    /// Client (marked-up) quote of `symbol` for group `group`.
    pub fn group_quote(&self, group: &str, symbol: &str) -> Option<Quote> {
        let g = self.st.groups.get(group)?;
        self.client_quote(g, symbol).ok()
    }
    /// Floating P&L of a position at the account's client quote, in the
    /// account currency.
    pub fn position_pnl(&self, id: PositionId) -> R<Money> {
        let p = self.st.positions.get(&id).ok_or("unknown position")?;
        let a = &self.st.accounts[&p.account];
        let g = &self.st.groups[&a.group];
        let spec = &self.st.symbols[&p.symbol];
        let q = self.client_quote(g, &p.symbol).map_err(e2s)?;
        risk::floating_pnl(spec, &p.view(), q, g.currency, &self.st.quotes).map_err(e2s)
    }
    /// Latest raw LP quote for a symbol.
    pub fn snapshot_quote(&self, symbol: &str) -> Option<Quote> {
        self.st.quotes.get(symbol)
    }
    pub fn lp_order(&self, id: LpOrderId) -> Option<&LpOrder> {
        self.st.lp_orders.get(&id)
    }
    /// Net omnibus LP position per symbol (raw lots, signed).
    /// Every order sent to the LP, oldest first.
    pub fn lp_orders(&self) -> impl Iterator<Item = &LpOrder> {
        self.st.lp_orders.values()
    }

    pub fn omnibus_net(&self, symbol: &str) -> i64 {
        *self.st.omnibus_net.get(symbol).unwrap_or(&0)
    }
    pub fn swap_config(&self) -> &SwapConfig {
        &self.st.swap
    }
    pub fn calendar(&self) -> &TradingCalendar {
        &self.st.calendar
    }
    /// UTC day (days since epoch) of the last rollover, `None` = never.
    pub fn last_rollover_day(&self) -> Option<u64> {
        self.st.last_rollover_day.checked_sub(1)
    }
    pub fn hedge_policy(&self) -> &HedgePolicy {
        &self.st.hedge
    }
    /// Broker hedge at the LP for the B-book excess (raw lots, signed).
    pub fn hedge_net(&self, symbol: &str) -> i64 {
        *self.st.hedge_net.get(symbol).unwrap_or(&0)
    }
    /// Hedge orders in flight (raw lots, signed).
    pub fn hedge_pending(&self, symbol: &str) -> i64 {
        *self.st.hedge_pending.get(symbol).unwrap_or(&0)
    }
    /// Realised hedge-book result, minor units of the symbol's quote currency.
    pub fn hedge_realized(&self, symbol: &str) -> i128 {
        *self.st.hedge_realized.get(symbol).unwrap_or(&0)
    }
    /// Net B-book client position per symbol (raw lots, signed).
    pub fn b_book_net(&self, symbol: &str) -> i64 {
        self.st
            .positions
            .values()
            .filter(|p| p.routing == Routing::BBook && p.symbol == symbol)
            .map(|p| p.side.sign() * p.volume.raw())
            .sum()
    }
    pub fn flow(&self, account: AccountNo) -> Option<&FlowStats> {
        self.st.flow.get(&account)
    }
    pub fn toxicity(&self, account: AccountNo) -> u8 {
        self.st.flow.get(&account).map_or(0, FlowStats::toxicity)
    }
    /// Deal by id.
    pub fn deal(&self, id: u64) -> Option<&Deal> {
        id.checked_sub(1)
            .and_then(|i| self.st.deals.get(usize::try_from(i).ok()?))
    }
    /// All deals, oldest first.
    pub fn deals(&self) -> &[Deal] {
        &self.st.deals
    }
    /// Engine-booked balance moves (copy fees, negative balance
    /// compensation), oldest first.
    pub fn cash_moves(&self) -> &[CashMove] {
        &self.st.cash_moves
    }
    pub fn order_by_client_id(&self, account: AccountNo, clid: &str) -> Option<&Order> {
        self.st
            .client_ids
            .get(&format!("{account}:{clid}"))
            .and_then(|id| self.st.orders.get(id))
    }

    pub fn balance(&self, account: AccountNo) -> R<Money> {
        let a = self.st.accounts.get(&account).ok_or("unknown account")?;
        let g = self.st.groups.get(&a.group).ok_or("unknown group")?;
        Ok(self.ledger.balance(a.ledger_id, g.currency))
    }

    /// Balance, equity, margin and free margin at current client quotes.
    pub fn account_risk(&self, account: AccountNo) -> R<AccountRisk> {
        let a = self.st.accounts.get(&account).ok_or("unknown account")?;
        let g = self.st.groups.get(&a.group).ok_or("unknown group")?;
        let views: Vec<_> = self
            .positions_of(account)
            .iter()
            .map(|p| p.view())
            .collect();
        self.risk_for(g, self.ledger.balance(a.ledger_id, g.currency), &views)
            .map_err(e2s)
    }

    fn risk_for(
        &self,
        g: &GroupConfig,
        balance: Money,
        views: &[risk::PositionView],
    ) -> Result<AccountRisk, RiskError> {
        let mut floating = Money::zero(g.currency);
        for v in views {
            let spec = &self.st.symbols[&v.symbol];
            let q = self.client_quote(g, &v.symbol)?;
            floating = floating.checked_add(risk::floating_pnl(
                spec,
                v,
                q,
                g.currency,
                &self.st.quotes,
            )?)?;
        }
        let margin =
            risk::total_margin(&g.at(self.st.now), &self.st.symbols, views, &self.st.quotes)?;
        AccountRisk::new(balance, floating, margin)
    }

    /// Markup on `side` of `symbol` for group `g` (ask for Buy, bid for Sell).
    fn markup(&self, g: &GroupConfig, symbol: &str, side: Side) -> Price {
        let point = self.st.symbols.get(symbol).map_or(0, |s| s.point().raw());
        Price::from_raw(point * g.markup_points_for(symbol, side))
    }

    /// Markup for one order: a routing-rule override, else the group's.
    fn order_markup(&self, o: &Order, g: &GroupConfig, side: Side) -> Price {
        match o.markup_override {
            Some(pts) => {
                let point = self
                    .st
                    .symbols
                    .get(&o.req.symbol)
                    .map_or(0, |s| s.point().raw());
                Price::from_raw(point * pts)
            }
            None => self.markup(g, &o.req.symbol, side),
        }
    }

    /// Routing rule table (top to bottom).
    pub fn rules(&self) -> &[RoutingRule] {
        &self.st.rules
    }

    /// First enabled rule matching an order of `account` (in `group`) on `symbol`.
    pub fn match_rule(&self, group: &str, req: &NewOrder, pending: bool) -> Option<&RoutingRule> {
        let account = req.account;
        let symbol = req.symbol.as_str();
        let hour = ((self.st.now / 1_000_000_000) % 86_400 / 3_600) as u8;
        let nop: i64 = self
            .st
            .positions
            .values()
            .filter(|p| p.account == account && p.symbol == symbol)
            .map(|p| p.side.sign() * p.volume.raw())
            .sum();
        // the account's opening fills of the last 24 h (deals are in time order)
        let since = self.st.now.saturating_sub(24 * 3_600_000_000_000);
        let recent: Vec<(u64, i64)> = self
            .st
            .deals
            .iter()
            .rev()
            .take_while(|d| d.ts >= since)
            .filter(|d| d.account == account && d.entry == DealEntry::In)
            .map(|d| (d.ts, d.volume.raw() / 1_000_000))
            .collect();
        let ctx = risk::RuleCtx {
            group,
            account,
            symbol,
            centilots: req.volume.raw() / 1_000_000,
            pending,
            hour_utc: hour,
            toxicity: self.toxicity(account),
            nop_centilots: nop.abs() / 1_000_000,
            recent_opens: &recent,
            scalper: self
                .st
                .flow
                .get(&account)
                .is_some_and(FlowStats::is_scalper),
            platform: req.platform,
            ip: req.ip.as_deref(),
            news_times: &self.st.news_times,
            now_ns: self.st.now,
        };
        self.st.rules.iter().find(|r| r.matches(&ctx))
    }

    fn client_quote(&self, g: &GroupConfig, symbol: &str) -> Result<Quote, RiskError> {
        let q = self
            .st
            .quotes
            .get(symbol)
            .ok_or_else(|| RiskError::NoQuote(symbol.into()))?;
        Ok(q.with_markups(
            self.markup(g, symbol, Side::Sell),
            self.markup(g, symbol, Side::Buy),
        ))
    }

    // ------------------------------------------------------------------
    // Snapshots / determinism
    // ------------------------------------------------------------------

    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            state: self.st.clone(),
            ledger: self.ledger.snapshot(),
        }
    }

    pub fn restore(snap: &EngineSnapshot, router: Box<dyn LpRouter>) -> R<Engine> {
        Ok(Engine {
            st: snap.state.clone(),
            ledger: Ledger::restore(&snap.ledger, []).map_err(e2s)?,
            router,
            events: Vec::new(),
        })
    }

    /// Rebuilds an engine from a journal without re-sending LP orders.
    pub fn replay<'a>(
        config: EngineConfig,
        journal: impl IntoIterator<Item = &'a Envelope>,
    ) -> Engine {
        let mut e = Engine::new(config, Box::new(NullRouter));
        for env in journal {
            e.apply(env);
        }
        e
    }

    /// Canonical serialization of the full state.
    pub fn state_digest(&self) -> String {
        serde_json::to_string(&(&self.st, self.ledger.state_digest())).unwrap_or_default()
    }

    /// Engine-level invariants (ledger zero-sum, omnibus = Σ A-book clients).
    pub fn check_invariants(&self) -> R<()> {
        self.ledger.check_invariants()?;
        let mut net: BTreeMap<&str, i64> = BTreeMap::new();
        for p in self.st.positions.values() {
            if p.routing == Routing::ABook {
                *net.entry(&p.symbol).or_default() += p.side.sign() * p.volume.raw();
            }
            if p.volume.raw() <= 0 || p.closing > p.volume {
                return Err(format!("position {} bad volume", p.id));
            }
        }
        // LP fills held back for a one-shot client fill (tek kalem) are the
        // client's exposure already; a close reduces, an open adds
        for o in self.st.orders.values() {
            if o.chain_fills.is_empty() || o.routing != Routing::ABook {
                continue;
            }
            let held: i64 = o.chain_fills.iter().map(|f| f.volume.raw()).sum();
            *net.entry(&o.req.symbol).or_default() += o.req.side.sign() * held;
        }
        for (sym, v) in &self.st.omnibus_net {
            let expect = *net.get(sym.as_str()).unwrap_or(&0) + self.hedge_net(sym);
            if *v != expect {
                return Err(format!(
                    "omnibus {sym} {v} != clients {:?} + hedge {}",
                    net.get(sym.as_str()),
                    self.hedge_net(sym)
                ));
            }
        }
        for o in self.st.orders.values() {
            if o.filled > o.req.volume {
                return Err(format!("order {} overfilled", o.id));
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Command processing
    // ------------------------------------------------------------------

    /// Applies one journaled command; returns the produced events.
    pub fn apply(&mut self, env: &Envelope) -> Vec<Event> {
        self.st.seq = env.seq;
        self.st.now = self.st.now.max(env.ts);
        if let Err(reason) = self.dispatch(&env.cmd) {
            self.events.push(Event::CommandRejected { reason });
        }
        self.expire_orders();
        if !self.st.copy_subs.is_empty() {
            self.copy_reconcile();
        }
        std::mem::take(&mut self.events)
    }

    fn dispatch(&mut self, cmd: &Command) -> R<()> {
        match cmd {
            Command::AddSymbol(s) => {
                self.st.symbols.insert(s.symbol.clone(), s.clone());
            }
            Command::SetGroup(g) => {
                self.st.groups.insert(g.name.clone(), g.clone());
            }
            Command::SetRules(rules) => {
                self.st.rules = rules.clone();
            }
            Command::SetSwapConfig(c) => {
                self.st.swap = c.clone();
            }
            Command::SetCalendar(c) => {
                self.st.calendar = c.clone();
            }
            Command::SetNewsTimes(t) => {
                let mut t = t.clone();
                t.sort_unstable();
                t.dedup();
                self.st.news_times = t;
            }
            Command::SetHedge(policy) => {
                self.st.hedge = policy.clone();
                let symbols: Vec<String> = self.st.hedge_net.keys().cloned().collect();
                for sym in symbols {
                    self.rebalance_hedge(&sym);
                }
            }
            Command::OpenAccount { account, group } => {
                if !self.st.groups.contains_key(group) {
                    return Err("unknown group".into());
                }
                if self.st.accounts.contains_key(account) {
                    return Err("account exists".into());
                }
                let lid = AccountId(CLIENT_LEDGER_BASE + account);
                self.ledger
                    .open_account(
                        lid,
                        AccountKind::Client,
                        Some(BROKER_BOOK),
                        format!("client-{account}"),
                    )
                    .map_err(e2s)?;
                self.st.accounts.insert(
                    *account,
                    Account {
                        id: *account,
                        group: group.clone(),
                        ledger_id: lid,
                        margin_call: false,
                    },
                );
                self.events.push(Event::AccountOpened { account: *account });
            }
            Command::SetAccountGroup { account, group } => {
                let acc = self.st.accounts.get(account).ok_or("unknown account")?;
                let old = self.st.groups.get(&acc.group).ok_or("unknown group")?;
                let new = self.st.groups.get(group).ok_or("unknown group")?;
                if new.currency != old.currency {
                    return Err("group currency differs".into());
                }
                if self.st.positions.values().any(|p| p.account == *account) {
                    return Err("account has open positions".into());
                }
                if self
                    .st
                    .orders
                    .values()
                    .any(|o| o.req.account == *account && !o.status.is_terminal())
                {
                    return Err("account has working orders".into());
                }
                // copy subscriptions were admitted for the account's group (hedging
                // follower, copy-enabled groups): a move would bypass that
                if self.st.copy_subs.values().any(|s| {
                    (s.active || s.closing) && (s.follower == *account || s.provider == *account)
                }) {
                    return Err("account has an active copy subscription".into());
                }
                self.st.accounts.get_mut(account).expect("account").group = group.clone();
                self.events.push(Event::AccountGroupChanged {
                    account: *account,
                    group: group.clone(),
                });
            }
            Command::Deposit {
                account,
                amount,
                key,
            } => self.cash(*account, *amount, key, TxnKind::Deposit)?,
            Command::Withdraw {
                account,
                amount,
                key,
            } => {
                let r = self.account_risk(*account)?;
                if amount.minor > r.free_margin.minor {
                    return Err("withdrawal exceeds free margin".into());
                }
                self.cash(
                    *account,
                    amount.checked_neg().map_err(e2s)?,
                    key,
                    TxnKind::Withdrawal,
                )?
            }
            Command::Quote { symbol, bid, ask } => {
                if bid > ask || !bid.is_positive() {
                    return Err("crossed or invalid quote".into());
                }
                self.st.quotes.set(
                    symbol,
                    Quote {
                        bid: *bid,
                        ask: *ask,
                    },
                );
                self.on_quote(symbol);
                self.resolve_markouts(symbol);
            }
            Command::PlaceOrder(o) => self.place_order(o.clone(), None, OrderOrigin::Client)?,
            Command::ModifyOrder {
                account,
                order_id,
                change,
            } => self.modify_order(*account, *order_id, change)?,
            Command::CancelOrder { account, order_id } => {
                let o = self.st.orders.get(order_id).ok_or("unknown order")?;
                if o.req.account != *account {
                    return Err("not your order".into());
                }
                if !o.is_pending() {
                    return Err("order not cancellable".into());
                }
                self.finish_order(*order_id, OrderStatus::Cancelled);
            }
            Command::ModifyPosition {
                account,
                position_id,
                sl,
                tp,
                trailing_points,
            } => {
                let p = self
                    .st
                    .positions
                    .get(position_id)
                    .filter(|p| p.account == *account)
                    .ok_or("unknown position")?;
                if trailing_points.is_some_and(|t| t <= 0) {
                    return Err("trailing distance must be positive".into());
                }
                // SL / TP must be on the correct side of the closing price
                let g = &self.st.groups[&self.st.accounts[account].group];
                let px = self
                    .client_quote(g, &p.symbol)
                    .map_err(e2s)?
                    .for_side(p.side.opposite());
                let s = p.side.sign();
                if sl.is_some_and(|sl| (sl.raw() - px.raw()) * s >= 0)
                    || tp.is_some_and(|tp| (tp.raw() - px.raw()) * s <= 0)
                {
                    return Err("invalid SL/TP".into());
                }
                let p = self.st.positions.get_mut(position_id).expect("position");
                p.sl = *sl;
                p.tp = *tp;
                p.trailing_points = *trailing_points;
                self.events.push(Event::PositionModified {
                    position_id: *position_id,
                    sl: *sl,
                    tp: *tp,
                });
                let sym = p.symbol.clone();
                self.sync_resting(*position_id);
                self.on_quote(&sym);
            }
            Command::ClosePosition {
                account,
                position_id,
                volume,
                client_order_id,
            } => {
                let p = self
                    .st
                    .positions
                    .get(position_id)
                    .filter(|p| p.account == *account)
                    .ok_or("unknown position")?;
                let v = volume.unwrap_or(p.free_volume());
                if v > p.free_volume() || !v.is_positive() {
                    return Err("close volume exceeds open volume".into());
                }
                let mut o =
                    NewOrder::market(*account, client_order_id, &p.symbol, p.side.opposite(), v);
                o.client_order_id = client_order_id.clone();
                let pid = *position_id;
                self.place_order(o, Some(pid), OrderOrigin::Client)?;
                self.sync_resting(pid);
            }
            Command::LpFill {
                lp_order_id,
                exec_id,
                volume,
                price,
            } => self.on_lp_fill(*lp_order_id, exec_id, *volume, *price)?,
            Command::LpReject {
                lp_order_id,
                reason,
            } => self.on_lp_reject(*lp_order_id, reason)?,
            Command::LpRouted { lp_order_id, lp } => {
                if let Some(l) = self.st.lp_orders.get_mut(lp_order_id) {
                    l.lp = Some(lp.clone());
                }
            }
            Command::FlushLp => self.flush_lp(),
            Command::Rollover => {
                self.rollover()?;
                // daily performance-fee settlement (high-water mark)
                let providers: BTreeSet<AccountNo> =
                    self.st.copy_subs.keys().map(|k| k.1).collect();
                for p in providers {
                    self.copy_settle(p)?;
                }
            }
            Command::CopySubscribe {
                follower,
                provider,
                ratio_bps,
                equity_stop_pct,
                perf_fee_bps,
            } => self.copy_subscribe(
                *follower,
                *provider,
                *ratio_bps,
                *equity_stop_pct,
                *perf_fee_bps,
            )?,
            Command::CopyUnsubscribe {
                follower,
                provider,
                close,
            } => {
                let key = (*follower, *provider);
                if !self.st.copy_subs.contains_key(&key) {
                    return Err("no such subscription".into());
                }
                self.copy_stop(key, "unsubscribed", *close);
            }
            Command::CopySettle { provider } => self.copy_settle(*provider)?,
            Command::Tick => {}
        }
        Ok(())
    }

    fn cash(&mut self, account: AccountNo, amount: Money, key: &str, kind: TxnKind) -> R<()> {
        let a = self.st.accounts.get(&account).ok_or("unknown account")?;
        let g = &self.st.groups[&a.group];
        if amount.currency != g.currency {
            return Err("currency differs from account currency".into());
        }
        let lid = a.ledger_id;
        self.post(
            kind,
            format!("cash:{account}:{key}"),
            vec![(lid, amount), (BANK, amount.checked_neg().map_err(e2s)?)],
        )?;
        self.balance_event(account);
        Ok(())
    }

    fn post(&mut self, kind: TxnKind, key: String, postings: Vec<(AccountId, Money)>) -> R<()> {
        let postings: Vec<Posting> = postings
            .into_iter()
            .filter(|(_, m)| !m.is_zero())
            .map(|(a, m)| Posting::new(a, m))
            .collect();
        if postings.is_empty() {
            return Ok(());
        }
        self.ledger
            .post(TxnRequest {
                idempotency_key: key,
                kind,
                postings,
                memo: String::new(),
                ts: self.st.now,
            })
            .map(|_| ())
            .map_err(e2s)
    }

    fn balance_event(&mut self, account: AccountNo) {
        if let Ok(b) = self.balance(account) {
            self.events.push(Event::BalanceChanged {
                account,
                balance: b,
            });
        }
    }

    // ------------------------------------------------------------------
    // Orders
    // ------------------------------------------------------------------

    fn modify_order(&mut self, account: AccountNo, id: OrderId, c: &OrderChange) -> R<()> {
        let o = self.st.orders.get(&id).ok_or("unknown order")?;
        if o.req.account != account {
            return Err("not your order".into());
        }
        if !o.is_pending() {
            return Err("order not modifiable".into());
        }
        if !c.volume.is_positive() {
            return Err("invalid volume".into());
        }
        let g = self.st.groups[&self.st.accounts[&account].group].clone();
        let old = o.req.clone();
        let mut new = old.clone();
        new.volume = c.volume;
        new.limit_price = c.limit_price;
        new.stop_price = c.stop_price;
        new.sl = c.sl;
        new.tp = c.tp;
        new.trailing_points = c.trailing_points;
        new.expire_at = c.expire_at;
        let sym = new.symbol.clone();
        self.st.orders.get_mut(&id).expect("order").req = new;
        if let Err(e) = self.validate_order(id, &g) {
            self.st.orders.get_mut(&id).expect("order").req = old;
            return Err(e);
        }
        // A new price is a new attempt.
        self.st.orders.get_mut(&id).expect("order").rearm_px = None;
        self.events.push(Event::OrderModified { order_id: id });
        if let Some(lp) = self.st.orders[&id].lp_resting {
            let o = &self.st.orders[&id];
            let m = self.order_markup(o, &g, o.req.side).raw() * o.req.side.sign();
            let net = |p: Price| Price::from_raw(p.raw() - m);
            let rem = o.remaining();
            match (o.req.order_type, o.req.limit_price, o.req.stop_price) {
                (OrderType::Limit, Some(l), _) => self.replace_resting(lp, Some(net(l)), None, rem),
                (OrderType::Stop, _, Some(st)) => {
                    self.replace_resting(lp, None, Some(net(st)), rem)
                }
                _ => self.cancel_resting(lp),
            }
        }
        self.check_pending(&sym);
        Ok(())
    }

    fn place_order(
        &mut self,
        req: NewOrder,
        close: Option<PositionId>,
        origin: OrderOrigin,
    ) -> R<()> {
        self.place_order_with(req, close, origin, true)
    }

    /// [`Self::place_order`]; `start = false` creates and accepts the order
    /// without executing it (a child whose fill is already known).
    fn place_order_with(
        &mut self,
        req: NewOrder,
        close: Option<PositionId>,
        origin: OrderOrigin,
        start: bool,
    ) -> R<()> {
        let key = format!("{}:{}", req.account, req.client_order_id);
        if let Some(&id) = self.st.client_ids.get(&key) {
            self.events.push(Event::DuplicateOrder { order_id: id });
            return Ok(());
        }
        let acc = self
            .st
            .accounts
            .get(&req.account)
            .ok_or("unknown account")?;
        let g = self.st.groups[&acc.group].clone();
        let id = self.st.next_id;
        self.st.next_id += 1;
        self.st.client_ids.insert(key, id);
        // First matching routing rule decides the book and may override group levers.
        let pending = req.order_type != OrderType::Market;
        let rule = self.match_rule(&acc.group, &req, pending).cloned();
        let mut routing = rule
            .as_ref()
            .map_or(g.routing, |r| r.book_for(id, g.routing));
        let mut rule_tag = rule.as_ref().map(|r| r.id.clone());
        // B-book exposure guard: over the limit, risk-increasing flow goes A-book.
        if routing == Routing::BBook
            && close.is_none()
            && self.st.hedge.enabled
            && self.st.hedge.mode == HedgeMode::SwitchToABook
            && self.b_book_over_limit(&req.symbol, req.account, req.side, req.volume)
        {
            routing = Routing::ABook;
            rule_tag = Some("hedge:limit".into());
        }
        let order = Order {
            id,
            req,
            status: OrderStatus::New,
            filled: Qty::ZERO,
            avg_price: Price::ZERO,
            routing,
            close_position: close,
            position: None,
            stop_triggered: false,
            working: false,
            reject_reason: None,
            created_ts: self.st.now,
            origin,
            rearm_px: None,
            lp_attempts: 0,
            rule: rule_tag,
            markup_override: rule.as_ref().and_then(|r| r.markup_points),
            max_slippage_override: rule.as_ref().and_then(|r| r.max_slippage_points),
            partial_fill_override: rule.as_ref().and_then(|r| r.partial_fill),
            copy_from: self.st.pending_copy.take(),
            lp_resting: None,
            chain_fills: Vec::new(),
        };
        self.st.orders.insert(id, order);
        if let Err(reason) = self.validate_order(id, &g) {
            self.reject(id, reason);
            return Ok(());
        }
        self.set_status(id, OrderStatus::Accepted);
        self.events.push(Event::OrderAccepted { order_id: id });
        if let Some(pid) = close {
            let v = self.st.orders[&id].req.volume;
            if let Some(p) = self.st.positions.get_mut(&pid) {
                p.closing = Qty::from_raw(p.closing.raw() + v.raw());
            }
        }
        if !start {
            return Ok(());
        }
        let o = &self.st.orders[&id];
        if o.req.order_type == OrderType::Market {
            self.execute(id);
        } else {
            let sym = o.req.symbol.clone();
            self.st.pending_ids.insert(id);
            if !self.rest_entry_at_lp(id) {
                self.check_pending(&sym);
            }
        }
        Ok(())
    }

    fn validate_order(&self, id: OrderId, g: &GroupConfig) -> R<()> {
        let o = &self.st.orders[&id];
        let r = &o.req;
        let spec = self.st.symbols.get(&r.symbol).ok_or("unknown symbol")?;
        // market hours: closing orders are always allowed (risk reduction); a
        // copy open is a new trade like the client's own
        if o.close_position.is_none() && matches!(o.origin, OrderOrigin::Client | OrderOrigin::Copy)
        {
            if !spec.enabled {
                return Err("symbol disabled".into());
            }
            if self.st.calendar.is_holiday(self.st.now) {
                return Err("market closed (holiday)".into());
            }
            if !spec.is_open_at(self.st.now) {
                return Err("market closed".into());
            }
        }
        let need_limit = matches!(r.order_type, OrderType::Limit | OrderType::StopLimit);
        let need_stop = matches!(r.order_type, OrderType::Stop | OrderType::StopLimit);
        if need_limit != r.limit_price.is_some() || need_stop != r.stop_price.is_some() {
            return Err("price fields do not match order type".into());
        }
        if let Some(t) = r.trailing_points {
            if t <= 0 {
                return Err("trailing distance must be positive".into());
            }
        }
        if r.expire_at.is_some_and(|t| t <= self.st.now) {
            return Err("already expired".into());
        }
        let cq = self.client_quote(g, &r.symbol).map_err(e2s)?;
        let entry = match r.side {
            Side::Buy => cq.ask,
            Side::Sell => cq.bid,
        };
        let entry = r.limit_price.or(r.stop_price).unwrap_or(entry);
        // SL/TP must be on the correct side of the entry price.
        let s = r.side.sign();
        if r.sl.is_some_and(|sl| (sl.raw() - entry.raw()) * s >= 0)
            || r.tp.is_some_and(|tp| (tp.raw() - entry.raw()) * s <= 0)
        {
            return Err("invalid SL/TP".into());
        }
        let acc = &self.st.accounts[&r.account];
        let views: Vec<_> = self
            .positions_of(r.account)
            .iter()
            .map(|p| p.view())
            .collect();
        let cqs: BTreeMap<String, Quote> = views
            .iter()
            .map(|v| v.symbol.clone())
            .chain(std::iter::once(r.symbol.clone()))
            .filter_map(|s| self.client_quote(g, &s).ok().map(|q| (s, q)))
            .collect();
        let intent = OrderIntent {
            symbol: &r.symbol,
            side: r.side,
            volume: r.volume,
            requested_price: r.requested_price,
            price: r.limit_price.or(r.stop_price),
        };
        let _ = spec;
        risk::pre_trade_check(
            &g.at(self.st.now),
            &self.st.symbols,
            self.ledger.balance(acc.ledger_id, g.currency),
            &views,
            &intent,
            &self.st.quotes,
            &cqs,
            o.close_position.is_some(),
        )
        .map_err(e2s)?;
        Ok(())
    }

    fn set_status(&mut self, id: OrderId, to: OrderStatus) {
        let o = self.st.orders.get_mut(&id).expect("order");
        debug_assert!(o.status.can_transition(to), "{:?} -> {:?}", o.status, to);
        o.status = to;
    }

    fn reject(&mut self, id: OrderId, reason: String) {
        self.set_status(id, OrderStatus::Rejected);
        let o = self.st.orders.get_mut(&id).expect("order");
        o.reject_reason = Some(reason.clone());
        o.working = false;
        self.release_closing(id);
        self.copy_order_ended(id);
        self.events.push(Event::OrderRejected {
            order_id: id,
            reason,
        });
    }

    fn release_closing(&mut self, id: OrderId) {
        let o = &self.st.orders[&id];
        if let Some(pid) = o.close_position {
            let rem = o.remaining();
            if let Some(p) = self.st.positions.get_mut(&pid) {
                p.closing = Qty::from_raw((p.closing.raw() - rem.raw()).max(0));
            }
            self.sync_resting(pid);
        }
    }

    /// Terminal transition for cancel/expire (keeps partial fills).
    fn finish_order(&mut self, id: OrderId, to: OrderStatus) {
        if let Some(lp) = self.st.orders[&id].lp_resting {
            self.cancel_resting(lp);
        }
        self.release_closing(id);
        self.set_status(id, to);
        self.st.orders.get_mut(&id).expect("order").working = false;
        self.copy_order_ended(id);
        self.events.push(match to {
            OrderStatus::Expired => Event::OrderExpired { order_id: id },
            _ => Event::OrderCancelled { order_id: id },
        });
    }

    /// Currently pending order ids (prunes the index).
    fn pending_orders(&mut self) -> Vec<OrderId> {
        let orders = &self.st.orders;
        self.st.pending_ids.retain(|id| orders[id].is_pending());
        self.st.pending_ids.iter().copied().collect()
    }

    fn cancel_oco(&mut self, id: OrderId) {
        let Some(grp) = self.st.orders[&id].req.oco_group else {
            return;
        };
        let acc = self.st.orders[&id].req.account;
        let others: Vec<OrderId> = self
            .st
            .orders
            .values()
            .filter(|o| {
                o.id != id && o.req.account == acc && o.req.oco_group == Some(grp) && o.is_pending()
            })
            .map(|o| o.id)
            .collect();
        for o in others {
            self.finish_order(o, OrderStatus::Cancelled);
        }
    }

    /// Starts execution of an accepted order (market or triggered pending).
    fn execute(&mut self, id: OrderId) {
        let o = &self.st.orders[&id];
        let acc = &self.st.accounts[&o.req.account];
        let g = self.st.groups[&acc.group].clone();
        self.st.orders.get_mut(&id).expect("order").working = true;
        self.cancel_oco(id);
        // Client market orders that open/increase exposure carry the client price
        // seen at execution as their reference (slippage cap, circuit breaker,
        // slippage reports). Closes, stop-outs and triggered stops stay unbounded:
        // they must fill.
        let o = &self.st.orders[&id];
        if o.req.requested_price.is_none()
            && o.req.order_type == OrderType::Market
            && o.close_position.is_none()
            && o.routing == Routing::ABook
        {
            if let Ok(q) = self.client_quote(&g, &o.req.symbol) {
                let px = q.for_side(o.req.side);
                self.st
                    .orders
                    .get_mut(&id)
                    .expect("order")
                    .req
                    .requested_price = Some(px);
            }
        }
        let o = &self.st.orders[&id];
        match o.routing {
            Routing::BBook => {
                let q = match self.client_quote(&g, &o.req.symbol) {
                    Ok(q) => q,
                    Err(e) => return self.reject(id, e.to_string()),
                };
                let price = q.for_side(o.req.side);
                let v = o.remaining();
                self.fill_child(id, v, price, price);
            }
            Routing::ABook => {
                if let Some(l) = o.limit_leg() {
                    // "Limit or better" is guaranteed by the LP, not by us: the client
                    // limit net of the markup goes out as an IOC limit order.
                    let m = self.order_markup(o, &g, o.req.side).raw() * o.req.side.sign();
                    let lp_limit = Price::from_raw(l.raw() - m);
                    self.send_lp_limit(o.req.symbol.clone(), o.req.side, vec![id], Some(lp_limit));
                } else if self.st.config.aggregate_a_book
                    && o.close_position.is_none()
                    && o.max_slippage_override.or(g.max_slippage_points).is_none()
                    && o.req.max_deviation_points.is_none()
                {
                    // one LP order per (symbol, side): no per-order guard
                    self.st.pending_lp.push(id);
                } else if let Some(lp_limit) = self.slippage_limit(o, &g) {
                    self.send_lp_limit(o.req.symbol.clone(), o.req.side, vec![id], Some(lp_limit));
                } else {
                    self.send_lp(o.req.symbol.clone(), o.req.side, vec![id]);
                }
            }
        }
    }

    /// Slippage cap: the LP may fill a market order up to `max` points past the
    /// requested price (client terms), as an IOC limit net of the markup.
    fn slippage_limit(&self, o: &Order, g: &GroupConfig) -> Option<Price> {
        let req = o.req.requested_price?;
        let point = self.st.symbols[&o.req.symbol].point().raw().max(1);
        // No configured cap: a circuit breaker of SLIPPAGE_GUARD_BPS of the
        // requested price (flash crash / bad tick only; never a requote in
        // normal markets), in whole points.
        let guard = (req.raw() as i128 * SLIPPAGE_GUARD_BPS as i128 / 10_000) as i64 / point;
        let max = o
            .max_slippage_override
            .or(g.max_slippage_points)
            .unwrap_or(guard);
        let max = o.req.max_deviation_points.map_or(max, |d| d.clamp(0, max));
        let m = self.order_markup(o, g, o.req.side).raw() * o.req.side.sign();
        Some(Price::from_raw(
            req.raw() + o.req.side.sign() * point * max - m,
        ))
    }

    fn send_lp(&mut self, symbol: String, side: Side, children: Vec<OrderId>) {
        self.send_lp_limit(symbol, side, children, None);
    }

    fn send_lp_limit(
        &mut self,
        symbol: String,
        side: Side,
        children: Vec<OrderId>,
        limit: Option<Price>,
    ) {
        let volume = Qty::from_raw(
            children
                .iter()
                .map(|c| self.st.orders[c].lp_open().raw())
                .sum(),
        );
        // FOK only when every child's group wants all-or-none (a batch is one LP order).
        let all_or_none = children
            .iter()
            .all(|c| self.policy(*c) == PartialFill::AllOrNone);
        for c in &children {
            self.st.orders.get_mut(c).expect("order").lp_attempts += 1;
        }
        let id = self.st.next_id;
        self.st.next_id += 1;
        let req = LpOrderRequest {
            lp_order_id: id,
            symbol: symbol.clone(),
            side,
            volume,
            limit,
            all_or_none,
            resting: false,
            revision: 0,
            stop: None,
        };
        let sent = self.st.quotes.get(&symbol);
        self.st.lp_orders.insert(
            id,
            LpOrder {
                id,
                symbol,
                side,
                volume,
                filled: Qty::ZERO,
                children,
                done: false,
                fills: Vec::new(),
                created_ts: self.st.now,
                reject_reason: None,
                limit,
                sent_bid: sent.map(|q| q.bid),
                sent_ask: sent.map(|q| q.ask),
                lp: None,
                hedge: false,
                resting: false,
                position: None,
                revision: 0,
                stop: None,
                replaced_ts: 0,
            },
        );
        self.router.send(&req);
        self.events.push(Event::LpOrderSent {
            lp_order_id: id,
            volume,
        });
    }

    /// Partial-fill policy of the order: a rule override, else the group's.
    fn policy(&self, id: OrderId) -> PartialFill {
        let o = &self.st.orders[&id];
        if let Some(p) = o.partial_fill_override {
            return p;
        }
        let acc = &self.st.accounts[&o.req.account];
        self.st.groups[&acc.group].partial_fill
    }

    fn flush_lp(&mut self) {
        let pending = std::mem::take(&mut self.st.pending_lp);
        let mut groups: BTreeMap<(String, Side), Vec<OrderId>> = BTreeMap::new();
        for id in pending {
            let o = &self.st.orders[&id];
            if o.status.is_terminal() {
                continue;
            }
            groups
                .entry((o.req.symbol.clone(), o.req.side))
                .or_default()
                .push(id);
        }
        for ((sym, side), ids) in groups {
            self.send_lp(sym, side, ids);
        }
    }

    /// Would `side × volume` of `account` push B-book exposure over a limit
    /// (symbol, client, total)? Risk-reducing orders never do.
    fn b_book_over_limit(&self, symbol: &str, account: AccountNo, side: Side, volume: Qty) -> bool {
        let h = &self.st.hedge;
        let net = self.b_book_net(symbol);
        let proj = net + side.sign() * volume.raw();
        if proj.abs() <= net.abs() {
            return false;
        }
        if h.symbol_limit(symbol).is_some_and(|l| proj.abs() > l.raw()) {
            return true;
        }
        if let Some(l) = h.account_limit {
            let mine: i64 = self
                .st
                .positions
                .values()
                .filter(|p| {
                    p.routing == Routing::BBook && p.symbol == symbol && p.account == account
                })
                .map(|p| p.side.sign() * p.volume.raw())
                .sum();
            if (mine + side.sign() * volume.raw()).abs() > l.raw() {
                return true;
            }
        }
        if let Some(l) = h.total_limit {
            let mut total: i64 = 0;
            for s in self.st.symbols.keys() {
                total += if s == symbol {
                    proj.abs()
                } else {
                    self.b_book_net(s).abs()
                };
            }
            if total > l.raw() {
                return true;
            }
        }
        false
    }

    /// HedgeExcess: brings the broker hedge at the LP to its target for the
    /// symbol — the excess over the limit (× ratio) while over it, zero once
    /// exposure has fallen to `release_pct` of the limit — counting orders in
    /// flight, in 0.01-lot steps.
    fn rebalance_hedge(&mut self, symbol: &str) {
        let h = &self.st.hedge;
        if !h.enabled || h.mode != HedgeMode::HedgeExcess {
            return;
        }
        let Some(limit) = h.symbol_limit(symbol).map(|q| q.raw()) else {
            return;
        };
        let net = self.b_book_net(symbol);
        let cur = self.hedge_net(symbol) + self.hedge_pending(symbol);
        let target = if net.abs() > limit {
            let excess = net - net.signum() * limit;
            -(excess as i128 * h.hedge_ratio_pct as i128 / 100) as i64
        } else if net.abs() as i128 * 100 <= limit as i128 * h.release_pct as i128 {
            0
        } else {
            cur
        };
        const STEP: i64 = 1_000_000; // 0.01 lot
        let delta = (target - cur) / STEP * STEP;
        if delta == 0 {
            return;
        }
        let side = if delta > 0 { Side::Buy } else { Side::Sell };
        self.send_hedge(symbol.to_string(), side, Qty::from_raw(delta.abs()));
    }

    fn send_hedge(&mut self, symbol: String, side: Side, volume: Qty) {
        let id = self.st.next_id;
        self.st.next_id += 1;
        let req = LpOrderRequest {
            lp_order_id: id,
            symbol: symbol.clone(),
            side,
            volume,
            limit: None,
            all_or_none: false,
            resting: false,
            revision: 0,
            stop: None,
        };
        let sent = self.st.quotes.get(&symbol);
        *self.st.hedge_pending.entry(symbol.clone()).or_default() += side.sign() * volume.raw();
        self.st.lp_orders.insert(
            id,
            LpOrder {
                id,
                symbol,
                side,
                volume,
                filled: Qty::ZERO,
                children: Vec::new(),
                done: false,
                fills: Vec::new(),
                created_ts: self.st.now,
                reject_reason: None,
                limit: None,
                sent_bid: sent.map(|q| q.bid),
                sent_ask: sent.map(|q| q.ask),
                lp: None,
                hedge: true,
                resting: false,
                position: None,
                revision: 0,
                stop: None,
                replaced_ts: 0,
            },
        );
        self.router.send(&req);
        self.events.push(Event::LpOrderSent {
            lp_order_id: id,
            volume,
        });
    }

    /// Fill of a broker hedge order: moves the omnibus and the hedge book;
    /// a reducing fill realises P&L (broker book vs. LP counterparty).
    fn on_hedge_fill(
        &mut self,
        lp_id: LpOrderId,
        exec_id: &str,
        volume: Qty,
        price: Price,
    ) -> R<()> {
        let (symbol, side) = {
            let lp = self.st.lp_orders.get_mut(&lp_id).expect("lp");
            lp.filled = Qty::from_raw(lp.filled.raw() + volume.raw());
            lp.done = lp.filled >= lp.volume;
            lp.fills.push(LpExec {
                exec_id: exec_id.to_string(),
                volume,
                price,
                ts: self.st.now,
            });
            (lp.symbol.clone(), lp.side)
        };
        self.apply_hedge_fill(&symbol, side, volume, price, exec_id);
        Ok(())
    }

    /// Books an LP fill on the broker hedge book (omnibus, pending, average,
    /// realised P&L of the reduced part).
    fn apply_hedge_fill(
        &mut self,
        symbol: &str,
        side: Side,
        volume: Qty,
        price: Price,
        exec_id: &str,
    ) {
        let symbol = symbol.to_string();
        let signed = side.sign() * volume.raw();
        *self.st.hedge_pending.entry(symbol.clone()).or_default() -= signed;
        *self.st.omnibus_net.entry(symbol.clone()).or_default() += signed;
        let cur = self.hedge_net(&symbol);
        let avg = *self.st.hedge_avg.get(&symbol).unwrap_or(&0);
        if cur == 0 || cur.signum() == signed.signum() {
            // opening / adding: volume-weighted average
            let total = cur.abs() as i128 + volume.raw() as i128;
            let new_avg = (avg as i128 * cur.abs() as i128
                + price.raw() as i128 * volume.raw() as i128)
                / total;
            self.st.hedge_avg.insert(symbol.clone(), new_avg as i64);
        } else {
            // reducing (possibly flipping): realise on the closed part
            let closed = cur.abs().min(volume.raw());
            if let Some(spec) = self.st.symbols.get(&symbol).cloned() {
                let pos_side = if cur > 0 { Side::Buy } else { Side::Sell };
                if let Ok(pnl) = risk::pnl_money(
                    &spec,
                    pos_side,
                    Price::from_raw(avg),
                    price,
                    Qty::from_raw(closed),
                    spec.quote,
                    &self.st.quotes,
                ) {
                    *self.st.hedge_realized.entry(symbol.clone()).or_default() += pnl.minor;
                    let _ = self.post(
                        TxnKind::RealizedPnl,
                        format!("hedge:{exec_id}"),
                        vec![
                            (BROKER_BOOK, pnl),
                            (LP_COUNTERPARTY, Money::new(-pnl.minor, pnl.currency)),
                        ],
                    );
                }
            }
            if volume.raw() > closed {
                self.st.hedge_avg.insert(symbol.clone(), price.raw());
            }
        }
        *self.st.hedge_net.entry(symbol.clone()).or_default() += signed;
        if self.hedge_net(&symbol) == 0 {
            self.st.hedge_avg.remove(&symbol);
        }
    }

    // ---- LP-resting orders (GroupConfig::lp_resting) --------------------
    //
    // A-book TP and pending limit entries rest at the LP as GTC limit orders.
    // The LP's fill is the event: our own quote never triggers them, so a
    // spike the LP did not trade at closes nothing here, and the client gets
    // the LP's price plus the markup.

    /// Keeps the LP-side TP order of a position in step with its TP and its
    /// free volume (sends, replaces or cancels as needed).
    fn sync_resting(&mut self, pid: PositionId) {
        self.sync_resting_leg(pid, false);
        self.sync_resting_leg(pid, true);
    }

    /// One leg: the TP as a GTC limit, or (`stop = true`) the SL as a GTC
    /// stop, both net of the markup and sized to the free volume.
    fn sync_resting_leg(&mut self, pid: PositionId, stop: bool) {
        let Some(p) = self.st.positions.get(&pid).cloned() else {
            return;
        };
        let g = self.st.groups[&self.st.accounts[&p.account].group].clone();
        let side = p.side.opposite();
        let vol = p.free_volume();
        let level = if stop { p.sl } else { p.tp };
        let target = level
            .filter(|_| p.routing == Routing::ABook && g.lp_resting && vol.is_positive())
            .map(|px| {
                let m = self.markup(&g, &p.symbol, side).raw() * side.sign();
                Price::from_raw(px.raw() - m)
            });
        let current = if stop { p.lp_sl } else { p.lp_tp };
        match (current, target) {
            (None, None) => {}
            (None, Some(px)) => {
                let (limit, stop_px) = if stop {
                    (None, Some(px))
                } else {
                    (Some(px), None)
                };
                let id = self.send_resting(
                    p.symbol.clone(),
                    side,
                    vol,
                    limit,
                    stop_px,
                    Some(pid),
                    Vec::new(),
                );
                let pm = self.st.positions.get_mut(&pid).expect("position");
                if stop {
                    pm.lp_sl = Some(id);
                } else {
                    pm.lp_tp = Some(id);
                }
            }
            (Some(id), None) => self.cancel_resting(id),
            (Some(id), Some(px)) => {
                let lp = &self.st.lp_orders[&id];
                let rem = Qty::from_raw(lp.volume.raw() - lp.filled.raw());
                let (limit, stop_px) = if stop {
                    (None, Some(px))
                } else {
                    (Some(px), None)
                };
                if lp.limit != limit || lp.stop != stop_px || rem != vol {
                    self.replace_resting(id, limit, stop_px, vol);
                }
            }
        }
    }

    /// A just-accepted pending limit entry of an A-book `lp_resting` group
    /// goes to the LP as a GTC limit (net of the markup). Returns whether it did.
    fn rest_entry_at_lp(&mut self, id: OrderId) -> bool {
        let o = &self.st.orders[&id];
        let g = &self.st.groups[&self.st.accounts[&o.req.account].group];
        if !(g.lp_resting && o.routing == Routing::ABook) {
            return false;
        }
        let (limit, stop) = match o.req.order_type {
            OrderType::Limit => (o.req.limit_price, None),
            OrderType::Stop => (None, o.req.stop_price),
            _ => return false, // stop-limit: our trigger, then an IOC limit
        };
        let m = self.order_markup(o, g, o.req.side).raw() * o.req.side.sign();
        let net = |p: Price| Price::from_raw(p.raw() - m);
        let (symbol, side, vol) = (o.req.symbol.clone(), o.req.side, o.remaining());
        let lp = self.send_resting(
            symbol,
            side,
            vol,
            limit.map(net),
            stop.map(net),
            None,
            vec![id],
        );
        self.st.orders.get_mut(&id).expect("order").lp_resting = Some(lp);
        true
    }

    fn send_resting(
        &mut self,
        symbol: String,
        side: Side,
        volume: Qty,
        limit: Option<Price>,
        stop: Option<Price>,
        position: Option<PositionId>,
        children: Vec<OrderId>,
    ) -> LpOrderId {
        let id = self.st.next_id;
        self.st.next_id += 1;
        let req = LpOrderRequest {
            lp_order_id: id,
            symbol: symbol.clone(),
            side,
            volume,
            limit,
            all_or_none: false,
            resting: true,
            revision: 0,
            stop,
        };
        let sent = self.st.quotes.get(&symbol);
        self.st.lp_orders.insert(
            id,
            LpOrder {
                id,
                symbol,
                side,
                volume,
                filled: Qty::ZERO,
                children,
                done: false,
                fills: Vec::new(),
                created_ts: self.st.now,
                reject_reason: None,
                limit,
                sent_bid: sent.map(|q| q.bid),
                sent_ask: sent.map(|q| q.ask),
                lp: None,
                hedge: false,
                resting: true,
                position,
                revision: 0,
                stop,
                replaced_ts: 0,
            },
        );
        self.router.send(&req);
        self.events.push(Event::LpOrderSent {
            lp_order_id: id,
            volume,
        });
        self.events.push(Event::LpRestingChanged {
            lp_order_id: id,
            active: true,
        });
        id
    }

    /// New price and/or remaining quantity for a resting order (the LP
    /// quantity is the total: filled + remaining).
    fn replace_resting(
        &mut self,
        id: LpOrderId,
        limit: Option<Price>,
        stop: Option<Price>,
        remaining: Qty,
    ) {
        let now = self.st.now;
        let Some(lp) = self.st.lp_orders.get_mut(&id) else {
            return;
        };
        if lp.done {
            return;
        }
        let total = Qty::from_raw(lp.filled.raw() + remaining.raw());
        // A trailing stop moves on every tick: price-only replaces go out at
        // most once a second (the next sync catches up); size changes at once.
        if total == lp.volume
            && lp.replaced_ts > 0
            && now.saturating_sub(lp.replaced_ts) < REPLACE_MIN_NS
        {
            return;
        }
        lp.revision += 1;
        lp.limit = limit;
        lp.stop = stop;
        lp.volume = total;
        lp.replaced_ts = now;
        let req = LpOrderRequest {
            lp_order_id: id,
            symbol: lp.symbol.clone(),
            side: lp.side,
            volume: lp.volume,
            limit,
            all_or_none: false,
            resting: true,
            revision: lp.revision,
            stop,
        };
        self.router.replace(&req);
    }

    /// Cancel at the LP; the order stays open here until the LP confirms
    /// (`LpReject`), so a fill racing the cancel is still booked.
    fn cancel_resting(&mut self, id: LpOrderId) {
        let Some(lp) = self.st.lp_orders.get(&id) else {
            return;
        };
        if lp.done {
            return;
        }
        let req = LpOrderRequest {
            lp_order_id: id,
            symbol: lp.symbol.clone(),
            side: lp.side,
            volume: lp.volume,
            limit: lp.limit,
            all_or_none: false,
            resting: true,
            revision: lp.revision,
            stop: lp.stop,
        };
        let (pos, children) = (lp.position, lp.children.clone());
        self.detach_resting(id, pos, &children);
        self.router.cancel(&req);
    }

    /// Forgets the LP-side order on the position / client orders (they fall
    /// back to our own trigger).
    fn detach_resting(&mut self, id: LpOrderId, pos: Option<PositionId>, children: &[OrderId]) {
        if let Some(p) = pos.and_then(|pid| self.st.positions.get_mut(&pid)) {
            if p.lp_tp == Some(id) {
                p.lp_tp = None;
            }
            if p.lp_sl == Some(id) {
                p.lp_sl = None;
            }
        }
        for c in children {
            if let Some(o) = self.st.orders.get_mut(c) {
                if o.lp_resting == Some(id) {
                    o.lp_resting = None;
                }
            }
        }
        self.events.push(Event::LpRestingChanged {
            lp_order_id: id,
            active: false,
        });
    }

    /// Fill of a position's LP-side TP: the close child is created now and
    /// filled at the LP price plus the markup. Volume beyond the position's
    /// free volume (a manual close raced the LP) is booked on the hedge book
    /// and flattened at once, so omnibus = A-book net + hedge holds.
    fn on_resting_tp_fill(
        &mut self,
        lp_id: LpOrderId,
        exec_id: &str,
        volume: Qty,
        price: Price,
    ) -> R<()> {
        let (symbol, side, pid) = {
            let lp = self.st.lp_orders.get_mut(&lp_id).expect("lp");
            lp.filled = Qty::from_raw(lp.filled.raw() + volume.raw());
            lp.done = lp.filled >= lp.volume;
            lp.fills.push(LpExec {
                exec_id: exec_id.to_string(),
                volume,
                price,
                ts: self.st.now,
            });
            (lp.symbol.clone(), lp.side, lp.position.expect("resting tp"))
        };
        let is_sl = self
            .st
            .positions
            .get(&pid)
            .is_some_and(|p| p.lp_sl == Some(lp_id));
        let closable = self
            .st
            .positions
            .get(&pid)
            .filter(|p| p.lp_tp == Some(lp_id) || p.lp_sl == Some(lp_id))
            .map_or(0, |p| p.free_volume().raw().min(volume.raw()));
        if closable > 0 {
            let p = self.st.positions[&pid].clone();
            let (tag, origin) = if is_sl {
                ("sl", OrderOrigin::StopLoss)
            } else {
                ("tp", OrderOrigin::TakeProfit)
            };
            let clid = format!("{tag}-{pid}-{}", self.st.seq);
            let o = NewOrder::market(p.account, &clid, &symbol, side, Qty::from_raw(closable));
            self.place_order_with(o, Some(pid), origin, false)?;
            let oid = self
                .order_by_client_id(p.account, &clid)
                .map(|o| o.id)
                .ok_or("tp child not created")?;
            self.st
                .lp_orders
                .get_mut(&lp_id)
                .expect("lp")
                .children
                .push(oid);
            *self.st.omnibus_net.entry(symbol.clone()).or_default() += side.sign() * closable;
            let g = self.st.groups[&self.st.accounts[&p.account].group].clone();
            let m = self.order_markup(&self.st.orders[&oid], &g, side).raw() * side.sign();
            let client_px = Price::from_raw(price.raw() + m);
            self.fill_child(oid, Qty::from_raw(closable), client_px, price);
        }
        let excess = volume.raw() - closable;
        if excess > 0 {
            let ex = Qty::from_raw(excess);
            *self.st.hedge_pending.entry(symbol.clone()).or_default() += side.sign() * excess;
            self.apply_hedge_fill(&symbol, side, ex, price, exec_id);
            self.send_hedge(symbol.clone(), side.opposite(), ex);
        }
        if self.st.lp_orders[&lp_id].done {
            self.detach_resting(lp_id, Some(pid), &[]);
        }
        Ok(())
    }

    fn on_lp_fill(&mut self, lp_id: LpOrderId, exec_id: &str, volume: Qty, price: Price) -> R<()> {
        // per LP order: exec ids are only unique per LP session (a simulator
        // restarts at E1), while a replayed fill repeats both
        if !self.st.lp_exec_ids.insert(format!("{lp_id}:{exec_id}")) {
            return Ok(()); // duplicate execution report
        }
        let lp = self.st.lp_orders.get(&lp_id).ok_or("unknown LP order")?;
        if lp.done {
            return Err("LP order already complete".into());
        }
        if lp.hedge {
            return self.on_hedge_fill(lp_id, exec_id, volume, price);
        }
        if lp.resting && lp.position.is_some() {
            return self.on_resting_tp_fill(lp_id, exec_id, volume, price);
        }
        let children: Vec<(OrderId, i64)> = lp
            .children
            .iter()
            .map(|c| (*c, self.st.orders[c].lp_open().raw()))
            .collect();
        let (symbol, side) = (lp.symbol.clone(), lp.side);
        let allocs = allocate(volume.raw(), &children, self.st.config.allocation);
        let allocated: i64 = allocs.iter().map(|a| a.1).sum();
        {
            let lp = self.st.lp_orders.get_mut(&lp_id).expect("lp");
            lp.filled = Qty::from_raw(lp.filled.raw() + allocated);
            lp.done = lp.filled >= lp.volume;
            lp.fills.push(LpExec {
                exec_id: exec_id.to_string(),
                volume,
                price,
                ts: self.st.now,
            });
        }
        *self.st.omnibus_net.entry(symbol.clone()).or_default() += side.sign() * allocated;
        let (done, single) = {
            let lp = &self.st.lp_orders[&lp_id];
            (lp.done, lp.children.len() == 1)
        };
        for (oid, q) in allocs {
            if single && self.coalesces(oid) {
                // tek kalem: hold the LP fill; the client sees one fill when
                // the retry chain is over (this LP order done, or no retry left)
                let now = self.st.now;
                self.st
                    .orders
                    .get_mut(&oid)
                    .expect("order")
                    .chain_fills
                    .push(LpExec {
                        exec_id: exec_id.to_string(),
                        volume: Qty::from_raw(q),
                        price,
                        ts: now,
                    });
                if done {
                    self.flush_chain(oid);
                }
                continue;
            }
            let client_px = self.client_fill_price(oid, side, price);
            self.fill_child(oid, Qty::from_raw(q), client_px, price);
        }
        let lp = &self.st.lp_orders[&lp_id];
        if lp.resting && lp.done {
            let children = lp.children.clone();
            self.detach_resting(lp_id, None, &children);
        }
        Ok(())
    }

    /// Client price of an LP fill: LP price plus the markup; an improvement
    /// on the requested price stays with us unless the group passes it on.
    fn client_fill_price(&self, oid: OrderId, side: Side, price: Price) -> Price {
        let o = &self.st.orders[&oid];
        let g = &self.st.groups[&self.st.accounts[&o.req.account].group];
        let m = self.order_markup(o, g, side).raw() * side.sign();
        let client_px = Price::from_raw(price.raw() + m);
        if !g.pass_price_improvement {
            if let Some(req) = o.req.requested_price {
                if (req.raw() - client_px.raw()) * side.sign() > 0 {
                    return req;
                }
            }
        }
        client_px
    }

    /// A market order whose LP remainder is retried (`PartialFill::Retry`)
    /// is shown to the client as one fill at the end of the chain.
    fn coalesces(&self, oid: OrderId) -> bool {
        self.st.orders[&oid].req.order_type == OrderType::Market
            && matches!(self.policy(oid), PartialFill::Retry { .. })
    }

    /// Applies the held chain fills of an order as one fill at their VWAP.
    fn flush_chain(&mut self, oid: OrderId) {
        let fills = std::mem::take(&mut self.st.orders.get_mut(&oid).expect("order").chain_fills);
        if fills.is_empty() {
            return;
        }
        let vol: i64 = fills.iter().map(|f| f.volume.raw()).sum();
        let notional: i128 = fills
            .iter()
            .map(|f| f.volume.raw() as i128 * f.price.raw() as i128)
            .sum();
        let vwap = Price::from_raw(
            money::div_round(notional, vol as i128, Rounding::HalfEven).unwrap_or(0) as i64,
        );
        let side = self.st.orders[&oid].req.side;
        let client_px = self.client_fill_price(oid, side, vwap);
        self.fill_child(oid, Qty::from_raw(vol), client_px, vwap);
    }

    fn on_lp_reject(&mut self, lp_id: LpOrderId, reason: &str) -> R<()> {
        let lp = self
            .st
            .lp_orders
            .get_mut(&lp_id)
            .ok_or("unknown LP order")?;
        if lp.done {
            return Err("LP order already complete".into());
        }
        lp.done = true;
        lp.reject_reason = Some(reason.to_string());
        if lp.resting {
            let (pos, children) = (lp.position, lp.children.clone());
            self.detach_resting(lp_id, pos, &children);
            if let Some(c) = children.first().copied() {
                let sym = self.st.orders[&c].req.symbol.clone();
                self.check_pending(&sym);
            }
            return Ok(());
        }
        if lp.hedge {
            let left = lp.side.sign() * (lp.volume.raw() - lp.filled.raw());
            let sym = lp.symbol.clone();
            *self.st.hedge_pending.entry(sym.clone()).or_default() -= left;
            return Ok(());
        }
        let children = lp.children.clone();
        let was_limit = lp.limit.is_some();
        for c in children {
            let o = &self.st.orders[&c];
            if o.status.is_terminal() {
                continue;
            }
            if was_limit && o.limit_leg().is_some() {
                // IOC limit not (fully) filled at the LP: keep waiting for the price.
                let acc = &self.st.accounts[&o.req.account];
                let g = self.st.groups[&acc.group].clone();
                let px = self
                    .client_quote(&g, &o.req.symbol)
                    .ok()
                    .map(|q| q.for_side(o.req.side));
                let o = self.st.orders.get_mut(&c).expect("order");
                o.working = false;
                o.rearm_px = px;
                self.st.pending_ids.insert(c);
                continue;
            }
            match self.policy(c) {
                PartialFill::Retry { max_attempts } if o.lp_attempts < max_attempts => {
                    // Try again at the LP with what is left, inside the same slippage cap.
                    let (sym, side) = (o.req.symbol.clone(), o.req.side);
                    let g = self.st.groups[&self.st.accounts[&o.req.account].group].clone();
                    let limit = self.slippage_limit(o, &g);
                    self.send_lp_limit(sym, side, vec![c], limit);
                    continue;
                }
                _ => {}
            }
            // the chain is over: whatever the LP gave reaches the client now, as one fill
            self.flush_chain(c);
            let o = &self.st.orders[&c];
            if o.status.is_terminal() {
                continue;
            }
            if o.filled.is_zero() {
                self.reject(c, format!("LP: {reason}"));
            } else {
                self.finish_order(c, OrderStatus::Cancelled);
            }
        }
        Ok(())
    }

    /// Applies a fill of `v` lots to client order `id` at client price
    /// `price` (LP price `lp_price`).
    fn fill_child(&mut self, id: OrderId, v: Qty, price: Price, lp_price: Price) {
        let exec = self.st.next_exec;
        self.st.next_exec += 1;
        let (account, symbol, side, close) = {
            let o = self.st.orders.get_mut(&id).expect("order");
            let prev = o.filled.raw() as i128;
            let nf = prev + v.raw() as i128;
            o.avg_price = Price::from_raw(
                money::div_round(
                    o.avg_price.raw() as i128 * prev + price.raw() as i128 * v.raw() as i128,
                    nf,
                    Rounding::HalfEven,
                )
                .unwrap_or(0) as i64,
            );
            o.filled = Qty::from_raw(nf as i64);
            (
                o.req.account,
                o.req.symbol.clone(),
                o.req.side,
                o.close_position,
            )
        };
        let status = if self.st.orders[&id].remaining().is_zero() {
            OrderStatus::Filled
        } else {
            OrderStatus::PartiallyFilled
        };
        self.set_status(id, status);
        if status == OrderStatus::Filled {
            self.st.orders.get_mut(&id).expect("order").working = false;
        }
        self.events.push(Event::OrderFilled {
            order_id: id,
            volume: v,
            price,
            status,
        });
        let acc = self.st.accounts[&account].clone();
        let g = self.st.groups[&acc.group].clone();
        let spec = self.st.symbols[&symbol].clone();
        if let Some(req) = self.st.orders[&id].req.requested_price {
            // Flow profile: how much better than requested did the client get?
            let point = spec.point().raw().max(1);
            let gain = (req.raw() - price.raw()) * side.sign() / point;
            self.st.flow.entry(account).or_default().record_fill(gain);
        }
        // Markout reference: the raw mid now, not the client price, which
        // carries the half spread and the markup and would bias every sample
        // against the client.
        let mid = self
            .st
            .quotes
            .get(&symbol)
            .map(|q| (q.bid.raw() + q.ask.raw()) / 2);
        if let Some(mid) = mid.filter(|_| self.st.orders[&id].origin == OrderOrigin::Client) {
            if self.st.markouts.len() >= MARKOUT_MAX_PENDING {
                self.st.markouts.pop_front();
            }
            self.st.markouts.push_back(MarkoutSample {
                account,
                symbol: symbol.clone(),
                sign: side.sign(),
                mid,
                price: mid,
                ts: self.st.now,
                done: 0,
            });
        }
        let mut commission = Money::zero(g.currency);
        // commission per side: the group's model, else the symbol's per-lot amount
        let c = match g.commission {
            Some(GroupCommission::PerLot { minor }) => Money::new(minor as i128, g.currency)
                .mul_ratio(v.raw() as i128, SCALE as i128, Rounding::HalfUp)
                .ok(),
            Some(GroupCommission::PerMillion { minor }) => {
                // notional in the quote currency: lots × contract size × price
                let units = v.raw() as i128 * spec.contract_size as i128; // scaled 1e8
                let notional = units * price.raw() as i128 / SCALE as i128; // scaled 1e8
                Money::from_scaled(notional, spec.quote, Rounding::HalfUp)
                    .ok()
                    .and_then(|n| self.st.quotes.convert(n, g.currency, Rounding::HalfUp).ok())
                    .and_then(|n| {
                        n.mul_ratio(
                            minor as i128,
                            1_000_000 * 10i128.pow(n.currency.minor_exponent()),
                            Rounding::HalfUp,
                        )
                        .ok()
                    })
            }
            None if !spec.commission_per_lot.is_zero() => spec
                .commission_per_lot
                .mul_ratio(v.raw() as i128, SCALE as i128, Rounding::HalfUp)
                .ok()
                .and_then(|c| self.st.quotes.convert(c, g.currency, Rounding::HalfUp).ok()),
            None => None,
        };
        if let Some(c) = c.filter(|c| c.minor != 0) {
            {
                if self
                    .post(
                        TxnKind::Commission,
                        format!("comm:{exec}"),
                        vec![
                            (acc.ledger_id, Money::new(-c.minor, c.currency)),
                            (BROKER_BOOK, c),
                        ],
                    )
                    .is_ok()
                {
                    commission = Money::new(-c.minor, c.currency);
                }
            }
        }
        let routing = self.st.orders[&id].routing;
        let o = &self.st.orders[&id];
        if let Some(src) = o.copy_from.filter(|_| o.origin == OrderOrigin::Copy) {
            // a copy order filled: its kind (open / close) stops backing off
            let close = o.close_position.is_some();
            if let Some(s) = self.st.copy_subs.get_mut(&(account, src.0)) {
                if let Some(r) = s.retry.get_mut(&src.1) {
                    if close {
                        (r.close_fails, r.close_next_ts) = (0, 0);
                    } else {
                        (r.fails, r.next_ts) = (0, 0);
                    }
                    if r.is_clear() {
                        s.retry.remove(&src.1);
                    }
                }
            }
        }
        let mut left = v;
        // the fill's commission is attributed to its first deal
        let lp_px = (routing == Routing::ABook).then_some(lp_price);
        // `copy`: the copy source of the position, read before a close removes it
        let mut deal = |e: &mut Engine,
                        pid: PositionId,
                        entry: DealEntry,
                        vol: Qty,
                        pnl: Money,
                        legs: (i128, i128),
                        copy: Option<(AccountNo, PositionId)>| {
            let c = std::mem::replace(&mut commission, Money::zero(g.currency));
            e.add_deal(id, pid, entry, vol, price, pnl, c, lp_px, legs, copy);
        };
        let copy_of =
            |e: &Engine, pid: PositionId| e.st.positions.get(&pid).and_then(|p| p.copy_from);
        if let Some(pid) = close {
            if let Some(p) = self.st.positions.get_mut(&pid) {
                p.closing = Qty::from_raw((p.closing.raw() - v.raw()).max(0));
            }
            if self.st.positions.contains_key(&pid) {
                let copy = copy_of(self, pid);
                let (pnl, broker, lp) = self.reduce_position(pid, v, price, lp_price, exec);
                deal(self, pid, DealEntry::Out, v, pnl, (broker, lp), copy);
            }
            left = Qty::ZERO;
        } else if g.margin_mode == MarginMode::Netting {
            let existing = self
                .st
                .positions
                .values()
                .find(|p| p.account == account && p.symbol == symbol)
                .map(|p| (p.id, p.side, p.volume, p.copy_from));
            if let Some((pid, pside, pvol, copy)) = existing {
                if pside != side {
                    let r = v.min(pvol);
                    let (pnl, broker, lp) = self.reduce_position(pid, r, price, lp_price, exec);
                    deal(self, pid, DealEntry::Out, r, pnl, (broker, lp), copy);
                    left = Qty::from_raw(v.raw() - r.raw());
                } else {
                    self.increase_position(pid, v, price, lp_price);
                    let z = Money::zero(g.currency);
                    deal(self, pid, DealEntry::In, v, z, (0, 0), copy);
                    left = Qty::ZERO;
                }
            }
        } else if let Some(pid) = self.st.orders[&id].position {
            if self.st.positions.contains_key(&pid) {
                self.increase_position(pid, v, price, lp_price);
                let copy = copy_of(self, pid);
                let z = Money::zero(g.currency);
                deal(self, pid, DealEntry::In, v, z, (0, 0), copy);
                left = Qty::ZERO;
            }
        }
        if left.is_positive() {
            let pid = self.st.next_id;
            self.st.next_id += 1;
            let r = &self.st.orders[&id].req;
            let pos = Position {
                id: pid,
                account,
                symbol: symbol.clone(),
                side,
                volume: left,
                closing: Qty::ZERO,
                open_price: price,
                lp_open_price: lp_price,
                sl: r.sl,
                tp: r.tp,
                trailing_points: r.trailing_points,
                routing,
                opened_ts: self.st.now,
                swap_minor: 0,
                swap_fee_minor: 0,
                copy_from: self.st.orders[&id].copy_from,
                lp_tp: None,
                lp_sl: None,
            };
            self.st.positions.insert(pid, pos);
            self.st.orders.get_mut(&id).expect("order").position = Some(pid);
            self.events.push(Event::PositionOpened { position_id: pid });
            self.sync_resting(pid);
            let copy = copy_of(self, pid);
            let z = Money::zero(g.currency);
            deal(self, pid, DealEntry::In, left, z, (0, 0), copy);
        }
        self.balance_event(account);
        self.rebalance_hedge(&symbol);
    }

    #[allow(clippy::too_many_arguments)]
    fn add_deal(
        &mut self,
        order_id: OrderId,
        position_id: PositionId,
        entry: DealEntry,
        volume: Qty,
        price: Price,
        pnl: Money,
        commission: Money,
        lp_price: Option<Price>,
        legs: (i128, i128),
        copy: Option<(AccountNo, PositionId)>,
    ) {
        let o = &self.st.orders[&order_id];
        let id = self.st.deals.len() as u64 + 1;
        self.st.deals.push(Deal {
            id,
            order_id,
            account: o.req.account,
            position_id,
            symbol: o.req.symbol.clone(),
            side: o.req.side,
            entry,
            volume,
            price,
            pnl,
            commission,
            ts: self.st.now,
            reason: o.origin,
            lp_price,
            broker_pnl: legs.0,
            lp_pnl: legs.1,
            swap: std::mem::take(&mut self.st.pending_deal_swap),
            swap_fee: std::mem::take(&mut self.st.pending_deal_swap_fee),
        });
        self.events.push(Event::DealAdded { deal_id: id });
        // Copy result: every deal of a copy position counts, whoever closes it
        // (provider-driven, the follower, SL / TP, stop-out), and the opening
        // commission too.
        if let Some((provider, _)) = copy {
            let d = self.st.deals.last().expect("deal");
            let r = d.pnl.minor + d.commission.minor + d.swap;
            if let Some(s) = self.st.copy_subs.get_mut(&(d.account, provider)) {
                s.realized += r;
            }
        }
    }

    fn increase_position(&mut self, pid: PositionId, v: Qty, price: Price, lp_price: Price) {
        let p = self.st.positions.get_mut(&pid).expect("position");
        let (a, b) = (p.volume.raw() as i128, v.raw() as i128);
        let avg = |x: Price, y: Price| {
            Price::from_raw(
                money::div_round(
                    x.raw() as i128 * a + y.raw() as i128 * b,
                    a + b,
                    Rounding::HalfEven,
                )
                .unwrap_or(0) as i64,
            )
        };
        p.open_price = avg(p.open_price, price);
        p.lp_open_price = avg(p.lp_open_price, lp_price);
        p.volume = Qty::from_raw((a + b) as i64);
    }

    /// Closes `v` lots of a position and books realized P&L. Returns the client
    /// P&L and the broker and LP legs (minor units of the account currency).
    fn reduce_position(
        &mut self,
        pid: PositionId,
        v: Qty,
        price: Price,
        lp_price: Price,
        exec: u64,
    ) -> (Money, i128, i128) {
        let p = self.st.positions[&pid].clone();
        let acc = self.st.accounts[&p.account].clone();
        let g = self.st.groups[&acc.group].clone();
        let spec = self.st.symbols[&p.symbol].clone();
        let q = &self.st.quotes;
        let pnl = risk::pnl_money(&spec, p.side, p.open_price, price, v, g.currency, q)
            .unwrap_or(Money::zero(g.currency));
        let mut postings = vec![(acc.ledger_id, pnl)];
        let mut legs = (-pnl.minor, 0i128);
        match p.routing {
            Routing::BBook => postings.push((BROKER_BOOK, Money::new(-pnl.minor, pnl.currency))),
            Routing::ABook => {
                let lp_pnl =
                    risk::pnl_money(&spec, p.side, p.lp_open_price, lp_price, v, g.currency, q)
                        .unwrap_or(Money::zero(g.currency));
                postings.push((
                    BROKER_BOOK,
                    Money::new(lp_pnl.minor - pnl.minor, pnl.currency),
                ));
                postings.push((LP_COUNTERPARTY, Money::new(-lp_pnl.minor, pnl.currency)));
                legs = (lp_pnl.minor - pnl.minor, lp_pnl.minor);
            }
        }
        let _ = self.post(TxnKind::RealizedPnl, format!("pnl:{exec}:{pid}"), postings);
        // the closed part takes its share of the accumulated swap with it
        if (p.swap_minor != 0 || p.swap_fee_minor != 0) && p.volume.raw() > 0 {
            let share = p.swap_minor * v.raw() as i128 / p.volume.raw() as i128;
            let fee = p.swap_fee_minor * v.raw() as i128 / p.volume.raw() as i128;
            if let Some(pm) = self.st.positions.get_mut(&pid) {
                pm.swap_minor -= share;
                pm.swap_fee_minor -= fee;
            }
            self.st.pending_deal_swap = share;
            self.st.pending_deal_swap_fee = fee;
        }
        let hold_secs = self.st.now.saturating_sub(p.opened_ts) / 1_000_000_000;
        self.st
            .flow
            .entry(p.account)
            .or_default()
            .record_close(hold_secs, pnl.minor, legs.0);
        let remaining = Qty::from_raw(p.volume.raw() - v.raw());
        if remaining.is_positive() {
            let pm = self.st.positions.get_mut(&pid).expect("position");
            pm.volume = remaining;
            pm.closing = pm.closing.min(remaining);
            self.sync_resting(pid);
        } else {
            self.st.positions.remove(&pid);
            for lp in [p.lp_tp, p.lp_sl].into_iter().flatten() {
                self.cancel_resting(lp);
            }
        }
        self.events.push(Event::PositionClosed {
            position_id: pid,
            volume: v,
            price,
            pnl,
            remaining,
        });
        self.negative_balance_protection(p.account);
        (pnl, legs.0, legs.1)
    }

    fn negative_balance_protection(&mut self, account: AccountNo) {
        if self.st.positions.values().any(|p| p.account == account) {
            return;
        }
        let acc = self.st.accounts[&account].clone();
        let g = &self.st.groups[&acc.group];
        let bal = self.ledger.balance(acc.ledger_id, g.currency);
        if let Some(c) = risk::negative_balance_compensation(g, bal) {
            let key = format!("nbp:{}:{account}", self.st.seq);
            if self
                .post(
                    TxnKind::NegativeBalanceProtection,
                    key,
                    vec![
                        (acc.ledger_id, c),
                        (BROKER_BOOK, Money::new(-c.minor, c.currency)),
                    ],
                )
                .is_ok()
            {
                self.st.cash_moves.push(CashMove {
                    account,
                    kind: CashMoveKind::NegativeBalanceCompensation,
                    amount: c,
                    counterparty: None,
                    ts: self.st.now,
                });
                self.events
                    .push(Event::NegativeBalanceCompensated { account, amount: c });
            }
        }
    }

    // ------------------------------------------------------------------
    // Market data driven logic
    // ------------------------------------------------------------------

    fn on_quote(&mut self, symbol: &str) {
        self.check_pending(symbol);
        self.check_sl_tp(symbol);
        self.check_margin();
    }

    fn check_pending(&mut self, symbol: &str) {
        let ids: Vec<OrderId> = self
            .pending_orders()
            .into_iter()
            .filter(|i| self.st.orders[i].req.symbol == symbol)
            .collect();
        for id in ids {
            let o = &self.st.orders[&id];
            if !o.is_pending() {
                continue; // cancelled by OCO earlier in this loop
            }
            if o.lp_resting.is_some() {
                continue; // the LP decides: it rests there
            }
            let g = self.st.groups[&self.st.accounts[&o.req.account].group].clone();
            let Ok(q) = self.client_quote(&g, symbol) else {
                continue;
            };
            let px = q.for_side(o.req.side);
            let s = o.req.side.sign();
            // After an unfilled LP attempt the market must improve on that price first.
            if o.rearm_px.is_some_and(|r| (r.raw() - px.raw()) * s <= 0) {
                continue;
            }
            // buy limit: ask <= limit; sell limit: bid >= limit
            let limit_hit = |l: Price| (l.raw() - px.raw()) * s >= 0;
            // buy stop: ask >= stop; sell stop: bid <= stop
            let stop_hit = |st: Price| (px.raw() - st.raw()) * s >= 0;
            let fire = match o.req.order_type {
                OrderType::Market => false,
                OrderType::Limit => o.req.limit_price.is_some_and(limit_hit),
                OrderType::Stop => o.req.stop_price.is_some_and(stop_hit),
                OrderType::StopLimit => {
                    if !o.stop_triggered && o.req.stop_price.is_some_and(stop_hit) {
                        self.st.orders.get_mut(&id).expect("order").stop_triggered = true;
                        self.events.push(Event::OrderTriggered { order_id: id });
                    }
                    let o = &self.st.orders[&id];
                    o.stop_triggered && o.req.limit_price.is_some_and(limit_hit)
                }
            };
            if fire {
                // re-check margin at activation
                if let Err(e) = self.validate_order(id, &g) {
                    self.reject(id, e);
                    continue;
                }
                self.events.push(Event::OrderTriggered { order_id: id });
                self.execute(id);
            }
        }
    }

    fn close_internal(
        &mut self,
        pid: PositionId,
        tag: &str,
        origin: OrderOrigin,
    ) -> Option<OrderId> {
        let p = self.st.positions.get(&pid)?;
        let v = p.free_volume();
        if !v.is_positive() {
            return None;
        }
        let clid = format!("{tag}-{pid}-{}", self.st.seq);
        let o = NewOrder::market(p.account, &clid, &p.symbol, p.side.opposite(), v);
        let account = p.account;
        self.place_order(o, Some(pid), origin).ok()?;
        self.order_by_client_id(account, &clid).map(|o| o.id)
    }

    fn check_sl_tp(&mut self, symbol: &str) {
        let ids: Vec<PositionId> = self
            .st
            .positions
            .values()
            .filter(|p| p.symbol == symbol)
            .map(|p| p.id)
            .collect();
        for pid in ids {
            let Some(p) = self.st.positions.get(&pid) else {
                continue;
            };
            let g = self.st.groups[&self.st.accounts[&p.account].group].clone();
            let Ok(q) = self.client_quote(&g, symbol) else {
                continue;
            };
            let px = q.for_side(p.side.opposite());
            let s = p.side.sign();
            // server-side trailing stop: activates once in profit by distance
            if let Some(t) = p.trailing_points {
                let dist = self.st.symbols[symbol].point().raw() * t;
                let cand = px.raw() - s * dist;
                let in_profit = (cand - p.open_price.raw()) * s > 0;
                let better = p.sl.is_none_or(|sl| (cand - sl.raw()) * s > 0);
                if in_profit && better {
                    let pm = self.st.positions.get_mut(&pid).expect("position");
                    pm.sl = Some(Price::from_raw(cand));
                    let (sl, tp) = (pm.sl, pm.tp);
                    self.events.push(Event::PositionModified {
                        position_id: pid,
                        sl,
                        tp,
                    });
                    self.sync_resting_leg(pid, true);
                }
            }
            let p = &self.st.positions[&pid];
            let sl_hit = p.lp_sl.is_none() && p.sl.is_some_and(|sl| (px.raw() - sl.raw()) * s <= 0);
            let tp_hit = p.lp_tp.is_none() && p.tp.is_some_and(|tp| (px.raw() - tp.raw()) * s >= 0);
            if sl_hit || tp_hit {
                if sl_hit {
                    self.close_internal(pid, "sl", OrderOrigin::StopLoss);
                } else {
                    self.close_internal(pid, "tp", OrderOrigin::TakeProfit);
                }
            }
        }
    }

    fn check_margin(&mut self) {
        let accounts: BTreeSet<AccountNo> = self.st.positions.values().map(|p| p.account).collect();
        for a in accounts {
            let g = self.st.groups[&self.st.accounts[&a].group].clone();
            // liquidate largest loss first until above stop-out level
            while let Ok(r) = self.account_risk(a) {
                let mc = r.is_margin_call(&g);
                let was = self.st.accounts[&a].margin_call;
                if mc && !was {
                    self.events.push(Event::MarginCall { account: a });
                }
                self.st.accounts.get_mut(&a).expect("account").margin_call = mc;
                if !r.is_stop_out(&g) {
                    break;
                }
                let mut cands = Vec::new();
                for p in self.positions_of(a) {
                    if !p.free_volume().is_positive() {
                        continue;
                    }
                    let spec = &self.st.symbols[&p.symbol];
                    let Ok(q) = self.client_quote(&g, &p.symbol) else {
                        continue;
                    };
                    if let Ok(pnl) =
                        risk::floating_pnl(spec, &p.view(), q, g.currency, &self.st.quotes)
                    {
                        cands.push((p.id, pnl));
                    }
                }
                let Some(&pid) = risk::liquidation_order(&cands).first() else {
                    break;
                };
                self.events.push(Event::StopOut {
                    account: a,
                    position_id: pid,
                });
                let routing = self.st.positions[&pid].routing;
                let closed = self
                    .close_internal(pid, "so", OrderOrigin::StopOut)
                    .and_then(|oid| self.st.orders.get(&oid))
                    .is_some_and(|o| o.status == OrderStatus::Filled);
                if !closed || routing == Routing::ABook {
                    // A-book: wait for the LP fill before re-evaluating
                    break;
                }
            }
        }
    }

    fn expire_orders(&mut self) {
        let now = self.st.now;
        if self.st.pending_ids.is_empty() {
            return;
        }
        let ids: Vec<OrderId> = self
            .pending_orders()
            .into_iter()
            .filter(|i| self.st.orders[i].req.expire_at.is_some_and(|t| t <= now))
            .collect();
        for id in ids {
            self.finish_order(id, OrderStatus::Expired);
        }
    }

    /// Daily swap: once per UTC day (idempotent across restarts), skipped on
    /// weekends when configured, three days on the symbol's triple day,
    /// scaled by the group multiplier; charged to the client against the
    /// book's counterparty and accumulated on the position for the reports.
    fn rollover(&mut self) -> R<()> {
        let day = self.st.now / 86_400_000_000_000;
        let skip = |e: &mut Engine, reason: &str| {
            e.events.push(Event::Rollover {
                applied: false,
                positions: 0,
                reason: reason.into(),
            });
            Ok(())
        };
        if !self.st.swap.enabled {
            return skip(self, "disabled");
        }
        // stored as day + 1 so a fresh journal (0) never looks "already done"
        if day + 1 == self.st.last_rollover_day {
            return skip(self, "already applied today");
        }
        let weekday = risk::weekday_utc(self.st.now);
        if self.st.swap.skip_weekend && (weekday == 0 || weekday == 6) {
            return skip(self, "weekend");
        }
        if self.st.calendar.is_holiday(self.st.now) {
            return skip(self, "holiday");
        }
        self.st.last_rollover_day = day + 1;
        let ps: Vec<Position> = self.st.positions.values().cloned().collect();
        let mut touched = BTreeSet::new();
        let mut n = 0u32;
        for p in ps {
            let spec = self.st.symbols[&p.symbol].clone();
            let acc = self.st.accounts[&p.account].clone();
            let g = self.st.groups[&acc.group].clone();
            let days = if weekday == spec.triple_swap_day {
                3
            } else {
                1
            };
            let swap_free = g.swap_multiplier_pct == 0;
            let m = if swap_free {
                // swap-free: flat fee per lot after the grace period
                let age_days = self.st.now.saturating_sub(p.opened_ts) / 86_400_000_000_000;
                if g.swap_free_fee_per_lot <= 0 || age_days < g.swap_free_grace_days as u64 {
                    continue;
                }
                let scaled = -(g.swap_free_fee_per_lot as i128)
                    * (money::SCALE as i128 / 100)
                    * p.volume.raw() as i128
                    / money::SCALE as i128
                    * days;
                Money::from_scaled(scaled, g.currency, Rounding::HalfEven).map_err(e2s)?
            } else {
                let scaled =
                    risk::swap_scaled(&spec, p.side, p.volume, g.swap_multiplier_pct) * days;
                if scaled == 0 {
                    continue;
                }
                let m = Money::from_scaled(scaled, spec.quote, Rounding::HalfEven).map_err(e2s)?;
                self.st
                    .quotes
                    .convert(m, g.currency, Rounding::HalfEven)
                    .map_err(e2s)?
            };
            if m.is_zero() {
                continue;
            }
            // The swap-free fee is our own charge: the broker book takes it on
            // either book (the LP never sees it). A real swap is settled by
            // the book's counterparty.
            let (cp, key) = match (swap_free, p.routing) {
                (true, _) => (BROKER_BOOK, format!("swapfree:{day}:{}", p.id)),
                (false, Routing::ABook) => (LP_COUNTERPARTY, format!("swap:{day}:{}", p.id)),
                (false, Routing::BBook) => (BROKER_BOOK, format!("swap:{day}:{}", p.id)),
            };
            self.post(
                TxnKind::Swap,
                key,
                vec![(acc.ledger_id, m), (cp, Money::new(-m.minor, m.currency))],
            )?;
            if let Some(pm) = self.st.positions.get_mut(&p.id) {
                pm.swap_minor += m.minor;
                if swap_free {
                    pm.swap_fee_minor += m.minor;
                }
            }
            touched.insert(p.account);
            n += 1;
        }
        for a in touched {
            self.balance_event(a);
        }
        self.events.push(Event::Rollover {
            applied: true,
            positions: n,
            reason: if weekday == 3 {
                "triple day".into()
            } else {
                String::new()
            },
        });
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Copy trading: every follower position mirrors one provider position.
// Runs after each command, so a replay of the journal reproduces it.
// ----------------------------------------------------------------------

/// Wait after a copy order of a provider position failed; it doubles with
/// every failure in a row, up to `COPY_RETRY_MAX_NS`.
const COPY_RETRY_NS: u64 = 5_000_000_000;
const COPY_RETRY_MAX_NS: u64 = 3_600_000_000_000;
/// Failures in a row after which a provider position is no longer opened for
/// the follower (counted as copied; its closes are still followed).
const COPY_OPEN_MAX_FAILS: u32 = 8;

fn copy_backoff_ns(fails: u32) -> u64 {
    let doublings = fails.saturating_sub(1).min(20);
    COPY_RETRY_NS
        .saturating_mul(1 << doublings)
        .min(COPY_RETRY_MAX_NS)
}

type CopyKey = (AccountNo, AccountNo);
type CopySrc = (AccountNo, PositionId);

impl Engine {
    /// Active and stopped subscriptions.
    pub fn copy_subscriptions(&self) -> Vec<&CopySubscription> {
        self.st.copy_subs.values().collect()
    }

    fn copy_subscribe(
        &mut self,
        follower: AccountNo,
        provider: AccountNo,
        ratio_bps: u32,
        equity_stop_pct: u32,
        perf_fee_bps: u32,
    ) -> R<()> {
        if follower == provider {
            return Err("cannot copy yourself".into());
        }
        if !(1..=100_000).contains(&ratio_bps) || equity_stop_pct > 100 || perf_fee_bps > 5_000 {
            return Err("ratio 1..100000 bps, equity stop 0..100 %, fee 0..5000 bps".into());
        }
        let fa = self.st.accounts.get(&follower).ok_or("unknown follower")?;
        let pa = self.st.accounts.get(&provider).ok_or("unknown provider")?;
        let (fg, pg) = (&self.st.groups[&fa.group], &self.st.groups[&pa.group]);
        if fg.margin_mode != MarginMode::Hedging {
            return Err("follower account must be in a hedging group".into());
        }
        if fg.currency != pg.currency {
            return Err("follower and provider currencies differ".into());
        }
        // no chains, in either direction
        let subs = || self.st.copy_subs.values().filter(|s| s.active);
        if subs().any(|s| s.follower == provider) {
            return Err("provider is itself copying (no chains)".into());
        }
        if subs().any(|s| s.provider == follower) {
            return Err("follower is itself copied (no chains)".into());
        }
        let key = (follower, provider);
        if self.st.copy_subs.get(&key).is_some_and(|p| p.closing) {
            return Err("copies of the stopped subscription are still being closed".into());
        }
        // new terms: the result so far is settled at the fee it was earned
        // under; a part the follower cannot pay now is waived, never charged
        // later at the new fee
        if self
            .st
            .copy_subs
            .get(&key)
            .is_some_and(|p| p.perf_fee_bps != perf_fee_bps)
        {
            self.copy_settle_one(key)?;
            let s = self.st.copy_subs.get_mut(&key).expect("sub");
            s.hwm = s.hwm.max(s.realized);
        }
        // after the settlement: the equity stop counts from what is left
        let eq = self.account_risk(follower)?.equity.minor;
        let prev = self.st.copy_subs.get(&key);
        let running = prev.is_some_and(|p| p.active);
        let old_ratio = prev.map_or(ratio_bps, |p| p.ratio_bps);
        // Copies already made stay managed (never liquidated by a re-subscribe).
        // After a stop, those of provider positions closed meanwhile were the
        // follower's to keep and are no longer tracked. A higher ratio applies
        // to new provider volume only (no top-up in the middle of a trade); a
        // lower one brings the copies down on the next pass.
        let mut copied = prev.map(|p| p.copied.clone()).unwrap_or_default();
        copied.retain(|pid, sent| match self.st.positions.get(pid) {
            Some(p) => {
                let (sym, v) = (&p.symbol, p.volume.raw());
                let more = self.copy_want(sym, v, ratio_bps) - self.copy_want(sym, v, old_ratio);
                *sent += more.max(0);
                true
            }
            None => running,
        });
        let sub = CopySubscription {
            follower,
            provider,
            ratio_bps,
            equity_stop_pct,
            perf_fee_bps,
            // a change of a running subscription keeps its start; after a stop
            // only provider positions opened from now on are new
            since_ts: prev
                .filter(|p| p.active)
                .map_or(self.st.now, |p| p.since_ts),
            start_equity: eq,
            realized: prev.map_or(0, |p| p.realized),
            hwm: prev.map_or(0, |p| p.hwm),
            fees_paid: prev.map_or(0, |p| p.fees_paid),
            copied,
            retry: prev.map(|p| p.retry.clone()).unwrap_or_default(),
            active: true,
            stopped_reason: None,
            closing: false,
        };
        self.st.copy_subs.insert(key, sub);
        self.events.push(Event::CopyChanged {
            follower,
            provider,
            active: true,
        });
        Ok(())
    }

    /// Follower lots for `pvol` provider lots of `symbol` (rounded down to the lot step).
    fn copy_want(&self, symbol: &str, pvol: i64, ratio_bps: u32) -> i64 {
        let step = self
            .st
            .symbols
            .get(symbol)
            .map_or(1, |s| s.lot_step.raw().max(1));
        ((pvol as i128 * ratio_bps as i128 / 10_000) as i64 / step) * step
    }

    /// Ends a subscription; with `close` its copies are closed (see `closing`).
    fn copy_stop(&mut self, key: CopyKey, reason: &str, close: bool) {
        let s = self.st.copy_subs.get_mut(&key).expect("sub");
        s.active = false;
        s.stopped_reason = Some(reason.into());
        s.closing |= close;
        if s.closing {
            self.copy_wind_down(key);
        }
        self.events.push(Event::CopyChanged {
            follower: key.0,
            provider: key.1,
            active: false,
        });
    }

    fn is_copy_open(o: &Order, follower: AccountNo) -> bool {
        o.req.account == follower
            && o.copy_from.is_some()
            && o.close_position.is_none()
            && !o.status.is_terminal()
    }

    /// Follower lots (signed by nothing: all copies of one provider position
    /// share its side) still open or being opened for `src`.
    fn copy_held(&self, follower: AccountNo, src: CopySrc) -> i64 {
        let open: i64 = self
            .st
            .positions
            .values()
            .filter(|p| p.account == follower && p.copy_from == Some(src))
            .map(|p| p.free_volume().raw())
            .sum();
        let working: i64 = self
            .st
            .orders
            .values()
            .filter(|o| Self::is_copy_open(o, follower) && o.copy_from == Some(src))
            .map(|o| o.remaining().raw())
            .sum();
        open + working
    }

    /// Provider positions the follower still holds a copy of, or is still
    /// opening one for (an open in flight at the LP).
    fn copy_sources(&self, follower: AccountNo, provider: AccountNo) -> BTreeSet<CopySrc> {
        let positions = self
            .st
            .positions
            .values()
            .filter(|p| p.account == follower)
            .filter_map(|p| p.copy_from);
        let opening = self
            .st
            .orders
            .values()
            .filter(|o| Self::is_copy_open(o, follower))
            .filter_map(|o| o.copy_from);
        positions
            .chain(opening)
            .filter(|s| s.0 == provider)
            .collect()
    }

    /// No back-off pending for the copy opens (or, with `close`, the copy
    /// closes) of provider position `pid`.
    fn copy_due(&self, key: CopyKey, pid: PositionId, close: bool) -> bool {
        self.st.copy_subs[&key]
            .retry
            .get(&pid)
            .is_none_or(|r| self.st.now >= if close { r.close_next_ts } else { r.next_ts })
    }

    fn copy_set_sent(&mut self, key: CopyKey, pid: PositionId, lots: i64) {
        if let Some(s) = self.st.copy_subs.get_mut(&key) {
            s.copied.insert(pid, lots);
        }
    }

    /// A copy order for `src` did not (fully) go through: `unsent` lots of an
    /// open come off `copied` (they are sent again), and the position's opens
    /// (or, with `close`, its closes) back off.
    fn copy_failed(&mut self, follower: AccountNo, src: CopySrc, unsent: i64, close: bool) {
        let now = self.st.now;
        let Some(s) = self.st.copy_subs.get_mut(&(follower, src.0)) else {
            return;
        };
        if let Some(c) = s.copied.get_mut(&src.1) {
            *c = (*c - unsent).max(0);
        }
        let r = s.retry.entry(src.1).or_default();
        if close {
            r.close_fails = r.close_fails.saturating_add(1);
            r.close_next_ts = now.saturating_add(copy_backoff_ns(r.close_fails));
        } else {
            r.fails = r.fails.saturating_add(1);
            r.next_ts = now.saturating_add(copy_backoff_ns(r.fails));
        }
    }

    /// Terminal hook (rejected / cancelled): a copy order that ended with an
    /// unfilled remainder, at placement or later at the LP.
    fn copy_order_ended(&mut self, id: OrderId) {
        let o = &self.st.orders[&id];
        let Some(src) = o.copy_from.filter(|_| o.origin == OrderOrigin::Copy) else {
            return;
        };
        let rem = o.remaining().raw();
        if rem <= 0 {
            return;
        }
        let close = o.close_position.is_some();
        let unsent = if close { 0 } else { rem };
        let follower = o.req.account;
        self.copy_failed(follower, src, unsent, close);
    }

    fn copy_order_id(&mut self, src: CopySrc) -> String {
        self.st.copy_seq += 1;
        format!("copy:{}:{}:{}", src.0, src.1, self.st.copy_seq)
    }

    /// Opens `v` lots for `src` (already counted in `copied`: a rejection now
    /// or at the LP takes the unfilled part back off through the hook).
    fn copy_open(&mut self, follower: AccountNo, src: CopySrc, symbol: &str, side: Side, v: Qty) {
        let clid = self.copy_order_id(src);
        let mut o = NewOrder::market(follower, &clid, symbol, side, v);
        o.platform = risk::Platform::Copy;
        self.st.pending_copy = Some(src);
        let r = self.place_order(o, None, OrderOrigin::Copy);
        self.st.pending_copy = None;
        if r.is_err() {
            self.copy_failed(follower, src, v.raw(), false);
        }
    }

    /// Closes up to `v` lots of the follower's copies of `src`, oldest first.
    fn copy_reduce(&mut self, follower: AccountNo, src: CopySrc, mut v: i64) {
        let ps: Vec<(PositionId, String, Side, i64)> = self
            .st
            .positions
            .values()
            .filter(|p| {
                p.account == follower && p.copy_from == Some(src) && p.free_volume().is_positive()
            })
            .map(|p| (p.id, p.symbol.clone(), p.side, p.free_volume().raw()))
            .collect();
        for (pid, symbol, side, free) in ps {
            if v <= 0 {
                break;
            }
            let take = free.min(v);
            v -= take;
            let clid = self.copy_order_id(src);
            let mut o = NewOrder::market(
                follower,
                &clid,
                &symbol,
                side.opposite(),
                Qty::from_raw(take),
            );
            o.platform = risk::Platform::Copy;
            self.st.pending_copy = Some(src);
            let r = self.place_order(o, Some(pid), OrderOrigin::Copy);
            self.st.pending_copy = None;
            if r.is_err() {
                self.copy_failed(follower, src, 0, true);
            }
        }
    }

    /// Closes the copies of a stopped subscription (with back-off after failed
    /// closes); an open still in flight is closed once it fills.
    fn copy_wind_down(&mut self, key: CopyKey) {
        let (follower, provider) = key;
        for src in self.copy_sources(follower, provider) {
            if self.copy_due(key, src.1, true) {
                self.copy_reduce(follower, src, i64::MAX);
            }
        }
        if self.copy_sources(follower, provider).is_empty() {
            let s = self.st.copy_subs.get_mut(&key).expect("sub");
            s.closing = false;
            s.retry.clear();
        }
    }

    fn copy_reconcile(&mut self) {
        let keys: Vec<CopyKey> = self
            .st
            .copy_subs
            .iter()
            .filter(|(_, s)| s.active || s.closing)
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            let (follower, provider) = key;
            let s = &self.st.copy_subs[&key];
            if !s.active {
                self.copy_wind_down(key);
                continue;
            }
            // equity stop
            if s.equity_stop_pct > 0 {
                if let Ok(r) = self.account_risk(follower) {
                    let floor = s.start_equity * (100 - s.equity_stop_pct as i128) / 100;
                    if r.equity.minor < floor {
                        self.copy_stop(key, "equity stop", true);
                        continue;
                    }
                }
            }
            let (since, ratio) = (s.since_ts, s.ratio_bps);
            // provider positions opened since the subscription, and the ones
            // already copied (a changed subscription keeps managing them)
            let targets: Vec<(PositionId, String, Side, i64)> = self
                .st
                .positions
                .values()
                .filter(|p| {
                    p.account == provider && (p.opened_ts >= since || s.copied.contains_key(&p.id))
                })
                .map(|p| (p.id, p.symbol.clone(), p.side, p.volume.raw()))
                .collect();
            for (pid, symbol, side, pvol) in &targets {
                // desired follower lots
                let want = self.copy_want(symbol, *pvol, ratio);
                let min = self.st.symbols.get(symbol).map_or(0, |s| s.min_lot.raw());
                let s = &self.st.copy_subs[&key];
                let sent = s.copied.get(pid).copied().unwrap_or(0);
                let fails = s.retry.get(pid).map_or(0, |r| r.fails);
                let src = (provider, *pid);
                if want > sent {
                    // provider opened / increased: send only the new part (a
                    // follower's own close is never re-opened)
                    if !self.copy_due(key, *pid, false) {
                        // the open backs off
                    } else if fails >= COPY_OPEN_MAX_FAILS {
                        // keeps failing (margin, volume limits): skip it
                        self.copy_set_sent(key, *pid, want);
                    } else if want - sent >= min {
                        self.copy_set_sent(key, *pid, want);
                        let v = Qty::from_raw(want - sent);
                        self.copy_open(follower, src, symbol, *side, v);
                    }
                    // below the minimum lot: waits for more provider volume
                    continue;
                }
                if want < sent {
                    self.copy_set_sent(key, *pid, want);
                }
                // provider reduced, or an earlier reduce did not fill: bring
                // the copies down to `want` (closes in flight are not held)
                let held = self.copy_held(follower, src);
                if held > want && self.copy_due(key, *pid, true) {
                    self.copy_reduce(follower, src, held - want);
                }
            }
            // provider positions gone: close what is left of their copies
            let live: BTreeSet<PositionId> = targets.iter().map(|t| t.0).collect();
            let gone: Vec<PositionId> = self.st.copy_subs[&key]
                .copied
                .keys()
                .filter(|p| !live.contains(p))
                .copied()
                .collect();
            for pid in gone {
                let src = (provider, pid);
                if self.copy_due(key, pid, true) && self.copy_held(follower, src) > 0 {
                    self.copy_reduce(follower, src, i64::MAX);
                }
                // an open still in flight keeps the entry: it is closed once filled
                if !self.copy_sources(follower, provider).contains(&src) {
                    if let Some(s) = self.st.copy_subs.get_mut(&key) {
                        s.copied.remove(&pid);
                        s.retry.remove(&pid);
                    }
                }
            }
        }
    }

    /// Performance fee: `perf_fee_bps` of the realized copy result above the
    /// high-water mark moves from the follower to the provider.
    fn copy_settle(&mut self, provider: AccountNo) -> R<()> {
        let keys: Vec<CopyKey> = self
            .st
            .copy_subs
            .keys()
            .filter(|k| k.1 == provider)
            .copied()
            .collect();
        for k in keys {
            self.copy_settle_one(k)?;
        }
        Ok(())
    }

    /// Settles one subscription. The fee never takes more than the follower
    /// could withdraw (no negative balance, no stop-out; with open positions
    /// it may take all free margin, which can flag a margin call); the part of the
    /// gain left unpaid stays above the mark for a later settlement. With a
    /// zero fee the mark still moves, so a later fee is never charged on
    /// results earned under the old terms.
    fn copy_settle_one(&mut self, k: CopyKey) -> R<()> {
        let s = self.st.copy_subs[&k].clone();
        let gain = s.realized - s.hwm;
        if gain <= 0 {
            return Ok(());
        }
        let bps = s.perf_fee_bps as i128;
        let due = gain * bps / 10_000;
        let free = self
            .account_risk(s.follower)
            .map_or(0, |r| r.free_margin.minor.min(r.balance.minor))
            .max(0);
        let fee = due.min(free);
        // the mark moves over the part of the gain the fee was taken on
        let charged = if fee == due { gain } else { fee * 10_000 / bps };
        if fee > 0 {
            let fa = self.st.accounts[&s.follower].clone();
            let pa = self.st.accounts[&s.provider].clone();
            let ccy = self.st.groups[&fa.group].currency;
            let m = Money::new(fee, ccy);
            let day = self.st.now / 86_400_000_000_000;
            self.post(
                TxnKind::Adjustment,
                format!(
                    "copyfee:{day}:{}:{}:{}:{}",
                    s.follower, s.provider, s.hwm, s.realized
                ),
                vec![(fa.ledger_id, Money::new(-fee, ccy)), (pa.ledger_id, m)],
            )?;
            self.balance_event(s.follower);
            self.balance_event(s.provider);
            let ts = self.st.now;
            self.st.cash_moves.extend([
                CashMove {
                    account: s.follower,
                    kind: CashMoveKind::CopyFee,
                    amount: Money::new(-fee, ccy),
                    counterparty: Some(s.provider),
                    ts,
                },
                CashMove {
                    account: s.provider,
                    kind: CashMoveKind::CopyFeeIncome,
                    amount: m,
                    counterparty: Some(s.follower),
                    ts,
                },
            ]);
            self.events.push(Event::CopyFee {
                follower: s.follower,
                provider: s.provider,
                amount: m,
            });
        }
        let sm = self.st.copy_subs.get_mut(&k).expect("sub");
        sm.hwm += charged;
        sm.fees_paid += fee;
        Ok(())
    }
}

impl Engine {
    /// Measures pending fills of `symbol` whose horizons have passed: the raw
    /// mid move (no markup) since the fill in the client's favour, in points.
    fn resolve_markouts(&mut self, symbol: &str) {
        if self.st.markouts.is_empty() {
            return;
        }
        let Some(q) = self.st.quotes.get(symbol) else {
            return;
        };
        let Some(point) = self.st.symbols.get(symbol).map(|s| s.point().raw().max(1)) else {
            return;
        };
        let mid = (q.bid.raw() + q.ask.raw()) / 2;
        let now = self.st.now;
        let all = (1u8 << risk::FlowStats::MARKOUT_SECS.len()) - 1;
        let mut i = 0;
        while i < self.st.markouts.len() {
            let m = &mut self.st.markouts[i];
            if m.symbol == symbol && m.mid == 0 {
                // no reference mid (snapshot of an older engine): drop it
                m.done = all;
            } else if m.symbol == symbol {
                for (h, secs) in risk::FlowStats::MARKOUT_SECS.iter().enumerate() {
                    if m.done & (1 << h) == 0 && now >= m.ts + secs * 1_000_000_000 {
                        m.done |= 1 << h;
                        let pts = (mid - m.mid) * m.sign / point;
                        let acc = m.account;
                        self.st.flow.entry(acc).or_default().record_markout(h, pts);
                    }
                }
            }
            if self.st.markouts[i].done == all {
                self.st.markouts.remove(i);
            } else {
                i += 1;
            }
        }
    }
}
