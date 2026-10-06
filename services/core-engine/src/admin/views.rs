//! Engine state -> back-office DTOs (shapes of `apps/backoffice/src/lib/schemas.ts`).
//! Money is emitted in integer minor units, prices/lots as display floats.

use super::store::AdminState;
use ledger::TxnKind;
use money::{Price, Qty};
use oms::{Engine, OrderStatus, OrderType, Position};
use risk::{
    AssetClass, EsmaPreset, GroupCommission, GroupConfig, MarginMode, PartialFill, Routing, Side,
    SymbolSpec,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn price_f(p: Price) -> f64 {
    p.raw() as f64 / 1e8
}
pub fn qty_f(q: Qty) -> f64 {
    q.raw() as f64 / 1e8
}
/// Display float -> fixed point (1e8).
pub fn fixed(v: f64) -> i64 {
    (v * 1e8).round() as i64
}
fn minor(v: i128) -> i64 {
    v.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// ns since epoch -> ISO-8601 UTC with milliseconds.
pub fn iso(ns: u64) -> String {
    let ms = ns / 1_000_000;
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // civil-from-days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60,
        ms % 1000
    )
}

pub fn side_str(s: Side) -> &'static str {
    match s {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}
pub fn book(r: Routing) -> &'static str {
    match r {
        Routing::ABook => "A",
        Routing::BBook => "B",
    }
}

pub fn client(e: &Engine, id: u64, admin: &AdminState) -> Option<Value> {
    let a = e.account(id)?;
    let g = e.group(&a.group)?;
    let credit = admin.credit_of(id);
    let (balance, equity, margin) = match e.account_risk(id) {
        Ok(r) => (r.balance.minor, r.equity.minor, r.margin.minor),
        Err(_) => {
            let b = e.balance(id).map(|m| m.minor).unwrap_or(0);
            (b, b, 0)
        }
    };
    let equity = equity + credit as i128;
    Some(json!({
        "id": id.to_string(),
        "login": id,
        "parentId": null,
        "name": admin.profiles.get(&id).map_or_else(|| format!("Account {id}"), |p| p.name.clone()),
        "email": admin.profiles.get(&id).map_or_else(|| format!("account{id}@example.com"), |p| p.email.clone()),
        "country": "ZZ",
        "group": a.group,
        "status": "active",
        "kyc": admin.kyc_of(id),
        "currency": g.currency.as_str(),
        "leverage": g.leverage,
        "balance": minor(balance),
        "credit": credit,
        "equity": minor(equity),
        "margin": minor(margin),
        "marginCall": a.margin_call,
        "positions": e.positions_of(id).len(),
        "createdAt": iso(0),
        "lastIp": "",
    }))
}

pub fn clients(e: &Engine, admin: &AdminState, search: Option<&str>) -> Value {
    let s = search.map(|s| s.trim().to_lowercase()).unwrap_or_default();
    Value::Array(
        e.accounts()
            .filter(|a| {
                s.is_empty() || a.id.to_string().contains(&s) || a.group.to_lowercase().contains(&s)
            })
            .filter_map(|a| client(e, a.id, admin))
            .collect(),
    )
}

pub fn position(e: &Engine, p: &Position) -> Value {
    let group = e
        .account(p.account)
        .map(|a| a.group.clone())
        .unwrap_or_default();
    let current = e
        .group_quote(&group, &p.symbol)
        .map(|q| price_f(q.for_side(p.side.opposite())))
        .unwrap_or(price_f(p.open_price));
    let pnl = e.position_pnl(p.id).map(|m| minor(m.minor)).unwrap_or(0);
    json!({
        "id": p.id.to_string(),
        "clientId": p.account.to_string(),
        "login": p.account,
        "symbol": p.symbol,
        "side": side_str(p.side),
        "lots": qty_f(p.volume),
        "openPrice": price_f(p.open_price),
        "currentPrice": current,
        "pnl": pnl,
        "swap": 0,
        "book": book(p.routing),
        "openedAt": iso(p.opened_ts),
    })
}

pub fn all_positions(e: &Engine) -> Vec<&Position> {
    e.accounts().flat_map(|a| e.positions_of(a.id)).collect()
}

pub fn positions(e: &Engine) -> Value {
    Value::Array(
        all_positions(e)
            .into_iter()
            .map(|p| position(e, p))
            .collect(),
    )
}

