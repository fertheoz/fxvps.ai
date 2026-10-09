//! MT5 plugin bridge institutions (console "Kurumlar"). The list lives in
//! `<data_dir>/bridge/kurumlar.json`, shared with the client-gateway `/bridge`
//! (it reloads the file on change); live sessions are read from
//! `<data_dir>/bridge/durum.json`, written by the gateway every two seconds.
//! Keys are generated here, shown once, and stored only as SHA-256.

use super::auth::Actor;
use super::store::AdminCmd;
use super::{need, AdminCtx, ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::Json;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Institution {
    pub id: String,
    pub key_sha256: String,
    pub account: String,
    #[serde(default = "default_rate")]
    pub orders_per_sec: u32,
    #[serde(default)]
    pub ips: Vec<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub created_ns: u64,
}

fn default_rate() -> u32 {
    100
}

fn dir(ctx: &AdminCtx) -> PathBuf {
    ctx.data_dir.join("bridge")
}

fn file(ctx: &AdminCtx) -> PathBuf {
    dir(ctx).join("kurumlar.json")
}

fn load(ctx: &AdminCtx) -> Result<Vec<Institution>, ApiError> {
    match std::fs::read(file(ctx)) {
        Ok(b) => serde_json::from_slice(&b)
            .map_err(|e| ApiError::internal(format!("kurumlar.json: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(ApiError::internal(format!("kurumlar.json: {e}"))),
    }
}

/// Engine logins of the bridge institutions (their books are not client
/// positions).
pub(super) fn institution_logins(
    ctx: &AdminCtx,
) -> Result<std::collections::BTreeSet<u64>, ApiError> {
    Ok(load(ctx)?
        .iter()
        .filter_map(|i| i.account.trim().parse().ok())
        .collect())
}

fn save(ctx: &AdminCtx, list: &[Institution]) -> Result<(), ApiError> {
    std::fs::create_dir_all(dir(ctx)).map_err(|e| ApiError::internal(e.to_string()))?;
    let tmp = dir(ctx).join("kurumlar.json.tmp");
    let body = serde_json::to_vec_pretty(list).map_err(|e| ApiError::internal(e.to_string()))?;
    std::fs::write(&tmp, body).map_err(|e| ApiError::internal(e.to_string()))?;
    std::fs::rename(&tmp, file(ctx)).map_err(|e| ApiError::internal(e.to_string()))
}

fn new_key() -> (String, String) {
    let mut b = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut b);
    let key: String = format!(
        "fxk_{}",
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    );
    let sha = Sha256::digest(key.as_bytes())
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect();
    (key, sha)
}

fn status(ctx: &AdminCtx) -> Value {
    std::fs::read(dir(ctx).join("durum.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!({"sessions": []}))
}

const DAY_NS: u64 = 86_400_000_000_000;

/// `{h24, d7}` activity per institution account (engine query).
async fn activities(
    ctx: &AdminCtx,
    list: &[Institution],
) -> Result<std::collections::HashMap<String, Value>, ApiError> {
    let accounts: Vec<(String, u64)> = list
        .iter()
        .filter_map(|i| i.account.parse().ok().map(|n| (i.id.clone(), n)))
        .collect();
    let now = super::routes::now_ns();
    ctx.q(move |e| {
        accounts
            .into_iter()
            .map(|(id, n)| {
                (
                    id,
                    json!({
                        "h24": super::views::account_activity(e, n, now.saturating_sub(DAY_NS)),
                        "d7": super::views::account_activity(e, n, now.saturating_sub(7 * DAY_NS)),
                    }),
                )
            })
            .collect()
    })
    .await
}

fn view(i: &Institution, st: &Value) -> Value {
    let sessions: Vec<&Value> = st["sessions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|s| s["institution"] == i.id.as_str())
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": i.id, "name": i.name, "account": i.account, "ordersPerSec": i.orders_per_sec,
        "ips": i.ips, "createdNs": i.created_ns, "sessions": sessions,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstitutionReq {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    account: String,
    #[serde(default)]
    orders_per_sec: Option<u32>,
    #[serde(default)]
    ips: Vec<String>,
}

fn validate(account_problem: Option<String>, r: &InstitutionReq) -> Result<(), ApiError> {
    if let Some(p) = account_problem {
        return Err(ApiError::bad(p));
    }
    if let Some(rate) = r.orders_per_sec {
        if !(1..=1000).contains(&rate) {
            return Err(ApiError::bad("ordersPerSec must be 1..1000"));
        }
    }
    for ip in &r.ips {
        if ip.parse::<std::net::IpAddr>().is_err() {
            return Err(ApiError::bad(format!("invalid IP {ip:?}")));
        }
    }
    Ok(())
}

/// `None` = usable as an institution account; `Some(reason)` otherwise. The
/// MT5 side nets positions per symbol, so the account must be in a NETTING
/// group or the reconciliation can never agree.
async fn account_problem(ctx: &AdminCtx, account: &str) -> Result<Option<String>, ApiError> {
    let Ok(no) = account.trim().parse::<u64>() else {
        return Ok(Some(format!("unknown account {account:?}")));
    };
    ctx.q(move |e| {
        let Some(a) = e.account(no) else {
            return Some(format!("unknown account {no}"));
        };
        match e.group(&a.group) {
            Some(g) if g.margin_mode == risk::MarginMode::Netting => None,
            Some(g) => Some(format!(
                "account {no} is in group {:?} (hedging); an institution account must be in a NETTING group",
                g.name
            )),
            None => Some(format!("account {no}: unknown group {:?}", a.group)),
        }
    })
    .await
}

pub async fn list(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "lp.view")?;
    let st = status(&ctx);
    let insts = load(&ctx)?;
    let act = activities(&ctx, &insts).await?;
    let list: Vec<Value> = insts
        .iter()
        .map(|i| {
            let mut v = view(i, &st);
            v["activity"] = act.get(&i.id).cloned().unwrap_or(Value::Null);
            v
        })
        .collect();
    Ok(Json(json!({
        "institutions": list,
        "endpoint": std::env::var("CORE_BRIDGE_PUBLIC_URL").unwrap_or_else(|_| "wss://trade.fxvps.ai/bridge".into()),
        "statusAt": st["at"],
    })))
}

pub async fn create(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(r): Json<InstitutionReq>,
) -> ApiResult {
    need(&actor, "lp.manage")?;
    let id = r.id.trim().to_ascii_lowercase();
    if id.len() < 2
        || id.len() > 40
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::bad("id: 2-40 chars, a-z 0-9 - _"));
    }
    validate(account_problem(&ctx, &r.account).await?, &r)?;
    let mut store = ctx.store.lock().await;
    let mut list = load(&ctx)?;
    if list.iter().any(|i| i.id == id) {
        return Err(ApiError::bad(format!("institution {id} exists")));
    }
    let (key, sha) = new_key();
    let inst = Institution {
        id: id.clone(),
        key_sha256: sha,
        account: r.account.trim().to_string(),
        orders_per_sec: r.orders_per_sec.unwrap_or(100),
        ips: r.ips,
        name: r.name.trim().to_string(),
        created_ns: super::routes::now_ns(),
    };
    list.push(inst.clone());
    save(&ctx, &list)?;
    store.append(
        &actor,
        AdminCmd::LpConfigSaved {
            details: format!("bridge institution {id} created (account {})", inst.account),
        },
    )?;
    drop(store);
    ctx.notify(&["listInstitutions", "listAudit"]);
    Ok(Json(
        json!({ "institution": view(&inst, &status(&ctx)), "key": key }),
    ))
}

pub async fn update(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(r): Json<InstitutionReq>,
) -> ApiResult {
    need(&actor, "lp.manage")?;
    validate(account_problem(&ctx, &r.account).await?, &r)?;
    let mut store = ctx.store.lock().await;
    let mut list = load(&ctx)?;
    let Some(inst) = list.iter_mut().find(|i| i.id == id) else {
        return Err(ApiError::not_found("unknown institution"));
    };
    inst.account = r.account.trim().to_string();
    if let Some(rate) = r.orders_per_sec {
        inst.orders_per_sec = rate;
    }
    inst.ips = r.ips;
    if !r.name.trim().is_empty() {
        inst.name = r.name.trim().to_string();
    }
    let out = inst.clone();
    save(&ctx, &list)?;
    store.append(
        &actor,
        AdminCmd::LpConfigSaved {
            details: format!("bridge institution {id} updated"),
        },
    )?;
    drop(store);
    ctx.notify(&["listInstitutions", "listAudit"]);
    Ok(Json(view(&out, &status(&ctx))))
}

pub async fn rotate(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
) -> ApiResult {
    need(&actor, "lp.manage")?;
    let mut store = ctx.store.lock().await;
    let mut list = load(&ctx)?;
    let Some(inst) = list.iter_mut().find(|i| i.id == id) else {
        return Err(ApiError::not_found("unknown institution"));
    };
    let (key, sha) = new_key();
    inst.key_sha256 = sha;
    save(&ctx, &list)?;
    store.append(
        &actor,
        AdminCmd::LpConfigSaved {
            details: format!("bridge institution {id} key rotated"),
        },
    )?;
    drop(store);
    ctx.notify(&["listInstitutions", "listAudit"]);
    Ok(Json(json!({ "key": key })))
}

pub async fn remove(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
) -> ApiResult {
    need(&actor, "lp.manage")?;
    let mut store = ctx.store.lock().await;
    let mut list = load(&ctx)?;
    let n = list.len();
    list.retain(|i| i.id != id);
    if list.len() == n {
        return Err(ApiError::not_found("unknown institution"));
    }
    save(&ctx, &list)?;
    store.append(
        &actor,
        AdminCmd::LpConfigSaved {
            details: format!("bridge institution {id} removed"),
        },
    )?;
    drop(store);
    ctx.notify(&["listInstitutions", "listAudit"]);
    Ok(Json(json!({ "ok": true })))
}

// ---- Denetçi (auditor) -------------------------------------------------------

fn audit_dir(ctx: &AdminCtx) -> PathBuf {
    ctx.data_dir.join("denetim")
}

pub async fn audit_status(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "lp.view")?;
    let d = audit_dir(&ctx);
    let status: Value = std::fs::read(d.join("durum.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!(null));
    let settings: Value = std::fs::read(d.join("ayar.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!({"autoheal": false, "maxLots": 5, "maxPerMin": 5}));
    let corrections: Vec<Value> = std::fs::read_to_string(d.join("duzeltmeler.jsonl"))
        .unwrap_or_default()
        .lines()
        .rev()
        .take(100)
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    Ok(Json(json!({
        "status": status, "settings": settings, "corrections": corrections,
        // age by the server clock: a skewed browser clock must not mark it silent
        "ageMs": status["at"].as_u64().map(|at| (super::routes::now_ns() / 1_000_000).saturating_sub(at)),
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditSettings {
    autoheal: bool,
    max_lots: f64,
    max_per_min: u32,
}

pub async fn audit_settings(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(s): Json<AuditSettings>,
) -> ApiResult {
    need(&actor, "lp.manage")?;
    if !(0.0..=100.0).contains(&s.max_lots) || s.max_per_min > 60 {
        return Err(ApiError::bad("maxLots 0..100, maxPerMin 0..60"));
    }
    let d = audit_dir(&ctx);
    std::fs::create_dir_all(&d).map_err(|e| ApiError::internal(e.to_string()))?;
    let mut v: Value = std::fs::read(d.join("ayar.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!({}));
    v["autoheal"] = json!(s.autoheal);
    v["maxLots"] = json!(s.max_lots);
    v["maxPerMin"] = json!(s.max_per_min);
    std::fs::write(d.join("ayar.json"), v.to_string())
        .map_err(|e| ApiError::internal(e.to_string()))?;
    ctx.store.lock().await.append(
        &actor,
        AdminCmd::LpConfigSaved {
            details: format!(
                "denetçi: autoheal {}, max {} lots, {}/min",
                s.autoheal, s.max_lots, s.max_per_min
            ),
        },
    )?;
    ctx.notify(&["getAudit", "listAudit"]);
    Ok(Json(v))
}

/// Zero point: the auditor closes its history at this moment (both sides
/// were flattened by hand). Recorded in the admin audit log with the time.
pub async fn audit_reset(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "lp.manage")?;
    let d = audit_dir(&ctx);
    std::fs::create_dir_all(&d).map_err(|e| ApiError::internal(e.to_string()))?;
    let mut v: Value = std::fs::read(d.join("ayar.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!({"autoheal": false, "maxLots": 5, "maxPerMin": 5}));
    let at = super::routes::now_ns() / 1_000_000;
    v["resetAt"] = json!(at);
    std::fs::write(d.join("ayar.json"), v.to_string())
        .map_err(|e| ApiError::internal(e.to_string()))?;
    ctx.store.lock().await.append(
        &actor,
        AdminCmd::LpConfigSaved {
            details: format!("denetçi sıfır noktası {at}"),
        },
    )?;
    ctx.notify(&["getAudit", "listAudit"]);
    Ok(Json(json!({ "resetAt": at })))
}

// ---- Partner (institution) overview -----------------------------------------

/// What an institution's own staff may see: its institutions (by tenant
/// accounts), the account snapshots, open positions and recent deals. An
/// operator without a tenant (admin) sees all institutions.
pub async fn partner_overview(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "partner.view")?;
    let allowed = super::routes::tenant_logins(&ctx, &actor).await?;
    let st = status(&ctx);
    let insts: Vec<Institution> = load(&ctx)?
        .into_iter()
        .filter(|i| {
            allowed.as_ref().is_none_or(|set| {
                i.account
                    .parse::<u64>()
                    .ok()
                    .is_some_and(|n| set.contains(&n))
            })
        })
        .collect();
    let accounts: Vec<u64> = insts
        .iter()
        .filter_map(|i| i.account.parse().ok())
        .collect();
    let admin_state = ctx.view_state().await;
    let (accs, positions, deals) = ctx
        .q(move |e| {
            let accs: Vec<Value> = accounts
                .iter()
                .filter_map(|id| super::views::client(e, *id, &admin_state))
                .collect();
            let positions: Vec<Value> = accounts
                .iter()
                .flat_map(|id| e.positions_of(*id))
                .map(|p| super::views::position(e, p))
                .collect();
            let deals: Vec<Value> = match super::views::trades(e, 0, u64::MAX, 5_000) {
                Value::Array(rows) => rows
                    .into_iter()
                    .filter(|r| r["login"].as_u64().is_some_and(|l| accounts.contains(&l)))
                    .rev()
                    .take(100)
                    .collect(),
                _ => Vec::new(),
            };
            (accs, positions, deals)
        })
        .await?;
    let act = activities(&ctx, &insts).await?;
    Ok(Json(json!({
        "institutions": insts.iter().map(|i| {
            let mut v = view(i, &st);
            // the partner sees its volume, not our revenue
            let mut a = act.get(&i.id).cloned().unwrap_or(Value::Null);
            for w in ["h24", "d7"] {
                if let Some(o) = a[w].as_object_mut() { o.remove("revenue"); }
            }
            v["activity"] = a;
            v
        }).collect::<Vec<_>>(),
        "accounts": accs,
        "positions": positions,
        "deals": deals,
        "endpoint": std::env::var("CORE_BRIDGE_PUBLIC_URL").unwrap_or_else(|_| "wss://trade.fxvps.ai/bridge".into()),
        "statusAt": st["at"],
    })))
}
