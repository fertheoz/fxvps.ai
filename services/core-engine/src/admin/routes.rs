//! `/v1/*` handlers. Every handler re-checks RBAC; every mutation is
//! journaled (engine command and/or admin record with the actor).

use super::auth::{self, Actor, Claims, Role};
use super::store::{
    AdminCmd, AdminState, AdminUserRec, AlertSettings, BalanceKind, BalanceOp, FundingKind,
    FundingMethod, FundingRequest, FundingStatus, KycDoc, OpStatus, SettingsRec,
};
use super::{need, views, AdminCtx, ApiError, ApiResult, ClientActor};
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
        .route("/v1/accounts/{id}/profile", patch(set_profile))
        .route("/v1/accounts/{id}/kyc/documents", get(kyc_docs))
        .route("/v1/accounts/{id}/kyc/documents/{doc}", get(kyc_doc_file))
        .route("/v1/accounts/{id}/ib", patch(set_ib))
        .route("/v1/funding", get(funding_list))
        .route("/v1/funding/{id}/decide", post(funding_decide))
        .route("/v1/reports/ib", get(ib_report))
        .route("/v1/copy", get(super::copy_admin::list))
        .route(
            "/v1/copy/strategies/{account}",
            put(super::copy_admin::save_strategy),
        )
        .route("/v1/copy/subscribe", post(super::copy_admin::subscribe))
        .route("/v1/copy/unsubscribe", post(super::copy_admin::unsubscribe))
        .route("/v1/copy/settle", post(super::copy_admin::settle))
        .route("/v1/client/me", get(client_me))
        .route("/v1/client/sentiment", get(client_sentiment))
        .route("/v1/client/brand", get(client_brand))
        .route("/v1/client/statement", get(client_statement))
        .route("/v1/client/copy", get(super::copy_admin::client_list))
        .route(
            "/v1/client/copy/subscribe",
            post(super::copy_admin::client_subscribe),
        )
        .route(
            "/v1/client/copy/unsubscribe",
            post(super::copy_admin::client_unsubscribe),
        )
        .route("/v1/client/ib", get(client_ib))
        .route("/v1/client/ib/link", post(client_ib_link))
        .route("/v1/client/calendar", get(super::econ_admin::client_list))
        .route("/v1/client/funding", post(client_funding_request))
        .route("/v1/client/kyc/documents", post(client_kyc_upload))
        .route("/v1/reports/transactions", get(transactions))
        .route("/v1/reports/best-execution", get(best_execution))
        .route("/v1/audit/chain", get(audit_chain))
        .route("/v1/approvals", get(list_approvals))
        .route("/v1/approvals/{id}/approve", post(approve))
        .route("/v1/approvals/{id}/reject", post(reject))
        .route("/v1/groups", get(list_groups))
        .route("/v1/rules", get(list_rules).put(save_rules))
        .route("/v1/rules/dry-run", get(rules_dry_run))
        .route("/v1/rules/versions", get(rule_versions))
        .route(
            "/v1/rules/versions/{id}/restore",
            post(rule_version_restore),
        )
        .route(
            "/v1/settings/alerts",
            get(alert_settings_get).put(alert_settings_put),
        )
        .route("/v1/settings/calendar", get(calendar_get).put(calendar_put))
        .route(
            "/v1/econ-calendar",
            get(super::econ_admin::list).post(super::econ_admin::create),
        )
        .route("/v1/econ-calendar/import", post(super::econ_admin::import))
        .route(
            "/v1/econ-calendar/{id}",
            put(super::econ_admin::update).delete(super::econ_admin::remove),
        )
        .route(
            "/v1/settings/statement-email",
            get(super::statement::mail_status),
        )
        .route(
            "/v1/settings/statement-email/test",
            post(super::statement::mail_test),
        )
        .route("/v1/lp/sim/state", get(sim_state))
        .route("/v1/lp/sim/shock", post(sim_shock))
        .route("/v1/lp/sim/scenario", post(sim_scenario))
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
        .route("/v1/perf", get(perf))
        .route("/v1/tenants", get(list_tenants).put(save_tenants))
        .route("/v1/alerts/{id}/ack", post(ack_alert))
        .route("/v1/reports/clients", get(client_flow))
        .route("/v1/lp/sessions", get(lp_sessions))
        .route("/v1/lp/config", get(lp_config_get).put(lp_config_put))
        .route(
            "/v1/bridge/institutions",
            get(super::bridge_admin::list).post(super::bridge_admin::create),
        )
        .route(
            "/v1/bridge/institutions/{id}",
            put(super::bridge_admin::update).delete(super::bridge_admin::remove),
        )
        .route(
            "/v1/bridge/institutions/{id}/rotate-key",
            post(super::bridge_admin::rotate),
        )
        .route(
            "/v1/denetim",
            get(super::bridge_admin::audit_status).put(super::bridge_admin::audit_settings),
        )
        .route("/v1/denetim/reset", post(super::bridge_admin::audit_reset))
        .route(
            "/v1/partner/overview",
            get(super::bridge_admin::partner_overview),
        )
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
    pub(super) async fn q<T: Send + 'static>(
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

    /// Heavy, read-only report query: runs on the read replica after a
    /// refresh (never blocks trading); falls back to the writer thread.
    pub(super) async fn qr<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Engine) -> T + Send + 'static,
    ) -> Result<T, ApiError> {
        let Some(rep) = self.replica.clone() else {
            return self.q(f).await;
        };
        let out = tokio::task::spawn_blocking(move || {
            let mut r = rep.blocking_lock();
            if let Err(e) = r.refresh() {
                tracing::warn!(error = %e, "replica refresh failed");
            }
            f(r.engine())
        })
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(out)
    }

    pub(super) async fn cmd(&self, c: Command) -> Result<Vec<Event>, ApiError> {
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
        // Everything the read views and the alerter use; not the users, the
        // idempotency index or the audit trail. Never serialized whole.
        AdminState {
            credit: st.credit.clone(),
            kyc: st.kyc.clone(),
            profiles: st.profiles.clone(),
            ops: st.ops.clone(),
            settings: st.settings.clone(),
            funding: st.funding.clone(),
            kyc_docs: st.kyc_docs.clone(),
            ib_share: st.ib_share.clone(),
            ib_of: st.ib_of.clone(),
            ib_plan: st.ib_plan.clone(),
            tenants: st.tenants.clone(),
            strategies: st.strategies.clone(),
            // the alerter reads its channels and thresholds through this view
            alerts: st.alerts.clone(),
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

pub(super) fn now_ns() -> u64 {
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
        ctx.qr(move |e| views::dashboard_series(e, &st, now, &range))
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
    let __allowed = tenant_logins(&ctx, &actor).await?;
    let st = ctx.view_state().await;
    Ok(Json(tenant_filter(
        ctx.q(move |e| views::clients(e, &st, q.search.as_deref()))
            .await?,
        &__allowed,
    )))
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
            profile: super::store::ClientProfile {
                name,
                email,
                lei: None,
            },
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
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
struct ProfileReq {
    #[serde(default)]
    lei: Option<String>,
}

/// LEI must be 20 alphanumerics (ISO 17442) or empty.
async fn set_profile(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(req): Json<ProfileReq>,
) -> ApiResult {
    need(&actor, "clients.edit")?;
    let account = parse_id(&id)?;
    let lei = req
        .lei
        .map(|l| l.trim().to_uppercase())
        .filter(|l| !l.is_empty());
    if let Some(l) = &lei {
        if l.len() != 20 || !l.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(ApiError::bad("LEI must be 20 alphanumeric characters"));
        }
    }
    if !ctx.q(move |e| e.account(account).is_some()).await? {
        return Err(ApiError::not_found("unknown account"));
    }
    ctx.store
        .lock()
        .await
        .append(&actor, AdminCmd::ProfileUpdated { account, lei })?;
    ctx.notify(&["listClients", "getClient", "listAudit"]);
    let st = ctx.view_state().await;
    ctx.q(move |e| views::client(e, account, &st))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("unknown account"))
}

#[derive(Deserialize)]
struct ReportRange {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
}

fn parse_iso_ns(s: &str) -> Option<u64> {
    // YYYY-MM-DD or full RFC 3339 (date part only is used for day bounds)
    let d = s.get(0..10)?;
    let mut it = d.split('-');
    let (y, m, day): (i64, i64, i64) = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days).ok().map(|d| d * 86_400_000_000_000)
}

/// MiFIR-style transaction report rows (`?from=YYYY-MM-DD&to=YYYY-MM-DD`, to exclusive; default last 30 days).
async fn transactions(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Query(q): Query<ReportRange>,
) -> ApiResult {
    need(&actor, "reports.view")?;
    let __allowed = tenant_logins(&ctx, &actor).await?;
    let now = now_ns();
    let from = q
        .from
        .as_deref()
        .and_then(parse_iso_ns)
        .unwrap_or(now.saturating_sub(30 * DAY_NS));
    let to =
        q.to.as_deref()
            .and_then(parse_iso_ns)
            .map(|t| t + DAY_NS)
            .unwrap_or(u64::MAX);
    let st = ctx.view_state().await;
    Ok(Json(tenant_filter(
        ctx.qr(move |e| views::transactions(e, &st, from, to))
            .await?,
        &__allowed,
    )))
}

/// Best-execution summary per venue and asset class (`?from=&to=`, default last 30 days).
async fn best_execution(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Query(q): Query<ReportRange>,
) -> ApiResult {
    need(&actor, "reports.view")?;
    let now = now_ns();
    let from = q
        .from
        .as_deref()
        .and_then(parse_iso_ns)
        .unwrap_or(now.saturating_sub(30 * DAY_NS));
    let to =
        q.to.as_deref()
            .and_then(parse_iso_ns)
            .map(|t| t + DAY_NS)
            .unwrap_or(u64::MAX);
    Ok(Json(
        ctx.qr(move |e| views::best_execution(e, from, to)).await?,
    ))
}