pub fn orders(e: &Engine) -> Value {
    Value::Array(
        e.orders()
            .filter(|o| !o.status.is_terminal() && o.req.order_type != OrderType::Market)
            .map(|o| {
                let ty = match o.req.order_type {
                    OrderType::Stop => "stop",
                    OrderType::StopLimit => "stop_limit",
                    _ => "limit",
                };
                let status = if o.working {
                    "pending_lp"
                } else if o.status == OrderStatus::PartiallyFilled {
                    "partially_filled"
                } else {
                    "working"
                };
                let price = o.req.limit_price.or(o.req.stop_price).map_or(0.0, price_f);
                json!({
                    "id": o.id.to_string(),
                    "clientId": o.req.account.to_string(),
                    "login": o.req.account,
                    "symbol": o.req.symbol,
                    "side": side_str(o.req.side),
                    "type": ty,
                    "lots": qty_f(o.remaining()),
                    "price": price,
                    "status": status,
                    "createdAt": iso(o.created_ts),
                })
            })
            .collect(),
    )
}

pub fn exposure(e: &Engine) -> Value {
    #[derive(Default)]
    struct X {
        net: i128,
        a: i128,
        b: i128,
    }
    let mut m: BTreeMap<String, X> = BTreeMap::new();
    for p in all_positions(e) {
        let x = m.entry(p.symbol.clone()).or_default();
        let signed = p.side.sign() as i128 * p.volume.raw() as i128;
        x.net += signed;
        match p.routing {
            Routing::ABook => x.a += signed,
            Routing::BBook => x.b += signed,
        }
    }
    for s in e.symbols() {
        if e.omnibus_net(&s.symbol) != 0 {
            m.entry(s.symbol.clone()).or_default();
        }
    }
    let mut out: Vec<Value> = m
        .into_iter()
        .map(|(sym, x)| {
            let notional = e.symbol_spec(&sym).map_or(0, |s| {
                // lots(1e8) * contract size -> base units -> base minor units
                x.net * s.contract_size as i128 * 10i128.pow(s.base.minor_exponent()) / 100_000_000
            });
            json!({
                "symbol": sym,
                "netLots": x.net as f64 / 1e8,
                "notional": minor(notional),
                "aBookLots": x.a as f64 / 1e8,
                "bBookLots": x.b as f64 / 1e8,
                "lpLots": e.omnibus_net(&sym) as f64 / 1e8,
            })
        })
        .collect();
    out.sort_by_key(|v| std::cmp::Reverse(v["notional"].as_i64().unwrap_or(0).unsigned_abs()));
    Value::Array(out)
}

pub fn margin_calls(e: &Engine, admin: &AdminState) -> Value {
    let mut rows: Vec<(i128, Value)> = Vec::new();
    for a in e.accounts() {
        let (Ok(r), Some(g)) = (e.account_risk(a.id), e.group(&a.group)) else {
            continue;
        };
        if !(r.is_margin_call(g) || a.margin_call) {
            continue;
        }
        let Some(lvl) = r.margin_level_x100() else {
            continue;
        };
        let state = if r.is_stop_out(g) {
            "stop_out"
        } else {
            "margin_call"
        };
        if let Some(c) = client(e, a.id, admin) {
            rows.push((
                lvl,
                json!({ "client": c, "marginLevel": lvl as f64 / 100.0, "state": state }),
            ));
        }
    }
    rows.sort_by_key(|(l, _)| *l);
    Value::Array(rows.into_iter().map(|(_, v)| v).collect())
}

pub fn group(g: &GroupConfig, all_symbols: &[String]) -> Value {
    let symbols: Vec<String> = match &g.allowed_symbols {
        Some(s) => s.iter().cloned().collect(),
        None => all_symbols.to_vec(),
    };
    json!({
        "id": g.name,
        "name": g.name,
        "currency": g.currency.as_str(),
        "leverage": g.leverage,
        "marginMode": match g.margin_mode { MarginMode::Hedging => "retail_hedged", MarginMode::Netting => "retail_netting" },
        "marginCallPct": g.margin_call_pct,
        "stopOutPct": g.stop_out_pct,
        "commissionType": match g.commission { Some(GroupCommission::PerLot { .. }) => "per_lot", Some(GroupCommission::PerMillion { .. }) => "per_million", None => "symbol" },
        "commissionValue": match g.commission { Some(GroupCommission::PerLot { minor }) | Some(GroupCommission::PerMillion { minor }) => minor, None => 0 },
        "markupPoints": g.markup_points,
        "markupBidPoints": g.markup_bid_points,
        "markupAskPoints": g.markup_ask_points,
        "symbolMarkups": g.symbol_markup_points,
        "maxSlippagePoints": g.max_slippage_points,
        "passPriceImprovement": g.pass_price_improvement,
        "swapMultiplier": 1,
        "book": book(g.routing),
        "symbols": symbols,
        "esma": g.esma.map(|p| match p { EsmaPreset::Retail => "retail", EsmaPreset::Professional => "professional" }),
        "partialFill": match g.partial_fill {
            PartialFill::CancelRemainder => "cancel",
            PartialFill::Retry { .. } => "retry",
            PartialFill::AllOrNone => "all_or_none",
        },
        "maxAttempts": match g.partial_fill { PartialFill::Retry { max_attempts } => max_attempts, _ => 3 },
        "negativeBalanceProtection": g.negative_balance_protection,
    })
}

