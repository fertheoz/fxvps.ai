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

fn validate(ctx_accounts: bool, r: &InstitutionReq) -> Result<(), ApiError> {
    if !ctx_accounts {
        return Err(ApiError::bad(format!("unknown account {:?}", r.account)));
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

async fn account_exists(ctx: &AdminCtx, account: &str) -> Result<bool, ApiError> {
    let Ok(no) = account.trim().parse::<u64>() else {
        return Ok(false);
    };
    ctx.q(move |e| e.account(no).is_some()).await
}

pub async fn list(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "lp.view")?;
    let st = status(&ctx);
    let list: Vec<Value> = load(&ctx)?.iter().map(|i| view(i, &st)).collect();
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
    validate(account_exists(&ctx, &r.account).await?, &r)?;
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
    validate(account_exists(&ctx, &r.account).await?, &r)?;
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