/// Audit hash-chain status: recomputed over every record.
async fn audit_chain(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "audit.view")?;
    let st = &ctx.store.lock().await.state;
    let (verified, head, broken) = match st.verify_chain() {
        Ok(h) => (true, h, None),
        Err(id) => (false, String::new(), Some(id)),
    };
    let chained = st.audit.iter().filter(|a| !a.hash.is_empty()).count();
    Ok(Json(json!({
        "count": st.audit.len(),
        "chained": chained,
        "verified": verified,
        "headHash": head,
        "brokenAt": broken,
        "lastAt": st.audit.last().map(|a| views::iso(a.at)),
    })))
}

// ---------------------------------------------------------------------------
// stage 12: client self-service (funding, KYC documents), IB
// ---------------------------------------------------------------------------

fn client_as_actor(c: &ClientActor) -> Actor {
    Actor {
        sub: c.sub.clone(),
        name: format!("client:{}", c.name),
        role: Role::Readonly,
        mfa_ok: true,
    }
}

/// Everything the terminal's account dialog needs in one call.
async fn client_me(State(ctx): State<AdminCtx>, client: ClientActor) -> ApiResult {
    let st = ctx.view_state().await;
    let logins: Vec<(String, u64)> = client.logins.clone();
    let accounts = ctx
        .q(move |e| {
            logins
                .iter()
                .filter_map(|(ext, l)| {
                    let v = views::client(e, *l, &st)?;
                    Some(json!({
                        "externalId": ext, "login": l, "name": v["name"], "group": v["group"],
                        "currency": v["currency"], "balance": v["balance"], "equity": v["equity"],
                        "margin": v["margin"], "kyc": v["kyc"],
                    }))
                })
                .collect::<Vec<Value>>()
        })
        .await?;
    let st = ctx.view_state().await;
    let mine = |a: u64| client.owns(a);
    let funding: Vec<Value> = st
        .funding
        .values()
        .filter(|f| mine(f.account))
        .map(|f| json!(f))
        .collect();
    let documents: Vec<Value> = st
        .kyc_docs
        .iter()
        .filter(|d| mine(d.account))
        .map(|d| json!({ "id": d.id, "account": d.account, "kind": d.kind, "filename": d.filename, "size": d.size, "uploadedAt": views::iso(d.uploaded_at) }))
        .collect();
    Ok(Json(json!({
        "subject": client.sub,
        "name": client.name,
        "accounts": accounts,
        "unknownAccounts": client.account_ids.iter().filter(|a| client.login_of(a).is_none()).collect::<Vec<_>>(),
        "funding": funding,
        "documents": documents,
        "instructions": st.settings.funding,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientFundingReq {
    account: String,
    kind: FundingKind,
    method: FundingMethod,
    /// Minor units of the account currency.
    amount: i64,
    #[serde(default)]
    details: String,
}

async fn client_funding_request(
    State(ctx): State<AdminCtx>,
    client: ClientActor,
    Json(req): Json<ClientFundingReq>,
) -> ApiResult {
    client.interactive()?;
    let login = client
        .login_of(&req.account)
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "forbidden", "not your account"))?;
    if req.amount <= 0 {
        return Err(ApiError::bad("amount must be positive (minor units)"));
    }
    let details: String = req.details.trim().chars().take(300).collect();
    if details.is_empty() {
        return Err(ApiError::bad(
            "details are required (tx hash, sender, destination address or IBAN)",
        ));
    }
    let (ccy, balance) = ctx
        .q(move |e| {
            let a = e.account(login)?;
            let g = e.group(&a.group)?;
            Some((
                g.currency.as_str().to_string(),
                e.balance(login).map(|m| m.minor as i64).unwrap_or(0),
            ))
        })
        .await?
        .ok_or_else(|| ApiError::not_found("unknown account"))?;
    let mut store = ctx.store.lock().await;
    let fi = store.state.settings.funding.clone();
    match (req.kind, req.method) {
        (FundingKind::Deposit, FundingMethod::UsdtTrc20) if fi.usdt_trc20_address.is_empty() => {
            return Err(ApiError::bad("crypto deposits are not enabled"))
        }
        (FundingKind::Deposit, FundingMethod::Bank) if fi.bank_details.is_empty() => {
            return Err(ApiError::bad("bank deposits are not enabled"))
        }
        _ => {}
    }
    match req.kind {
        FundingKind::Deposit if fi.min_deposit_minor > 0 && req.amount < fi.min_deposit_minor => {
            return Err(ApiError::bad("below the minimum deposit"))
        }
        FundingKind::Withdraw
            if fi.min_withdraw_minor > 0 && req.amount < fi.min_withdraw_minor =>
        {
            return Err(ApiError::bad("below the minimum withdrawal"))
        }
        FundingKind::Withdraw if req.amount > balance => {
            return Err(ApiError::bad("exceeds the account balance"))
        }
        _ => {}
    }
    let open = store
        .state
        .funding
        .values()
        .filter(|f| f.account == login && f.status == FundingStatus::Requested)
        .count();
    if open >= 5 {
        return Err(ApiError::bad("too many open requests"));
    }
    let fr = FundingRequest {
        id: format!("fr-{}", store.next_seq()),
        account: login,
        kind: req.kind,
        method: req.method,
        amount: req.amount,
        currency: ccy,
        details,
        requested_by: client.name.clone(),
        requested_at: now_ns(),
        status: FundingStatus::Requested,
        decided_by: None,
        decided_at: None,
        note: None,
        op_id: None,
    };
    store.append(
        &client_as_actor(&client),
        AdminCmd::FundingRequested { req: fr.clone() },
    )?;
    drop(store);
    ctx.notify(&["listFunding", "listAudit"]);
    Ok(Json(json!(fr)))
}

#[derive(Deserialize)]
struct UploadQuery {
    account: String,
}

const KYC_KINDS: [&str; 5] = ["id_front", "id_back", "proof_of_address", "selfie", "other"];
const KYC_TYPES: [&str; 4] = ["image/jpeg", "image/png", "image/webp", "application/pdf"];

/// Raw upload: body = file bytes, `X-Filename`, `X-Doc-Kind`, `Content-Type`.
async fn client_kyc_upload(
    State(ctx): State<AdminCtx>,
    client: ClientActor,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult {
    client.interactive()?;
    let login = client
        .login_of(&q.account)
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "forbidden", "not your account"))?;
    let hdr = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let kind = hdr("x-doc-kind").unwrap_or_else(|| "other".into());
    if !KYC_KINDS.contains(&kind.as_str()) {
        return Err(ApiError::bad("invalid document kind"));
    }
    let content_type = hdr("content-type")
        .map(|c| c.split(';').next().unwrap_or("").trim().to_lowercase())
        .unwrap_or_default();
    if !KYC_TYPES.contains(&content_type.as_str()) {
        return Err(ApiError::bad("only JPEG, PNG, WebP or PDF"));
    }
    if body.is_empty() || body.len() > 6 * 1024 * 1024 {
        return Err(ApiError::bad("file must be 1 B .. 6 MB"));
    }
    let filename: String = hdr("x-filename")
        .unwrap_or_else(|| "document".into())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .take(80)
        .collect();
    let filename = if filename.is_empty() {
        "document".to_string()
    } else {
        filename
    };
    let sha = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&body))
    };
    let mut store = ctx.store.lock().await;
    if store
        .state
        .kyc_docs
        .iter()
        .filter(|d| d.account == login)
        .count()
        >= 20
    {
        return Err(ApiError::bad("document limit reached"));
    }
    let id = format!("kd-{}", store.next_seq());
    let dir = ctx.data_dir.join("kyc").join(login.to_string());
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(dir.join(format!("{id}-{filename}")), &body).await?;
    let doc = KycDoc {
        id,
        account: login,
        kind,
        filename,
        content_type,
        size: body.len() as u64,
        sha256: sha,
        uploaded_by: client.name.clone(),
        uploaded_at: now_ns(),
    };
    let actor = client_as_actor(&client);
    store.append(&actor, AdminCmd::KycDocAdded { doc: doc.clone() })?;
    if store.state.kyc_of(login) == "none" {
        store.append(
            &actor,
            AdminCmd::KycSet {
                account: login,
                kyc: "pending".into(),
                note: Some("documents uploaded by the client".into()),
            },
        )?;
    }
    drop(store);
    ctx.notify(&["listClients", "getClient", "listKycDocs", "listAudit"]);
    Ok(Json(
        json!({ "id": doc.id, "kind": doc.kind, "filename": doc.filename, "size": doc.size, "uploadedAt": views::iso(doc.uploaded_at) }),
    ))
}

async fn kyc_docs(State(ctx): State<AdminCtx>, actor: Actor, Path(id): Path<String>) -> ApiResult {
    need(&actor, "clients.view")?;
    let account = parse_id(&id)?;
    let st = &ctx.store.lock().await.state;
    Ok(Json(Value::Array(
        st.kyc_docs
            .iter()
            .filter(|d| d.account == account)
            .map(|d| json!({ "id": d.id, "account": d.account, "kind": d.kind, "filename": d.filename, "contentType": d.content_type, "size": d.size, "sha256": d.sha256, "uploadedBy": d.uploaded_by, "uploadedAt": views::iso(d.uploaded_at) }))
            .collect(),
    )))
}