pub fn symbol_names(e: &Engine) -> Vec<String> {
    e.symbols().map(|s| s.symbol.clone()).collect()
}

pub fn groups(e: &Engine) -> Value {
    let names = symbol_names(e);
    Value::Array(e.groups().map(|g| group(g, &names)).collect())
}

pub fn category(c: AssetClass) -> &'static str {
    match c {
        AssetClass::MajorFx | AssetClass::MinorFx => "fx",
        AssetClass::Gold => "metal",
        AssetClass::MajorIndex | AssetClass::MinorIndex | AssetClass::Equity => "index",
        AssetClass::Commodity => "energy",
        AssetClass::Crypto => "crypto",
    }
}

pub fn symbol(s: &SymbolSpec) -> Value {
    let lev = EsmaPreset::Retail
        .max_leverage(s.asset_class)
        .unwrap_or(1)
        .max(1);
    json!({
        "name": s.symbol,
        "description": format!("{}/{}", s.base, s.quote),
        "category": category(s.asset_class),
        "digits": s.digits,
        "contractSize": s.contract_size,
        "tickSize": price_f(s.point()),
        "marginPct": (10_000.0 / lev as f64).round() / 100.0,
        "minLot": qty_f(s.min_lot),
        "maxLot": qty_f(s.max_lot),
        "lotStep": qty_f(s.lot_step),
        "swapLong": price_f(s.swap_long),
        "swapShort": price_f(s.swap_short),
        "swapType": "money",
        "tripleSwapDay": "wed",
        "tradeSessions": [],
        "enabled": true,
        "lp": "LP",
    })
}

pub fn symbols(e: &Engine) -> Value {
    Value::Array(e.symbols().map(symbol).collect())
}

/// ESMA presets offered by `/v1/risk/presets` (ids match the mock).
pub const PRESETS: [(&str, &str, u32); 5] = [
    ("esma-fx-major", "ESMA retail — FX majors (1:30)", 30),
    (
        "esma-fx-minor",
        "ESMA retail — FX minors, gold, major indices (1:20)",
        20,
    ),
    (
        "esma-commodity",
        "ESMA retail — other commodities, minor indices (1:10)",
        10,
    ),
    ("esma-equity", "ESMA retail — single equities (1:5)", 5),
    ("esma-crypto", "ESMA retail — crypto (1:2)", 2),
];

pub fn presets() -> Value {
    Value::Array(
        PRESETS
            .iter()
            .map(|(id, label, lev)| {
                json!({ "id": id, "label": label, "maxLeverage": lev, "marginCallPct": 100,
                        "stopOutPct": 50, "negativeBalanceProtection": true })
            })
            .collect(),
    )
}

/// Closed-trade history: one row per closing (`Out`) deal, newest first. The
/// open price is the volume-weighted price of the position's `In` deals.
pub fn trades(e: &Engine) -> Value {
    let deals = e.deals();
    let mut rows: Vec<Value> = deals
        .iter()
        .filter(|d| d.entry == oms::DealEntry::Out)
        .map(|d| {
            let (mut vol, mut notional) = (0f64, 0f64);
            for i in deals
                .iter()
                .filter(|i| i.position_id == d.position_id && i.entry == oms::DealEntry::In)
            {
                vol += qty_f(i.volume);
                notional += qty_f(i.volume) * price_f(i.price);
            }
            let close = price_f(d.price);
            let open = if vol > 0.0 { notional / vol } else { close };
            let routing = e
                .account(d.account)
                .and_then(|a| e.group(&a.group))
                .map(|g| book(g.routing))
                .unwrap_or("A");
            json!({
                "id": d.id.to_string(),
                "login": d.account,
                "symbol": d.symbol,
                // The closing deal trades against the position's side.
                "side": side_str(d.side.opposite()),
                "lots": qty_f(d.volume),
                "openPrice": open,
                "closePrice": close,
                "pnl": minor(d.pnl.minor),
                "commission": minor(d.commission.minor),
                "swap": 0,
                "book": routing,
                "closedAt": iso(d.ts),
            })
        })
        .collect();
    rows.reverse();
    Value::Array(rows)
}

/// Per-symbol accumulator of the execution-quality report.
#[derive(Default)]
struct ExecAgg {
    orders: u32,
    filled: u32,
    partial: u32,
    rejected: u32,
    slips: Vec<f64>,
    improved: u32,
    capture_sum: f64,
    capture_n: u32,
    latencies_ms: Vec<f64>,
    lp_slips: Vec<f64>,
    attempts_sum: u32,
}

