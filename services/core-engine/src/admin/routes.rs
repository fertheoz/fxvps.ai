//! `/v1/*` handlers. Every handler re-checks RBAC; every mutation is
//! journaled (engine command and/or admin record with the actor).

use super::auth::{self, Actor, Claims, Role};
use super::store::{
    AdminCmd, AdminState, AdminUserRec, BalanceKind, BalanceOp, OpStatus, SettingsRec,
};
use super::{need, views, AdminCtx, ApiError, ApiResult};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use money::{Currency, Money, Price, Qty};
use oms::{Command, Engine, Event};
use risk::{
    AssetClass, EsmaPreset, GroupCommission, GroupConfig, MarginMode, PartialFill, Routing,
    RoutingRule, SymbolSpec,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn router() -> Router<AdminCtx> {
    Router::new()
        .route("/auth/dev-token", post(dev_token))
        .route("/auth/access-token", post(access_token))
        .route("/v1/me", get(me))
        .route("/v1/stream", get(super::stream::stream))
        .route("/v1/stream/ticket", post(super::stream::ticket))
        .route("/v1/dashboard", get(dashboard))
        .route("/v1/dashboard/series", get(dashboard_series))
        .route("/v1/exposure", get(exposure))
        .route("/v1/accounts", get(list_accounts).post(open_account))
        .route("/v1/accounts/{id}", get(get_account))
        .route("/v1/accounts/{id}/positions", get(account_positions))
        .route("/v1/accounts/{id}/balance-ops", post(balance_op))
        .route("/v1/accounts/{id}/kyc", patch(set_kyc))
        .route("/v1/accounts/{id}/group", patch(set_group))
        .route("/v1/approvals", get(list_approvals))
        .route("/v1/approvals/{id}/approve", post(approve))
        .route("/v1/approvals/{id}/reject", post(reject))
        .route("/v1/groups", get(list_groups))
        .route("/v1/rules", get(list_rules).put(save_rules))
        .route("/v1/rules/dry-run", get(rules_dry_run))
        .route("/v1/groups/{id}", put(save_group))
        .route("/v1/groups/{id}/apply-preset", post(apply_preset))
        .route("/v1/symbols", get(list_symbols))
        .route("/v1/symbols/{name}", put(save_symbol))
        .route("/v1/positions", get(list_positions))
        .route("/v1/orders", get(list_orders))
        .route("/v1/positions/force-close", post(force_close))
        .route("/v1/risk/margin-calls", get(margin_calls))
        .route("/v1/risk/presets", get(presets))
        .route("/v1/risk/hedge", get(hedge_get).put(hedge_put))
        .route("/v1/settings/swap", get(swap_get).put(swap_put))
        .route("/v1/settings/swap/rollover", post(swap_rollover))
        .route("/v1/alerts", get(list_alerts))
        .route("/v1/alerts/{id}/ack", post(ack_alert))
        .route("/v1/reports/clients", get(client_flow))
        .route("/v1/lp/sessions", get(lp_sessions))
        .route("/v1/lp/config", get(lp_config_get).put(lp_config_put))
        .route("/v1/lp/sessions/{id}/reconnect", post(lp_reconnect))
        .route("/v1/lp/aggregation", get(lp_agg_get).put(lp_agg_put))
        .route("/v1/reports/lp", get(lp_report))
        .route("/v1/reports/trades", get(trades))
        .route("/v1/reports/statements", get(statements))
        .route("/v1/reports/lp-executions", get(lp_executions))
        .route("/v1/reports/execution", get(execution))
        .route("/v1/reports/revenue", get(revenue))
        .route("/v1/audit", get(audit))
        .route("/v1/admin-users", get(list_users))
        .route("/v1/admin-users/{id}", put(save_user))
        .route("/v1/settings", get(get_settings).put(save_settings))
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

const ENGINE_TOPICS: [&str; 8] = [
    "dashboard",
    "exposure",
    "listClients",
    "getClient",
    "listPositions",
    "listOrders",
    "marginCalls",
    "statements",
];

impl AdminCtx {
    async fn q<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Engine) -> T + Send + 'static,
    ) -> Result<T, ApiError> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.engine
            .query(move |e| {
                let _ = tx.send(f(e));
                Value::Null
            })
            .await
            .map_err(ApiError::internal)?;
        rx.recv().map_err(|e| ApiError::internal(e.to_string()))
    }

    async fn cmd(&self, c: Command) -> Result<Vec<Event>, ApiError> {
        let ev = self.engine.command(c).await.map_err(ApiError::internal)?;
        if let Some(Event::CommandRejected { reason }) = ev
            .iter()
            .find(|e| matches!(e, Event::CommandRejected { .. }))
        {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "rejected",
                reason.clone(),
            ));
        }
        Ok(ev)
    }

    /// Credit/KYC view of the admin state (cheap clone for engine queries).
    pub(super) async fn view_state(&self) -> AdminState {
        let st = &self.store.lock().await.state;
        AdminState {
            credit: st.credit.clone(),
            kyc: st.kyc.clone(),
            profiles: st.profiles.clone(),
            ..Default::default()
        }
    }
}

fn parse_id(s: &str) -> Result<u64, ApiError> {
    s.parse()
        .map_err(|_| ApiError::bad(format!("invalid id {s:?}")))
}

fn parse_ccy(s: &str) -> Result<Currency, ApiError> {
    s.parse()
        .map_err(|_| ApiError::bad(format!("invalid currency {s:?}")))
}

const DAY_NS: u64 = 86_400_000_000_000;

fn day(ns: u64) -> u64 {
    ns / DAY_NS
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// auth
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DevTokenReq {
    role: String,
    name: Option<String>,
}

/// `POST /auth/dev-token` — only with `CORE_DEV_AUTH=1` (404 otherwise), which
/// the binary accepts only on a loopback bind and without production keys.
/// Signs with the random dev key; `admin` needs `CORE_DEV_AUTH_ADMIN=1`. The
/// `sub` is derived from the role by the server (a caller-supplied `sub` is
/// ignored), so four-eyes cannot be satisfied by choosing two subjects.
async fn dev_token(State(ctx): State<AdminCtx>, Json(req): Json<DevTokenReq>) -> ApiResult {
    if !ctx.auth.dev_enabled() {
        return Err(ApiError::not_found("dev auth disabled"));
    }
    let role = Role::parse(&req.role).ok_or_else(|| ApiError::bad("unknown role"))?;
    if !ctx.auth.dev_allows(role) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "dev admin tokens need CORE_DEV_AUTH_ADMIN=1",
        ));
    }
    let now = auth::unix_now();
    let sub = format!("dev-{}", role.as_str());
    let claims = Claims {
        name: Some(req.name.unwrap_or_else(|| sub.clone())),
        sub,
        role,
        exp: now + 8 * 3600,
        iat: Some(now),
        amr: vec!["dev".into(), "mfa".into()],
        iss: None,
    };
    let token = ctx
        .auth
        .sign_dev(&claims)
        .ok_or_else(|| ApiError::internal("signing failed"))?;
    Ok(Json(
        json!({ "token": token, "expiresAt": views::iso(claims.exp * 1_000_000_000) }),
    ))
}

