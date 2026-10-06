//! The single-writer OMS/risk/ledger state machine.

use crate::alloc::{allocate, AllocationMode};
use crate::router::{LpOrderRequest, LpRouter, NullRouter};
use crate::types::*;
use ledger::{AccountId, AccountKind, Ledger, LedgerSnapshot, Posting, TxnKind, TxnRequest};
use money::{Money, Price, Qty, Rounding, SCALE};
use risk::{AccountRisk, OrderIntent, PartialFill, Quote, QuoteBook, RiskError};
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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub struct EngineConfig {
    pub allocation: AllocationMode,
    /// Queue A-book market orders and send one LP order per (symbol, side)
    /// on `Command::FlushLp`.
    pub aggregate_a_book: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
struct State {
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
}

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
    /// Deal by id.
    pub fn deal(&self, id: u64) -> Option<&Deal> {
        id.checked_sub(1)
            .and_then(|i| self.st.deals.get(usize::try_from(i).ok()?))
    }
    /// All deals, oldest first.
    pub fn deals(&self) -> &[Deal] {
        &self.st.deals
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
        let margin = risk::total_margin(g, &self.st.symbols, views, &self.st.quotes)?;
        AccountRisk::new(balance, floating, margin)
    }

    fn markup(&self, g: &GroupConfig, symbol: &str) -> Price {
        let point = self.st.symbols.get(symbol).map_or(0, |s| s.point().raw());
        Price::from_raw(point * g.markup_points)
    }

    fn client_quote(&self, g: &GroupConfig, symbol: &str) -> Result<Quote, RiskError> {
        let q = self
            .st
            .quotes
            .get(symbol)
            .ok_or_else(|| RiskError::NoQuote(symbol.into()))?;
        Ok(q.with_markup(self.markup(g, symbol)))
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
        for (sym, v) in &self.st.omnibus_net {
            if *v != *net.get(sym.as_str()).unwrap_or(&0) {
                return Err(format!(
                    "omnibus {sym} {v} != clients {:?}",
                    net.get(sym.as_str())
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
                self.place_order(o, Some(*position_id), OrderOrigin::Client)?;
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
            Command::FlushLp => self.flush_lp(),
            Command::Rollover => self.rollover()?,
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
        self.check_pending(&sym);
        Ok(())
    }

    fn place_order(
        &mut self,
        req: NewOrder,
        close: Option<PositionId>,
        origin: OrderOrigin,
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
        let order = Order {
            id,
            req,
            status: OrderStatus::New,
            filled: Qty::ZERO,
            avg_price: Price::ZERO,
            routing: g.routing,
            close_position: close,
            position: None,
            stop_triggered: false,
            working: false,
            reject_reason: None,
            created_ts: self.st.now,
            origin,
            rearm_px: None,
            lp_attempts: 0,
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
        let o = &self.st.orders[&id];
        if o.req.order_type == OrderType::Market {
            self.execute(id);
        } else {
            let sym = o.req.symbol.clone();
            self.st.pending_ids.insert(id);
            self.check_pending(&sym);
        }
        Ok(())
    }

    fn validate_order(&self, id: OrderId, g: &GroupConfig) -> R<()> {
        let o = &self.st.orders[&id];
        let r = &o.req;
        let spec = self.st.symbols.get(&r.symbol).ok_or("unknown symbol")?;
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
            g,
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
        }
    }

    /// Terminal transition for cancel/expire (keeps partial fills).
    fn finish_order(&mut self, id: OrderId, to: OrderStatus) {
        self.release_closing(id);
        self.set_status(id, to);
        self.st.orders.get_mut(&id).expect("order").working = false;
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
                    let m = self.markup(&g, &o.req.symbol).raw() * o.req.side.sign();
                    let lp_limit = Price::from_raw(l.raw() - m);
                    self.send_lp_limit(o.req.symbol.clone(), o.req.side, vec![id], Some(lp_limit));
                } else if self.st.config.aggregate_a_book && o.close_position.is_none() {
                    self.st.pending_lp.push(id);
                } else {
                    self.send_lp(o.req.symbol.clone(), o.req.side, vec![id]);
                }
            }
        }
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
                .map(|c| self.st.orders[c].remaining().raw())
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
            },
        );
        self.router.send(&req);
        self.events.push(Event::LpOrderSent {
            lp_order_id: id,
            volume,
        });
    }

    /// Partial-fill policy of the order's group.
    fn policy(&self, id: OrderId) -> PartialFill {
        let o = &self.st.orders[&id];
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

    fn on_lp_fill(&mut self, lp_id: LpOrderId, exec_id: &str, volume: Qty, price: Price) -> R<()> {
        if !self.st.lp_exec_ids.insert(exec_id.to_string()) {
            return Ok(()); // duplicate execution report
        }
        let lp = self.st.lp_orders.get(&lp_id).ok_or("unknown LP order")?;
        if lp.done {
            return Err("LP order already complete".into());
        }
        let children: Vec<(OrderId, i64)> = lp
            .children
            .iter()
            .map(|c| (*c, self.st.orders[c].remaining().raw()))
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
        for (oid, q) in allocs {
            let acc = &self.st.accounts[&self.st.orders[&oid].req.account];
            let g = &self.st.groups[&acc.group];
            let m = self.markup(g, &symbol).raw() * side.sign();
            self.fill_child(
                oid,
                Qty::from_raw(q),
                Price::from_raw(price.raw() + m),
                price,
            );
        }
        Ok(())
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
                    // Try again at the LP with what is left.
                    let (sym, side) = (o.req.symbol.clone(), o.req.side);
                    self.send_lp(sym, side, vec![c]);
                    continue;
                }
                PartialFill::BookRemainder => {
                    // The broker book takes the remainder at the client price.
                    let acc = &self.st.accounts[&o.req.account];
                    let g = self.st.groups[&acc.group].clone();
                    // No quote: fall through to cancel.
                    if let Ok(q) = self.client_quote(&g, &o.req.symbol) {
                        let price = q.for_side(o.req.side);
                        let v = o.remaining();
                        self.fill_child(c, v, price, price);
                        continue;
                    }
                }
                _ => {}
            }
            let o = &self.st.orders[&c];
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
        let mut commission = Money::zero(g.currency);
        // commission per lot per side
        if !spec.commission_per_lot.is_zero() {
            let c = spec
                .commission_per_lot
                .mul_ratio(v.raw() as i128, SCALE as i128, Rounding::HalfUp)
                .ok()
                .and_then(|c| self.st.quotes.convert(c, g.currency, Rounding::HalfUp).ok());
            if let Some(c) = c {
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
        let mut left = v;
        // the fill's commission is attributed to its first deal
        let lp_px = (routing == Routing::ABook).then_some(lp_price);
        let mut deal = |e: &mut Engine,
                        pid: PositionId,
                        entry: DealEntry,
                        vol: Qty,
                        pnl: Money,
                        legs: (i128, i128)| {
            let c = std::mem::replace(&mut commission, Money::zero(g.currency));
            e.add_deal(id, pid, entry, vol, price, pnl, c, lp_px, legs);
        };
        if let Some(pid) = close {
            if let Some(p) = self.st.positions.get_mut(&pid) {
                p.closing = Qty::from_raw((p.closing.raw() - v.raw()).max(0));
            }
            if self.st.positions.contains_key(&pid) {
                let (pnl, broker, lp) = self.reduce_position(pid, v, price, lp_price, exec);
                deal(self, pid, DealEntry::Out, v, pnl, (broker, lp));
            }
            left = Qty::ZERO;
        } else if g.margin_mode == MarginMode::Netting {
            let existing = self
                .st
                .positions
                .values()
                .find(|p| p.account == account && p.symbol == symbol)
                .map(|p| (p.id, p.side, p.volume));
            if let Some((pid, pside, pvol)) = existing {
                if pside != side {
                    let r = v.min(pvol);
                    let (pnl, broker, lp) = self.reduce_position(pid, r, price, lp_price, exec);
                    deal(self, pid, DealEntry::Out, r, pnl, (broker, lp));
                    left = Qty::from_raw(v.raw() - r.raw());
                } else {
                    self.increase_position(pid, v, price, lp_price);
                    deal(self, pid, DealEntry::In, v, Money::zero(g.currency), (0, 0));
                    left = Qty::ZERO;
                }
            }
        } else if let Some(pid) = self.st.orders[&id].position {
            if self.st.positions.contains_key(&pid) {
                self.increase_position(pid, v, price, lp_price);
                deal(self, pid, DealEntry::In, v, Money::zero(g.currency), (0, 0));
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
            };
            self.st.positions.insert(pid, pos);
            self.st.orders.get_mut(&id).expect("order").position = Some(pid);
            self.events.push(Event::PositionOpened { position_id: pid });
            deal(
                self,
                pid,
                DealEntry::In,
                left,
                Money::zero(g.currency),
                (0, 0),
            );
        }
        self.balance_event(account);
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
        });
        self.events.push(Event::DealAdded { deal_id: id });
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
        let remaining = Qty::from_raw(p.volume.raw() - v.raw());
        if remaining.is_positive() {
            let pm = self.st.positions.get_mut(&pid).expect("position");
            pm.volume = remaining;
            pm.closing = pm.closing.min(remaining);
        } else {
            self.st.positions.remove(&pid);
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
                }
            }
            let p = &self.st.positions[&pid];
            let sl_hit = p.sl.is_some_and(|sl| (px.raw() - sl.raw()) * s <= 0);
            let tp_hit = p.tp.is_some_and(|tp| (px.raw() - tp.raw()) * s >= 0);
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

    fn rollover(&mut self) -> R<()> {
        let ps: Vec<Position> = self.st.positions.values().cloned().collect();
        for p in ps {
            let spec = &self.st.symbols[&p.symbol];
            let acc = &self.st.accounts[&p.account];
            let g = &self.st.groups[&acc.group];
            let rate = match p.side {
                Side::Buy => spec.swap_long,
                Side::Sell => spec.swap_short,
            };
            let scaled = rate.raw() as i128 * p.volume.raw() as i128 / SCALE as i128;
            let m = Money::from_scaled(scaled, spec.quote, Rounding::HalfEven).map_err(e2s)?;
            let m = self
                .st
                .quotes
                .convert(m, g.currency, Rounding::HalfEven)
                .map_err(e2s)?;
            let cp = match p.routing {
                Routing::ABook => LP_COUNTERPARTY,
                Routing::BBook => BROKER_BOOK,
            };
            let lid = acc.ledger_id;
            self.post(
                TxnKind::Swap,
                format!("swap:{}:{}", self.st.seq, p.id),
                vec![(lid, m), (cp, Money::new(-m.minor, m.currency))],
            )?;
        }
        Ok(())
    }
}