impl ExecAgg {
    fn json(&self, symbol: &str) -> Value {
        let mut s = self.slips.clone();
        s.sort_by(|a, b| a.total_cmp(b));
        let avg = |v: &[f64]| {
            if v.is_empty() {
                0.0
            } else {
                v.iter().sum::<f64>() / v.len() as f64
            }
        };
        let pct = |v: &[f64], p: f64| {
            if v.is_empty() {
                0.0
            } else {
                v[((v.len() - 1) as f64 * p).round() as usize]
            }
        };
        let p95 = pct(&s, 0.95);
        let mut lat = self.latencies_ms.clone();
        lat.sort_by(|a, b| a.total_cmp(b));
        let rate = |n: u32| {
            if self.orders == 0 {
                0.0
            } else {
                f64::from(n) / f64::from(self.orders)
            }
        };
        json!({
            "symbol": symbol,
            "orders": self.orders,
            "fillRate": rate(self.filled + self.partial),
            "partialRate": rate(self.partial),
            "rejectRate": rate(self.rejected),
            "avgSlipPts": avg(&s),
            "p95SlipPts": p95,
            "improvedRate": if s.is_empty() { 0.0 } else { f64::from(self.improved) / s.len() as f64 },
            "avgCapturePts": if self.capture_n == 0 { 0.0 } else { self.capture_sum / f64::from(self.capture_n) },
            "avgLpSlipPts": avg(&self.lp_slips),
            "avgLatencyMs": avg(&lat),
            "p50LatencyMs": pct(&lat, 0.5),
            "p95LatencyMs": pct(&lat, 0.95),
            "avgAttempts": rate(self.attempts_sum),
        })
    }
}

/// Execution quality: one row per client order that reached a terminal state
/// (newest first, capped), with the client's slippage against the price it
/// asked for (points, positive = worse for the client), the LP leg behind it
/// (average price, slippage against the LP quote at send time, time to first
/// fill, fills, attempts) and the difference we captured between the client
/// and LP prices. Plus a per-symbol summary.
pub fn execution(e: &Engine, admin: &AdminState) -> Value {
    // Every LP order a client order took part in, oldest first (re-arms send again).
    let mut lp_of: BTreeMap<oms::OrderId, Vec<&oms::LpOrder>> = BTreeMap::new();
    for l in e.lp_orders() {
        for c in &l.children {
            lp_of.entry(*c).or_default().push(l);
        }
    }
    let points: BTreeMap<&str, f64> = e
        .symbols()
        .map(|s| (s.symbol.as_str(), price_f(s.point())))
        .collect();
    let mut by_symbol: BTreeMap<String, ExecAgg> = BTreeMap::new();
    let mut rows: Vec<Value> = Vec::new();
    for o in e.orders() {
        if !matches!(
            o.status,
            OrderStatus::Filled | OrderStatus::PartiallyFilled | OrderStatus::Rejected
        ) {
            continue;
        }
        let point = points.get(o.req.symbol.as_str()).copied().unwrap_or(0.0);
        let sign = if o.req.side == Side::Buy { 1.0 } else { -1.0 };
        let requested = o.req.requested_price.or(o.req.limit_price).map(price_f);
        let filled = o.filled.raw() > 0;
        let fill = filled.then(|| price_f(o.avg_price));
        let slip = match (requested, fill) {
            (Some(r), Some(f)) if point > 0.0 => Some(sign * (f - r) / point),
            _ => None,
        };
        let attempts = lp_of.get(&o.id);
        let lp = attempts.and_then(|v| v.last().copied());
        let lp_avg = lp.and_then(|l| {
            let (mut lots, mut notional) = (0f64, 0f64);
            for f in &l.fills {
                lots += qty_f(f.volume);
                notional += qty_f(f.volume) * price_f(f.price);
            }
            (lots > 0.0).then_some(notional / lots)
        });
        let capture = match (fill, lp_avg) {
            (Some(f), Some(l)) if point > 0.0 => Some(sign * (f - l) / point),
            _ => None,
        };
        let latency_ms = lp.and_then(|l| {
            l.fills
                .iter()
                .map(|f| f.ts)
                .min()
                .map(|t| t.saturating_sub(l.created_ts) as f64 / 1e6)
        });
        // What the LP did to us: fill vs. the LP quote we saw when sending (same sign convention).
        let lp_sent = lp
            .and_then(|l| {
                if l.side == Side::Buy {
                    l.sent_ask
                } else {
                    l.sent_bid
                }
            })
            .map(price_f);
        let lp_slip = match (lp_avg, lp_sent) {
            (Some(a), Some(s)) if point > 0.0 => Some(sign * (a - s) / point),
            _ => None,
        };
        let attempts = attempts.map_or(0, |v| v.len());
        let agg = by_symbol.entry(o.req.symbol.clone()).or_default();
        agg.orders += 1;
        match o.status {
            OrderStatus::Filled => agg.filled += 1,
            OrderStatus::PartiallyFilled => agg.partial += 1,
            _ => agg.rejected += 1,
        }
        if let Some(s) = slip {
            agg.slips.push(s);
            if s < 0.0 {
                agg.improved += 1;
            }
        }
        if let Some(c) = capture {
            agg.capture_sum += c;
            agg.capture_n += 1;
        }
        if let Some(ms) = latency_ms {
            agg.latencies_ms.push(ms);
        }
        if let Some(s) = lp_slip {
            agg.lp_slips.push(s);
        }
        agg.attempts_sum += attempts as u32;
        rows.push(json!({
            "id": o.id.to_string(),
            "at": iso(o.created_ts),
            "login": o.req.account,
            "name": admin.profiles.get(&o.req.account).map(|p| p.name.clone()),
            "symbol": o.req.symbol,
            "side": side_str(o.req.side),
            "type": match o.req.order_type {
                OrderType::Market => "market",
                OrderType::Limit => "limit",
                OrderType::Stop => "stop",
                OrderType::StopLimit => "stop_limit",
            },
            "lots": qty_f(o.req.volume),
            "filledLots": qty_f(o.filled),
            "status": match o.status {
                OrderStatus::Filled => "filled",
                OrderStatus::PartiallyFilled => "partial",
                _ => "rejected",
            },
            "reason": o.reject_reason,
            "book": if lp.is_some() { "A" } else { book(o.routing) },
            "requested": requested,
            "fill": fill,
            "clientSlipPts": slip,
            "lpPrice": lp_avg,
            "lpSentPrice": lp_sent,
            "lpSlipPts": lp_slip,
            "attempts": attempts,
            "capturePts": capture,
            "lpLatencyMs": latency_ms,
            "lpFills": lp.map_or(0, |l| l.fills.len()),
            "lpStatus": lp.map(|l| if l.reject_reason.is_some() { "rejected" } else if l.done { "filled" } else { "working" }),
            "rearmed": o.rearm_px.is_some(),
        }));
    }
    rows.reverse();
    rows.truncate(1000);
    let summary: Vec<Value> = by_symbol.iter().map(|(s, a)| a.json(s)).collect();
    json!({ "rows": rows, "bySymbol": summary })
}

