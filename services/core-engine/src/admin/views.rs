//! Engine state -> back-office DTOs (shapes of `apps/backoffice/src/lib/schemas.ts`).
//! Money is emitted in integer minor units, prices/lots as display floats.

use super::store::AdminState;
use money::{Price, Qty};
use oms::{Engine, OrderStatus, OrderType, Position};
use risk::{AssetClass, EsmaPreset, GroupConfig, MarginMode, Routing, Side, SymbolSpec};
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
        "commissionType": "per_lot",
        "commissionValue": 0,
        "markupPoints": g.markup_points,
        "swapMultiplier": 1,
        "book": book(g.routing),
        "symbols": symbols,
        "esma": g.esma.map(|p| match p { EsmaPreset::Retail => "retail", EsmaPreset::Professional => "professional" }),
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
                } else if r.since_ms == 0 {
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
                })
            })
            .collect(),
    )
}