/// Header Cloudflare adds to every request that passed an Access policy.
pub const CF_ACCESS_HEADER: &str = "cf-access-jwt-assertion";

/// Exchanges a verified Cloudflare Access assertion for a 1 h console session
/// token (role from `CORE_CF_ACCESS_ADMINS`). Only with `CORE_CF_ACCESS_TEAM`.
async fn access_token(State(ctx): State<AdminCtx>, headers: HeaderMap) -> ApiResult {
    if !ctx.auth.session_enabled() {
        return Err(ApiError::not_found("Cloudflare Access login disabled"));
    }
    let assertion = headers
        .get(CF_ACCESS_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            ApiError::unauthorized(
                "no Cloudflare Access assertion (open the console through its Access URL)",
            )
        })?;
    let actor = ctx
        .auth
        .verify(assertion)
        .map_err(|e| ApiError::unauthorized(format!("Access assertion rejected: {}", e.0)))?;
    let now = auth::unix_now();
    let claims = Claims {
        sub: actor.sub,
        name: Some(actor.name),
        role: actor.role,
        exp: now + 3600,
        iat: Some(now),
        amr: vec!["cf-access".into()],
        iss: None,
    };
    let token = ctx
        .auth
        .sign_session(&claims)
        .ok_or_else(|| ApiError::internal("signing failed"))?;
    Ok(Json(
        json!({ "token": token, "expiresAt": views::iso(claims.exp * 1_000_000_000) }),
    ))
}

async fn me(actor: Actor) -> ApiResult {
    let perms: Vec<&str> = auth::PERMISSIONS
        .iter()
        .copied()
        .filter(|p| actor.can(p))
        .collect();
    Ok(Json(
        json!({ "sub": actor.sub, "name": actor.name, "role": actor.role, "permissions": perms,
                // false: mutating permissions answer `mfa_required` until a 2FA login
                "mfaOk": actor.mfa_ok }),
    ))
}

// ---------------------------------------------------------------------------
// dashboard / accounts
// ---------------------------------------------------------------------------

async fn dashboard(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "dashboard.view")?;
    let (ops, today) = {
        let st = &ctx.store.lock().await.state;
        let ops: Vec<BalanceOp> = st
            .ops
            .values()
            .filter(|o| o.status == OpStatus::Applied)
            .cloned()
            .collect();
        (ops, day(now_ns()))
    };
    let applied_at = |o: &BalanceOp| o.decided_at.unwrap_or(o.requested_at);
    let sum = |kind: BalanceKind, d: u64| -> i64 {
        ops.iter()
            .filter(|o| o.kind == kind && day(applied_at(o)) == d)
            .map(|o| o.amount)
            .sum()
    };
    let series: Vec<Value> = (0..7u64)
        .rev()
        .map(|back| {
            let d = today - back;
            json!({
                "day": views::iso(d * 86_400_000_000_000)[..10].to_string(),
                "deposits": sum(BalanceKind::Deposit, d),
                "withdrawals": sum(BalanceKind::Withdraw, d),
            })
        })
        .collect();
    let (dep, wd) = (
        sum(BalanceKind::Deposit, today),
        sum(BalanceKind::Withdraw, today),
    );
    let (lp_up, lp_total) = ctx
        .lp_status
        .as_ref()
        .and_then(|s| s.read().ok().map(|t| t.clone()))
        .map(|rows| (rows.iter().filter(|r| r.logged_on).count(), rows.len()))
        .unwrap_or((0, 0));
    let since = now_ns().saturating_sub(DAY_NS);
    let v = ctx
        .q(move |e| {
            let ps = views::all_positions(e);
            let active: BTreeSet<u64> = ps.iter().map(|p| p.account).collect();
            let (mut a, mut b) = (0i128, 0i128);
            for p in &ps {
                let pnl = e.position_pnl(p.id).map(|m| m.minor).unwrap_or(0);
                match p.routing {
                    Routing::ABook => a += pnl,
                    Routing::BBook => b += pnl,
                }
            }
            // A-book client P&L is hedged at the LP: broker revenue is the realized
            // markup (plus commission) booked in the ledger over the last 24 h.
            let _ = a;
            let rev = views::revenue(e, since);
            let a_rev = rev["last24h"]["markup"].as_i64().unwrap_or(0)
                + rev["last24h"]["commission"].as_i64().unwrap_or(0);
            json!({
                "activeAccounts": active.len(),
                "totalAccounts": e.accounts().count(),
                "depositsToday": dep,
                "withdrawalsToday": wd,
                "aBookPnl": a_rev,
                // broker is the counterparty of B-book client P&L
                "bBookPnl": -(b as i64),
                "openPositions": ps.len(),
                "lpUp": lp_up,
                "lpTotal": lp_total,
                "pnlSeries": [],
                "depositSeries": series,
            })
        })
        .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
struct RangeQuery {
    #[serde(default)]
    range: Option<String>,
}

async fn dashboard_series(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Query(q): Query<RangeQuery>,
) -> ApiResult {
    need(&actor, "dashboard.view")?;
    let range = match q.range.as_deref() {
        Some(r @ ("today" | "24h" | "7d" | "30d")) => r.to_string(),
        None => "24h".to_string(),
        _ => return Err(ApiError::bad("range must be today, 24h, 7d or 30d")),
    };
    let st = ctx.view_state().await;
    let now = now_ns();
    Ok(Json(
        ctx.q(move |e| views::dashboard_series(e, &st, now, &range))
            .await?,
    ))
}

async fn exposure(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "dashboard.view")?;
    Ok(Json(ctx.q(views::exposure).await?))
}

#[derive(Deserialize)]
struct Search {
    search: Option<String>,
}

async fn list_accounts(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Query(q): Query<Search>,
) -> ApiResult {
    need(&actor, "clients.view")?;
    let st = ctx.view_state().await;
    Ok(Json(
        ctx.q(move |e| views::clients(e, &st, q.search.as_deref()))
            .await?,
    ))
}

#[derive(Deserialize)]
struct OpenAccountReq {
    name: String,
    email: String,
    group: String,
}

/// Opens a client account (engine `OpenAccount`, next free number >= 100001);
/// funding is a separate deposit (`/balance-ops`, 4-eyes rules apply).
async fn open_account(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(req): Json<OpenAccountReq>,
) -> ApiResult {
    need(&actor, "clients.edit")?;
    let name = req.name.trim().to_string();
    let email = req.email.trim().to_lowercase();
    if name.chars().count() < 2 || name.chars().count() > 100 {
        return Err(ApiError::bad("name must be 2-100 characters"));
    }
    if !email.contains('@') || email.len() > 254 {
        return Err(ApiError::bad("invalid e-mail"));
    }
    let group = req.group.clone();
    let (exists, next) = ctx
        .q(move |e| {
            let next = e
                .accounts()
                .map(|a| a.id)
                .max()
                .unwrap_or(100_000)
                .max(100_000)
                + 1;
            (e.group(&group).is_some(), next)
        })
        .await?;
    if !exists {
        return Err(ApiError::bad(format!("unknown group {:?}", req.group)));
    }
    let mut store = ctx.store.lock().await;
    ctx.cmd(Command::OpenAccount {
        account: next,
        group: req.group.clone(),
    })
    .await?;
    store.append(
        &actor,
        AdminCmd::AccountOpened {
            account: next,
            group: req.group,
            profile: super::store::ClientProfile { name, email },
        },
    )?;
    drop(store);
    ctx.notify(&["listClients", "listAudit", "dashboard"]);
    let st = ctx.view_state().await;
    ctx.q(move |e| views::client(e, next, &st))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::internal("account not visible after open"))
}