async fn kyc_doc_file(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path((id, doc)): Path<(String, String)>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    need(&actor, "clients.view")?;
    let account = parse_id(&id)?;
    let d = ctx
        .store
        .lock()
        .await
        .state
        .kyc_docs
        .iter()
        .find(|d| d.account == account && d.id == doc)
        .cloned()
        .ok_or_else(|| ApiError::not_found("unknown document"))?;
    let path = ctx
        .data_dir
        .join("kyc")
        .join(account.to_string())
        .join(format!("{}-{}", d.id, d.filename));
    let bytes = tokio::fs::read(&path).await?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, d.content_type.clone()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("inline; filename=\"{}\"", d.filename),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IbReq {
    /// IB share of this account's clients' commission, 0..50 (this account is an IB).
    #[serde(default)]
    share_pct: Option<u8>,
    /// IB this (client) account belongs to; null clears.
    #[serde(default)]
    ib_account: Option<Option<u64>>,
    /// Rebate per closed lot (minor units).
    #[serde(default)]
    per_lot_cents: Option<i64>,
    /// Override on sub-IBs' clients (percent).
    #[serde(default)]
    override_pct: Option<u8>,
    /// Referral code; empty clears.
    #[serde(default)]
    code: Option<String>,
}

fn valid_ib_code(c: &str) -> bool {
    (3..=20).contains(&c.len()) && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

async fn set_ib(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(req): Json<IbReq>,
) -> ApiResult {
    need(&actor, "clients.edit")?;
    let account = parse_id(&id)?;
    if !ctx.q(move |e| e.account(account).is_some()).await? {
        return Err(ApiError::not_found("unknown account"));
    }
    if let Some(p) = req.share_pct {
        if p > 50 {
            return Err(ApiError::bad("sharePct must be 0..50"));
        }
    }
    if let Some(Some(ib)) = req.ib_account {
        if ib == account || !ctx.q(move |e| e.account(ib).is_some()).await? {
            return Err(ApiError::bad("unknown IB account"));
        }
    }
    if req
        .per_lot_cents
        .is_some_and(|v| !(0..=10_000).contains(&v))
        || req.override_pct.is_some_and(|v| v > 50)
    {
        return Err(ApiError::bad("perLotCents 0..10000, overridePct 0..50"));
    }
    let mut store = ctx.store.lock().await;
    if let Some(p) = req.share_pct {
        store.append(&actor, AdminCmd::IbShareSet { account, pct: p })?;
    }
    if req.per_lot_cents.is_some() || req.override_pct.is_some() || req.code.is_some() {
        let mut plan = store
            .state
            .ib_plan
            .get(&account)
            .cloned()
            .unwrap_or_default();
        if let Some(v) = req.per_lot_cents {
            plan.per_lot_cents = v;
        }
        if let Some(v) = req.override_pct {
            plan.override_pct = v;
        }
        if let Some(code) = req.code.as_deref().map(str::trim) {
            let code = code.to_uppercase();
            if !code.is_empty() {
                if !valid_ib_code(&code) {
                    return Err(ApiError::bad("code: 3-20 letters, digits or '-'"));
                }
                if store
                    .state
                    .ib_plan
                    .iter()
                    .any(|(a, p)| *a != account && p.code == code)
                {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "conflict",
                        "code already used",
                    ));
                }
            }
            plan.code = code;
        }
        store.append(&actor, AdminCmd::IbPlanSet { account, plan })?;
    }
    if let Some(ib) = req.ib_account {
        store.append(&actor, AdminCmd::IbLinked { account, ib })?;
    }
    drop(store);
    ctx.notify(&["listClients", "getClient", "ibReport", "listAudit"]);
    let st = ctx.view_state().await;
    ctx.q(move |e| views::client(e, account, &st))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("unknown account"))
}

#[derive(Deserialize)]
struct FundingQuery {
    #[serde(default)]
    status: Option<String>,
}

async fn funding_list(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Query(q): Query<FundingQuery>,
) -> ApiResult {
    need(&actor, "clients.view")?;
    let __allowed = tenant_logins(&ctx, &actor).await?;
    let st = ctx.view_state().await;
    Ok(Json(tenant_filter(
        views::funding(&st, q.status.as_deref()),
        &__allowed,
    )))
}

#[derive(Deserialize)]
struct DecideReq {
    /// "approve" | "reject" | "paid"
    decision: String,
    #[serde(default)]
    note: Option<String>,
}

/// Staff decision on a client funding request. Approving creates the balance
/// operation (subject to the 4-eyes threshold like any other balance op).
async fn funding_decide(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(req): Json<DecideReq>,
) -> ApiResult {
    let note = req
        .note
        .map(|n| n.trim().chars().take(300).collect::<String>())
        .filter(|n| !n.is_empty());
    let mut store = ctx.store.lock().await;
    let fr = store
        .state
        .funding
        .get(&id)
        .cloned()
        .ok_or_else(|| ApiError::not_found("unknown funding request"))?;
    let (status, op_id) = match req.decision.as_str() {
        "reject" => {
            need(&actor, "clients.edit")?;
            if fr.status != FundingStatus::Requested {
                return Err(ApiError::bad("already decided"));
            }
            (FundingStatus::Rejected, None)
        }
        "paid" => {
            need(&actor, "clients.edit")?;
            if fr.kind != FundingKind::Withdraw || fr.status != FundingStatus::Approved {
                return Err(ApiError::bad(
                    "only an approved withdrawal can be marked paid",
                ));
            }
            (FundingStatus::Paid, None)
        }
        "approve" => {
            if fr.status != FundingStatus::Requested {
                return Err(ApiError::bad("already decided"));
            }
            let kind = match fr.kind {
                FundingKind::Deposit => BalanceKind::Deposit,
                FundingKind::Withdraw => BalanceKind::Withdraw,
            };
            need(&actor, kind.permission())?;
            let account = fr.account;
            let balance = ctx
                .q(move |e| e.balance(account).map(|m| m.minor as i64).unwrap_or(0))
                .await?;
            let mut op = BalanceOp {
                id: format!("op-{}", store.next_seq()),
                account,
                kind,
                amount: fr.amount,
                currency: fr.currency.clone(),
                reason: format!(
                    "client request {} via {:?}: {}",
                    fr.id, fr.method, fr.details
                ),
                idempotency_key: format!("funding:{}", fr.id),
                requested_by: actor.clone(),
                requested_at: now_ns(),
                status: OpStatus::PendingApproval,
                decided_by: None,
                decided_at: None,
                decision_note: None,
                new_balance: balance,
                new_credit: store.state.credit_of(account),
            };
            if fr.amount >= store.state.settings.four_eyes_threshold {
                store.append(&actor, AdminCmd::BalanceQueued { op: op.clone() })?;
            } else {
                let (b, c) = execute(&ctx, &store.state, &op).await?;
                op.status = OpStatus::Applied;
                op.new_balance = b;
                op.new_credit = c;
                store.append(&actor, AdminCmd::BalanceApplied { op: op.clone() })?;
            }
            (FundingStatus::Approved, Some(op.id))
        }
        _ => return Err(ApiError::bad("decision must be approve, reject or paid")),
    };
    store.append(
        &actor,
        AdminCmd::FundingDecided {
            id: id.clone(),
            status,
            note,
            op_id,
        },
    )?;
    let out = store.state.funding.get(&id).cloned();
    drop(store);
    ctx.notify(&[
        "listFunding",
        "listApprovals",
        "listClients",
        "getClient",
        "listAudit",
        "statements",
        "dashboard",
    ]);
    Ok(Json(json!(out)))
}

/// IB report: per IB account, linked clients' volume and commission in
/// `[from, to)` and the IB's share.
async fn ib_report(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Query(q): Query<ReportRange>,
) -> ApiResult {
    need(&actor, "reports.view")?;
    let now = now_ns();
    let from = q
        .from
        .as_deref()
        .and_then(parse_iso_ns)
        .unwrap_or(now.saturating_sub(30 * DAY_NS));
    let to =
        q.to.as_deref()
            .and_then(parse_iso_ns)
            .map(|t| t + DAY_NS)
            .unwrap_or(u64::MAX);
    let st = ctx.view_state().await;
    Ok(Json(
        ctx.qr(move |e| views::ib_report(e, &st, from, to)).await?,
    ))
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
            note: req
                .note
                .map(|n| n.trim().chars().take(500).collect())
                .filter(|n: &String| !n.is_empty()),
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
    let rules_json = serde_json::to_string(&rules).unwrap_or_default();
    ctx.cmd(Command::SetRules(rules)).await?;
    store.append(
        &actor,
        AdminCmd::RulesSaved {
            count: n,
            rules_json,
        },
    )?;
    drop(store);
    ctx.notify(&["listRules", "rulesDryRun", "listAudit"]);
    Ok(Json(ctx.q(views::rules).await?))
}

async fn rule_versions(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "groups.view")?;
    let st = &ctx.store.lock().await.state;
    Ok(Json(Value::Array(
        st.rule_versions
            .iter()
            .rev()
            .map(|v| json!({ "id": v.id, "at": views::iso(v.at), "actor": v.actor, "count": v.count }))
            .collect(),
    )))
}

/// Re-applies a saved rule table (becomes the newest version).
async fn rule_version_restore(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
) -> ApiResult {
    need(&actor, "groups.edit")?;
    let json_rules = ctx
        .store
        .lock()
        .await
        .state
        .rule_versions
        .iter()
        .find(|v| v.id == id)
        .map(|v| v.rules_json.clone())
        .ok_or_else(|| ApiError::not_found("unknown rule version"))?;
    let rules: Vec<RoutingRule> = serde_json::from_str(&json_rules)
        .map_err(|e| ApiError::bad(format!("stored rules unreadable: {e}")))?;
    let n = rules.len();
    let mut store = ctx.store.lock().await;
    ctx.cmd(Command::SetRules(rules)).await?;
    store.append(
        &actor,
        AdminCmd::RulesSaved {
            count: n,
            rules_json: json_rules,
        },
    )?;
    drop(store);
    ctx.notify(&["listRules", "rulesDryRun", "ruleVersions", "listAudit"]);
    Ok(Json(ctx.q(views::rules).await?))
}

