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
use std::collections::{BTreeMap, BTreeSet};

pub fn price_f(p: Price) -> f64 {
    p.raw() as f64 / 1e8
}
/// A computed price (VWAP, averages) shown at the symbol's digits: the
/// industry convention, and it hides float noise like 1.3261799999999997.
pub fn round_px(e: &Engine, symbol: &str, x: f64) -> f64 {
    let d = e.symbol_spec(symbol).map_or(5, |s| s.digits);
    let f = 10f64.powi(d as i32);
    (x * f).round() / f
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
        "lei": admin.profiles.get(&id).and_then(|p| p.lei.clone()),
        "ibSharePct": admin.ib_share.get(&id).copied().unwrap_or(0),
        "ibPlan": admin.ib_plan.get(&id),
        "ibAccount": admin.ib_of.get(&id).copied(),
        "kycDocs": admin.kyc_docs.iter().filter(|d| d.account == id).count(),
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
        "swap": minor(p.swap_minor),
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
            let h = e.hedge_policy();
            let limit = h.enabled.then(|| h.symbol_limit(&sym)).flatten();
            let hedge = e.hedge_net(&sym);
            json!({
                "symbol": sym,
                "netLots": x.net as f64 / 1e8,
                "notional": minor(notional),
                "aBookLots": x.a as f64 / 1e8,
                "bBookLots": x.b as f64 / 1e8,
                "lpLots": e.omnibus_net(&sym) as f64 / 1e8,
                "hedgeLots": hedge as f64 / 1e8,
                "hedgePendingLots": e.hedge_pending(&sym) as f64 / 1e8,
                "unhedgedBLots": (x.b as i64 + hedge) as f64 / 1e8,
                "limitLots": limit.map(|q| q.raw() as f64 / 1e8),
                "overLimit": limit.is_some_and(|l| (x.b as i64).abs() > l.raw()),
                "hedgeRealized": e.hedge_realized(&sym) as f64
                    / 10f64.powi(e.symbol_spec(&sym).map_or(2, |s| s.quote.minor_exponent() as i32)),
                "hedgeCurrency": e.symbol_spec(&sym).map(|s| s.quote.to_string()),
                "volDailyPct": e.volatility_daily(&sym) * 100.0,
                "varUsd": e.var_symbol_usd(&sym).map(|m| m.minor as f64 / 100.0),
            })
        })
        .collect();
    out.sort_by_key(|v| std::cmp::Reverse(v["notional"].as_i64().unwrap_or(0).unsigned_abs()));
    Value::Array(out)
}

