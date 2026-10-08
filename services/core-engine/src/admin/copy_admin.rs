//! Copy trading (console "Copy trading", terminal "Strategies"). The engine
//! mirrors provider positions into follower accounts (`oms` copy_reconcile);
//! this module keeps the strategy catalogue (admin store) and turns console /
//! client requests into journaled engine commands.
//!
//! Gate: both sides of a subscription (follower and strategy provider) must
//! be in a group listed in `CORE_COPY_GROUPS` (comma separated, default the
//! demo groups), so the money path stays off for live groups until it is
//! deliberately enabled and a fee never moves between a demo and a live
//! account. The engine keeps them there: an account with an active
//! subscription cannot change group.

use super::auth::Actor;
use super::store::AdminCmd;
use super::{need, AdminCtx, ApiError, ApiResult, ClientActor};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oms::{Command, CopySubscription, DealEntry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StrategyRec {
    pub account: u64,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Performance fee above the high-water mark (bps).
    pub perf_fee_bps: u32,
    /// Listed in the terminal's strategy showcase.
    pub public: bool,
}

const DAY_NS: u64 = 86_400_000_000_000;

fn allowed_groups() -> Vec<String> {
    std::env::var("CORE_COPY_GROUPS")
        .unwrap_or_else(|_| "demo-retail,demo-hedge".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Refuses an account whose group copy trading is not enabled for.
async fn copy_group(ctx: &AdminCtx, account: u64, who: &str) -> Result<(), ApiError> {
    let group = ctx
        .q(move |e| e.account(account).map(|a| a.group.clone()))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("unknown {who}")))?;
    if !allowed_groups().contains(&group) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "copy_disabled",
            format!("copy trading is not enabled for group {group} ({who})"),
        ));
    }
    Ok(())
}

fn sub_json(s: &CopySubscription) -> Value {
    json!({
        "follower": s.follower, "provider": s.provider, "ratioBps": s.ratio_bps,
        "equityStopPct": s.equity_stop_pct, "perfFeeBps": s.perf_fee_bps,
        "since": super::views::iso(s.since_ts), "active": s.active,
        "stoppedReason": s.stopped_reason, "realized": s.realized as i64,
        "hwm": s.hwm as i64, "feesPaid": s.fees_paid as i64,
        "openCopies": s.copied.len(),
    })
}

/// Days of the showcase window (and points of the daily equity curve).
const CURVE_DAYS: u64 = 30;

/// Closed-deal equity curve of a strategy over the showcase window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Curve {
    /// Largest peak-to-trough fall, minor units of the account currency.
    pub max_dd: i128,
    /// Largest fall relative to the peak it fell from (%); `None` without a
    /// positive starting balance.
    pub max_dd_pct: Option<f64>,
    /// Value at the end of each day, oldest first (`days` points).
    pub daily: Vec<i128>,
}