async fn get_account(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
) -> ApiResult {
    need(&actor, "clients.view")?;
    let id = parse_id(&id)?;
    let st = ctx.view_state().await;
    ctx.q(move |e| views::client(e, id, &st))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("unknown account"))
}

async fn account_positions(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
) -> ApiResult {
    need(&actor, "positions.view")?;
    let id = parse_id(&id)?;
    Ok(Json(
        ctx.q(move |e| {
            Value::Array(
                e.positions_of(id)
                    .into_iter()
                    .map(|p| views::position(e, p))
                    .collect(),
            )
        })
        .await?,
    ))
}

// ---------------------------------------------------------------------------
// balance operations + 4-eyes approvals
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BalanceOpReq {
    #[serde(rename = "type")]
    kind: BalanceKind,
    amount: i64,
    currency: String,
    reason: String,
    idempotency_key: Option<String>,
}

fn op_result(op: &BalanceOp, replayed: bool) -> Value {
    json!({
        "id": op.id,
        "status": match op.status { OpStatus::Applied => "applied", OpStatus::PendingApproval => "pending_approval", OpStatus::Rejected => "rejected" },
        "newBalance": op.new_balance,
        "newCredit": op.new_credit,
        "replayed": replayed,
    })
}

fn op_json(op: &BalanceOp) -> Value {
    json!({
        "id": op.id,
        "clientId": op.account.to_string(),
        "login": op.account,
        "type": op.kind.as_str(),
        "amount": op.amount,
        "currency": op.currency,
        "reason": op.reason,
        "idempotencyKey": op.idempotency_key,
        "requestedBy": op.requested_by.name,
        "requestedBySub": op.requested_by.sub,
        "requestedByRole": op.requested_by.role,
        "requestedAt": views::iso(op.requested_at),
        "status": op_result(op, false)["status"],
        "decidedBy": op.decided_by.as_ref().map(|a| a.name.clone()),
        "decidedAt": op.decided_at.map(views::iso),
        "note": op.decision_note,
    })
}

/// Executes a balance op against the engine; returns (new balance, new credit).
async fn execute(ctx: &AdminCtx, st: &AdminState, op: &BalanceOp) -> Result<(i64, i64), ApiError> {
    let ccy = parse_ccy(&op.currency)?;
    let amount = Money::new(op.amount as i128, ccy);
    let key = format!("bo:{}", op.idempotency_key);
    let mut credit = st.credit_of(op.account);
    match op.kind {
        BalanceKind::Deposit => {
            ctx.cmd(Command::Deposit {
                account: op.account,
                amount,
                key,
            })
            .await?;
        }
        BalanceKind::Withdraw => {
            ctx.cmd(Command::Withdraw {
                account: op.account,
                amount,
                key,
            })
            .await?;
        }
        BalanceKind::Credit => credit += op.amount,
    }
    let account = op.account;
    let bal = ctx
        .q(move |e| e.balance(account).map(|m| m.minor as i64).unwrap_or(0))
        .await?;
    Ok((bal, credit))
}

async fn balance_op(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<BalanceOpReq>,
) -> ApiResult {
    need(&actor, req.kind.permission())?;
    let account = parse_id(&id)?;
    let header_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let key = match (header_key, req.idempotency_key) {
        (Some(h), Some(b)) if h != b => {
            return Err(ApiError::bad("Idempotency-Key header differs from body"))
        }
        (Some(k), _) | (None, Some(k)) => k,
        (None, None) => return Err(ApiError::bad("Idempotency-Key is required")),
    };
    if key.is_empty() || key.len() > 128 {
        return Err(ApiError::bad("invalid Idempotency-Key"));
    }
    if req.amount <= 0 {
        return Err(ApiError::bad(
            "amount must be a positive integer in minor units",
        ));
    }
    let reason = req.reason.trim().to_string();
    if reason.chars().count() < 5 || reason.chars().count() > 500 {
        return Err(ApiError::bad("reason is required (5-500 chars)"));
    }
    let mut store = ctx.store.lock().await;
    if let Some(op) = store
        .state
        .keys
        .get(&key)
        .and_then(|i| store.state.ops.get(i))
    {
        if op.account != account
            || op.kind != req.kind
            || op.amount != req.amount
            || op.currency != req.currency
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "Idempotency-Key was used for a different request",
            ));
        }
        return Ok(Json(op_result(op, true)));
    }
    let acct = ctx
        .q(move |e| {
            let a = e.account(account)?;
            let g = e.group(&a.group)?;
            Some((
                g.currency.as_str().to_string(),
                e.balance(account).map(|m| m.minor as i64).unwrap_or(0),
            ))
        })
        .await?;
    let Some((ccy, balance)) = acct else {
        return Err(ApiError::not_found("unknown account"));
    };
    if ccy != req.currency {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "currency_mismatch",
            format!("account currency is {ccy}"),
        ));
    }
    let mut op = BalanceOp {
        id: format!("op-{}", store.next_seq()),
        account,
        kind: req.kind,
        amount: req.amount,
        currency: req.currency,
        reason,
        idempotency_key: key,
        requested_by: actor.clone(),
        requested_at: now_ns(),
        status: OpStatus::PendingApproval,
        decided_by: None,
        decided_at: None,
        decision_note: None,
        new_balance: balance,
        new_credit: store.state.credit_of(account),
    };
    let rec = if req.amount >= store.state.settings.four_eyes_threshold {
        store.append(&actor, AdminCmd::BalanceQueued { op: op.clone() })?
    } else {
        let (b, c) = execute(&ctx, &store.state, &op).await?;
        op.status = OpStatus::Applied;
        op.new_balance = b;
        op.new_credit = c;
        store.append(&actor, AdminCmd::BalanceApplied { op: op.clone() })?
    };
    let _ = rec;
    let op = store.state.ops[&op.id].clone();
    drop(store);
    ctx.notify(&[
        "listClients",
        "getClient",
        "listApprovals",
        "listAudit",
        "dashboard",
        "statements",
    ]);
    Ok(Json(op_result(&op, false)))
}

#[derive(Deserialize)]
struct ApprovalsQuery {
    status: Option<String>,
}