/// Currency-leg exposure (`GET /v1/exposure/currency`): per currency the net
/// amount (major units) of the A-book and the B-book, their USD value and
/// the B-book cap.
pub fn currency_exposure(e: &Engine) -> Value {
    let h = e.hedge_policy();
    let mut rows: Vec<Value> = e
        .currency_exposure()
        .into_iter()
        .map(|(ccy, (a, b))| {
            let div = 10f64.powi(ccy.minor_exponent() as i32);
            let usd = |m: i128| e.to_usd_minor(ccy, m).map(|u| u as f64 / 100.0);
            let limit = h.currency_limits_usd.get(&ccy.to_string()).copied();
            let b_usd = usd(b);
            json!({
                "currency": ccy.to_string(),
                "aAmount": a as f64 / div,
                "bAmount": b as f64 / div,
                "netAmount": (a + b) as f64 / div,
                "aUsd": usd(a),
                "bUsd": b_usd,
                "netUsd": usd(a + b),
                "limitUsd": limit,
                "overLimit": limit.is_some_and(|l| b_usd.is_some_and(|u| u.abs() > l as f64)),
            })
        })
        .collect();
    rows.sort_by(|x, y| {
        y["netUsd"]
            .as_f64()
            .unwrap_or(0.0)
            .abs()
            .partial_cmp(&x["netUsd"].as_f64().unwrap_or(0.0).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Value::Array(rows)
}

/// B-book exposure / auto-hedge policy (`GET /v1/risk/hedge`), lots as floats.
pub fn hedge_policy(e: &Engine) -> Value {
    let h = e.hedge_policy();
    let lots = |q: Option<Qty>| q.map(qty_f);
    json!({
        "enabled": h.enabled,
        "mode": h.mode,
        "defaultSymbolLimit": lots(h.default_symbol_limit),
        "symbolLimits": h.symbol_limits.iter().map(|(s, q)| (s.clone(), qty_f(*q))).collect::<BTreeMap<_, _>>(),
        "totalLimit": lots(h.total_limit),
        "accountLimit": lots(h.account_limit),
        "hedgeRatioPct": h.hedge_ratio_pct,
        "releasePct": h.release_pct,
        "sliceLots": lots(h.slice_lots),
        "sliceIntervalS": h.slice_interval_s,
        "varLimitUsd": h.var_limit_usd,
        "varTotalUsd": e.var_total_usd().minor as f64 / 100.0,
        "currencyLimitsUsd": h.currency_limits_usd,
        "newsWindowMin": h.news_window_min,
        "newsAction": h.news_action,
        "inNewsWindow": e.in_news_window(),
        "burstWindowMin": h.burst_window_min,
        "burstAccountLots": lots(h.burst_account_lots),
        "burstSymbolLots": lots(h.burst_symbol_lots),
    })
}

/// Per-client flow profile (`GET /v1/reports/clients`): holding times, win
/// rate, captured price improvement and the toxicity score the rules use.
pub fn client_flow(e: &Engine, admin: &AdminState) -> Value {
    let mut rows: Vec<Value> = e
        .accounts()
        .filter_map(|a| {
            let f = e.flow(a.id)?;
            let g = e.group(&a.group)?;
            let div = 10f64.powi(g.currency.minor_exponent() as i32);
            Some(json!({
                "login": a.id,
                "name": admin.profiles.get(&a.id).map(|p| p.name.clone()),
                "group": a.group,
                "currency": g.currency.to_string(),
                "trades": f.trades,
                "fills": f.fills,
                "avgHoldSecs": f.avg_hold_secs(),
                "shortHoldPct": f.short_hold_ratio() * 100.0,
                "winRate": f.win_rate() * 100.0,
                "realisedPnl": f.pnl_minor as f64 / div,
                "brokerPnl": f.broker_pnl_minor as f64 / div,
                "avgSlipGainPoints": f.avg_slip_gain_points(),
                "markout1s": f.avg_markout(0),
                "markout5s": f.avg_markout(1),
                "markout60s": f.avg_markout(2),
                "toxicity": f.toxicity(),
            }))
        })
        .collect();
    rows.sort_by(|a, b| {
        b["toxicity"]
            .as_u64()
            .cmp(&a["toxicity"].as_u64())
            .then_with(|| b["trades"].as_u64().cmp(&a["trades"].as_u64()))
    });
    Value::Array(rows)
}

/// LP that executed `order_id` (the omnibus order whose children include it).
fn venue_of(e: &Engine, order_id: u64) -> Option<String> {
    e.lp_orders()
        .find(|l| l.children.contains(&order_id))
        .and_then(|l| l.lp.clone())
}

fn asset_class_name(a: AssetClass) -> &'static str {
    match a {
        AssetClass::MajorFx | AssetClass::MinorFx => "FX",
        AssetClass::Gold => "METAL",
        AssetClass::MajorIndex | AssetClass::MinorIndex => "INDEX",
        AssetClass::Commodity => "COMMODITY",
        AssetClass::Equity => "EQUITY",
        AssetClass::Crypto => "CRYPTO",
    }
}

/// Transaction report (MiFIR RTS 22 field subset) — one row per deal in
/// `[from, to)`: timestamps, identifiers (client LEI / login, our LEI),
/// instrument, price, quantity in units, notional, capacity (DEAL = we are
/// principal on the B-book, MTCH = matched principal against the LP), venue.
pub fn transactions(e: &Engine, admin: &AdminState, from: u64, to: u64) -> Value {
    let broker_lei = admin.settings.broker_lei.clone();
    let rows: Vec<Value> = e
        .deals()
        .iter()
        .filter(|d| d.ts >= from && d.ts < to)
        .map(|d| {
            let spec = e.symbol_spec(&d.symbol);
            let a_book = d.lp_price.is_some()
                || e.account(d.account)
                    .and_then(|a| e.group(&a.group))
                    .is_some_and(|g| g.routing == Routing::ABook);
            let profile = admin.profiles.get(&d.account);
            let client_id = profile
                .and_then(|p| p.lei.clone())
                .unwrap_or_else(|| format!("CLIENT-{}", d.account));
            let units = qty_f(d.volume) * spec.map_or(100_000, |s| s.contract_size) as f64;
            let venue_lp = if a_book {
                venue_of(e, d.order_id)
            } else {
                None
            };
            let (buyer, seller) = match d.side {
                Side::Buy => (
                    client_id.clone(),
                    if a_book {
                        venue_lp.clone().unwrap_or_else(|| "LP".into())
                    } else {
                        broker_lei.clone()
                    },
                ),
                Side::Sell => (
                    if a_book {
                        venue_lp.clone().unwrap_or_else(|| "LP".into())
                    } else {
                        broker_lei.clone()
                    },
                    client_id.clone(),
                ),
            };
            json!({
                "txId": format!("D{}", d.id),
                "tradingDateTime": iso(d.ts),
                "executingEntity": broker_lei,
                "buyerId": buyer,
                "sellerId": seller,
                "clientLogin": d.account,
                "clientName": profile.map(|p| p.name.clone()),
                "instrument": d.symbol,
                "assetClass": spec.map(|s| asset_class_name(s.asset_class)),
                "isin": "",
                "side": side_str(d.side),
                "entry": if d.entry == oms::DealEntry::In { "open" } else { "close" },
                "price": price_f(d.price),
                "priceCurrency": spec.map(|s| s.quote.to_string()),
                "quantityLots": qty_f(d.volume),
                "quantityUnits": units,
                "notional": units * price_f(d.price),
                "tradingCapacity": if a_book { "MTCH" } else { "DEAL" },
                "venue": "XOFF",
                "executionLp": venue_lp,
                "book": if a_book { "A" } else { "B" },
                "commission": minor(d.commission.minor),
                "swap": minor(d.swap),
                "realisedPnl": minor(d.pnl.minor),
                "reason": format!("{:?}", d.reason),
            })
        })
        .collect();
    json!({ "from": iso(from), "to": if to == u64::MAX { Value::Null } else { json!(iso(to)) }, "rows": rows })
}

/// Best-execution summary (RTS 27/28 spirit) per venue × asset class over
/// orders created in `[from, to)`: share of volume, fill rate, client slippage,
/// price improvement, LP latency, rejects.
pub fn best_execution(e: &Engine, from: u64, to: u64) -> Value {
    #[derive(Default)]
    struct Acc {
        orders: u32,
        filled: u32,
        rejected: u32,
        lots: f64,
        slips: Vec<f64>,
        improved: u32,
        priced: u32,
        lat: Vec<f64>,
    }
    let points: BTreeMap<&str, f64> = e
        .symbols()
        .map(|s| (s.symbol.as_str(), price_f(s.point())))
        .collect();
    let mut by: BTreeMap<(String, &'static str), Acc> = BTreeMap::new();
    let mut total_lots = 0f64;
    for o in e
        .orders()
        .filter(|o| o.created_ts >= from && o.created_ts < to)
    {
        let class = e
            .symbol_spec(&o.req.symbol)
            .map_or("FX", |s| asset_class_name(s.asset_class));
        let venue = match o.routing {
            Routing::BBook => "B-book".to_string(),
            Routing::ABook => venue_of(e, o.id).unwrap_or_else(|| "LP".into()),
        };
        let a = by.entry((venue, class)).or_default();
        a.orders += 1;
        if o.status == OrderStatus::Rejected {
            a.rejected += 1;
        }
        if o.filled.raw() > 0 {
            a.filled += 1;
            a.lots += qty_f(o.filled);
            total_lots += qty_f(o.filled);
            if let (Some(req), Some(&point)) =
                (o.req.requested_price, points.get(o.req.symbol.as_str()))
            {
                if point > 0.0 {
                    let sign = if o.req.side == Side::Buy { 1.0 } else { -1.0 };
                    let slip = sign * (price_f(o.avg_price) - price_f(req)) / point;
                    a.slips.push(slip);
                    a.priced += 1;
                    if slip < 0.0 {
                        a.improved += 1;
                    }
                }
            }
            if let Some(l) = e.lp_orders().find(|l| l.children.contains(&o.id)) {
                if let Some(t) = l.fills.iter().map(|f| f.ts).min() {
                    a.lat.push(t.saturating_sub(l.created_ts) as f64 / 1e6);
                }
            }
        }
    }
    let rows: Vec<Value> = by
        .into_iter()
        .map(|((venue, class), mut a)| {
            let p50_latency = {
                let mut v = a.lat.clone();
                v.sort_by(|x, y| x.total_cmp(y));
                if v.is_empty() {
                    0.0
                } else {
                    v[(v.len() - 1) / 2]
                }
            };
            json!({
                "venue": venue,
                "assetClass": class,
                "orders": a.orders,
                "filled": a.filled,
                "rejected": a.rejected,
                "fillRate": if a.orders > 0 { f64::from(a.filled) / f64::from(a.orders) } else { 0.0 },
                "lots": a.lots,
                "volumeSharePct": if total_lots > 0.0 { a.lots / total_lots * 100.0 } else { 0.0 },
                "avgClientSlipPts": mean(&a.slips),
                "p95ClientSlipPts": p95(&mut a.slips),
                "priceImprovementPct": if a.priced > 0 { f64::from(a.improved) / f64::from(a.priced) * 100.0 } else { 0.0 },
                "p50LatencyMs": p50_latency,
                "p95LatencyMs": p95(&mut a.lat),
            })
        })
        .collect();
    json!({ "from": iso(from), "to": if to == u64::MAX { Value::Null } else { json!(iso(to)) }, "rows": rows })
}

/// Client funding requests for the back office, newest first.
pub fn funding(admin: &AdminState, status: Option<&str>) -> Value {
    let mut rows: Vec<&super::store::FundingRequest> = admin
        .funding
        .values()
        .filter(|f| match status {
            None | Some("all") | Some("") => true,
            // open: waiting for a decision, or paid after it and not reviewed yet
            Some("open") => {
                f.status == super::store::FundingStatus::Requested
                    || super::usdt_watch::late_unhandled(f)
            }
            Some(s) => format!("{:?}", f.status).to_lowercase() == s,
        })
        .collect();
    rows.sort_by_key(|f| std::cmp::Reverse(f.requested_at));
    Value::Array(
        rows.into_iter()
            .map(|f| {
                let mut v = json!(f);
                v["requestedAtIso"] = json!(iso(f.requested_at));
                v["decidedAtIso"] = f
                    .decided_at
                    .map(iso)
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                v["clientName"] = admin
                    .profiles
                    .get(&f.account)
                    .map(|p| p.name.clone())
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                v
            })
            .collect(),
    )
}

/// IB accrual for deals in [from, to): share of the clients' commission and
/// A-book markup, per-lot rebate on closing deals, and overrides that sub-IBs'
/// revenue pays up the chain (at most three levels). A deal counts for the IB
/// (and the chain above it) the client was linked to when the deal happened;
/// revenue in a currency other than the IB's is not shared (no conversion).
pub fn ib_report(e: &Engine, admin: &AdminState, from: u64, to: u64) -> Value {
    #[derive(Default)]
    struct Acc {
        clients: std::collections::BTreeSet<u64>,
        lots: f64,
        commission: i128,
        markup: i128,
        deals: u32,
        rebate: i128,
        overrides: i128,
    }
    let plan = |ib: u64| admin.ib_plan.get(&ib).cloned().unwrap_or_default();
    let ccy_of = |acc: u64| {
        e.account(acc)
            .and_then(|a| e.group(&a.group))
            .map(|g| g.currency)
    };
    let mut by: BTreeMap<u64, Acc> = BTreeMap::new();
    for (client, ib) in &admin.ib_of {
        by.entry(*ib).or_default().clients.insert(*client);
    }
    for d in e.deals().iter().filter(|d| d.ts >= from && d.ts < to) {
        let Some(ib) = admin.ib_at(d.account, d.ts) else {
            continue;
        };
        let markup = if d.entry == oms::DealEntry::Out && d.lp_price.is_some() {
            d.broker_pnl
        } else {
            0
        };
        let base = -d.commission.minor + markup;
        let same_ccy = |x: u64| ccy_of(x) == Some(d.commission.currency);
        let a = by.entry(ib).or_default();
        a.deals += 1;
        a.lots += qty_f(d.volume);
        if same_ccy(ib) {
            a.commission += -d.commission.minor;
            a.markup += markup;
        }
        if d.entry == oms::DealEntry::Out {
            a.rebate +=
                plan(ib).per_lot_cents as i128 * d.volume.raw() as i128 / money::SCALE as i128;
        }
        // the chain as it stood at the deal's time; every IB is paid once
        let mut seen = std::collections::BTreeSet::from([d.account, ib]);
        let mut cur = ib;
        for _ in 0..3 {
            let Some(parent) = admin.ib_at(cur, d.ts) else {
                break;
            };
            if !seen.insert(parent) {
                break;
            }
            let pct = plan(parent).override_pct as i128;
            if pct > 0 && same_ccy(parent) {
                by.entry(parent).or_default().overrides += base * pct / 100;
            }
            cur = parent;
        }
    }
    let rows: Vec<Value> = by
        .into_iter()
        .map(|(ib, a)| {
            let pct = admin.ib_share.get(&ib).copied().unwrap_or(0) as i128;
            let p = plan(ib);
            let ccy = ccy_of(ib).map(|c| c.to_string());
            let share = (a.commission + a.markup) * pct / 100;
            let paid_from = admin.ib_paid_from(ib);
            json!({
                "ib": ib,
                "name": admin.profiles.get(&ib).map(|p| p.name.clone()),
                "currency": ccy,
                "sharePct": pct,
                "perLotCents": p.per_lot_cents,
                "overridePct": p.override_pct,
                "code": p.code,
                "parent": admin.ib_of.get(&ib),
                "clients": a.clients.len(),
                "deals": a.deals,
                "lots": a.lots,
                "commission": minor(a.commission),
                "markup": minor(a.markup),
                "share": minor(share),
                "rebate": minor(a.rebate),
                "override": minor(a.overrides),
                "payout": minor(share + a.rebate + a.overrides),
                // last UTC day paid out, and a payout waiting for approval
                "paidThrough": (paid_from > 0).then(|| iso((paid_from - 1) * 86_400_000_000_000)[..10].to_string()),
                "payoutPending": admin.ib_payout_pending(ib).map(|o| o.id.clone()),
            })
        })
        .collect();
    json!({ "from": iso(from), "to": if to == u64::MAX { Value::Null } else { json!(iso(to)) }, "rows": rows })
}

pub fn weekday_name(d: u8) -> &'static str {
    ["sun", "mon", "tue", "wed", "thu", "fri", "sat"][(d % 7) as usize]
}

pub fn weekday_index(name: &str) -> Option<u8> {
    ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
        .iter()
        .position(|d| *d == name)
        .map(|i| i as u8)
}

/// Rollover schedule plus when it last ran (`GET /v1/settings/swap`).
pub fn swap_config(e: &Engine) -> Value {
    let c = e.swap_config();
    json!({
        "enabled": c.enabled,
        "rolloverHourUtc": c.rollover_hour_utc,
        "skipWeekend": c.skip_weekend,
        "lastRolloverAt": e.last_rollover_day().map(|d| iso(d * 86_400_000_000_000)),
    })
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
        "weekendLeverage": g.weekend_leverage,
        "leverageWindows": g.leverage_windows.iter().map(|w| json!({
            "fromMs": w.from_ns / 1_000_000, "toMs": w.to_ns / 1_000_000, "leverage": w.leverage,
        })).collect::<Vec<_>>(),
        "passPriceImprovement": g.pass_price_improvement,
        "lpResting": g.lp_resting,
        "markupWindows": g.markup_windows.iter().map(|w| json!({
            "weekdays": w.weekdays, "fromMin": w.from_min, "toMin": w.to_min, "addPoints": w.add_points,
        })).collect::<Vec<_>>(),
        "newsMarkup": g.news_markup.map(|n| json!({ "windowMin": n.window_min, "addPoints": n.add_points })),
        "markupBands": g.markup_bands.iter().map(|b| json!({ "fromLots": b.from_centilots as f64 / 100.0, "addPoints": b.add_points })).collect::<Vec<_>>(),
        "minSpreadPoints": g.min_spread_points,
        "maxSpreadPoints": g.max_spread_points,
        "skew": g.skew.map(|s| json!({ "pointsPerLot": s.centipoints_per_lot as f64 / 100.0, "maxPoints": s.max_points })),
        "swapMultiplier": g.swap_multiplier_pct as f64 / 100.0,
        "leverageTiers": g.leverage_tiers.iter().map(|t| json!({"from": t.from, "leverage": t.leverage})).collect::<Vec<_>>(),
        "swapFreeFee": g.swap_free_fee_per_lot as f64 / 100.0,
        "swapFreeGraceDays": g.swap_free_grace_days,
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
        "swapType": match s.swap_mode { risk::SwapMode::Money => "money", risk::SwapMode::Points => "points" },
        "tripleSwapDay": weekday_name(s.triple_swap_day),
        "tradeSessions": s.sessions.iter().map(|x| json!({ "day": weekday_name(x.day), "open": format!("{:02}:{:02}", x.open_min / 60, x.open_min % 60), "close": format!("{:02}:{:02}", x.close_min / 60, x.close_min % 60) })).collect::<Vec<_>>(),
        "enabled": s.enabled,
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
/// Reconciliation: the last 100 closed deals, client side and LP side next to
/// each other with the broker's legs in their own columns. For an A-book deal
/// `client P&L + markup = LP P&L` (the invariant the row's `ok` checks); the
/// commission of the closed part (its share of the opening deals' commission
/// plus the closing deal's) and the swap-free fee are the broker's too.
/// Closing deals in `[from, to)`, newest first, at most `limit`.
pub fn reconciliation(e: &Engine, from: u64, to: u64, limit: usize) -> Value {
    let deals = e.deals();
    let chain = lp_chain(e);
    let mut rows: Vec<Value> = deals
        .iter()
        .rev()
        .filter(|d| d.entry == oms::DealEntry::Out && d.ts >= from && d.ts < to)
        .take(limit)
        .map(|d| {
            let (mut vol, mut notional, mut lp_notional, mut in_comm) = (0f64, 0f64, 0f64, 0i128);
            let mut lp_open = true;
            let mut opened = u64::MAX;
            for i in deals
                .iter()
                .filter(|i| i.position_id == d.position_id && i.entry == oms::DealEntry::In)
            {
                opened = opened.min(i.ts);
                let v = qty_f(i.volume);
                vol += v;
                notional += v * price_f(i.price);
                match i.lp_price {
                    Some(p) => lp_notional += v * price_f(p),
                    None => lp_open = false,
                }
                in_comm += i.commission.minor;
            }
            let open = if vol > 0.0 {
                round_px(e, &d.symbol, notional / vol)
            } else {
                price_f(d.price)
            };
            let open_lp = (lp_open && vol > 0.0).then(|| round_px(e, &d.symbol, lp_notional / vol));
            let a_book = d.lp_price.is_some();
            // the closed part's share of the opening commission + the closing deal's
            let share = if vol > 0.0 {
                (in_comm as f64 * qty_f(d.volume) / vol).round() as i128
            } else {
                0
            };
            let commission = -(share + d.commission.minor);
            let swap_fee = -d.swap_fee;
            let broker = d.broker_pnl + commission + swap_fee;
            let ok = if a_book {
                d.pnl.minor + d.broker_pnl == d.lp_pnl
            } else {
                d.lp_pnl == 0 && d.broker_pnl == -d.pnl.minor
            };
            // detail: every deal of the position, and the LP chain behind each order
            let position_deals: Vec<Value> = deals
                .iter()
                .filter(|x| x.position_id == d.position_id)
                .map(|x| json!({
                    "dealId": x.id.to_string(), "orderId": x.order_id.to_string(), "at": iso(x.ts),
                    "entry": format!("{:?}", x.entry), "side": side_str(x.side), "lots": qty_f(x.volume),
                    "price": price_f(x.price), "lpPrice": x.lp_price.map(price_f), "reason": format!("{:?}", x.reason),
                    "pnl": minor(x.pnl.minor), "lpPnl": minor(x.lp_pnl), "markup": minor(x.broker_pnl),
                    "commission": minor(-x.commission.minor), "swap": minor(x.swap), "swapFee": minor(-x.swap_fee),
                }))
                .collect();
            let order_ids: Vec<oms::OrderId> = {
                let mut v: Vec<oms::OrderId> = deals
                    .iter()
                    .filter(|x| x.position_id == d.position_id)
                    .map(|x| x.order_id)
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            };
            let orders: Vec<Value> = order_ids
                .iter()
                .filter_map(|id| e.order(*id))
                .map(|o| {
                    let lps: Vec<Value> = chain
                        .get(&o.id)
                        .map(|v| {
                            v.iter()
                                .enumerate()
                                .filter_map(|(i, lid)| e.lp_orders().find(|l| l.id == *lid).map(|l| (i, l)))
                                .map(|(i, l)| {
                                    let mut x = lp_order_detail(e, l, (i + 1, v.len()));
                                    x["lpOrderId"] = json!(l.id.to_string());
                                    x["lp"] = json!(l.lp);
                                    x["lots"] = json!(qty_f(l.volume));
                                    x["side"] = json!(side_str(l.side));
                                    x
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut c = client_order_detail(e, o);
                    c["lpOrders"] = Value::Array(lps);
                    c
                })
                .collect();
            // the closing order's own story for the (optional) columns
            let closing = orders.iter().find(|o| o["orderId"].as_str() == Some(&d.order_id.to_string()));
            let lp_last = closing.and_then(|o| o["lpOrders"].as_array()).and_then(|v| v.last().cloned());
            let lp_ids: Vec<String> = closing
                .and_then(|o| o["lpOrders"].as_array())
                .map(|v| v.iter().filter_map(|l| l["clOrdId"].as_str().map(String::from)).collect())
                .unwrap_or_default();
            json!({
                "openAt": (opened != u64::MAX).then(|| iso(opened)),
                "holdSecs": (opened != u64::MAX).then(|| d.ts.saturating_sub(opened) / 1_000_000_000),
                "orderId": d.order_id.to_string(),
                "platform": closing.map(|o| o["platform"].clone()).unwrap_or(Value::Null),
                "origin": closing.map(|o| o["origin"].clone()).unwrap_or(Value::Null),
                "rule": closing.map(|o| o["rule"].clone()).unwrap_or(Value::Null),
                "clientSlipPts": closing.map(|o| o["clientSlipPts"].clone()).unwrap_or(Value::Null),
                "attempts": closing.and_then(|o| o["lpOrders"].as_array().map(|v| v.len())).unwrap_or(0),
                "latencyMs": lp_last.as_ref().map(|l| l["firstFillMs"].clone()).unwrap_or(Value::Null),
                "lpSlipPts": lp_last.as_ref().map(|l| l["lpSlipPts"].clone()).unwrap_or(Value::Null),
                "lpKind": lp_last.as_ref().map(|l| l["kind"].clone()).unwrap_or(Value::Null),
                "lpOrderIds": lp_ids.join(", "),
                "swap": minor(d.swap),
                "detail": { "deals": position_deals, "orders": orders },
                "id": d.id.to_string(),
                "at": iso(d.ts),
                "login": d.account,
                "position": d.position_id.to_string(),
                "symbol": d.symbol,
                "side": side_str(d.side.opposite()),
                "lots": qty_f(d.volume),
                "book": if a_book { "A" } else { "B" },
                "reason": format!("{:?}", d.reason),
                "openClient": open,
                "openLp": open_lp,
                "closeClient": price_f(d.price),
                "closeLp": d.lp_price.map(price_f),
                "clientPnl": minor(d.pnl.minor),
                "lpPnl": minor(d.lp_pnl),
                "markup": minor(d.broker_pnl),
                "commission": minor(commission),
                "swapFee": minor(swap_fee),
                "broker": minor(broker),
                "ok": ok,
            })
        })
        .collect();
    rows.shrink_to_fit();
    Value::Array(rows)
}

/// Closed trades (closing deals) in `[from, to)`, newest first, at most `limit`.
pub fn trades(e: &Engine, from: u64, to: u64, limit: usize) -> Value {
    let deals = e.deals();
    let rows: Vec<Value> = deals
        .iter()
        .rev()
        .filter(|d| d.entry == oms::DealEntry::Out && d.ts >= from && d.ts < to)
        .take(limit)
        .map(|d| {
            let (mut vol, mut notional, mut opened) = (0f64, 0f64, u64::MAX);
            for i in deals
                .iter()
                .filter(|i| i.position_id == d.position_id && i.entry == oms::DealEntry::In)
            {
                vol += qty_f(i.volume);
                notional += qty_f(i.volume) * price_f(i.price);
                opened = opened.min(i.ts);
            }
            let order = e.order(d.order_id);
            let close = price_f(d.price);
            let open = if vol > 0.0 { round_px(e, &d.symbol, notional / vol) } else { close };
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
                "swap": minor(d.swap),
                "book": routing,
                "closedAt": iso(d.ts),
                "positionId": d.position_id.to_string(),
                "orderId": d.order_id.to_string(),
                "openAt": (opened != u64::MAX).then(|| iso(opened)),
                "holdSecs": (opened != u64::MAX).then(|| d.ts.saturating_sub(opened) / 1_000_000_000),
                "reason": format!("{:?}", d.reason),
                "lpOpenPrice": d.lp_price.map(|_| price_f(d.lp_price.unwrap_or(d.price))),
                "platform": order.map(|o| format!("{:?}", o.req.platform).to_lowercase()),
                "origin": order.map(|o| format!("{:?}", o.origin)),
                "rule": order.and_then(|o| o.rule.clone()),
                "commissionOpen": minor(deals.iter().filter(|i| i.position_id == d.position_id && i.entry == oms::DealEntry::In).map(|i| i.commission.minor).sum::<i128>()),
            })
        })
        .collect();
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
/// LP-side detail of one LP order: what went out (type, prices, the quote we
/// saw), every report with its latency, the attempt index in the client
/// order's chain, slippage against the sent quote (points, + = worse for us).
fn lp_order_detail(e: &Engine, l: &oms::LpOrder, attempt: (usize, usize)) -> Value {
    let point = e.symbol_spec(&l.symbol).map_or(0.0, |s| price_f(s.point()));
    let sign = if l.side == Side::Buy { 1.0 } else { -1.0 };
    let (mut lots, mut notional) = (0f64, 0f64);
    for f in &l.fills {
        lots += qty_f(f.volume);
        notional += qty_f(f.volume) * price_f(f.price);
    }
    let avg = (lots > 0.0).then_some(notional / lots);
    let sent = if l.side == Side::Buy {
        l.sent_ask
    } else {
        l.sent_bid
    }
    .map(price_f);
    let slip = match (avg, sent) {
        (Some(a), Some(s)) if point > 0.0 => Some(sign * (a - s) / point),
        _ => None,
    };
    let first = l.fills.iter().map(|f| f.ts).min();
    let last = l.fills.iter().map(|f| f.ts).max();
    let ms = |t: Option<u64>| t.map(|t| t.saturating_sub(l.created_ts) as f64 / 1e6);
    let kind = if l.hedge {
        "hedge"
    } else if l.resting && l.stop.is_some() {
        "stop GTC"
    } else if l.resting {
        "limit GTC"
    } else if l.limit.is_some() {
        "limit IOC"
    } else {
        "market IOC"
    };
    json!({
        "clOrdId": format!("LP-{}", l.id),
        "kind": kind,
        "limit": l.limit.map(price_f),
        "stop": l.stop.map(price_f),
        "resting": l.resting,
        "revision": l.revision,
        "hedge": l.hedge,
        "sentBid": l.sent_bid.map(price_f),
        "sentAsk": l.sent_ask.map(price_f),
        "sentAt": iso(l.created_ts),
        "attempt": attempt.0,
        "attempts": attempt.1,
        "firstFillMs": ms(first),
        "lastFillMs": ms(last),
        "lpSlipPts": slip,
        "fills": l.fills.iter().map(|f| json!({
            "execId": f.exec_id, "lots": qty_f(f.volume), "price": price_f(f.price), "at": iso(f.ts),
            "latencyMs": f.ts.saturating_sub(l.created_ts) as f64 / 1e6,
        })).collect::<Vec<_>>(),
        "reason": l.reject_reason,
        "done": l.done,
    })
}

/// Client-side detail of one order: what was asked, what was given, the
/// markup and the slippage against the request (points, + = worse for the client).
fn client_order_detail(e: &Engine, o: &oms::Order) -> Value {
    let point = e
        .symbol_spec(&o.req.symbol)
        .map_or(0.0, |s| price_f(s.point()));
    let sign = if o.req.side == Side::Buy { 1.0 } else { -1.0 };
    let requested = o
        .req
        .requested_price
        .or(o.req.limit_price)
        .or(o.req.stop_price)
        .map(price_f);
    let fill = (o.filled.raw() > 0).then(|| price_f(o.avg_price));
    let slip = match (requested, fill) {
        (Some(r), Some(f)) if point > 0.0 => Some(sign * (f - r) / point),
        _ => None,
    };
    json!({
        "orderId": o.id.to_string(),
        "clientOrderId": o.req.client_order_id,
        "login": o.req.account,
        "side": side_str(o.req.side),
        "kind": format!("{:?}", o.req.order_type).to_lowercase(),
        "origin": format!("{:?}", o.origin),
        "platform": format!("{:?}", o.req.platform).to_lowercase(),
        "ip": o.req.ip,
        "lots": qty_f(o.req.volume),
        "filledLots": qty_f(o.filled),
        "requested": requested,
        "price": fill,
        "clientSlipPts": slip,
        "status": format!("{:?}", o.status),
        "reason": o.reject_reason,
        "lpAttempts": o.lp_attempts,
        "createdAt": iso(o.created_ts),
        "rule": o.rule,
        "book": if o.routing == Routing::ABook { "A" } else { "B" },
        "maxDeviationPts": o.req.max_deviation_points,
        "markupOverridePts": o.markup_override,
    })
}

/// Every LP order each client order took part in, oldest first.
fn lp_chain(e: &Engine) -> BTreeMap<oms::OrderId, Vec<oms::LpOrderId>> {
    let mut m: BTreeMap<oms::OrderId, Vec<oms::LpOrderId>> = BTreeMap::new();
    for l in e.lp_orders() {
        for c in &l.children {
            m.entry(*c).or_default().push(l.id);
        }
    }
    m
}

/// LP orders created in `[from, to)`, newest first, at most `limit`.
pub fn lp_executions(e: &Engine, from: u64, to: u64, limit: usize) -> Value {
    let chain = lp_chain(e);
    let mut all: Vec<&oms::LpOrder> = e
        .lp_orders()
        .filter(|l| l.created_ts >= from && l.created_ts < to)
        .collect();
    all.reverse();
    all.truncate(limit);
    let rows: Vec<Value> = all
        .into_iter()
        .map(|l| {
            let (mut lots, mut notional) = (0f64, 0f64);
            for f in &l.fills {
                lots += qty_f(f.volume);
                notional += qty_f(f.volume) * price_f(f.price);
            }
            // attempt index of this LP order in its (first) client order's chain
            let attempt = l
                .children
                .first()
                .and_then(|c| chain.get(c))
                .map_or((1, 1), |v| {
                    (
                        v.iter().position(|x| *x == l.id).map_or(1, |p| p + 1),
                        v.len(),
                    )
                });
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
                    let mut v = json!({
                        "orderId": o.id.to_string(),
                        "login": o.req.account,
                        "lots": qty_f(o.filled),
                        "price": price_f(o.avg_price),
                    });
                    v["detail"] = client_order_detail(e, o);
                    v
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
            let detail = lp_order_detail(e, l, attempt);
            let login = clients.first().and_then(|c| c["login"].as_u64());
            let order_ids: Vec<String> = l.children.iter().map(|c| c.to_string()).collect();
            json!({
                "id": l.id.to_string(),
                "lp": l.lp,
                "symbol": l.symbol,
                "side": side_str(l.side),
                "lots": qty_f(l.volume),
                "filledLots": lots,
                "avgPrice": if lots > 0.0 { round_px(e, &l.symbol, notional / lots) } else { 0.0 },
                "status": status,
                "reason": l.reject_reason,
                "createdAt": iso(l.created_ts),
                "fills": fills,
                "clients": clients,
                "clOrdId": detail["clOrdId"].clone(),
                "kind": detail["kind"].clone(),
                "attempt": detail["attempt"].clone(),
                "attempts": detail["attempts"].clone(),
                "firstFillMs": detail["firstFillMs"].clone(),
                "lastFillMs": detail["lastFillMs"].clone(),
                "lpSlipPts": detail["lpSlipPts"].clone(),
                "sentBid": detail["sentBid"].clone(),
                "sentAsk": detail["sentAsk"].clone(),
                "limit": detail["limit"].clone(),
                "stop": detail["stop"].clone(),
                "revision": l.revision,
                "login": login,
                "orderIds": order_ids.join(", "),
                "fillCount": l.fills.len(),
                "detail": detail,
            })
        })
        .collect();
    Value::Array(rows)
}

#[derive(Default, Clone, Copy)]
struct RevenueTotals {
    markup: i128,
    b_book: i128,
    commission: i128,
    swap: i128,
    lp: i128,
}

impl RevenueTotals {
    fn add(&mut self, kind: TxnKind, a_book: bool, broker: i128, lp: i128) {
        match (kind, a_book) {
            (TxnKind::Commission, _) => self.commission += broker,
            (TxnKind::Swap, _) => self.swap += broker,
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
            "swap": minor(self.swap),
            "lp": minor(self.lp),
            "total": minor(self.markup + self.b_book + self.commission + self.swap),
        })
    }
}

/// Broker revenue per deal (minor units of the account currency), newest first, with
/// totals overall and since `since_ns`: the markup of closing A-book deals (LP result
/// minus client result), the result of closing B-book deals (the broker is the
/// counterparty) and commission. `lp` is our own result at the LP.
/// Activity of one account since `since_ns`: deals, lots, our revenue
/// (A-book: LP spread + commission + swap-free fee; B-book: client loss +
/// commission + swap)
/// and the client's realized P&L, in account-currency minor units.
pub fn account_activity(e: &Engine, account: u64, since_ns: u64) -> Value {
    let a_book = e
        .account(account)
        .and_then(|a| e.group(&a.group))
        .is_some_and(|g| g.routing == Routing::ABook);
    let (mut deals, mut lots, mut revenue, mut client) = (0u64, 0i64, 0i128, 0i128);
    for d in e
        .deals()
        .iter()
        .filter(|d| d.account == account && d.ts >= since_ns)
    {
        deals += 1;
        lots += d.volume.raw();
        if d.entry == oms::DealEntry::Out {
            revenue += d.broker_pnl;
            client += d.pnl.minor;
        }
        revenue -= d.commission.minor;
        client += d.commission.minor;
        // B-book: the whole swap is ours; A-book: only the swap-free fee (the
        // real swap passes through to the LP)
        revenue -= if !a_book && d.lp_price.is_none() {
            d.swap
        } else {
            d.swap_fee
        };
        client += d.swap;
    }
    json!({
        "deals": deals,
        "lots": lots as f64 / 1e8,
        "revenue": minor(revenue),
        "clientPnl": minor(client),
    })
}

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
            let tag = match kind {
                TxnKind::Commission => ("c", "commission"),
                TxnKind::Swap => ("s", "swap"),
                _ => ("d", "pnl"),
            };
            rows.push(json!({
                "id": format!("{}{}", tag.0, d.id),
                "at": iso(d.ts),
                "kind": tag.1,
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
        if d.swap != 0 || d.swap_fee != 0 {
            // B-book: the swap is ours; A-book: it passes through to the LP,
            // except the swap-free fee, which is always ours
            if a_book {
                let pass = d.swap - d.swap_fee;
                row(TxnKind::Swap, d.swap, -d.swap_fee, -pass);
            } else {
                row(TxnKind::Swap, d.swap, -d.swap, 0);
            }
        }
    }
    rows.reverse();
    rows.truncate(500);
    json!({ "total": all.json(), "last24h": recent.json(), "rows": rows })
}

/// FIX session rows in the back office `FixSession` shape. Sequence numbers
/// and latency are not tracked by the gateway yet and are reported as 0.
/// Aggregation policy plus runtime per LP (`GET /v1/lp/aggregation`).
pub fn lp_aggregation(
    agg: &crate::lp_agg::Aggregator,
    sessions: &[fix_gateway::SessionStatus],
) -> Value {
    let cfg = agg.config();
    let lps: Vec<Value> = agg
        .runtime()
        .into_iter()
        .map(|r| {
            let up = |kind: fix_gateway::SessionKind| {
                sessions
                    .iter()
                    .any(|s| s.lp == r.name && s.kind == kind && s.logged_on)
            };
            json!({
                "name": r.name,
                "enabled": r.policy.enabled,
                "orders": r.policy.orders,
                "priority": r.policy.priority,
                "minLots": r.policy.min_lots.map(qty_f),
                "maxLots": r.policy.max_lots.map(qty_f),
                "symbols": r.policy.symbols,
                "quoting": r.quoting,
                "deviating": r.deviating,
                "silent": r.silent,
                "lastQuoteAt": (r.last_quote_ns > 0).then(|| iso(r.last_quote_ns)),
                "mdUp": up(fix_gateway::SessionKind::MarketData),
                "tradeUp": up(fix_gateway::SessionKind::Trading),
            })
        })
        .collect();
    json!({
        "mode": cfg.mode,
        "maxDeviationPoints": cfg.max_deviation_points,
        "maxQuoteAgeMs": cfg.max_quote_age_ms,
        "lps": lps,
    })
}

fn lp_pctl(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let i = ((v.len() as f64 - 1.0) * q).round() as usize;
    v[i.min(v.len() - 1)]
}

/// Per-LP execution quality (`GET /v1/reports/lp`): fill rate, rejects,
/// slippage against the quote at send time (points), first-fill latency.
pub fn lp_report(e: &Engine) -> Value {
    #[derive(Default)]
    struct Acc {
        orders: u64,
        filled: u64,
        partial: u64,
        rejected: u64,
        working: u64,
        req_lots: f64,
        fill_lots: f64,
        slip: Vec<f64>,
        lat: Vec<f64>,
        last: u64,
        symbols: std::collections::BTreeSet<String>,
    }
    let points: BTreeMap<String, f64> = e
        .symbols()
        .map(|s| (s.symbol.clone(), price_f(s.point())))
        .collect();
    let mut by: BTreeMap<String, Acc> = BTreeMap::new();
    for l in e.lp_orders() {
        let a = by
            .entry(l.lp.clone().unwrap_or_else(|| "unassigned".into()))
            .or_default();
        a.orders += 1;
        a.req_lots += qty_f(l.volume);
        let lots: f64 = l.fills.iter().map(|f| qty_f(f.volume)).sum();
        a.fill_lots += lots;
        if lots <= 0.0 && l.reject_reason.is_some() {
            a.rejected += 1;
        } else if !l.done {
            a.working += 1;
        } else if lots + 1e-9 < qty_f(l.volume) {
            a.partial += 1;
        } else {
            a.filled += 1;
        }
        a.symbols.insert(l.symbol.clone());
        if let Some(f) = l.fills.first() {
            a.lat
                .push(f.ts.saturating_sub(l.created_ts) as f64 / 1_000_000.0);
            a.last = a.last.max(f.ts);
        }
        let reference = match l.side {
            risk::Side::Buy => l.sent_ask,
            risk::Side::Sell => l.sent_bid,
        };
        if let (Some(r), Some(p)) = (reference, points.get(&l.symbol)) {
            if lots > 0.0 && *p > 0.0 {
                let notional: f64 = l
                    .fills
                    .iter()
                    .map(|f| qty_f(f.volume) * price_f(f.price))
                    .sum();
                let avg = notional / lots;
                let slip = match l.side {
                    risk::Side::Buy => avg - price_f(r),
                    risk::Side::Sell => price_f(r) - avg,
                } / p;
                a.slip.push(slip);
            }
        }
    }
    Value::Array(
        by.into_iter()
            .map(|(name, mut a)| {
                let avg_slip = if a.slip.is_empty() {
                    0.0
                } else {
                    a.slip.iter().sum::<f64>() / a.slip.len() as f64
                };
                json!({
                    "lp": name,
                    "orders": a.orders,
                    "filled": a.filled,
                    "partial": a.partial,
                    "rejected": a.rejected,
                    "working": a.working,
                    "requestedLots": a.req_lots,
                    "filledLots": a.fill_lots,
                    "fillRate": if a.req_lots > 0.0 { a.fill_lots / a.req_lots } else { 0.0 },
                    "rejectRate": if a.orders > 0 { a.rejected as f64 / a.orders as f64 } else { 0.0 },
                    "avgSlipPoints": avg_slip,
                    "p95SlipPoints": lp_pctl(&mut a.slip, 0.95),
                    "p50LatencyMs": lp_pctl(&mut a.lat, 0.5),
                    "p95LatencyMs": lp_pctl(&mut a.lat, 0.95),
                    "lastFillAt": (a.last > 0).then(|| iso(a.last)),
                    "symbols": a.symbols.len(),
                })
            })
            .collect(),
    )
}

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
                let lp = if r.lp.is_empty() {
                    r.target_comp_id.clone()
                } else {
                    r.lp.clone()
                };
                json!({
                    "id": format!("{lp}-{}", kind.to_lowercase()),
                    "lp": lp,
                    "kind": kind,
                    "senderCompId": r.sender_comp_id,
                    "targetCompId": r.target_comp_id,
                    "status": status,
                    "inSeq": r.in_seq,
                    "outSeq": 0,
                    "latencyMs": 0,
                    "rejects24h": r.rejects,
                    "lastHeartbeat": iso(r.last_msg_ms.max(r.since_ms).saturating_mul(1_000_000)),
                    "lastMsgAgeMs": if r.last_msg_ms > 0 { domain::now_ns() / 1_000_000 - r.last_msg_ms.min(domain::now_ns() / 1_000_000) } else { 0 },
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
    swap: i128,
    lots: f64,
    orders: u32,
    rejects: u32,
    slips: Vec<f64>,
    lat: Vec<f64>,
}

fn p95(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f64 * 0.95).round() as usize]
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
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
        // swap kept by us on B-book closes (A-book swap passes to the LP,
        // except the swap-free fee, which is ours on either book)
        let swap = match (d.entry, a_book(d)) {
            (oms::DealEntry::Out, false) => -d.swap,
            (oms::DealEntry::Out, true) => -d.swap_fee,
            _ => 0,
        };
        let lots = qty_f(d.volume);
        let target = if d.ts >= since { &mut total } else { &mut prev };
        target.markup += markup;
        target.commission += commission;
        target.b_book += b_book;
        target.swap += swap;
        target.lots += lots;
        if d.ts >= since {
            let i = (((d.ts - since) / bucket) as usize).min(n_buckets - 1);
            let b = &mut buckets[i];
            b.markup += markup;
            b.commission += commission;
            b.b_book += b_book;
            b.swap += swap;
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
                        let slip = sign * (price_f(o.avg_price) - price_f(req)) / point;
                        slips.push(slip);
                        buckets[i].slips.push(slip);
                    }
                }
            }
        }
    }
    // LP latency p95 (send -> first fill), overall and per bucket
    let mut lat: Vec<f64> = Vec::new();
    for l in e.lp_orders().filter(|l| l.created_ts >= since) {
        if let Some(t) = l.fills.iter().map(|f| f.ts).min() {
            let ms = t.saturating_sub(l.created_ts) as f64 / 1e6;
            lat.push(ms);
            let i = (((l.created_ts - since) / bucket) as usize).min(n_buckets - 1);
            buckets[i].lat.push(ms);
        }
    }
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
            "swap": minor(b.swap),
            "lots": b.lots,
            "orders": b.orders,
            "rejects": b.rejects,
            "avgSlipPts": mean(&b.slips),
            "p95LatencyMs": p95(&mut b.lat.clone()),
            "fills": b.lat.len(),
        })
    };
    let totals_json = |b: &DashBucket| {
        json!({
            "revenue": minor(b.markup + b.commission + b.b_book + b.swap),
            "markup": minor(b.markup),
            "commission": minor(b.commission),
            "bBook": minor(b.b_book),
            "swap": minor(b.swap),
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
        let rule = e.match_rule(&acc.group, &o.req, pending);
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

/// Wire-log lines of one LP order: ClOrdID `id` and its revisions (`id-r1`,
/// `id-c2`…), plus every report under the OrderIDs those lines carry.
/// Newest 60 day files are scanned; at most 400 lines, oldest first.
pub fn fix_messages(dir: &std::path::Path, id: &str) -> Vec<Value> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files.reverse();
    let ours = |v: Option<&str>| v.is_some_and(|c| c == id || c.starts_with(&format!("{id}-")));
    let mut lines: Vec<Value> = Vec::new();
    let mut order_ids: BTreeSet<String> = BTreeSet::new();
    for f in files.iter().take(60) {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        for l in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(l) else {
                continue;
            };
            if ours(v["cl_ord_id"].as_str()) || ours(v["orig_cl_ord_id"].as_str()) {
                if let Some(o) = v["order_id"].as_str().filter(|o| !o.is_empty()) {
                    order_ids.insert(o.to_string());
                }
                lines.push(v);
            }
        }
    }
    // second pass: reports that only carry the LP's OrderID (e.g. unsolicited cancels)
    if !order_ids.is_empty() {
        for f in files.iter().take(60) {
            let Ok(text) = std::fs::read_to_string(f) else {
                continue;
            };
            for l in text.lines() {
                let Ok(v) = serde_json::from_str::<Value>(l) else {
                    continue;
                };
                let by_order = v["order_id"]
                    .as_str()
                    .is_some_and(|o| order_ids.contains(o));
                let already = ours(v["cl_ord_id"].as_str()) || ours(v["orig_cl_ord_id"].as_str());
                if by_order && !already {
                    lines.push(v);
                }
            }
        }
    }
    lines.sort_by_key(|v| v["ts_ms"].as_u64().unwrap_or(0));
    lines.truncate(400);
    lines
        .into_iter()
        .map(|v| {
            json!({
                "at": iso(v["ts_ms"].as_u64().unwrap_or(0) * 1_000_000),
                "lp": v["lp"], "dir": v["dir"], "msgType": v["msg_type"],
                "clOrdId": v["cl_ord_id"], "origClOrdId": v["orig_cl_ord_id"],
                "orderId": v["order_id"], "execId": v["exec_id"], "raw": v["raw"],
            })
        })
        .collect()
}

#[cfg(test)]
mod fix_messages_tests {
    use super::fix_messages;

    #[test]
    fn finds_revisions_and_reports_by_order_id() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026-10-08.jsonl");
        std::fs::write(&day, concat!(
            r#"{"ts_ms":3,"lp":"LMAX","dir":"in","msg_type":"8","cl_ord_id":"LP-170-r1","orig_cl_ord_id":"LP-170","order_id":"O1","exec_id":"E2","raw":"8=FIX.4.4|35=8|"}"#, "\n",
            r#"{"ts_ms":1,"lp":"LMAX","dir":"out","msg_type":"D","cl_ord_id":"LP-170","orig_cl_ord_id":null,"order_id":null,"exec_id":null,"raw":"8=FIX.4.4|35=D|"}"#, "\n",
            r#"{"ts_ms":2,"lp":"LMAX","dir":"out","msg_type":"D","cl_ord_id":"LP-1700","orig_cl_ord_id":null,"order_id":null,"exec_id":null,"raw":"8=FIX.4.4|35=D|"}"#, "\n",
            r#"{"ts_ms":4,"lp":"LMAX","dir":"in","msg_type":"8","cl_ord_id":null,"orig_cl_ord_id":null,"order_id":"O1","exec_id":"E3","raw":"8=FIX.4.4|35=8|37=O1|"}"#, "\n",
        )).unwrap();
        let rows = fix_messages(dir.path(), "LP-170");
        let ids: Vec<String> = rows
            .iter()
            .map(|r| r["clOrdId"].as_str().unwrap_or("-").to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["LP-170", "LP-170-r1", "-"],
            "oldest first, no LP-1700, order-id report included"
        );
        assert!(fix_messages(dir.path(), "LP-999").is_empty());
    }
}
