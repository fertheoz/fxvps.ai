//! Engine events -> account-level [`CoreEvent`]s (runs on the writer thread).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use domain::Fixed;
use money::{Money, Price, Qty};
use oms::{AccountNo, Engine, Event, OrderId, PositionId};
use tokio::sync::broadcast;

use crate::api::{
    AccountNames, AccountView, CoreEvent, DealPage, DealQuery, DealView, GroupQuote, OrderView,
    PositionView,
};

/// Lots -> base units.
pub fn units(lots: Qty, contract_size: i64) -> Fixed {
    Fixed::from_raw(lots.raw().saturating_mul(contract_size))
}

/// Money -> 1e8-scaled decimal.
pub fn money_fixed(m: Money) -> Fixed {
    Fixed::from_raw(
        m.to_scaled()
            .ok()
            .and_then(|v| i64::try_from(v).ok())
            .unwrap_or(0),
    )
}

/// Strips the modify suffix (`id~n`) from an engine client order id.
pub fn client_id(clid: &str) -> &str {
    clid.split_once('~').map_or(clid, |(b, _)| b)
}

pub fn order_view(
    e: &Engine,
    id: OrderId,
    last: Option<(Qty, Price)>,
    names: &AccountNames,
    ts_ns: u64,
) -> Option<OrderView> {
    let o = e.order(id)?;
    let spec = e.symbol_spec(&o.req.symbol)?;
    let cs = spec.contract_size;
    let point = spec.point();
    let r = &o.req;
    Some(OrderView {
        kind: r.order_type.into(),
        limit_price: r.limit_price.map(Into::into),
        stop_price: r.stop_price.map(Into::into),
        sl: r.sl.map(Into::into),
        tp: r.tp.map(Into::into),
        trailing_distance: r.trailing_points.map(|t| distance(point, t)),
        oco_group: r.oco_group,
        expire_at_ns: r.expire_at,
        position_id: o.position,
        close_position_id: o.close_position,
        stop_triggered: o.stop_triggered,
        created_ns: o.created_ts,
        account: names.name(o.req.account),
        client_order_id: client_id(&o.req.client_order_id).to_string(),
        order_id: o.id,
        symbol: o.req.symbol.clone(),
        side: o.req.side.into(),
        status: o.status,
        qty: units(o.req.volume, cs),
        filled_qty: units(o.filled, cs),
        avg_price: o.filled.is_positive().then(|| o.avg_price.into()),
        last_qty: last.map(|(q, _)| units(q, cs)),
        last_price: last.map(|(_, p)| p.into()),
        reason: o.reject_reason.clone(),
        ts_ns,
        lp_resting: o.lp_resting.is_some(),
    })
}

/// Trailing points -> price distance.
pub fn distance(point: Price, points: i64) -> Fixed {
    Fixed::from_raw(point.raw().saturating_mul(points))
}

/// View of an open position.
pub fn position_view(e: &Engine, id: PositionId, names: &AccountNames) -> Option<PositionView> {
    let p = e.position(id)?;
    let spec = e.symbol_spec(&p.symbol)?;
    let cs = spec.contract_size;
    let qty = units(p.volume, cs);
    Some(PositionView {
        account: names.name(p.account),
        position_id: p.id,
        symbol: p.symbol.clone(),
        side: p.side.into(),
        net_qty: if p.side.sign() < 0 {
            Fixed::ZERO - qty
        } else {
            qty
        },
        avg_price: p.open_price.into(),
        unrealized_pnl: e.position_pnl(p.id).map(money_fixed).unwrap_or(Fixed::ZERO),
        sl: p.sl.map(Into::into),
        tp: p.tp.map(Into::into),
        trailing_distance: p.trailing_points.map(|t| distance(spec.point(), t)),
        open_ts_ns: p.opened_ts,
        lp_tp: p.lp_tp.is_some(),
        lp_sl: p.lp_sl.is_some(),
    })
}

/// A closed position (`net_qty` = 0), reconstructed from its last deal.
fn closed_view(e: &Engine, id: PositionId, names: &AccountNames) -> Option<PositionView> {
    let d = e.deals().iter().rev().find(|d| d.position_id == id)?;
    Some(PositionView {
        account: names.name(d.account),
        position_id: id,
        symbol: d.symbol.clone(),
        side: d.side.into(),
        net_qty: Fixed::ZERO,
        avg_price: Fixed::ZERO,
        unrealized_pnl: Fixed::ZERO,
        sl: None,
        tp: None,
        trailing_distance: None,
        open_ts_ns: 0,
        lp_tp: false,
        lp_sl: false,
    })
}