async fn list_approvals(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Query(q): Query<ApprovalsQuery>,
) -> ApiResult {
    if ![
        "balance.approve",
        "balance.deposit",
        "balance.withdraw",
        "balance.credit",
    ]
    .iter()
    .any(|p| actor.can(p))
    {
        return Err(ApiError::forbidden("balance.approve"));
    }
    let st = &ctx.store.lock().await.state;
    let want = q.status.unwrap_or_else(|| "pending_approval".into());
    let mut v: Vec<&BalanceOp> = st
        .ops
        .values()
        .filter(|o| o.requested_at > 0)
        .filter(|o| want == "all" || op_result(o, false)["status"] == want.as_str())
        .collect();
    v.sort_by_key(|o| std::cmp::Reverse(o.requested_at));
    Ok(Json(Value::Array(v.into_iter().map(op_json).collect())))
}

fn pending(st: &AdminState, id: &str) -> Result<BalanceOp, ApiError> {
    let op = st
        .ops
        .get(id)
        .ok_or_else(|| ApiError::not_found("unknown approval"))?;
    if op.status != OpStatus::PendingApproval {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "not_pending",
            "approval already decided",
        ));
    }
    Ok(op.clone())
}

async fn approve(State(ctx): State<AdminCtx>, actor: Actor, Path(id): Path<String>) -> ApiResult {
    need(&actor, "balance.approve")?;
    let mut store = ctx.store.lock().await;
    let op = pending(&store.state, &id)?;
    if op.requested_by.sub == actor.sub {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "four_eyes",
            "the second approval must come from a different user",
        ));
    }
    let (b, c) = execute(&ctx, &store.state, &op).await?;
    store.append(
        &actor,
        AdminCmd::BalanceApproved {
            id: id.clone(),
            new_balance: b,
            new_credit: c,
        },
    )?;
    let v = op_json(&store.state.ops[&id]);
    drop(store);
    ctx.notify(&[
        "listClients",
        "getClient",
        "listApprovals",
        "listAudit",
        "dashboard",
        "statements",
    ]);
    Ok(Json(v))
}

#[derive(Deserialize, Default)]
struct RejectReq {
    reason: Option<String>,
}

async fn reject(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    body: Option<Json<RejectReq>>,
) -> ApiResult {
    need(&actor, "balance.approve")?;
    let note = body
        .and_then(|b| b.0.reason)
        .unwrap_or_default()
        .trim()
        .to_string();
    let mut store = ctx.store.lock().await;
    pending(&store.state, &id)?;
    store.append(
        &actor,
        AdminCmd::BalanceRejected {
            id: id.clone(),
            note,
        },
    )?;
    let v = op_json(&store.state.ops[&id]);
    drop(store);
    ctx.notify(&["listApprovals", "listAudit"]);
    Ok(Json(v))
}

#[derive(Deserialize)]
struct KycReq {
    kyc: String,
}

async fn set_kyc(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(req): Json<KycReq>,
) -> ApiResult {
    need(&actor, "clients.edit")?;
    let account = parse_id(&id)?;
    if !["none", "pending", "approved", "rejected"].contains(&req.kyc.as_str()) {
        return Err(ApiError::bad("invalid kyc status"));
    }
    if !ctx.q(move |e| e.account(account).is_some()).await? {
        return Err(ApiError::not_found("unknown account"));
    }
    ctx.store.lock().await.append(
        &actor,
        AdminCmd::KycSet {
            account,
            kyc: req.kyc,
        },
    )?;
    ctx.notify(&["listClients", "getClient", "listAudit"]);
    let st = ctx.view_state().await;
    ctx.q(move |e| views::client(e, account, &st))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("unknown account"))
}

#[derive(Deserialize)]
struct GroupReq {
    group: String,
}

/// Moves an account to another group (margin mode / routing); the engine
/// refuses while the account has open positions or working orders.
async fn set_group(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(req): Json<GroupReq>,
) -> ApiResult {
    need(&actor, "clients.edit")?;
    let account = parse_id(&id)?;
    let group = req.group.clone();
    let old = ctx
        .q(move |e| e.account(account).map(|a| a.group.clone()))
        .await?
        .ok_or_else(|| ApiError::not_found("unknown account"))?;
    ctx.cmd(Command::SetAccountGroup {
        account,
        group: group.clone(),
    })
    .await?;
    ctx.store.lock().await.append(
        &actor,
        AdminCmd::AccountGroupSet {
            account,
            group,
            old,
        },
    )?;
    ctx.notify(&["listClients", "getClient", "listAudit"]);
    let st = ctx.view_state().await;
    ctx.q(move |e| views::client(e, account, &st))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("unknown account"))
}

// ---------------------------------------------------------------------------
// groups / symbols (journaled as engine SetGroup / AddSymbol)
// ---------------------------------------------------------------------------

async fn list_rules(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "groups.view")?;
    Ok(Json(ctx.q(views::rules).await?))
}

