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

/// Strategy card: 30-day realized result, deals, win rate, followers.
async fn strategy_stats(ctx: &AdminCtx, recs: Vec<StrategyRec>) -> Result<Vec<Value>, ApiError> {
    ctx.q(move |e| {
        let now = e.now_ns();
        let subs = e.copy_subscriptions();
        recs.iter()
            .map(|r| {
                let since = now.saturating_sub(30 * DAY_NS);
                let closes: Vec<_> = e
                    .deals()
                    .iter()
                    .filter(|d| d.account == r.account && d.entry == DealEntry::Out && d.ts >= since)
                    .collect();
                let pnl: i128 = closes.iter().map(|d| d.pnl.minor + d.commission.minor + d.swap).sum();
                let wins = closes.iter().filter(|d| d.pnl.minor > 0).count();
                let followers = subs.iter().filter(|s| s.provider == r.account && s.active).count();
                let risk = e.account_risk(r.account).ok();
                let ret_pct = risk
                    .as_ref()
                    .filter(|k| k.balance.minor > 0)
                    .map(|k| (pnl as f64) * 100.0 / (k.balance.minor as f64));
                json!({
                    "account": r.account, "name": r.name, "description": r.description,
                    "perfFeeBps": r.perf_fee_bps, "public": r.public,
                    "pnl30d": pnl as i64, "return30dPct": ret_pct,
                    "deals30d": closes.len(),
                    "winRate": if closes.is_empty() { None } else { Some(wins as f64 / closes.len() as f64) },
                    "followers": followers,
                    "equity": risk.map(|k| k.equity.minor as i64),
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

fn writable(client: &ClientActor) -> Result<(), ApiError> {
    if client.read_only {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "read-only API key",
        ));
    }
    Ok(())
}

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
        // followers see the strategy, not the provider's account size
        if let Some(o) = s.as_object_mut() {
            o.remove("equity");
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
    writable(&client)?;
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
    writable(&client)?;
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