async fn alert_settings_get(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "settings.view")?;
    let a = ctx.store.lock().await.state.alerts.clone();
    let mut v = json!(a);
    // write-only secret
    v["telegramToken"] = json!("");
    v["telegramTokenSet"] = json!(
        !a.telegram_token.is_empty()
            || std::env::var("CORE_TELEGRAM_BOT_TOKEN").is_ok_and(|t| !t.is_empty())
    );
    Ok(Json(v))
}

async fn alert_settings_put(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(mut a): Json<AlertSettings>,
) -> ApiResult {
    need(&actor, "settings.edit")?;
    if a.fill_rate_floor_pct > 100 || a.latency_multiplier == 0 || a.lp_down_grace_s > 86_400 {
        return Err(ApiError::bad("invalid thresholds"));
    }
    if a.quiet_hours_utc.is_some_and(|(f, t)| f > 23 || t > 24)
        || a.daily_report_hour_utc.is_some_and(|h| h > 23)
    {
        return Err(ApiError::bad("hours must be within 0..24"));
    }
    if !a.webhook_url.is_empty()
        && !a.webhook_url.starts_with("https://")
        && !a.webhook_url.starts_with("http://127.0.0.1")
    {
        return Err(ApiError::bad("webhookUrl must be https"));
    }
    let stored_token = ctx.store.lock().await.state.alerts.telegram_token.clone();
    if a.telegram_token.is_empty() {
        a.telegram_token = stored_token; // keep the stored one
    }
    // Chat id left empty: ask Telegram who talked to the bot (the operator
    // pressed Start) and use that private chat; then confirm with a message.
    // The token never leaves the server.
    let mut detected: Option<String> = None;
    if !a.telegram_token.is_empty() && a.telegram_chat_id.is_empty() {
        let url = format!(
            "https://api.telegram.org/bot{}/getUpdates",
            a.telegram_token
        );
        let found: Option<i64> = match ctx.http.get(&url).send().await {
            Ok(r) => match r.json::<Value>().await {
                Ok(v) => v["result"].as_array().and_then(|arr| {
                    arr.iter().rev().find_map(|u| {
                        u["message"]["chat"]["id"]
                            .as_i64()
                            .or(u["my_chat_member"]["chat"]["id"].as_i64())
                    })
                }),
                Err(e) => {
                    tracing::warn!(error = %e, "telegram getUpdates: bad response");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "telegram getUpdates failed");
                None
            }
        };
        match found {
            Some(id) => {
                a.telegram_chat_id = id.to_string();
                detected = Some(a.telegram_chat_id.clone());
                let send = format!(
                    "https://api.telegram.org/bot{}/sendMessage",
                    a.telegram_token
                );
                let _ = ctx
                    .http
                    .post(&send)
                    .json(&json!({"chat_id": id, "text": "fxvps.ai: Telegram bağlantısı kuruldu. Uyarılar ve günlük rapor bu sohbete gelecek."}))
                    .send()
                    .await;
            }
            None => tracing::warn!("telegram chat id not detected: no message to the bot yet"),
        }
    }
    let mut store = ctx.store.lock().await;
    store.append(&actor, AdminCmd::AlertSettingsSaved { settings: a })?;
    let out = store.state.alerts.clone();
    drop(store);
    ctx.notify(&["getAlertSettings", "listAudit"]);
    let mut v = json!(out);
    v["telegramToken"] = json!("");
    v["telegramTokenSet"] = json!(!out.telegram_token.is_empty());
    v["telegramChatDetected"] = json!(detected);
    Ok(Json(v))
}

async fn calendar_get(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "settings.view")?;
    Ok(Json(ctx.q(|e| json!(e.calendar())).await?))
}

async fn calendar_put(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(c): Json<risk::TradingCalendar>,
) -> ApiResult {
    need(&actor, "settings.edit")?;
    c.validate().map_err(ApiError::bad)?;
    let n = c.holidays.len();
    let mut store = ctx.store.lock().await;
    ctx.cmd(Command::SetCalendar(c)).await?;
    store.append(
        &actor,
        AdminCmd::LpConfigSaved {
            details: format!("trading calendar: {n} holidays"),
        },
    )?;
    drop(store);
    ctx.notify(&["getCalendar", "listAudit"]);
    Ok(Json(ctx.q(|e| json!(e.calendar())).await?))
}

fn sim_url() -> Result<String, ApiError> {
    std::env::var("CORE_LP_SIM_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| v.trim_end_matches('/').to_string())
        .ok_or_else(|| ApiError::not_found("no LP simulator control here (CORE_LP_SIM_URL unset)"))
}

async fn sim_forward(
    ctx: &AdminCtx,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> ApiResult {
    let url = format!("{}{path}", sim_url()?);
    let mut r = ctx.http.request(method, &url);
    if let Some(b) = body {
        r = r.json(&b);
    }
    let resp = r
        .send()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "sim_unreachable", e.to_string()))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "sim_error",
            format!("simulator answered {status}"),
        ));
    }
    Ok(Json(v))
}

async fn sim_state(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "lp.view")?;
    sim_forward(&ctx, reqwest::Method::GET, "/state", None).await
}

async fn sim_shock(State(ctx): State<AdminCtx>, actor: Actor, Json(b): Json<Value>) -> ApiResult {
    need(&actor, "lp.manage")?;
    let out = sim_forward(&ctx, reqwest::Method::POST, "/shock", Some(b.clone())).await?;
    ctx.store.lock().await.append(
        &actor,
        AdminCmd::LpConfigSaved {
            details: format!("simulator shock {b}"),
        },
    )?;
    ctx.notify(&["listAudit"]);
    Ok(out)
}

async fn sim_scenario(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(b): Json<Value>,
) -> ApiResult {
    need(&actor, "lp.manage")?;
    let out = sim_forward(&ctx, reqwest::Method::POST, "/scenario", Some(b.clone())).await?;
    ctx.store.lock().await.append(
        &actor,
        AdminCmd::LpConfigSaved {
            details: format!("simulator scenario {b}"),
        },
    )?;
    ctx.notify(&["listAudit"]);
    Ok(out)
}

async fn rules_dry_run(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "groups.view")?;
    let since = now_ns().saturating_sub(DAY_NS);
    let st = ctx.view_state().await;
    Ok(Json(
        ctx.qr(move |e| views::rules_dry_run(e, &st, since)).await?,
    ))
}