async fn save_rules(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(rules): Json<Vec<RoutingRule>>,
) -> ApiResult {
    need(&actor, "groups.edit")?;
    if rules.len() > 200 {
        return Err(ApiError::bad("at most 200 rules"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for r in &rules {
        if r.id.is_empty() || r.id.len() > 32 || !seen.insert(r.id.clone()) {
            return Err(ApiError::bad("rule ids must be unique, 1-32 characters"));
        }
        if r.name.len() > 64 {
            return Err(ApiError::bad("rule name: at most 64 characters"));
        }
        if r.a_book_pct.is_some_and(|p| p > 100) {
            return Err(ApiError::bad("aBookPct must be 0..100"));
        }
        if r.markup_points.is_some_and(|p| !(0..=1000).contains(&p))
            || r.max_slippage_points
                .is_some_and(|p| !(0..=1000).contains(&p))
        {
            return Err(ApiError::bad("points must be 0..1000"));
        }
        if r.hours_utc.is_some_and(|(a, b)| a > 23 || b > 24) {
            return Err(ApiError::bad("hoursUtc must be within 0..24"));
        }
        if r.min_toxicity.is_some_and(|t| t > 100) || r.max_toxicity.is_some_and(|t| t > 100) {
            return Err(ApiError::bad("toxicity must be 0..100"));
        }
    }
    let n = rules.len();
    let mut store = ctx.store.lock().await;
    ctx.cmd(Command::SetRules(rules)).await?;
    store.append(&actor, AdminCmd::RulesSaved { count: n })?;
    drop(store);
    ctx.notify(&["listRules", "rulesDryRun", "listAudit"]);
    Ok(Json(ctx.q(views::rules).await?))
}

async fn rules_dry_run(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "groups.view")?;
    let since = now_ns().saturating_sub(DAY_NS);
    let st = ctx.view_state().await;
    Ok(Json(
        ctx.q(move |e| views::rules_dry_run(e, &st, since)).await?,
    ))
}

async fn list_groups(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "groups.view")?;
    Ok(Json(ctx.q(views::groups).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroupDto {
    currency: String,
    leverage: u32,
    margin_mode: String,
    margin_call_pct: f64,
    stop_out_pct: f64,
    #[serde(default)]
    markup_points: i64,
    book: String,
    #[serde(default)]
    symbols: Option<Vec<String>>,
    /// "retail" | "professional" | null (no ESMA leverage cap).
    #[serde(default)]
    esma: Option<String>,
    /// "cancel" | "retry" | "all_or_none" (what happens to an unfilled remainder).
    #[serde(default)]
    partial_fill: Option<String>,
    #[serde(default)]
    max_attempts: Option<u32>,
    #[serde(default)]
    markup_bid_points: Option<i64>,
    #[serde(default)]
    markup_ask_points: Option<i64>,
    /// symbol -> points (both sides), overrides the group markups.
    #[serde(default)]
    symbol_markups: Option<BTreeMap<String, i64>>,
    #[serde(default)]
    max_slippage_points: Option<i64>,
    #[serde(default)]
    pass_price_improvement: Option<bool>,
    /// "symbol" (use the symbol's per-lot commission) | "per_lot" | "per_million"; value in minor units.
    #[serde(default)]
    commission_type: Option<String>,
    #[serde(default)]
    commission_value: Option<i64>,
    /// Swap scale, 1.0 = the symbol's swap, 0 = swap-free.
    #[serde(default)]
    swap_multiplier: Option<f64>,
}

async fn group_json(ctx: &AdminCtx, name: String) -> ApiResult {
    ctx.q(move |e| {
        e.group(&name)
            .map(|g| views::group(g, &views::symbol_names(e)))
    })
    .await?
    .map(Json)
    .ok_or_else(|| ApiError::not_found("unknown group"))
}

async fn save_group(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(g): Json<GroupDto>,
) -> ApiResult {
    need(&actor, "groups.edit")?;
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-\\/".contains(c))
    {
        return Err(ApiError::bad("group id: 1-64 letters, digits, _ - / \\"));
    }
    if !(1..=3000).contains(&g.leverage) {
        return Err(ApiError::bad("leverage must be 1..3000"));
    }
    if !(0.0..=1000.0).contains(&g.margin_call_pct) || !(0.0..=1000.0).contains(&g.stop_out_pct) {
        return Err(ApiError::bad("margin levels must be 0..1000"));
    }
    if g.stop_out_pct >= g.margin_call_pct {
        return Err(ApiError::bad(
            "Stop-out level must be below margin call level",
        ));
    }
    if g.markup_points < 0 || g.markup_points > 1000 {
        return Err(ApiError::bad("markupPoints must be 0..1000"));
    }
    let ccy = parse_ccy(&g.currency)?;
    let routing = match g.book.as_str() {
        "A" => Routing::ABook,
        "B" => Routing::BBook,
        _ => return Err(ApiError::bad("book must be A or B")),
    };
    let mode = match g.margin_mode.as_str() {
        "retail_netting" => MarginMode::Netting,
        "retail_hedged" | "exchange" => MarginMode::Hedging,
        _ => return Err(ApiError::bad("invalid marginMode")),
    };
    let partial_fill = match g.partial_fill.as_deref() {
        None | Some("cancel") => PartialFill::CancelRemainder,
        Some("retry") => PartialFill::Retry {
            max_attempts: g.max_attempts.unwrap_or(3).clamp(1, 10),
        },
        Some("all_or_none") => PartialFill::AllOrNone,
        _ => {
            return Err(ApiError::bad(
                "partialFill must be cancel, retry or all_or_none",
            ))
        }
    };
    let esma = match g.esma.as_deref() {
        None | Some("") | Some("none") => None,
        Some("retail") => Some(EsmaPreset::Retail),
        Some("professional") => Some(EsmaPreset::Professional),
        _ => return Err(ApiError::bad("esma must be retail, professional or none")),
    };
    let name = id.clone();
    let (existing, all, in_use) = ctx
        .q(move |e| {
            (
                e.group(&name).cloned(),
                views::symbol_names(e),
                e.accounts().any(|a| a.group == name),
            )
        })
        .await?;
    if let Some(sy) = &g.symbols {
        if let Some(bad) = sy.iter().find(|s| !all.contains(s)) {
            return Err(ApiError::bad(format!("unknown symbol {bad}")));
        }
    }
    let mut cfg = existing
        .clone()
        .unwrap_or_else(|| GroupConfig::retail(&id, ccy, routing));
    if cfg.currency != ccy && in_use {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "in_use",
            "cannot change the currency of a group with accounts",
        ));
    }
    cfg.currency = ccy;
    cfg.leverage = g.leverage;
    cfg.margin_mode = mode;
    cfg.margin_call_pct = g.margin_call_pct.round() as i64;
    cfg.stop_out_pct = g.stop_out_pct.round() as i64;
    cfg.markup_points = g.markup_points;
    cfg.esma = esma;
    cfg.partial_fill = partial_fill;
    if let Some(m) = g.swap_multiplier {
        if !(0.0..=10.0).contains(&m) {
            return Err(ApiError::bad("swapMultiplier must be 0..10"));
        }
        cfg.swap_multiplier_pct = (m * 100.0).round() as u32;
    }
    let pts = |v: Option<i64>, what: &str| -> Result<Option<i64>, ApiError> {
        match v {
            Some(p) if !(0..=1000).contains(&p) => {
                Err(ApiError::bad(format!("{what} must be 0..1000")))
            }
            v => Ok(v),
        }
    };
    cfg.markup_bid_points = pts(g.markup_bid_points, "markupBidPoints")?;
    cfg.markup_ask_points = pts(g.markup_ask_points, "markupAskPoints")?;
    if let Some(sm) = g.symbol_markups {
        for (s, p) in &sm {
            if !all.contains(s) {
                return Err(ApiError::bad(format!(
                    "unknown symbol {s} in symbolMarkups"
                )));
            }
            pts(Some(*p), "symbolMarkups")?;
        }
        cfg.symbol_markup_points = sm;
    }
    cfg.max_slippage_points = pts(g.max_slippage_points, "maxSlippagePoints")?;
    if let Some(p) = g.pass_price_improvement {
        cfg.pass_price_improvement = p;
    }
    let value = g.commission_value.unwrap_or(0);
    if !(0..=10_000_000).contains(&value) {
        return Err(ApiError::bad(
            "commissionValue must be 0..10000000 (minor units)",
        ));
    }
    cfg.commission = match g.commission_type.as_deref() {
        None | Some("symbol") | Some("") => None,
        Some("per_lot") => Some(GroupCommission::PerLot { minor: value }),
        Some("per_million") | Some("percent") => Some(GroupCommission::PerMillion {
            minor: if g.commission_type.as_deref() == Some("percent") {
                value * 10_000
            } else {
                value
            },
        }),
        _ => {
            return Err(ApiError::bad(
                "commissionType must be symbol, per_lot or per_million",
            ))
        }
    };
    cfg.routing = routing;
    cfg.allowed_symbols = match g.symbols {
        Some(s) if s.len() != all.len() => Some(s.into_iter().collect()),
        Some(_) | None => None,
    };
    let details = format!(
        "leverage 1:{}, MC {}%, SO {}%, book {}, ESMA {}{}",
        cfg.leverage,
        cfg.margin_call_pct,
        cfg.stop_out_pct,
        g.book,
        match cfg.esma {
            Some(EsmaPreset::Retail) => "retail",
            Some(EsmaPreset::Professional) => "professional",
            None => "off",
        },
        if existing.is_none() { " (new)" } else { "" }
    );
    let mut store = ctx.store.lock().await;
    ctx.cmd(Command::SetGroup(cfg)).await?;
    store.append(
        &actor,
        AdminCmd::GroupSaved {
            group: id.clone(),
            details,
        },
    )?;
    drop(store);
    ctx.notify(&[
        "listGroups",
        "listAudit",
        "listClients",
        "getClient",
        "marginCalls",
    ]);
    group_json(&ctx, id).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresetReq {
    preset_id: String,
}

async fn apply_preset(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(req): Json<PresetReq>,
) -> ApiResult {
    need(&actor, "risk.edit")?;
    let (_, label, lev) = views::PRESETS
        .iter()
        .find(|p| p.0 == req.preset_id)
        .ok_or_else(|| ApiError::not_found("unknown preset"))?;
    let name = id.clone();
    let mut cfg = ctx
        .q(move |e| e.group(&name).cloned())
        .await?
        .ok_or_else(|| ApiError::not_found("unknown group"))?;
    cfg.leverage = *lev;
    cfg.margin_call_pct = 100;
    cfg.stop_out_pct = 50;
    cfg.negative_balance_protection = true;
    cfg.esma = Some(EsmaPreset::Retail);
    let mut store = ctx.store.lock().await;
    ctx.cmd(Command::SetGroup(cfg)).await?;
    store.append(
        &actor,
        AdminCmd::PresetApplied {
            group: id.clone(),
            preset: label.to_string(),
        },
    )?;
    drop(store);
    ctx.notify(&["listGroups", "listAudit", "marginCalls"]);
    group_json(&ctx, id).await
}

async fn list_symbols(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "symbols.view")?;
    Ok(Json(ctx.q(views::symbols).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolDto {
    category: String,
    digits: u32,
    contract_size: i64,
    min_lot: f64,
    max_lot: f64,
    lot_step: f64,
    #[serde(default)]
    swap_long: f64,
    #[serde(default)]
    swap_short: f64,
    /// "money" | "points"
    #[serde(default)]
    swap_type: Option<String>,
    /// "sun".. "sat"
    #[serde(default)]
    triple_swap_day: Option<String>,
}

async fn save_symbol(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(name): Path<String>,
    Json(s): Json<SymbolDto>,
) -> ApiResult {
    need(&actor, "symbols.edit")?;
    if name.is_empty()
        || name.len() > 32
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err(ApiError::bad("invalid symbol name"));
    }
    if s.digits > 8 || s.contract_size <= 0 {
        return Err(ApiError::bad("digits must be 0..8, contractSize > 0"));
    }
    if !(s.min_lot > 0.0 && s.lot_step > 0.0 && s.max_lot > 0.0) {
        return Err(ApiError::bad("lots must be positive"));
    }
    if s.min_lot > s.max_lot {
        return Err(ApiError::bad("minLot must be <= maxLot"));
    }
    let n = name.clone();
    let existing = ctx.q(move |e| e.symbol_spec(&n).cloned()).await?;
    let mut spec = match existing.clone() {
        Some(sp) => sp,
        None => {
            let (b, q) = if name.len() == 6 {
                (name[..3].parse().ok(), name[3..].parse().ok())
            } else {
                (None, None)
            };
            let (b, q): (Currency, Currency) =
                (b.unwrap_or(Currency::USD), q.unwrap_or(Currency::USD));
            SymbolSpec::fx(&name, b, q, s.digits)
        }
    };
    let class = match s.category.as_str() {
        "fx" if matches!(spec.asset_class, AssetClass::MajorFx | AssetClass::MinorFx) => {
            spec.asset_class
        }
        "fx" => AssetClass::MajorFx,
        "metal" => AssetClass::Gold,
        "index"
            if matches!(
                spec.asset_class,
                AssetClass::MajorIndex | AssetClass::MinorIndex | AssetClass::Equity
            ) =>
        {
            spec.asset_class
        }
        "index" => AssetClass::MajorIndex,
        "energy" => AssetClass::Commodity,
        "crypto" => AssetClass::Crypto,
        _ => return Err(ApiError::bad("invalid category")),
    };
    let before = existing
        .as_ref()
        .map(|e| (views::price_f(e.swap_long), views::price_f(e.swap_short)));
    spec.asset_class = class;
    spec.digits = s.digits;
    spec.contract_size = s.contract_size;
    spec.min_lot = Qty::from_raw(views::fixed(s.min_lot));
    spec.max_lot = Qty::from_raw(views::fixed(s.max_lot));
    spec.lot_step = Qty::from_raw(views::fixed(s.lot_step));
    spec.swap_long = Price::from_raw(views::fixed(s.swap_long));
    spec.swap_short = Price::from_raw(views::fixed(s.swap_short));
    spec.swap_mode = match s.swap_type.as_deref() {
        None | Some("money") => risk::SwapMode::Money,
        Some("points") => risk::SwapMode::Points,
        _ => return Err(ApiError::bad("swapType must be money or points")),
    };
    if let Some(d) = s.triple_swap_day.as_deref() {
        spec.triple_swap_day =
            views::weekday_index(d).ok_or_else(|| ApiError::bad("invalid tripleSwapDay"))?;
    }
    let details = match before {
        Some((l, sh)) => format!(
            "swapLong {l} → {}, swapShort {sh} → {}, digits {}",
            s.swap_long, s.swap_short, s.digits
        ),
        None => format!(
            "new symbol, digits {}, contract {}",
            s.digits, s.contract_size
        ),
    };
    let mut store = ctx.store.lock().await;
    ctx.cmd(Command::AddSymbol(spec.clone())).await?;
    store.append(
        &actor,
        AdminCmd::SymbolSaved {
            symbol: name.clone(),
            details,
        },
    )?;
    drop(store);
    ctx.notify(&["listSymbols", "listGroups", "listAudit"]);
    Ok(Json(views::symbol(&spec)))
}

// ---------------------------------------------------------------------------
// positions / orders / risk
// ---------------------------------------------------------------------------

async fn list_positions(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "positions.view")?;
    Ok(Json(ctx.q(views::positions).await?))
}

async fn list_orders(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "positions.view")?;
    Ok(Json(ctx.q(views::orders).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForceCloseReq {
    position_ids: Vec<String>,
}

async fn force_close(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(req): Json<ForceCloseReq>,
) -> ApiResult {
    need(&actor, "positions.forceClose")?;
    if req.position_ids.is_empty() || req.position_ids.len() > 1000 {
        return Err(ApiError::bad("positionIds: 1..1000 ids"));
    }
    let ids = req
        .position_ids
        .iter()
        .map(|s| parse_id(s))
        .collect::<Result<Vec<u64>, _>>()?;
    let mut store = ctx.store.lock().await;
    let seq = store.next_seq();
    let mut closed = 0u32;
    for &pid in &ids {
        let Some(account) = ctx.q(move |e| e.position(pid).map(|p| p.account)).await? else {
            continue;
        };
        let ev = ctx
            .engine
            .command(Command::ClosePosition {
                account,
                position_id: pid,
                volume: None,
                client_order_id: format!("bo-fc-{seq}-{pid}"),
            })
            .await
            .map_err(ApiError::internal)?;
        let failed = ev.iter().any(|e| {
            matches!(
                e,
                Event::CommandRejected { .. } | Event::OrderRejected { .. }
            )
        });
        if !failed
            && ev.iter().any(|e| {
                matches!(
                    e,
                    Event::PositionClosed { .. } | Event::OrderAccepted { .. }
                )
            })
        {
            closed += 1;
        }
    }
    store.append(
        &actor,
        AdminCmd::ForceClosed {
            positions: ids,
            closed,
        },
    )?;
    drop(store);
    let mut topics = ENGINE_TOPICS.to_vec();
    topics.push("listAudit");
    ctx.notify(&topics);
    Ok(Json(json!({ "closed": closed })))
}

async fn hedge_get(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "risk.view")?;
    Ok(Json(ctx.q(views::hedge_policy).await?))
}

async fn hedge_put(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(policy): Json<risk::HedgePolicy>,
) -> ApiResult {
    need(&actor, "risk.edit")?;
    policy.validate().map_err(ApiError::bad)?;
    let details = format!(
        "{} {:?} symbol={:?} total={:?} account={:?} ratio={}% release={}%",
        if policy.enabled { "on" } else { "off" },
        policy.mode,
        policy.default_symbol_limit.map(|q| q.raw() as f64 / 1e8),
        policy.total_limit.map(|q| q.raw() as f64 / 1e8),
        policy.account_limit.map(|q| q.raw() as f64 / 1e8),
        policy.hedge_ratio_pct,
        policy.release_pct
    );
    let mut store = ctx.store.lock().await;
    ctx.cmd(Command::SetHedge(policy)).await?;
    store.append(&actor, AdminCmd::HedgeSaved { details })?;
    drop(store);
    ctx.notify(&["hedgePolicy", "exposure", "listAudit"]);
    Ok(Json(ctx.q(views::hedge_policy).await?))
}

async fn swap_get(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "settings.view")?;
    Ok(Json(ctx.q(views::swap_config).await?))
}

async fn swap_put(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(c): Json<risk::SwapConfig>,
) -> ApiResult {
    need(&actor, "settings.edit")?;
    c.validate().map_err(ApiError::bad)?;
    let details = format!(
        "{} at {:02}:00 UTC, weekend {}",
        if c.enabled { "on" } else { "off" },
        c.rollover_hour_utc,
        if c.skip_weekend { "skipped" } else { "charged" }
    );
    let mut store = ctx.store.lock().await;
    ctx.cmd(Command::SetSwapConfig(c)).await?;
    store.append(&actor, AdminCmd::SwapConfigSaved { details })?;
    drop(store);
    ctx.notify(&["getSwapConfig", "listAudit"]);
    Ok(Json(ctx.q(views::swap_config).await?))
}

/// Runs today's rollover now (idempotent: a second call the same UTC day is a no-op).
async fn swap_rollover(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "risk.edit")?;
    let mut store = ctx.store.lock().await;
    let events = ctx.cmd(Command::Rollover).await?;
    let (applied, positions, reason) = events
        .iter()
        .find_map(|e| match e {
            Event::Rollover {
                applied,
                positions,
                reason,
            } => Some((*applied, *positions, reason.clone())),
            _ => None,
        })
        .unwrap_or((false, 0, "no result".into()));
    store.append(
        &actor,
        AdminCmd::RolloverRun {
            details: if applied {
                format!("{positions} positions {reason}")
            } else {
                format!("skipped: {reason}")
            },
        },
    )?;
    drop(store);
    ctx.notify(&[
        "getSwapConfig",
        "listPositions",
        "listTrades",
        "statements",
        "revenue",
        "listAudit",
    ]);
    Ok(Json(
        json!({ "applied": applied, "positions": positions, "reason": reason }),
    ))
}

async fn list_alerts(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "dashboard.view")?;
    Ok(Json(ctx.alerts.snapshot()))
}

async fn ack_alert(State(ctx): State<AdminCtx>, actor: Actor, Path(id): Path<String>) -> ApiResult {
    need(&actor, "dashboard.view")?;
    if !ctx.alerts.ack(&id) {
        return Err(ApiError::not_found("unknown or resolved alert"));
    }
    ctx.notify(&["listAlerts"]);
    Ok(Json(ctx.alerts.snapshot()))
}

async fn client_flow(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    let st = ctx.view_state().await;
    Ok(Json(ctx.q(move |e| views::client_flow(e, &st)).await?))
}

async fn margin_calls(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "risk.view")?;
    let st = ctx.view_state().await;
    Ok(Json(ctx.q(move |e| views::margin_calls(e, &st)).await?))
}