/// Orders sent to the LP with their executions and the client orders they were
/// allocated to, newest first.
pub fn lp_executions(e: &Engine) -> Value {
    let mut rows: Vec<Value> = e
        .lp_orders()
        .map(|l| {
            let (mut lots, mut notional) = (0f64, 0f64);
            for f in &l.fills {
                lots += qty_f(f.volume);
                notional += qty_f(f.volume) * price_f(f.price);
            }
            let status = if l.reject_reason.is_some() {
                "rejected"
            } else if l.done {
                "filled"
            } else if lots > 0.0 {
                "partial"
            } else {
                "working"
            };
            let clients: Vec<Value> = l
                .children
                .iter()
                .filter_map(|c| e.order(*c))
                .map(|o| {
                    json!({
                        "orderId": o.id.to_string(),
                        "login": o.req.account,
                        "lots": qty_f(o.filled),
                        "price": price_f(o.avg_price),
                    })
                })
                .collect();
            let fills: Vec<Value> = l
                .fills
                .iter()
                .map(|f| {
                    json!({
                        "execId": f.exec_id,
                        "lots": qty_f(f.volume),
                        "price": price_f(f.price),
                        "at": iso(f.ts),
                    })
                })
                .collect();
            json!({
                "id": l.id.to_string(),
                "symbol": l.symbol,
                "side": side_str(l.side),
                "lots": qty_f(l.volume),
                "filledLots": lots,
                "avgPrice": if lots > 0.0 { notional / lots } else { 0.0 },
                "status": status,
                "reason": l.reject_reason,
                "createdAt": iso(l.created_ts),
                "fills": fills,
                "clients": clients,
            })
        })
        .collect();
    rows.reverse();
    Value::Array(rows)
}

#[derive(Default, Clone, Copy)]
struct RevenueTotals {
    markup: i128,
    b_book: i128,
    commission: i128,
    lp: i128,
}

impl RevenueTotals {
    fn add(&mut self, kind: TxnKind, a_book: bool, broker: i128, lp: i128) {
        match (kind, a_book) {
            (TxnKind::Commission, _) => self.commission += broker,
            (_, true) => self.markup += broker,
            (_, false) => self.b_book += broker,
        }
        self.lp += lp;
    }