async fn list_groups(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "groups.view")?;
    Ok(Json(ctx.q(views::groups).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeverageTierDto {
    from: i64,
    leverage: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeverageWindowDto {
    from_ms: u64,
    to_ms: u64,
    leverage: u32,
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
    /// Leverage cap Friday 20:00 - Sunday 22:00 UTC (null/0 = none).
    #[serde(default)]
    weekend_leverage: Option<u32>,
    /// News windows: leverage cap between two instants (ms since epoch).
    #[serde(default)]
    leverage_windows: Option<Vec<LeverageWindowDto>>,
    /// Volume tiers: notional (group ccy) above `from` gets at most `leverage`.
    #[serde(default)]
    leverage_tiers: Option<Vec<LeverageTierDto>>,
    /// Swap-free fee per lot per night (account currency) and grace days.
    #[serde(default)]
    swap_free_fee: Option<f64>,
    #[serde(default)]
    swap_free_grace_days: Option<u32>,
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
    if let Some(ts) = g.leverage_tiers {
        if ts.len() > 20
            || ts
                .iter()
                .any(|t| t.from <= 0 || !(1..=1000).contains(&t.leverage))
        {
            return Err(ApiError::bad(
                "leverageTiers: from > 0, leverage 1..1000, at most 20",
            ));
        }
        let mut out: Vec<risk::LeverageTier> = ts
            .into_iter()
            .map(|t| risk::LeverageTier {
                from: t.from,
                leverage: t.leverage,
            })
            .collect();
        out.sort_by_key(|t| t.from);
        cfg.leverage_tiers = out;
    }
    if let Some(f) = g.swap_free_fee {
        if !(0.0..=1000.0).contains(&f) {
            return Err(ApiError::bad("swapFreeFee must be 0..1000"));
        }
        cfg.swap_free_fee_per_lot = (f * 100.0).round() as i64;
    }
    if let Some(d) = g.swap_free_grace_days {
        cfg.swap_free_grace_days = d.min(365);
    }
    if let Some(ws) = g.leverage_windows {
        if ws.len() > 50 {
            return Err(ApiError::bad("at most 50 leverage windows"));
        }
        let mut out = Vec::new();
        for w in ws {
            if w.to_ms <= w.from_ms || !(1..=1000).contains(&w.leverage) {
                return Err(ApiError::bad(
                    "leverageWindows: toMs must be after fromMs, leverage 1..1000",
                ));
            }
            out.push(risk::LeverageWindow {
                from_ns: w.from_ms.saturating_mul(1_000_000),
                to_ns: w.to_ms.saturating_mul(1_000_000),
                leverage: w.leverage,
            });
        }
        out.sort_by_key(|w| w.from_ns);
        cfg.leverage_windows = out;
    }
    cfg.weekend_leverage = match g.weekend_leverage.filter(|v| *v > 0) {
        Some(v) if v > 1000 => return Err(ApiError::bad("weekendLeverage must be 1..=1000")),
        v => v,
    };
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
    /// Weekly trading sessions `{day, open, close}` (HH:MM UTC); empty = always open.
    #[serde(default)]
    trade_sessions: Option<Vec<SessionDto>>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct SessionDto {
    day: String,
    open: String,
    close: String,
}

fn hhmm(s: &str) -> Option<u16> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h <= 24 && m < 60).then_some(h * 60 + m)
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
    if let Some(sessions) = s.trade_sessions {
        let mut out = Vec::new();
        for x in sessions {
            let day =
                views::weekday_index(&x.day).ok_or_else(|| ApiError::bad("invalid session day"))?;
            let (open, close) = (hhmm(&x.open), hhmm(&x.close));
            let (Some(open), Some(close)) = (open, close) else {
                return Err(ApiError::bad("session times must be HH:MM"));
            };
            if close <= open {
                return Err(ApiError::bad("session close must be after open"));
            }
            out.push(risk::TradingSession {
                day,
                open_min: open,
                close_min: close,
            });
        }
        if out.len() > 50 {
            return Err(ApiError::bad("too many sessions"));
        }
        spec.sessions = out;
    }
    if let Some(en) = s.enabled {
        spec.enabled = en;
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
    let __allowed = tenant_logins(&ctx, &actor).await?;
    Ok(Json(tenant_filter(
        ctx.q(views::positions).await?,
        &__allowed,
    )))
}

async fn list_orders(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "positions.view")?;
    let __allowed = tenant_logins(&ctx, &actor).await?;
    Ok(Json(tenant_filter(ctx.q(views::orders).await?, &__allowed)))
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

/// Engine latency budget and replica lag (`/v1/perf`).
async fn perf(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "dashboard.view")?;
    let lat = ctx.engine.latency();
    let writer_seq = ctx.q(|e| e.seq()).await?;
    let (replica_seq, reloads, applied) = match &ctx.replica {
        Some(r) => {
            let g = r.lock().await;
            (Some(g.seq()), g.reloads, g.applied)
        }
        None => (None, 0, 0),
    };
    Ok(Json(json!({
        "engine": lat,
        "writerSeq": writer_seq,
        "replica": replica_seq.map(|s| json!({ "seq": s, "lagCommands": writer_seq.saturating_sub(s), "reloads": reloads, "applied": applied })),
        "budget": { "p99Us": 5_000, "ok": lat.p99_us <= 5_000 },
        // last nightly load test summary (yuk-sinavi.sh writes it)
        "loadtest": std::env::var("CORE_LOADTEST_RESULT_FILE")
            .ok()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .map(|v| v["summary"].clone()),
    })))
}

async fn list_tenants(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "users.view")?;
    let st = &ctx.store.lock().await.state;
    Ok(Json(json!(st.tenants.values().collect::<Vec<_>>())))
}

async fn save_tenants(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(mut ts): Json<Vec<super::store::TenantRec>>,
) -> ApiResult {
    need(&actor, "users.edit")?;
    if ts.len() > 50 {
        return Err(ApiError::bad("at most 50 tenants"));
    }
    // Hostnames key the public branding lookup: store them in the form the
    // terminal compares (`location.hostname`) and give each one one owner.
    let mut owner: BTreeMap<String, String> = BTreeMap::new();
    for t in &mut ts {
        let mut hosts: Vec<String> = Vec::new();
        for h in &t.hostnames {
            let Some(h) = norm_hostname(h).map_err(ApiError::bad)? else {
                continue;
            };
            if let Some(o) = owner.get(&h).filter(|o| **o != t.id) {
                return Err(ApiError::bad(format!(
                    "hostname {h} is already used by tenant {o}"
                )));
            }
            owner.insert(h.clone(), t.id.clone());
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
        if hosts.len() > 20 {
            return Err(ApiError::bad("at most 20 hostnames per tenant"));
        }
        t.hostnames = hosts;
    }
    let mut ids = std::collections::BTreeSet::new();
    for t in &ts {
        if t.id.is_empty()
            || t.id.len() > 32
            || !t
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || !ids.insert(t.id.clone())
        {
            return Err(ApiError::bad(
                "tenant ids must be unique, 1-32 chars [a-z0-9-_]",
            ));
        }
        if t.name.trim().is_empty() || t.name.len() > 64 {
            return Err(ApiError::bad("tenant name 1-64 chars"));
        }
        let hex = |s: &str| {
            s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
        };
        if !t.brand_color.is_empty() && !hex(&t.brand_color) {
            return Err(ApiError::bad("brandColor must be #rrggbb"));
        }
        if !t.logo_url.is_empty() && (!t.logo_url.starts_with("https://") || t.logo_url.len() > 300)
        {
            return Err(ApiError::bad("logoUrl must be an https URL"));
        }
        if !t.support_email.is_empty()
            && (!t.support_email.contains('@') || t.support_email.len() > 120)
        {
            return Err(ApiError::bad("supportEmail is not an e-mail address"));
        }
    }
    let mut store = ctx.store.lock().await;
    store.append(&actor, AdminCmd::TenantsSaved { tenants: ts })?;
    let out = store.state.tenants.values().cloned().collect::<Vec<_>>();
    drop(store);
    ctx.notify(&["listTenants", "listAudit"]);
    Ok(Json(json!(out)))
}

/// A tenant hostname as the browser reports it (`location.hostname`):
/// trimmed and lower-cased; `Ok(None)` for a blank entry. No scheme, port,
/// path or spaces; labels of `[a-z0-9-]`.
fn norm_hostname(raw: &str) -> Result<Option<String>, String> {
    let h = raw.trim().to_ascii_lowercase();
    if h.is_empty() {
        return Ok(None);
    }
    let bad = |why: &str| Err(format!("hostname {:?}: {why}", raw.trim()));
    if h.contains("://") {
        return bad("no scheme (https://), host name only");
    }
    if h.contains(':') {
        return bad("no port, host name only");
    }
    if h.contains('/') || h.contains('?') || h.contains('#') {
        return bad("no path, host name only");
    }
    if h.chars().any(char::is_whitespace) {
        return bad("no spaces");
    }
    if h.len() > 253
        || h.split('.').any(|l| {
            l.is_empty()
                || l.len() > 63
                || l.starts_with('-')
                || l.ends_with('-')
                || !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    {
        return bad("not a valid host name");
    }
    Ok(Some(h))
}

/// Logins the actor may see: `None` = everything (no tenant), else the
/// accounts of the tenant's groups.
pub(super) async fn tenant_logins(
    ctx: &AdminCtx,
    actor: &Actor,
) -> Result<Option<std::collections::BTreeSet<u64>>, ApiError> {
    let groups: Vec<String> = {
        let st = &ctx.store.lock().await.state;
        let user = st.users.values().find(|u| {
            u.id == actor.sub || u.email.eq_ignore_ascii_case(&actor.name) || u.name == actor.name
        });
        let Some(tid) = user
            .and_then(|u| u.tenant.clone())
            .filter(|t| !t.is_empty())
        else {
            return Ok(None);
        };
        st.tenants
            .get(&tid)
            .map(|t| t.groups.clone())
            .unwrap_or_default()
    };
    let set = ctx
        .q(move |e| {
            e.accounts()
                .filter(|a| groups.contains(&a.group))
                .map(|a| a.id)
                .collect::<std::collections::BTreeSet<u64>>()
        })
        .await?;
    Ok(Some(set))
}

/// Keeps only rows whose `login` / `clientLogin` / `account` is in `allowed`
/// (arrays, or objects with a `rows` array).
fn tenant_filter(v: Value, allowed: &Option<std::collections::BTreeSet<u64>>) -> Value {
    let Some(set) = allowed else {
        return v;
    };
    let keep = |row: &Value| {
        ["login", "clientLogin", "account"]
            .iter()
            .filter_map(|k| row[*k].as_u64())
            .next()
            .is_none_or(|l| set.contains(&l))
    };
    match v {
        Value::Array(rows) => Value::Array(rows.into_iter().filter(keep).collect()),
        Value::Object(mut o) => {
            if let Some(Value::Array(rows)) = o.remove("rows") {
                o.insert(
                    "rows".into(),
                    Value::Array(rows.into_iter().filter(keep).collect()),
                );
            }
            Value::Object(o)
        }
        other => other,
    }
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
    let __allowed = tenant_logins(&ctx, &actor).await?;
    let st = ctx.view_state().await;
    Ok(Json(tenant_filter(
        ctx.qr(move |e| views::client_flow(e, &st)).await?,
        &__allowed,
    )))
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
    Ok(Json(ctx.qr(views::lp_report).await?))
}

// ---------------------------------------------------------------------------
// reports / audit
// ---------------------------------------------------------------------------

async fn trades(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    let __allowed = tenant_logins(&ctx, &actor).await?;
    Ok(Json(tenant_filter(
        ctx.qr(views::trades).await?,
        &__allowed,
    )))
}

async fn lp_executions(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    Ok(Json(ctx.qr(views::lp_executions).await?))
}

async fn execution(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    let st = ctx.view_state().await;
    Ok(Json(ctx.qr(move |e| views::execution(e, &st)).await?))
}

async fn revenue(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    let since = now_ns().saturating_sub(DAY_NS);
    Ok(Json(ctx.qr(move |e| views::revenue(e, since)).await?))
}

async fn statements(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "reports.view")?;
    let __allowed = tenant_logins(&ctx, &actor).await?;
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
    Ok(Json(tenant_filter(
        ctx.qr(move |e| {
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
        &__allowed,
    )))
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
                        "action": a.action, "target": a.target, "details": a.details,
                        "hash": a.hash })
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
    if !s.broker_lei.is_empty()
        && (s.broker_lei.len() != 20 || !s.broker_lei.chars().all(|c| c.is_ascii_alphanumeric()))
    {
        return Err(ApiError::bad(
            "brokerLei must be 20 alphanumeric characters or empty",
        ));
    }
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

/// Distinct holders a symbol needs, the caller not counted, before its
/// sentiment is published.
const SENTIMENT_MIN_HOLDERS: usize = 10;
/// A symbol is suppressed while one holder (the caller aside) has more than
/// this share of its lots.
const SENTIMENT_MAX_SHARE_PCT: i128 = 50;

/// Client sentiment per symbol: share of open lots that are long, within the
/// caller's white-label tenant (outside every tenant: the platform's own
/// groups). Copy-trading mirrors and bridge institution accounts are not
/// client books and do not count. Published only where at least
/// `SENTIMENT_MIN_HOLDERS` other holders (one client's logins with the same
/// profile e-mail are one holder) have positions and none of them dominates,
/// so the caller cannot subtract their own lots and read another client's
/// book.
async fn client_sentiment(State(ctx): State<AdminCtx>, client: ClientActor) -> ApiResult {
    let skip = super::bridge_admin::institution_logins(&ctx)?;
    let tenants: Vec<Vec<String>> = {
        let st = &ctx.store.lock().await.state;
        st.tenants.values().map(|t| t.groups.clone()).collect()
    };
    let mine: BTreeSet<u64> = client.logins.iter().map(|(_, l)| *l).collect();
    let me = mine.clone();
    // whole-book scan: on the read replica, never the writer thread
    let book = ctx
        .qr(move |e| {
            let my_groups: BTreeSet<&str> = me
                .iter()
                .filter_map(|l| e.account(*l))
                .map(|a| a.group.as_str())
                .collect();
            let my_tenants: BTreeSet<&str> = tenants
                .iter()
                .filter(|gs| gs.iter().any(|g| my_groups.contains(g.as_str())))
                .flatten()
                .map(String::as_str)
                .collect();
            if my_tenants.is_empty() {
                let tenant_groups: BTreeSet<&str> =
                    tenants.iter().flatten().map(String::as_str).collect();
                sentiment_book(e, |g| !tenant_groups.contains(g), &skip)
            } else {
                sentiment_book(e, |g| my_tenants.contains(g), &skip)
            }
        })
        .await?;
    // holder identity (profile e-mail) of every login involved, copied under a short lock
    let emails: BTreeMap<u64, String> = {
        let st = &ctx.store.lock().await.state;
        book.values()
            .flat_map(|accs| accs.keys())
            .chain(mine.iter())
            .filter_map(|l| {
                let m = st.profiles.get(l)?.email.trim().to_ascii_lowercase();
                (!m.is_empty()).then_some((*l, m))
            })
            .collect()
    };
    let holder_of = |login: u64| match emails.get(&login) {
        Some(m) => Holder::Email(m.clone()),
        None => Holder::Login(login),
    };
    let caller: BTreeSet<Holder> = mine.iter().map(|l| holder_of(*l)).collect();
    Ok(Json(json!(sentiment_rows(book, holder_of, &caller))))
}

/// One client for the sentiment threshold: logins sharing a profile e-mail
/// are the same holder.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Holder {
    Email(String),
    Login(u64),
}

/// Open client lots per symbol and account (raw lots: long, short) in one
/// pass over the positions: no copy-trading mirrors, no `skip` accounts, only
/// accounts whose group passes `in_scope`.
fn sentiment_book(
    e: &Engine,
    in_scope: impl Fn(&str) -> bool,
    skip: &BTreeSet<u64>,
) -> BTreeMap<String, BTreeMap<u64, (i128, i128)>> {
    let mut scoped: BTreeMap<u64, bool> = BTreeMap::new();
    let mut out: BTreeMap<String, BTreeMap<u64, (i128, i128)>> = BTreeMap::new();
    for p in e.positions() {
        if p.copy_from.is_some() || skip.contains(&p.account) {
            continue;
        }
        let ok = *scoped
            .entry(p.account)
            .or_insert_with(|| e.account(p.account).is_some_and(|a| in_scope(&a.group)));
        if !ok {
            continue;
        }
        let r = out
            .entry(p.symbol.clone())
            .or_default()
            .entry(p.account)
            .or_default();
        let v = p.volume.raw() as i128;
        if p.side == oms::Side::Buy {
            r.0 += v;
        } else {
            r.1 += v;
        }
    }
    out
}

/// Published rows: `longPct` over every holder in scope (the caller
/// included), but the threshold and the dominance rule only over the others.
fn sentiment_rows(
    book: BTreeMap<String, BTreeMap<u64, (i128, i128)>>,
    holder_of: impl Fn(u64) -> Holder,
    caller: &BTreeSet<Holder>,
) -> Vec<Value> {
    book.into_iter()
        .filter_map(|(symbol, accs)| {
            let (mut long, mut short) = (0i128, 0i128);
            let mut holders: BTreeSet<Holder> = BTreeSet::new();
            let mut others: BTreeMap<Holder, i128> = BTreeMap::new();
            for (login, (l, s)) in accs {
                long += l;
                short += s;
                let h = holder_of(login);
                if !caller.contains(&h) {
                    *others.entry(h.clone()).or_default() += l + s;
                }
                holders.insert(h);
            }
            let total = long + short;
            let others_total: i128 = others.values().sum();
            let top = others.values().copied().max().unwrap_or(0);
            if others.len() < SENTIMENT_MIN_HOLDERS
                || total <= 0
                || top * 100 > others_total * SENTIMENT_MAX_SHARE_PCT
            {
                return None;
            }
            let long_pct = (long * 200 + total) / (total * 2); // rounded half up
            Some(json!({ "symbol": symbol, "longPct": long_pct as i64, "traders": holders.len() }))
        })
        .collect()
}

#[cfg(test)]
mod sentiment_tests {
    use super::*;

    const LOT: i128 = 100; // raw units do not matter for the shares

    fn book(rows: &[(&str, u64, i128, i128)]) -> BTreeMap<String, BTreeMap<u64, (i128, i128)>> {
        let mut b: BTreeMap<String, BTreeMap<u64, (i128, i128)>> = BTreeMap::new();
        for (sym, login, l, s) in rows {
            b.entry(sym.to_string())
                .or_default()
                .insert(*login, (*l, *s));
        }
        b
    }

    fn by_login(l: u64) -> Holder {
        Holder::Login(l)
    }

    #[test]
    fn needs_ten_other_holders() {
        // nine others + the caller: not enough
        let mut rows: Vec<(&str, u64, i128, i128)> =
            (1..=9).map(|l| ("EURUSD", l, LOT, 0)).collect();
        rows.push(("EURUSD", 100, 0, 5 * LOT));
        let caller = BTreeSet::from([Holder::Login(100)]);
        assert!(sentiment_rows(book(&rows), by_login, &caller).is_empty());
        // a tenth other holder publishes it; the share includes the caller
        rows.push(("EURUSD", 10, LOT, 0));
        let out = sentiment_rows(book(&rows), by_login, &caller);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["symbol"], "EURUSD");
        assert_eq!(out[0]["longPct"], 67); // 10 long of 15 lots
        assert_eq!(out[0]["traders"], 11);
    }

    #[test]
    fn logins_of_one_client_are_one_holder() {
        // ten logins, but 9 and 10 share an e-mail: nine holders
        let rows: Vec<(&str, u64, i128, i128)> = (1..=10).map(|l| ("XAUUSD", l, LOT, 0)).collect();
        let holder = |l: u64| {
            if l >= 9 {
                Holder::Email("same@example.com".into())
            } else {
                Holder::Login(l)
            }
        };
        assert!(sentiment_rows(book(&rows), holder, &BTreeSet::new()).is_empty());
        // the caller's own second login does not count either
        let rows: Vec<(&str, u64, i128, i128)> = (1..=11).map(|l| ("XAUUSD", l, LOT, 0)).collect();
        let caller = BTreeSet::from([Holder::Email("me@example.com".into())]);
        let mine = |l: u64| {
            if l >= 10 {
                Holder::Email("me@example.com".into())
            } else {
                Holder::Login(l)
            }
        };
        assert!(sentiment_rows(book(&rows), mine, &caller).is_empty());
    }

    #[test]
    fn a_dominant_holder_suppresses_the_symbol() {
        let mut rows: Vec<(&str, u64, i128, i128)> =
            (1..=10).map(|l| ("GBPUSD", l, LOT, 0)).collect();
        rows.push(("GBPUSD", 11, 0, 20 * LOT)); // 20 of 30 lots
        assert!(sentiment_rows(book(&rows), by_login, &BTreeSet::new()).is_empty());
        // exactly half is still published
        rows.pop();
        rows.push(("GBPUSD", 11, 0, 10 * LOT));
        let out = sentiment_rows(book(&rows), by_login, &BTreeSet::new());
        assert_eq!(out[0]["longPct"], 50);
        // the caller's own size never counts as dominance
        rows.pop();
        rows.push(("GBPUSD", 11, 0, 1_000 * LOT));
        let caller = BTreeSet::from([Holder::Login(11)]);
        assert_eq!(sentiment_rows(book(&rows), by_login, &caller).len(), 1);
    }

    #[test]
    fn book_skips_copies_institutions_and_other_tenants() {
        use money::{px, qty};
        use oms::{Envelope, NewOrder, NullRouter};
        let mut e = Engine::new(Default::default(), Box::new(NullRouter));
        let mut n = 0u64;
        let mut cmd = |e: &mut Engine, c: Command| {
            n += 1;
            e.apply(&Envelope {
                seq: n,
                ts: 1_000 * n,
                cmd: c,
            })
        };
        cmd(
            &mut e,
            Command::AddSymbol(SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5)),
        );
        for g in ["own", "acme"] {
            let mut gc = GroupConfig::retail(g, Currency::USD, Routing::BBook);
            gc.esma = None;
            gc.leverage = 100;
            cmd(&mut e, Command::SetGroup(gc));
        }
        cmd(
            &mut e,
            Command::Quote {
                symbol: "EURUSD".into(),
                bid: px("1.10000"),
                ask: px("1.10010"),
            },
        );
        for (acc, g) in [(1, "own"), (2, "own"), (3, "own"), (4, "acme")] {
            cmd(
                &mut e,
                Command::OpenAccount {
                    account: acc,
                    group: g.into(),
                },
            );
            cmd(
                &mut e,
                Command::Deposit {
                    account: acc,
                    amount: Money::parse("10000", Currency::USD).unwrap(),
                    key: format!("d{acc}"),
                },
            );
        }
        // 2 copies 1: its mirror is not a client decision
        cmd(
            &mut e,
            Command::CopySubscribe {
                follower: 2,
                provider: 1,
                ratio_bps: 10_000,
                equity_stop_pct: 0,
                perf_fee_bps: 0,
            },
        );
        for acc in [1u64, 3, 4] {
            cmd(
                &mut e,
                Command::PlaceOrder(NewOrder::market(
                    acc,
                    "o",
                    "EURUSD",
                    oms::Side::Buy,
                    qty("1"),
                )),
            );
        }
        assert_eq!(e.positions_of(2).len(), 1, "the copy exists");
        let skip = BTreeSet::from([3u64]); // a bridge institution
        let b = sentiment_book(&e, |g| g == "own", &skip);
        let accs: Vec<u64> = b["EURUSD"].keys().copied().collect();
        assert_eq!(accs, vec![1]);
        let b = sentiment_book(&e, |g| g == "acme", &BTreeSet::new());
        assert_eq!(b["EURUSD"].keys().copied().collect::<Vec<_>>(), vec![4]);
    }
}

// ------------------------------------------------------------- IB (client side)

pub(super) fn month_start(ns: u64) -> (u64, u64) {
    // (start of this month, start of last month), UTC
    let days = (ns / DAY_NS) as i64;
    let (y, m, _) = civil_from_days(days);
    let this = days_from_civil(y, m, 1) as u64 * DAY_NS;
    let (py, pm) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    let last = days_from_civil(py, pm, 1) as u64 * DAY_NS;
    (this, last)
}

pub(super) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub(super) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The signed-in client's IB dashboard: referral link, clients, this and last
/// month's accrual, and payouts (balance ops keyed `ibpay:<login>:...`).
async fn client_ib(State(ctx): State<AdminCtx>, client: ClientActor) -> ApiResult {
    let st = ctx.view_state().await;
    let mine: Vec<u64> = client
        .logins
        .iter()
        .map(|(_, l)| *l)
        .filter(|l| {
            st.ib_share.contains_key(l)
                || st.ib_plan.contains_key(l)
                || st.ib_of.values().any(|i| i == l)
        })
        .collect();
    let now = now_ns();
    let (this_m, last_m) = month_start(now);
    let st2 = st.clone();
    let (cur, prev) = ctx
        .q(move |e| {
            (
                views::ib_report(e, &st2, this_m, u64::MAX),
                views::ib_report(e, &st, last_m, this_m),
            )
        })
        .await?;
    let st = ctx.view_state().await;
    let row = |rep: &Value, ib: u64| {
        rep["rows"]
            .as_array()
            .and_then(|r| r.iter().find(|x| x["ib"] == json!(ib)).cloned())
            .unwrap_or(Value::Null)
    };
    let out: Vec<Value> = mine
        .iter()
        .map(|ib| {
            let plan = st.ib_plan.get(ib).cloned().unwrap_or_default();
            let payouts: Vec<Value> = st
                .ops
                .values()
                .filter(|o| o.account == *ib && o.idempotency_key.starts_with(&format!("ibpay:{ib}:")))
                .map(|o| json!({ "amount": o.amount, "currency": o.currency, "reason": o.reason, "status": o.status, "requestedAt": views::iso(o.requested_at) }))
                .collect();
            json!({
                "ib": ib,
                "code": plan.code,
                "sharePct": st.ib_share.get(ib).copied().unwrap_or(0),
                "perLotCents": plan.per_lot_cents,
                "overridePct": plan.override_pct,
                "clients": st.ib_of.values().filter(|i| *i == ib).count(),
                "thisMonth": row(&cur, *ib),
                "lastMonth": row(&prev, *ib),
                "payouts": payouts,
            })
        })
        .collect();
    Ok(Json(json!({ "ibs": out })))
}

#[derive(Deserialize)]
struct IbLinkReq {
    account: String,
    code: String,
}

/// A client joins an IB with its referral code (once; changes go through support).
async fn client_ib_link(
    State(ctx): State<AdminCtx>,
    client: ClientActor,
    Json(req): Json<IbLinkReq>,
) -> ApiResult {
    client.interactive()?;
    let login = client
        .login_of(&req.account)
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "forbidden", "not your account"))?;
    let code = req.code.trim().to_uppercase();
    let mut store = ctx.store.lock().await;
    if store.state.ib_of.contains_key(&login) {
        return Ok(Json(json!({ "ok": true, "already": true })));
    }
    let ib = store
        .state
        .ib_plan
        .iter()
        .find(|(_, p)| !p.code.is_empty() && p.code == code)
        .map(|(a, _)| *a)
        .ok_or_else(|| ApiError::not_found("unknown referral code"))?;
    if ib == login || client.owns(ib) {
        return Err(ApiError::bad("cannot refer yourself"));
    }
    store.append(
        &client_as_actor(&client),
        AdminCmd::IbLinked {
            account: login,
            ib: Some(ib),
        },
    )?;
    drop(store);
    ctx.notify(&["listClients", "ibReport", "listAudit"]);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct BrandQuery {
    host: String,
}