async fn presets(actor: Actor) -> ApiResult {
    need(&actor, "risk.view")?;
    Ok(Json(views::presets()))
}

async fn lp_sessions(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "lp.view")?;
    let rows = ctx
        .lp_status
        .as_ref()
        .and_then(|s| s.read().ok().map(|t| t.clone()))
        .unwrap_or_default();
    Ok(Json(views::lp_sessions(&rows)))
}

fn lp_admin(ctx: &AdminCtx) -> Result<&super::LpAdmin, ApiError> {
    ctx.lp_admin.as_ref().ok_or_else(|| {
        ApiError::not_found("LP config is not managed here (CORE_LP_ADMIN_URL unset)")
    })
}

/// Forwards to the gateway; maps its JSON error to an API error.
async fn gateway_call(ctx: &AdminCtx, put_body: Option<&Value>) -> Result<Value, ApiError> {
    let a = lp_admin(ctx)?;
    let url = format!("{}/config", a.url.trim_end_matches('/'));
    let req = match put_body {
        Some(b) => ctx.http.put(&url).json(b),
        None => ctx.http.get(&url),
    };
    let res = req.bearer_auth(&a.token).send().await.map_err(|e| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "lp_unreachable",
            format!("fix-gateway: {e}"),
        )
    })?;
    let status = res.status();
    let v: Value = res.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(v);
    }
    let msg = v["error"]
        .as_str()
        .unwrap_or("fix-gateway error")
        .to_string();
    Err(if status == StatusCode::BAD_REQUEST {
        ApiError::bad(msg)
    } else {
        ApiError::new(StatusCode::BAD_GATEWAY, "lp_error", msg)
    })
}