    fn json(self) -> Value {
        json!({
            "markup": minor(self.markup),
            "bBook": minor(self.b_book),
            "commission": minor(self.commission),
            "lp": minor(self.lp),
            "total": minor(self.markup + self.b_book + self.commission),
        })
    }
}

/// Broker revenue per deal (minor units of the account currency), newest first, with
/// totals overall and since `since_ns`: the markup of closing A-book deals (LP result
/// minus client result), the result of closing B-book deals (the broker is the
/// counterparty) and commission. `lp` is our own result at the LP.
pub fn revenue(e: &Engine, since_ns: u64) -> Value {
    let (mut all, mut recent) = (RevenueTotals::default(), RevenueTotals::default());
    let mut rows = Vec::new();
    for d in e.deals() {
        let a_book = d.lp_price.is_some()
            || e.account(d.account)
                .and_then(|a| e.group(&a.group))
                .is_some_and(|g| g.routing == Routing::ABook);
        let mut row = |kind: TxnKind, client: i128, broker: i128, lp: i128| {
            all.add(kind, a_book, broker, lp);
            if d.ts >= since_ns {
                recent.add(kind, a_book, broker, lp);
            }
            let commission = kind == TxnKind::Commission;
            rows.push(json!({
                "id": format!("{}{}", if commission { "c" } else { "d" }, d.id),
                "at": iso(d.ts),
                "kind": if commission { "commission" } else { "pnl" },
                "ref": format!("deal {} · position {}", d.id, d.position_id),
                "book": if a_book { "A" } else { "B" },
                "login": d.account,
                "symbol": d.symbol,
                "lots": qty_f(d.volume),
                "price": price_f(d.price),
                "lpPrice": d.lp_price.map(price_f),
                "client": minor(client),
                "broker": minor(broker),
                "lp": minor(lp),
            }));
        };
        if d.entry == oms::DealEntry::Out {
            row(TxnKind::RealizedPnl, d.pnl.minor, d.broker_pnl, d.lp_pnl);
        }
        if d.commission.minor != 0 {
            row(
                TxnKind::Commission,
                d.commission.minor,
                -d.commission.minor,
                0,
            );
        }
    }
    rows.reverse();
    rows.truncate(500);
    json!({ "total": all.json(), "last24h": recent.json(), "rows": rows })
}

/// FIX session rows in the back office `FixSession` shape. Sequence numbers
/// and latency are not tracked by the gateway yet and are reported as 0.
pub fn lp_sessions(rows: &[fix_gateway::SessionStatus]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| {
                let kind = match r.kind {
                    fix_gateway::SessionKind::MarketData => "MD",
                    fix_gateway::SessionKind::Trading => "TRADING",
                };
                let status = if r.logged_on {
                    "logged_on"
                } else if r.since_ms == 0 && r.last_down_reason.is_none() {
                    "connecting"
                } else {
                    "disconnected"
                };
                json!({
                    "id": format!("{}-{}", r.target_comp_id, kind.to_lowercase()),
                    "lp": r.target_comp_id,
                    "kind": kind,
                    "senderCompId": r.sender_comp_id,
                    "targetCompId": r.target_comp_id,
                    "status": status,
                    "inSeq": 0,
                    "outSeq": 0,
                    "latencyMs": 0,
                    "rejects24h": r.rejects,
                    "lastHeartbeat": iso(r.since_ms.saturating_mul(1_000_000)),
                    "lastError": r.last_down_reason,
                })
            })
            .collect(),
    )
}

/// Per-bucket accumulator of the dashboard series.
#[derive(Default, Clone)]
struct DashBucket {
    markup: i128,
    commission: i128,
    b_book: i128,
    lots: f64,
    orders: u32,
    rejects: u32,
}