/// Public white-label lookup for the terminal (no token: the login page is
/// branded too). Unknown hosts get the platform defaults.
async fn client_brand(State(ctx): State<AdminCtx>, Query(q): Query<BrandQuery>) -> ApiResult {
    let host = q.host.trim().to_ascii_lowercase();
    // Anonymous route: read just the brand fields under a short store lock
    // (no `view_state` clone of the whole admin state per request).
    let st = &ctx.store.lock().await.state;
    let t = (!host.is_empty() && host.len() <= 253)
        .then(|| {
            st.tenants
                .values()
                .find(|t| t.hostnames.iter().any(|h| h.eq_ignore_ascii_case(&host)))
        })
        .flatten();
    Ok(Json(match t {
        Some(t) => json!({
            "tenant": t.id, "name": t.name, "brandColor": t.brand_color,
            "logoUrl": t.logo_url, "supportEmail": t.support_email,
        }),
        None => {
            json!({ "tenant": null, "name": st.settings.broker_name, "brandColor": "", "logoUrl": "", "supportEmail": "" })
        }
    }))
}

#[derive(Deserialize)]
struct StatementQuery {
    account: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
}

/// The client's own statement for [from, to] (days, UTC): closed trades with
/// P&L, commission and swap, applied deposits / withdrawals, open positions
/// and the current balance. Default period: the last 30 days.
async fn client_statement(
    State(ctx): State<AdminCtx>,
    client: ClientActor,
    Query(q): Query<StatementQuery>,
) -> ApiResult {
    let login = client
        .login_of(&q.account)
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "forbidden", "not your account"))?;
    let now = now_ns();
    let from = q
        .from
        .as_deref()
        .and_then(parse_iso_ns)
        .unwrap_or(now.saturating_sub(30 * DAY_NS));
    let to =
        q.to.as_deref()
            .and_then(parse_iso_ns)
            .map(|t| t + DAY_NS)
            .unwrap_or(u64::MAX);
    // Only this account's applied cash ops, the name and the broker name,
    // copied under a short lock (no `view_state` clone of the whole state).
    let (ops, profile, broker) = {
        let st = &ctx.store.lock().await.state;
        let ops: Vec<(u64, Value)> = st
            .ops
            .values()
            .filter(|o| {
                o.account == login && o.status == OpStatus::Applied && o.kind != BalanceKind::Credit
            })
            .map(|o| (o.decided_at.unwrap_or(o.requested_at), o))
            .filter(|(t, _)| *t >= from && *t < to)
            .map(|(t, o)| {
                let kind = if o.kind == BalanceKind::Deposit {
                    "deposit"
                } else {
                    "withdraw"
                };
                (t, json!({ "at": views::iso(t), "kind": kind, "amount": o.amount, "reason": o.reason }))
            })
            .collect();
        let profile = st.profiles.get(&login).map(|p| p.name.clone());
        (ops, profile, st.settings.broker_name.clone())
    };
    // Full deal-history scan: on the read replica, never the writer thread.
    let body = ctx
        .qr(move |e| statement_body(e, login, from, to, ops))
        .await?
        .ok_or_else(|| ApiError::not_found("unknown account"))?;
    let mut body = body;
    body["name"] = json!(profile.unwrap_or(client.name));
    body["broker"] = json!(broker);
    Ok(Json(body))
}