pub fn positions_view(e: &Engine, account: AccountNo, names: &AccountNames) -> Vec<PositionView> {
    e.positions_of(account)
        .into_iter()
        .filter_map(|p| position_view(e, p.id, names))
        .collect()
}

/// Every order of an account that is not waiting any more (filled, cancelled,
/// rejected, expired, working), newest first, at most `limit`.
pub fn order_history_view(
    e: &Engine,
    account: AccountNo,
    names: &AccountNames,
    limit: usize,
) -> Vec<OrderView> {
    let mut v: Vec<OrderView> = e
        .orders()
        .filter(|o| o.req.account == account && !o.is_pending())
        .filter_map(|o| order_view(e, o.id, None, names, o.created_ts))
        .collect();
    v.sort_by_key(|o| std::cmp::Reverse(o.order_id));
    v.truncate(limit);
    v
}

/// Pending (working) orders of an account, oldest first.
pub fn pending_orders_view(e: &Engine, account: AccountNo, names: &AccountNames) -> Vec<OrderView> {
    e.orders()
        .filter(|o| o.req.account == account && o.is_pending())
        .filter_map(|o| order_view(e, o.id, None, names, o.created_ts))
        .collect()
}

pub fn deal_view(e: &Engine, id: u64, names: &AccountNames) -> Option<DealView> {
    let d = e.deal(id)?;
    let cs = e.symbol_spec(&d.symbol).map_or(1, |s| s.contract_size);
    let clid = e
        .order(d.order_id)
        .map(|o| client_id(&o.req.client_order_id).to_string())
        .unwrap_or_default();
    Some(DealView {
        account: names.name(d.account),
        deal_id: d.id,
        order_id: d.order_id,
        client_order_id: clid,
        position_id: d.position_id,
        symbol: d.symbol.clone(),
        side: d.side.into(),
        entry: d.entry,
        qty: units(d.volume, cs),
        price: d.price.into(),
        pnl: money_fixed(d.pnl),
        commission: money_fixed(d.commission),
        ts_ns: d.ts,
        reason: d.reason,
    })
}

/// One page of an account's deals (oldest first).
pub fn deal_page(e: &Engine, account: AccountNo, q: DealQuery, names: &AccountNames) -> DealPage {
    let limit = q.limit.max(1);
    let mut deals = Vec::new();
    let mut next = None;
    let start = usize::try_from(q.after).unwrap_or(usize::MAX);
    for d in e.deals().iter().skip(start) {
        if d.account != account || d.ts < q.from_ns || (q.to_ns != 0 && d.ts > q.to_ns) {
            continue;
        }
        if deals.len() == limit {
            next = deals.last().map(|v: &DealView| v.deal_id);
            break;
        }
        if let Some(v) = deal_view(e, d.id, names) {
            deals.push(v);
        }
    }
    DealPage { deals, next }
}

pub fn account_view(e: &Engine, account: AccountNo, names: &AccountNames) -> Option<AccountView> {
    let a = e.account(account)?;
    let g = e.group(&a.group)?;
    let r = e.account_risk(account).ok();
    let bal = e.balance(account).ok()?;
    let (equity, margin, free) = match &r {
        Some(r) => (r.equity, r.margin, r.free_margin),
        None => (bal, Money::zero(g.currency), bal),
    };
    Some(AccountView {
        account: names.name(account),
        group: a.group.clone(),
        currency: g.currency.to_string(),
        balance: money_fixed(bal),
        equity: money_fixed(equity),
        margin: money_fixed(margin),
        free_margin: money_fixed(free),
        margin_level_pct: r
            .as_ref()
            .and_then(|r| r.margin_level_x100())
            .and_then(|l| i64::try_from(l).ok())
            .map(|l| Fixed::from_raw(l.saturating_mul(1_000_000))),
        margin_call: a.margin_call,
        margin_mode: g.margin_mode,
        leverage: g.leverage,
        positions: positions_view(e, account, names),
    })
}