/// Dashboard series for `range` ("today" | "24h" | "7d" | "30d"): revenue legs,
/// volume and order counts per bucket, totals with the previous period for
/// comparison, top symbols / clients, accounts near margin call and an
/// execution-quality summary. Everything comes from the engine's deals,
/// orders and LP orders; nothing is sampled, so the series is exact.
pub fn dashboard_series(e: &Engine, admin: &AdminState, now_ns: u64, range: &str) -> Value {
    const HOUR: u64 = 3_600_000_000_000;
    const DAY: u64 = 24 * HOUR;
    let (since, bucket, label_day) = match range {
        "today" => (now_ns - now_ns % DAY, HOUR, false),
        "7d" => (now_ns.saturating_sub(7 * DAY), DAY, true),
        "30d" => (now_ns.saturating_sub(30 * DAY), DAY, true),
        _ => (now_ns.saturating_sub(DAY), HOUR, false),
    };
    let len = now_ns.saturating_sub(since).max(bucket);
    let prev_since = since.saturating_sub(len);
    let n_buckets = len.div_ceil(bucket) as usize;
    let mut buckets = vec![DashBucket::default(); n_buckets];
    let mut total = DashBucket::default();
    let mut prev = DashBucket::default();
    let name = |a: u64| {
        admin
            .profiles
            .get(&a)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| a.to_string())
    };
    let a_book = |d: &oms::Deal| {
        d.lp_price.is_some()
            || e.account(d.account)
                .and_then(|a| e.group(&a.group))
                .is_some_and(|g| g.routing == Routing::ABook)
    };
    // (symbol -> lots, revenue), (account -> pnl, lots)
    let mut by_symbol: BTreeMap<String, (f64, i128)> = BTreeMap::new();
    let mut by_client: BTreeMap<u64, (i128, f64)> = BTreeMap::new();
    for d in e.deals() {
        if d.ts < prev_since {
            continue;
        }
        let (markup, b_book) = if d.entry == oms::DealEntry::Out {
            if a_book(d) {
                (d.broker_pnl, 0)
            } else {
                (0, d.broker_pnl)
            }
        } else {
            (0, 0)
        };
        let commission = -d.commission.minor;
        let lots = qty_f(d.volume);
        let target = if d.ts >= since { &mut total } else { &mut prev };
        target.markup += markup;
        target.commission += commission;
        target.b_book += b_book;
        target.lots += lots;
        if d.ts >= since {
            let i = (((d.ts - since) / bucket) as usize).min(n_buckets - 1);
            let b = &mut buckets[i];
            b.markup += markup;
            b.commission += commission;
            b.b_book += b_book;
            b.lots += lots;
            let s = by_symbol.entry(d.symbol.clone()).or_default();
            s.0 += lots;
            s.1 += markup + commission + b_book;
            let c = by_client.entry(d.account).or_default();
            if d.entry == oms::DealEntry::Out {
                c.0 += d.pnl.minor;
            }
            c.1 += lots;
        }
    }
    // orders: counts, rejects, client slippage
    let points: BTreeMap<&str, f64> = e
        .symbols()
        .map(|s| (s.symbol.as_str(), price_f(s.point())))
        .collect();
    let (mut filled, mut slips) = (0u32, Vec::<f64>::new());
    for o in e.orders() {
        if o.created_ts < prev_since {
            continue;
        }
        let rejected = o.status == OrderStatus::Rejected;
        let target = if o.created_ts >= since {
            &mut total
        } else {
            &mut prev
        };
        target.orders += 1;
        if rejected {
            target.rejects += 1;
        }
        if o.created_ts >= since {
            let i = (((o.created_ts - since) / bucket) as usize).min(n_buckets - 1);
            buckets[i].orders += 1;
            if rejected {
                buckets[i].rejects += 1;
            }
            if o.filled.raw() > 0 {
                filled += 1;
                if let (Some(req), Some(&point)) =
                    (o.req.requested_price, points.get(o.req.symbol.as_str()))
                {
                    if point > 0.0 {
                        let sign = if o.req.side == Side::Buy { 1.0 } else { -1.0 };
                        slips.push(sign * (price_f(o.avg_price) - price_f(req)) / point);
                    }
                }
            }
        }
    }
    // LP latency p95 (send -> first fill)
    let mut lat: Vec<f64> = e
        .lp_orders()
        .filter(|l| l.created_ts >= since)
        .filter_map(|l| {
            l.fills
                .iter()
                .map(|f| f.ts)
                .min()
                .map(|t| t.saturating_sub(l.created_ts) as f64 / 1e6)
        })
        .collect();
    lat.sort_by(|a, b| a.total_cmp(b));
    let pct = |v: &[f64], p: f64| {
        if v.is_empty() {
            0.0
        } else {
            v[((v.len() - 1) as f64 * p).round() as usize]
        }
    };
    let avg = |v: &[f64]| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    // accounts near margin call
    let mut risk: Vec<Value> = e
        .accounts()
        .filter_map(|a| {
            let r = e.account_risk(a.id).ok()?;
            let level = r.margin_level_x100()?;
            (level < 200 * 100).then(|| {
                json!({
                    "login": a.id,
                    "name": name(a.id),
                    "marginLevelPct": level as f64 / 100.0,
                    "equity": minor(r.equity.minor),
                    "margin": minor(r.margin.minor),
                    "marginCall": a.margin_call,
                })
            })
        })
        .collect();
    risk.sort_by(|x, y| {
        x["marginLevelPct"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&y["marginLevelPct"].as_f64().unwrap_or(0.0))
    });
    risk.truncate(10);
    let mut top_symbols: Vec<(String, f64, i128)> =
        by_symbol.into_iter().map(|(s, (l, r))| (s, l, r)).collect();
    top_symbols.sort_by(|a, b| b.1.total_cmp(&a.1));
    top_symbols.truncate(8);
    let mut clients: Vec<(u64, i128, f64)> =
        by_client.into_iter().map(|(a, (p, l))| (a, p, l)).collect();
    clients.sort_by_key(|c| std::cmp::Reverse(c.1));
    let winners: Vec<Value> = clients
        .iter()
        .take(5)
        .map(|(a, p, l)| json!({"login": a, "name": name(*a), "pnl": minor(*p), "lots": l}))
        .collect();
    let losers: Vec<Value> = clients
        .iter()
        .rev()
        .take(5)
        .filter(|(_, p, _)| *p < 0)
        .map(|(a, p, l)| json!({"login": a, "name": name(*a), "pnl": minor(*p), "lots": l}))
        .collect();
    let bucket_json = |i: usize, b: &DashBucket| {
        let t = since + i as u64 * bucket;
        let iso_t = iso(t);
        json!({
            "t": iso_t,
            "label": if label_day { iso_t[5..10].to_string() } else { iso_t[11..16].to_string() },
            "markup": minor(b.markup),
            "commission": minor(b.commission),
            "bBook": minor(b.b_book),
            "lots": b.lots,
            "orders": b.orders,
            "rejects": b.rejects,
        })
    };
    let totals_json = |b: &DashBucket| {
        json!({
            "revenue": minor(b.markup + b.commission + b.b_book),
            "markup": minor(b.markup),
            "commission": minor(b.commission),
            "bBook": minor(b.b_book),
            "lots": b.lots,
            "orders": b.orders,
            "rejects": b.rejects,
        })
    };
    json!({
        "range": range,
        "since": iso(since),
        "buckets": buckets.iter().enumerate().map(|(i, b)| bucket_json(i, b)).collect::<Vec<_>>(),
        "totals": totals_json(&total),
        "previous": totals_json(&prev),
        "topSymbols": top_symbols.iter().map(|(s, l, r)| json!({"symbol": s, "lots": l, "revenue": minor(*r)})).collect::<Vec<_>>(),
        "winners": winners,
        "losers": losers,
        "risk": risk,
        "execution": {
            "orders": total.orders,
            "fillRate": if total.orders == 0 { 0.0 } else { f64::from(filled) / f64::from(total.orders) },
            "avgClientSlipPts": avg(&slips),
            "p95LatencyMs": pct(&lat, 0.95),
            "p50LatencyMs": pct(&lat, 0.5),
        },
    })
}