/// Statement of `login` for `[from, to)`: closed trades (out deals), totals
/// (commission of every deal in the period, opening side included), cash
/// rows (admin `ops` plus the engine's own balance moves: copy fees, negative
/// balance compensation), open positions and the current balance.
fn statement_body(
    e: &Engine,
    login: u64,
    from: u64,
    to: u64,
    mut cash: Vec<(u64, Value)>,
) -> Option<Value> {
    let a = e.account(login)?;
    let g = e.group(&a.group)?;
    let deals: Vec<&oms::Deal> = e
        .deals()
        .iter()
        .filter(|d| d.account == login && d.ts >= from && d.ts < to)
        .collect();
    let trades: Vec<Value> = deals
        .iter()
        .filter(|d| d.entry == oms::DealEntry::Out)
        .map(|d| {
            json!({
                "at": views::iso(d.ts), "symbol": d.symbol, "side": if d.side == oms::Side::Buy { "buy" } else { "sell" },
                "lots": views::qty_f(d.volume), "price": views::price_f(d.price),
                "pnl": d.pnl.minor as i64, "commission": d.commission.minor as i64, "swap": d.swap as i64,
                "position": d.position_id,
            })
        })
        .collect();
    let sum = |k: &str| {
        trades
            .iter()
            .map(|t| t[k].as_i64().unwrap_or(0))
            .sum::<i64>()
    };
    // Commission is charged per side; the opening side sits on the in deal.
    let commission: i64 = deals.iter().map(|d| d.commission.minor as i64).sum();
    let commission_open: i64 = deals
        .iter()
        .filter(|d| d.entry == oms::DealEntry::In)
        .map(|d| d.commission.minor as i64)
        .sum();
    let totals = json!({
        "trades": trades.len(),
        "lots": trades.iter().map(|t| t["lots"].as_f64().unwrap_or(0.0)).sum::<f64>(),
        "pnl": sum("pnl"), "commission": commission, "commissionOpen": commission_open, "swap": sum("swap"),
    });
    cash.extend(
        e.cash_moves()
            .iter()
            .filter(|m| m.account == login && m.ts >= from && m.ts < to)
            .map(|m| {
                let kind = match m.kind {
                    oms::CashMoveKind::CopyFee => "copy_fee",
                    oms::CashMoveKind::CopyFeeIncome => "copy_fee_income",
                    oms::CashMoveKind::NegativeBalanceCompensation => "nbp",
                };
                // amount stays a magnitude; the kind carries the direction
                (m.ts, json!({ "at": views::iso(m.ts), "kind": kind, "amount": (m.amount.minor as i64).abs(), "reason": "" }))
            }),
    );
    cash.sort_by_key(|(t, _)| *t);
    let cash: Vec<Value> = cash.into_iter().map(|(_, v)| v).collect();
    let positions: Vec<Value> = e
        .positions_of(login)
        .iter()
        .map(|p| {
            json!({
                "id": p.id, "symbol": p.symbol, "side": if p.side == oms::Side::Buy { "buy" } else { "sell" },
                "lots": views::qty_f(p.volume), "openPrice": views::price_f(p.open_price),
                "openedAt": views::iso(p.opened_ts), "swap": p.swap_minor as i64,
            })
        })
        .collect();
    let r = e.account_risk(login).ok()?;
    Some(json!({
        "account": login, "group": a.group, "currency": g.currency.to_string(),
        "from": views::iso(from), "to": if to == u64::MAX { Value::Null } else { json!(views::iso(to - 1)) },
        "generatedAt": views::iso(e.now_ns()),
        "balance": r.balance.minor as i64, "equity": r.equity.minor as i64, "margin": r.margin.minor as i64,
        "trades": trades, "totals": totals, "positions": positions, "cash": cash,
    }))
}