async fn lp_config_get(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "lp.view")?;
    Ok(Json(gateway_call(&ctx, None).await?))
}

/// Audit text without secrets: endpoints, CompIDs and whether passwords changed.
fn lp_config_details(v: &Value) -> String {
    let ep = |k: &str| {
        let e = &v[k];
        let pw = e["password"].as_str().is_some_and(|p| !p.is_empty());
        format!(
            "{k} {} {}->{}{}{}",
            e["addr"].as_str().unwrap_or("?"),
            e["sender_comp_id"].as_str().unwrap_or("?"),
            e["target_comp_id"].as_str().unwrap_or("?"),
            if e["tls"].is_object() { " tls" } else { "" },
            if pw { " (password changed)" } else { "" }
        )
    };
    format!(
        "{}; {}; {} instruments",
        ep("md"),
        ep("trade"),
        v["instruments"].as_array().map_or(0, Vec::len)
    )
}

async fn lp_config_put(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(body): Json<Value>,
) -> ApiResult {
    need(&actor, "lp.manage")?;
    let details = lp_config_details(&body);
    let saved = gateway_call(&ctx, Some(&body)).await?;
    ctx.store
        .lock()
        .await
        .append(&actor, AdminCmd::LpConfigSaved { details })?;
    ctx.notify(&["getLpConfig", "listFixSessions", "listAudit"]);
    Ok(Json(saved))
}

