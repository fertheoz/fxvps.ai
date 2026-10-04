//! Engine events -> account-level [`CoreEvent`]s (runs on the writer thread).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use domain::Fixed;
use money::{Money, Price, Qty};
use oms::{AccountNo, Engine, Event, OrderId};
use tokio::sync::broadcast;

use crate::api::{AccountNames, AccountView, CoreEvent, GroupQuote, OrderView, PositionView};

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
    let cs = e.symbol_spec(&o.req.symbol)?.contract_size;
    Some(OrderView {
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
    })
}

/// Net position of `account` in `symbol` (flat if none).
pub fn position_view(
    e: &Engine,
    account: AccountNo,
    symbol: &str,
    names: &AccountNames,
) -> PositionView {
    let cs = e.symbol_spec(symbol).map_or(1, |s| s.contract_size);
    let mut net: i128 = 0;
    let mut notional: i128 = 0;
    let mut gross: i128 = 0;
    let mut pnl = Fixed::ZERO;
    for p in e
        .positions_of(account)
        .into_iter()
        .filter(|p| p.symbol == symbol)
    {
        let v = p.volume.raw() as i128;
        net += v * p.side.sign() as i128;
        gross += v;
        notional += v * p.open_price.raw() as i128;
        if let Ok(m) = e.position_pnl(p.id) {
            pnl = pnl + money_fixed(m);
        }
    }
    let avg = if gross > 0 {
        (notional / gross) as i64
    } else {
        0
    };
    PositionView {
        account: names.name(account),
        symbol: symbol.to_string(),
        net_qty: units(Qty::from_raw(net as i64), cs),
        avg_price: Fixed::from_raw(avg),
        unrealized_pnl: pnl,
    }
}

pub fn positions_view(e: &Engine, account: AccountNo, names: &AccountNames) -> Vec<PositionView> {
    let syms: BTreeSet<String> = e
        .positions_of(account)
        .into_iter()
        .map(|p| p.symbol.clone())
        .collect();
    syms.iter()
        .map(|s| position_view(e, account, s, names))
        .collect()
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
        for ev in events {
            let (id, last) = match ev {
                Event::OrderAccepted { order_id }
                | Event::OrderCancelled { order_id }
                | Event::OrderExpired { order_id }
                | Event::OrderTriggered { order_id }
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
        for (a, syms) in touched {
            self.last_pnl.insert(a, ts);
            for s in syms {
                self.send(CoreEvent::Position(position_view(e, a, &s, &self.names)));
            }
            if let Some(v) = account_view(e, a, &self.names) {
                self.send(CoreEvent::Account(v));
            }
        }
    }
}