#[cfg(test)]
mod ib_month_tests {
    use super::*;

    #[test]
    fn month_starts() {
        // 2026-10-08 12:00 UTC
        let ns = (days_from_civil(2026, 10, 8) as u64) * DAY_NS + 12 * 3_600_000_000_000;
        let (this, last) = month_start(ns);
        assert_eq!(views::iso(this), "2026-10-01T00:00:00.000Z");
        assert_eq!(views::iso(last), "2026-09-01T00:00:00.000Z");
        let jan = (days_from_civil(2027, 1, 15) as u64) * DAY_NS;
        assert_eq!(views::iso(month_start(jan).1), "2026-12-01T00:00:00.000Z");
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;
    use money::{px, qty};
    use oms::{Envelope, NewOrder, NullRouter};
    use risk::{GroupCommission, GroupConfig, Routing, Side, SymbolSpec};

    #[test]
    fn hostnames_are_normalized_and_checked() {
        assert_eq!(
            norm_hostname("  Trade.ACME.com ").unwrap().as_deref(),
            Some("trade.acme.com")
        );
        assert_eq!(norm_hostname("   ").unwrap(), None);
        assert_eq!(
            norm_hostname("localhost").unwrap().as_deref(),
            Some("localhost")
        );
        for bad in [
            "https://trade.acme.com",
            "trade.acme.com:443",
            "trade.acme.com/",
            "trade.acme.com/login",
            "trade acme.com",
            "trade..acme.com",
            "-trade.acme.com",
            "trade_acme.com",
        ] {
            assert!(norm_hostname(bad).is_err(), "{bad} accepted");
        }
    }

    struct E {
        e: Engine,
        seq: u64,
        ts: u64,
    }

    impl E {
        fn cmd(&mut self, cmd: Command) -> Vec<Event> {
            self.seq += 1;
            self.ts += 1_000;
            self.e.apply(&Envelope {
                seq: self.seq,
                ts: self.ts,
                cmd,
            })
        }
    }

    /// Group "c": B-book, 3.50 per lot per side.
    fn engine() -> E {
        let mut h = E {
            e: Engine::new(Default::default(), Box::new(NullRouter)),
            seq: 0,
            ts: 1_000,
        };
        h.cmd(Command::AddSymbol(SymbolSpec::fx(
            "EURUSD",
            Currency::EUR,
            Currency::USD,
            5,
        )));
        let mut g = GroupConfig::retail("c", Currency::USD, Routing::BBook);
        g.esma = None;
        g.leverage = 100;
        g.commission = Some(GroupCommission::PerLot { minor: 350 });
        h.cmd(Command::SetGroup(g));
        h.cmd(Command::Quote {
            symbol: "EURUSD".into(),
            bid: px("1.10000"),
            ask: px("1.10010"),
        });
        h.cmd(Command::OpenAccount {
            account: 5,
            group: "c".into(),
        });
        h.cmd(Command::Deposit {
            account: 5,
            amount: Money::parse("10000", Currency::USD).unwrap(),
            key: "d5".into(),
        });
        h
    }

    #[test]
    fn statement_commission_includes_the_opening_side() {
        let mut h = engine();
        // round trip: open and close 1 lot -> 3.50 on each side
        h.cmd(Command::PlaceOrder(NewOrder::market(
            5,
            "o1",
            "EURUSD",
            Side::Buy,
            qty("1"),
        )));
        let pid = h.e.positions_of(5)[0].id;
        h.cmd(Command::ClosePosition {
            account: 5,
            position_id: pid,
            volume: None,
            client_order_id: "c1".into(),
        });
        // still open at the end of the period: its opening commission counts
        h.cmd(Command::PlaceOrder(NewOrder::market(
            5,
            "o2",
            "EURUSD",
            Side::Sell,
            qty("1"),
        )));
        let s = statement_body(&h.e, 5, 0, u64::MAX, vec![]).unwrap();
        assert_eq!(s["totals"]["trades"], 1);
        assert_eq!(s["trades"][0]["commission"], -350);
        assert_eq!(s["totals"]["commission"], -1050);
        assert_eq!(s["totals"]["commissionOpen"], -700);
        assert_eq!(s["positions"].as_array().unwrap().len(), 1);
        // the deposit is an engine command here, not an admin op: no cash rows
        assert_eq!(s["cash"], json!([]));
        // a period that ends before the trades shows none of them
        let s = statement_body(&h.e, 5, 0, 1, vec![]).unwrap();
        assert_eq!(s["totals"]["commission"], 0);
        assert!(statement_body(&h.e, 99, 0, u64::MAX, vec![]).is_none());
    }

    #[test]
    fn statement_lists_admin_ops_and_engine_cash_moves_in_time_order() {
        let mut h = engine();
        h.cmd(Command::OpenAccount {
            account: 6,
            group: "c".into(),
        });
        h.cmd(Command::Deposit {
            account: 6,
            amount: Money::parse("10000", Currency::USD).unwrap(),
            key: "d6".into(),
        });
        // follower 5 copies provider 6, 20 % performance fee
        h.cmd(Command::CopySubscribe {
            follower: 5,
            provider: 6,
            ratio_bps: 10_000,
            equity_stop_pct: 0,
            perf_fee_bps: 2_000,
        });
        h.cmd(Command::PlaceOrder(NewOrder::market(
            6,
            "p1",
            "EURUSD",
            Side::Buy,
            qty("1"),
        )));
        h.cmd(Command::Quote {
            symbol: "EURUSD".into(),
            bid: px("1.10500"),
            ask: px("1.10510"),
        });
        let pid = h.e.positions_of(6)[0].id;
        h.cmd(Command::ClosePosition {
            account: 6,
            position_id: pid,
            volume: None,
            client_order_id: "c1".into(),
        });
        h.cmd(Command::CopySettle { provider: 6 });
        let fee =
            h.e.cash_moves()
                .iter()
                .find(|m| m.account == 5)
                .expect("copy fee booked")
                .amount
                .minor as i64;
        assert!(fee < 0);
        let op = (
            1,
            json!({ "at": views::iso(1), "kind": "deposit", "amount": 100, "reason": "" }),
        );
        let s = statement_body(&h.e, 5, 0, u64::MAX, vec![op]).unwrap();
        let cash = s["cash"].as_array().unwrap();
        assert_eq!(cash.len(), 2);
        assert_eq!(cash[0]["kind"], "deposit");
        assert_eq!(cash[1]["kind"], "copy_fee");
        assert_eq!(cash[1]["amount"], -fee);
        let p = statement_body(&h.e, 6, 0, u64::MAX, vec![]).unwrap();
        assert_eq!(p["cash"][0]["kind"], "copy_fee_income");
        assert_eq!(p["cash"][0]["amount"], -fee);
    }
}