async fn lp_reconnect(actor: Actor, Path(_id): Path<String>) -> ApiResult {
    need(&actor, "lp.reconnect")?;
    Err(ApiError::not_found("unknown LP session"))
}

fn aggregator(ctx: &AdminCtx) -> Result<&std::sync::Arc<crate::lp_agg::Aggregator>, ApiError> {
    ctx.agg
        .as_ref()
        .ok_or_else(|| ApiError::not_found("no LP aggregation here (no in-process LP stack)"))
}

fn lp_sessions_snapshot(ctx: &AdminCtx) -> Vec<fix_gateway::SessionStatus> {
    ctx.lp_status
        .as_ref()
        .and_then(|s| s.read().ok().map(|t| t.clone()))
        .unwrap_or_default()
}

async fn lp_agg_get(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "lp.view")?;
    let agg = aggregator(&ctx)?;
    Ok(Json(views::lp_aggregation(
        agg,
        &lp_sessions_snapshot(&ctx),
    )))
}

async fn lp_agg_put(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(cfg): Json<crate::lp_agg::AggConfig>,
) -> ApiResult {
    need(&actor, "lp.manage")?;
    cfg.validate().map_err(ApiError::bad)?;
    let agg = aggregator(&ctx)?;
    agg.set_config(cfg.clone());
    ctx.store
        .lock()
        .await
        .append(&actor, AdminCmd::AggregationSaved { cfg })?;
    ctx.notify(&["getLpAggregation", "listAudit"]);
    Ok(Json(views::lp_aggregation(
        agg,
        &lp_sessions_snapshot(&ctx),
    )))
}

async fn lp_report(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    Ok(Json(ctx.q(views::lp_report).await?))
}

// ---------------------------------------------------------------------------
// reports / audit
// ---------------------------------------------------------------------------

async fn trades(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    Ok(Json(ctx.q(views::trades).await?))
}

async fn lp_executions(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    Ok(Json(ctx.q(views::lp_executions).await?))
}

async fn execution(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    let st = ctx.view_state().await;
    Ok(Json(ctx.q(move |e| views::execution(e, &st)).await?))
}

async fn revenue(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    let since = now_ns().saturating_sub(DAY_NS);
    Ok(Json(ctx.q(move |e| views::revenue(e, since)).await?))
}

async fn statements(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    let mut flows: BTreeMap<u64, (i64, i64)> = BTreeMap::new();
    for o in ctx.store.lock().await.state.ops.values() {
        if o.status != OpStatus::Applied {
            continue;
        }
        let f = flows.entry(o.account).or_default();
        match o.kind {
            BalanceKind::Deposit => f.0 += o.amount,
            BalanceKind::Withdraw => f.1 += o.amount,
            BalanceKind::Credit => {}
        }
    }
    Ok(Json(
        ctx.q(move |e| {
            Value::Array(
                e.accounts()
                    .filter_map(|a| {
                        let g = e.group(&a.group)?;
                        let closing = e.balance(a.id).ok()?.minor as i64;
                        let (dep, wd) = flows.get(&a.id).copied().unwrap_or_default();
                        let (mut commission, mut swap) = (0i128, 0i128);
                        for d in e.deals().iter().filter(|d| d.account == a.id) {
                            commission += d.commission.minor;
                            swap += d.swap;
                        }
                        for p in e.positions_of(a.id) {
                            swap += p.swap_minor;
                        }
                        let (commission, swap) = (commission as i64, swap as i64);
                        Some(json!({
                            "login": a.id,
                            "name": format!("Account {}", a.id),
                            "currency": g.currency.as_str(),
                            "opening": 0,
                            "deposits": dep,
                            "withdrawals": wd,
                            // trading result net of commission and swap
                            "pnl": closing - dep + wd - commission - swap,
                            "commission": commission,
                            "swap": swap,
                            "closing": closing,
                        }))
                    })
                    .collect(),
            )
        })
        .await?,
    ))
}

async fn audit(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "audit.view")?;
    let st = &ctx.store.lock().await.state;
    Ok(Json(Value::Array(
        st.audit
            .iter()
            .rev()
            .map(|a| {
                json!({ "id": a.id, "at": views::iso(a.at), "actor": a.actor, "role": a.role,
                        "action": a.action, "target": a.target, "details": a.details })
            })
            .collect(),
    )))
}

// ---------------------------------------------------------------------------
// users / settings
// ---------------------------------------------------------------------------

async fn list_users(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "users.view")?;
    let st = &ctx.store.lock().await.state;
    Ok(Json(json!(st.users.values().collect::<Vec<_>>())))
}

async fn save_user(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(u): Json<AdminUserRec>,
) -> ApiResult {
    need(&actor, "users.edit")?;
    if u.id != id || id.is_empty() {
        return Err(ApiError::bad("id must match the path"));
    }
    if u.name.trim().chars().count() < 2 || !u.email.contains('@') {
        return Err(ApiError::bad("name (min 2) and a valid email are required"));
    }
    let mut store = ctx.store.lock().await;
    store.append(&actor, AdminCmd::UserSaved { user: u.clone() })?;
    drop(store);
    ctx.notify(&["listUsers", "listAudit"]);
    Ok(Json(json!(u)))
}

async fn get_settings(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "settings.view")?;
    Ok(Json(json!(ctx.store.lock().await.state.settings)))
}

async fn save_settings(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(s): Json<SettingsRec>,
) -> ApiResult {
    need(&actor, "settings.edit")?;
    if s.broker_name.trim().chars().count() < 2
        || s.four_eyes_threshold <= 0
        || !(5..=480).contains(&s.session_timeout_min)
        || !["A", "B"].contains(&s.default_book.as_str())
    {
        return Err(ApiError::bad("invalid settings"));
    }
    parse_ccy(&s.base_currency)?;
    ctx.store.lock().await.append(
        &actor,
        AdminCmd::SettingsSaved {
            settings: s.clone(),
        },
    )?;
    ctx.notify(&["getSettings", "listAudit"]);
    Ok(Json(json!(s)))
}

pub(super) const LIVE_ENGINE_TOPICS: [&str; 8] = ENGINE_TOPICS;