/// `start`: balance at the start of the window; `results`: (ts, result) of
/// each closing deal in it (P&L + commission + swap, minor units). Daily
/// points close the days `from_day .. from_day + days` (days since epoch).
pub(crate) fn equity_curve(
    start: i128,
    results: &[(u64, i128)],
    from_day: u64,
    days: u64,
) -> Curve {
    let mut sorted = results.to_vec();
    sorted.sort_by_key(|r| r.0);
    let (mut v, mut peak, mut max_dd) = (start, start, 0i128);
    let mut max_dd_pct = (start > 0).then_some(0.0f64);
    for &(_, r) in &sorted {
        v += r;
        peak = peak.max(v);
        let dd = peak - v;
        max_dd = max_dd.max(dd);
        if peak > 0 && dd > 0 {
            let pct = dd as f64 * 100.0 / peak as f64;
            max_dd_pct = Some(max_dd_pct.map_or(pct, |m| m.max(pct)));
        }
    }
    let mut daily = Vec::with_capacity(days as usize);
    let (mut i, mut v) = (0usize, start);
    for d in 0..days {
        let end = (from_day + d + 1).saturating_mul(DAY_NS);
        while i < sorted.len() && sorted[i].0 < end {
            v += sorted[i].1;
            i += 1;
        }
        daily.push(v);
    }
    Curve {
        max_dd,
        max_dd_pct,
        daily,
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Strategy card: 30-day realized result, deals, win rate, followers, max
/// drawdown and a daily equity curve (closed deals only). Scans the deal
/// history, so it runs on the read replica.
async fn strategy_stats(ctx: &AdminCtx, recs: Vec<StrategyRec>) -> Result<Vec<Value>, ApiError> {
    let now = super::routes::now_ns();
    ctx.qr(move |e| {
        let subs = e.copy_subscriptions();
        let since = now.saturating_sub(CURVE_DAYS * DAY_NS);
        let from_day = (now / DAY_NS).saturating_sub(CURVE_DAYS - 1);
        recs.iter()
            .map(|r| {
                let closes: Vec<_> = e
                    .deals()
                    .iter()
                    .filter(|d| d.account == r.account && d.entry == DealEntry::Out && d.ts >= since)
                    .collect();
                // commission is charged per side: the opening side sits on the
                // in deal and counts at its own time
                let mut results: Vec<(u64, i128)> = closes
                    .iter()
                    .map(|d| (d.ts, d.pnl.minor + d.commission.minor + d.swap))
                    .collect();
                results.extend(
                    e.deals()
                        .iter()
                        .filter(|d| d.account == r.account && d.entry == DealEntry::In && d.ts >= since)
                        .map(|d| (d.ts, d.commission.minor)),
                );
                results.sort_by_key(|r| r.0);
                let pnl: i128 = results.iter().map(|r| r.1).sum();
                let wins = closes.iter().filter(|d| d.pnl.minor > 0).count();
                let followers = subs.iter().filter(|s| s.provider == r.account && s.active).count();
                let risk = e.account_risk(r.account).ok();
                let ret_pct = risk
                    .as_ref()
                    .filter(|k| k.balance.minor > 0)
                    .map(|k| (pnl as f64) * 100.0 / (k.balance.minor as f64));
                // balance before the window's closed results (deposits inside
                // the window count as if they were there from the start)
                let start = risk.as_ref().map_or(0, |k| k.balance.minor - pnl);
                let curve = equity_curve(start, &results, from_day, CURVE_DAYS);
                let return_curve: Vec<f64> = if start > 0 {
                    curve
                        .daily
                        .iter()
                        .map(|v| round2((v - start) as f64 * 100.0 / start as f64))
                        .collect()
                } else {
                    Vec::new()
                };
                json!({
                    "account": r.account, "name": r.name, "description": r.description,
                    "perfFeeBps": r.perf_fee_bps, "public": r.public,
                    "pnl30d": pnl as i64, "return30dPct": ret_pct,
                    "deals30d": closes.len(),
                    "winRate": if closes.is_empty() { None } else { Some(wins as f64 / closes.len() as f64) },
                    "followers": followers,
                    "equity": risk.map(|k| k.equity.minor as i64),
                    "maxDrawdown": curve.max_dd as i64,
                    "maxDrawdownPct": curve.max_dd_pct.map(round2),
                    "equityCurve": curve.daily.iter().map(|v| *v as i64).collect::<Vec<i64>>(),
                    "returnCurvePct": return_curve,
                })
            })
            .collect::<Vec<Value>>()
    })
    .await
}

// ------------------------------------------------------------- console

pub async fn list(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "clients.view")?;
    let recs: Vec<StrategyRec> = ctx
        .view_state()
        .await
        .strategies
        .values()
        .cloned()
        .collect();
    let strategies = strategy_stats(&ctx, recs).await?;
    let subs = ctx
        .q(|e| {
            e.copy_subscriptions()
                .into_iter()
                .map(sub_json)
                .collect::<Vec<_>>()
        })
        .await?;
    Ok(Json(
        json!({ "strategies": strategies, "subscriptions": subs, "groups": allowed_groups() }),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyReq {
    name: String,
    #[serde(default)]
    description: String,
    perf_fee_bps: u32,
    public: bool,
}

pub async fn save_strategy(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(account): Path<u64>,
    Json(r): Json<StrategyReq>,
) -> ApiResult {
    need(&actor, "clients.edit")?;
    let name = r.name.trim();
    if name.is_empty() || name.len() > 60 || r.description.len() > 500 {
        return Err(ApiError::bad(
            "name 1..60, description up to 500 characters",
        ));
    }
    if r.perf_fee_bps > 5_000 {
        return Err(ApiError::bad("perfFeeBps must be 0..5000"));
    }
    copy_group(&ctx, account, "account").await?;
    let rec = StrategyRec {
        account,
        name: name.into(),
        description: r.description.trim().into(),
        perf_fee_bps: r.perf_fee_bps,
        public: r.public,
    };
    ctx.store.lock().await.append(
        &actor,
        AdminCmd::StrategySaved {
            strategy: rec.clone(),
        },
    )?;
    ctx.notify(&["copyOverview", "listAudit"]);
    Ok(Json(json!(rec)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeReq {
    follower: u64,
    provider: u64,
    #[serde(default = "full")]
    ratio_bps: u32,
    #[serde(default)]
    equity_stop_pct: u32,
}

fn full() -> u32 {
    10_000
}

async fn subscribe_cmd(
    ctx: &AdminCtx,
    follower: u64,
    provider: u64,
    ratio: u32,
    stop: u32,
) -> ApiResult {
    let st = ctx.view_state().await;
    let strat = st
        .strategies
        .get(&provider)
        .cloned()
        .ok_or_else(|| ApiError::bad("provider is not a strategy"))?;
    // both sides: the fee moves from the follower's ledger account to the provider's
    copy_group(ctx, follower, "follower").await?;
    copy_group(ctx, provider, "provider").await?;
    ctx.cmd(Command::CopySubscribe {
        follower,
        provider,
        ratio_bps: ratio,
        equity_stop_pct: stop,
        perf_fee_bps: strat.perf_fee_bps,
    })
    .await?;
    ctx.notify(&["copyOverview", "clientCopy"]);
    Ok(Json(json!({ "ok": true })))
}

pub async fn subscribe(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(r): Json<SubscribeReq>,
) -> ApiResult {
    need(&actor, "clients.edit")?;
    subscribe_cmd(&ctx, r.follower, r.provider, r.ratio_bps, r.equity_stop_pct).await
}

#[derive(Deserialize)]
pub struct UnsubscribeReq {
    follower: u64,
    provider: u64,
    #[serde(default)]
    close: bool,
}

pub async fn unsubscribe(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(r): Json<UnsubscribeReq>,
) -> ApiResult {
    need(&actor, "clients.edit")?;
    ctx.cmd(Command::CopyUnsubscribe {
        follower: r.follower,
        provider: r.provider,
        close: r.close,
    })
    .await?;
    ctx.notify(&["copyOverview", "clientCopy"]);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct SettleReq {
    provider: u64,
}

pub async fn settle(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(r): Json<SettleReq>,
) -> ApiResult {
    need(&actor, "balance.approve")?;
    let ev = ctx
        .cmd(Command::CopySettle {
            provider: r.provider,
        })
        .await?;
    let fees: i64 = ev
        .iter()
        .filter_map(|e| match e {
            oms::Event::CopyFee { amount, .. } => Some(amount.minor as i64),
            _ => None,
        })
        .sum();
    ctx.notify(&["copyOverview"]);
    Ok(Json(json!({ "ok": true, "fees": fees })))
}

// ------------------------------------------------------------- client (terminal)

pub async fn client_list(State(ctx): State<AdminCtx>, client: ClientActor) -> ApiResult {
    let recs: Vec<StrategyRec> = ctx
        .view_state()
        .await
        .strategies
        .values()
        .filter(|r| r.public)
        .cloned()
        .collect();
    let mut strategies = strategy_stats(&ctx, recs).await?;
    for s in &mut strategies {
        // followers see the strategy, not the provider's account size: the
        // curve and the drawdown only as percentages
        if let Some(o) = s.as_object_mut() {
            o.remove("equity");
            o.remove("maxDrawdown");
            o.remove("equityCurve");
            // with the absolute result the balance is pnl / return: hide it too
            o.remove("pnl30d");
        }
    }
    let logins: Vec<u64> = client.logins.iter().map(|(_, l)| *l).collect();
    let mine = ctx
        .q(move |e| {
            e.copy_subscriptions()
                .into_iter()
                .filter(|s| logins.contains(&s.follower))
                .map(sub_json)
                .collect::<Vec<_>>()
        })
        .await?;
    Ok(Json(
        json!({ "strategies": strategies, "subscriptions": mine }),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSubscribeReq {
    account: String,
    provider: u64,
    #[serde(default = "full")]
    ratio_bps: u32,
    #[serde(default)]
    equity_stop_pct: u32,
}

pub async fn client_subscribe(
    State(ctx): State<AdminCtx>,
    client: ClientActor,
    Json(r): Json<ClientSubscribeReq>,
) -> ApiResult {
    client.interactive()?;
    let login = client
        .login_of(&r.account)
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "forbidden", "not your account"))?;
    let public = ctx
        .view_state()
        .await
        .strategies
        .get(&r.provider)
        .is_some_and(|s| s.public);
    if !public {
        return Err(ApiError::not_found("unknown strategy"));
    }
    subscribe_cmd(&ctx, login, r.provider, r.ratio_bps, r.equity_stop_pct).await
}

#[derive(Deserialize)]
pub struct ClientUnsubscribeReq {
    account: String,
    provider: u64,
    #[serde(default)]
    close: bool,
}

pub async fn client_unsubscribe(
    State(ctx): State<AdminCtx>,
    client: ClientActor,
    Json(r): Json<ClientUnsubscribeReq>,
) -> ApiResult {
    client.interactive()?;
    let login = client
        .login_of(&r.account)
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "forbidden", "not your account"))?;
    ctx.cmd(Command::CopyUnsubscribe {
        follower: login,
        provider: r.provider,
        close: r.close,
    })
    .await?;
    ctx.notify(&["copyOverview", "clientCopy"]);
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equity_curve_drawdown_and_daily_points() {
        // start 10 000.00; day 0: +500, day 1: -1 200, day 3: +300
        let day = 20_000u64;
        let at = |d: u64, h: u64| (day + d) * DAY_NS + h * 3_600_000_000_000;
        let results = [
            (at(1, 9), -120_000),
            (at(0, 10), 50_000),
            (at(3, 1), 30_000),
        ];
        let c = equity_curve(1_000_000, &results, day, 5);
        assert_eq!(c.daily, vec![1_050_000, 930_000, 930_000, 960_000, 960_000]);
        // peak 10 500.00 -> trough 9 300.00
        assert_eq!(c.max_dd, 120_000);
        let pct = c.max_dd_pct.unwrap();
        assert!(
            (pct - 120_000.0 * 100.0 / 1_050_000.0).abs() < 1e-9,
            "{pct}"
        );
        // the last point minus the start is the window's result
        let total: i128 = results.iter().map(|r| r.1).sum();
        assert_eq!(*c.daily.last().unwrap() - 1_000_000, total);
    }

    #[test]
    fn equity_curve_without_deals_or_balance() {
        let c = equity_curve(500_000, &[], 100, 30);
        assert_eq!(c.daily.len(), 30);
        assert!(c.daily.iter().all(|v| *v == 500_000));
        assert_eq!((c.max_dd, c.max_dd_pct), (0, Some(0.0)));
        // no positive balance: no percentage
        let c = equity_curve(0, &[(100 * DAY_NS, -5_000)], 100, 2);
        assert_eq!(c.daily, vec![-5_000, -5_000]);
        assert_eq!((c.max_dd, c.max_dd_pct), (5_000, None));
    }
}