/// Publishes [`CoreEvent`]s for each applied command.
pub struct Publisher {
    tx: broadcast::Sender<Arc<CoreEvent>>,
    names: AccountNames,
    /// Minimum interval between quote-driven P&L pushes per account.
    pnl_interval_ns: u64,
    last_pnl: HashMap<AccountNo, u64>,
}

impl Publisher {
    pub fn new(
        tx: broadcast::Sender<Arc<CoreEvent>>,
        names: AccountNames,
        pnl_interval_ns: u64,
    ) -> Publisher {
        Publisher {
            tx,
            names,
            pnl_interval_ns,
            last_pnl: HashMap::new(),
        }
    }

    fn send(&self, ev: CoreEvent) {
        let _ = self.tx.send(Arc::new(ev));
    }

    pub fn publish(&mut self, e: &Engine, quote: Option<&str>, events: &[Event], ts: u64) {
        // account -> touched symbols
        let mut touched: BTreeMap<AccountNo, BTreeSet<String>> = BTreeMap::new();
        let mut closed: Vec<PositionId> = Vec::new();
        for ev in events {
            let (id, last) = match ev {
                Event::DealAdded { deal_id } => {
                    if let Some(v) = deal_view(e, *deal_id, &self.names) {
                        self.send(CoreEvent::Deal(v));
                    }
                    continue;
                }
                Event::PositionClosed {
                    position_id,
                    remaining,
                    ..
                } => {
                    if remaining.is_zero() {
                        closed.push(*position_id);
                    }
                    continue;
                }
                Event::PositionModified { position_id, .. } => {
                    if let Some(p) = e.position(*position_id) {
                        touched
                            .entry(p.account)
                            .or_default()
                            .insert(p.symbol.clone());
                    }
                    continue;
                }
                Event::OrderAccepted { order_id }
                | Event::OrderCancelled { order_id }
                | Event::OrderExpired { order_id }
                | Event::OrderTriggered { order_id }
                | Event::OrderModified { order_id }
                | Event::OrderRejected { order_id, .. } => (*order_id, None),
                Event::OrderFilled {
                    order_id,
                    volume,
                    price,
                    ..
                } => (*order_id, Some((*volume, *price))),
                Event::BalanceChanged { account, .. }
                | Event::MarginCall { account }
                | Event::NegativeBalanceCompensated { account, .. } => {
                    touched.entry(*account).or_default();
                    continue;
                }
                _ => continue,
            };
            if let Some(v) = order_view(e, id, last, &self.names, ts) {
                if let Some(o) = e.order(id) {
                    touched
                        .entry(o.req.account)
                        .or_default()
                        .insert(o.req.symbol.clone());
                }
                self.send(CoreEvent::Order(v));
            }
        }
        if let Some(sym) = quote {
            for g in e.groups() {
                if let Some(q) = e.group_quote(&g.name, sym) {
                    self.send(CoreEvent::Quote(GroupQuote {
                        group: g.name.clone(),
                        symbol: sym.to_string(),
                        bid: q.bid.into(),
                        ask: q.ask.into(),
                        ts_ns: ts,
                    }));
                }
            }
            // floating P&L moved for holders of the symbol (throttled)
            let holders: BTreeSet<AccountNo> = e
                .accounts()
                .filter(|a| e.positions_of(a.id).iter().any(|p| p.symbol == sym))
                .map(|a| a.id)
                .collect();
            for a in holders {
                if touched.contains_key(&a) {
                    continue;
                }
                let last = self.last_pnl.get(&a).copied().unwrap_or(0);
                if ts.saturating_sub(last) >= self.pnl_interval_ns {
                    touched.entry(a).or_default().insert(sym.to_string());
                }
            }
        }
        for pid in closed {
            if let Some(v) = closed_view(e, pid, &self.names) {
                self.send(CoreEvent::Position(v));
            }
        }
        for (a, syms) in touched {
            self.last_pnl.insert(a, ts);
            for p in e.positions_of(a) {
                if syms.contains(&p.symbol) {
                    if let Some(v) = position_view(e, p.id, &self.names) {
                        self.send(CoreEvent::Position(v));
                    }
                }
            }
            if let Some(v) = account_view(e, a, &self.names) {
                self.send(CoreEvent::Account(v));
            }
        }
    }
}