/// Routing rule table as stored in the engine (camelCase JSON, see `RoutingRule`).
pub fn rules(e: &Engine) -> Value {
    serde_json::to_value(e.rules()).unwrap_or(Value::Array(Vec::new()))
}

/// Replays the rule table over the orders of the last period: how many orders
/// (and lots) each rule would have taken, what stays with the group default,
/// and a few sample orders with the rule and book they would get.
pub fn rules_dry_run(e: &Engine, admin: &AdminState, since_ns: u64) -> Value {
    let mut hits: BTreeMap<String, (u32, f64)> = BTreeMap::new();
    let mut unmatched = (0u32, 0f64);
    let mut samples: Vec<Value> = Vec::new();
    for o in e.orders().filter(|o| o.created_ts >= since_ns) {
        let Some(acc) = e.account(o.req.account) else {
            continue;
        };
        let Some(g) = e.group(&acc.group) else {
            continue;
        };
        let pending = o.req.order_type != OrderType::Market;
        let rule = e.match_rule(
            &acc.group,
            o.req.account,
            &o.req.symbol,
            o.req.volume,
            pending,
        );
        let lots = qty_f(o.req.volume);
        match rule {
            Some(r) => {
                let h = hits.entry(r.id.clone()).or_default();
                h.0 += 1;
                h.1 += lots;
            }
            None => {
                unmatched.0 += 1;
                unmatched.1 += lots;
            }
        }
        if samples.len() < 8 {
            let routing = rule.map_or(g.routing, |r| r.book_for(o.id, g.routing));
            samples.push(json!({
                "orderId": o.id.to_string(),
                "login": o.req.account,
                "name": admin.profiles.get(&o.req.account).map(|p| p.name.clone()),
                "symbol": o.req.symbol,
                "lots": lots,
                "rule": rule.map(|r| r.name.clone()),
                "routing": book(routing),
            }));
        }
    }
    json!({
        "since": iso(since_ns),
        "rules": e.rules().iter().map(|r| {
            let (orders, lots) = hits.get(&r.id).copied().unwrap_or_default();
            json!({ "id": r.id, "name": r.name, "enabled": r.enabled, "orders": orders, "lots": lots })
        }).collect::<Vec<_>>(),
        "unmatched": { "orders": unmatched.0, "lots": unmatched.1 },
        "samples": samples,
    })
}
