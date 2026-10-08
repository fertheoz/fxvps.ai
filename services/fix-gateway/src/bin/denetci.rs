//! Denetçi: watches every order the core sends to its LPs and every execution
//! they return (NATS), compares the primary LP's position with the core's
//! record (engine snapshot) and, when enabled, trades the difference at the
//! primary LP. See `fix_gateway::audit`.
//!
//! Env: NATS_URL, DENETCI_DATA (engine data dir, holds `snapshot.json`; the
//! auditor writes `denetim/`), DENETCI_LP_FILES (comma separated LP configs
//! for symbol / contract sizes), DENETCI_PRIMARY (LMAX), DENETCI_STAGING (SIM),
//! DENETCI_STATUS_FILE (OK/HATA line for the alert engine).
//!
//! DENETCI_LP_ACCOUNT (LMAX account id): every TRADES_EVERY the LP's own trade
//! list (TradeCaptureReportRequest) is fetched and compared by ExecID with
//! what we saw on the wire; trades from other channels join the LP net.
//!
//! Settings (console, `denetim/ayar.json`): `{"autoheal": false, "maxLots": 5,
//! "maxPerMin": 5}`. Corrections are logged to `denetim/duzeltmeler.jsonl`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use domain::{Fixed, Order, OrderType, Side, TimeInForce};
use fix_gateway::audit::{AuditCfg, Auditor, Correction, NetKey};
use fix_gateway::gateway::{GatewayEvent, OrderCommand};
use futures_util::StreamExt;
use serde_json::{json, Value};

/// ClOrdID of a correction: `AUD-<unix seconds>-<n>`. LMAX caps ClOrdID at
/// 20 characters (SessionReject "string length less than or equal to 20"),
/// so the stamp is in seconds, not milliseconds.
fn correction_cl_ord_id(now_ms: u64, seq: u64) -> String {
    format!("AUD-{}-{seq}", now_ms / 1000)
}

fn now_ns() -> u64 {
    domain::now_ns()
}

const TRADES_EVERY: Duration = Duration::from_secs(300);
/// Overlap between consecutive trade-list windows (ExecIDs de-duplicate).
const TRADES_OVERLAP_MS: u64 = 120_000;

fn fix_ts(ms: u64) -> String {
    fix_codec::time::utc_timestamp(std::time::UNIX_EPOCH + Duration::from_millis(ms))
}

/// Core symbol (EURUSD) -> (LP symbol, LP qty per engine lot); SecurityID -> LP symbol.
fn symbol_map(files: &[PathBuf]) -> (HashMap<String, (String, Fixed)>, HashMap<String, String>) {
    let mut m = HashMap::new();
    let mut by_id = HashMap::new();
    for f in files {
        let Ok(b) = std::fs::read(f) else {
            tracing::warn!(file = %f.display(), "LP config unreadable");
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&b) else {
            continue;
        };
        for i in v["instruments"].as_array().into_iter().flatten() {
            let Some(lp_sym) = i["symbol"].as_str() else {
                continue;
            };
            let cs = i["contract_size"].as_i64().unwrap_or(1).max(1);
            let lot = match lp_sym.split('/').next().unwrap_or("") {
                "XAU" | "XPT" | "XPD" => 100,
                "XAG" => 5_000,
                _ => 100_000,
            };
            // engine lots -> LP OrderQty: lot units / contract size
            let per_lot = Fixed::from_parts(lot * 1_0000 / cs, 4);
            m.entry(lp_sym.replace('/', ""))
                .or_insert((lp_sym.to_string(), per_lot));
            if let Some(id) = i["security_id"].as_str() {
                by_id.entry(id.to_string()).or_insert(lp_sym.to_string());
            }
        }
    }
    (m, by_id)
}

/// Fields of one TradeCaptureReport (AE), as the gateway publishes them.
fn parse_ae(
    fields: &[(u32, String)],
    by_id: &HashMap<String, String>,
) -> Option<(String, String, Side, Fixed, Fixed, bool)> {
    let get = |t: u32| {
        fields
            .iter()
            .find(|(k, _)| *k == t)
            .map(|(_, v)| v.as_str())
    };
    let exec = get(17)?.to_string();
    let symbol = get(48)
        .and_then(|id| by_id.get(id).cloned())
        .or_else(|| get(55).map(str::to_string))?;
    let side = match get(54)? {
        "1" => Side::Buy,
        "2" => Side::Sell,
        _ => return None,
    };
    let qty: Fixed = get(32)?.parse().ok()?;
    let px: Fixed = get(31)?.parse().ok()?;
    let last = get(912) == Some("Y");
    Some((exec, symbol, side, qty, px, last))
}

/// The core's cumulative LP net per (LP, LP symbol) from the engine snapshot.
fn engine_net(
    data: &Path,
    map: &HashMap<String, (String, Fixed)>,
    primary: &str,
) -> Option<(BTreeMap<NetKey, Fixed>, u64)> {
    let path = data.join("snapshot.json");
    let written_ms = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let b = std::fs::read(&path).ok()?;
    let v: Value = serde_json::from_slice(&b).ok()?;
    let orders = &v["state"]["lp_orders"];
    let list: Vec<&Value> = match orders {
        Value::Object(o) => o.values().collect(),
        Value::Array(a) => a.iter().collect(),
        _ => return None,
    };
    let mut out: BTreeMap<NetKey, Fixed> = BTreeMap::new();
    for o in list {
        let filled = o["filled"].as_i64().unwrap_or(0);
        if filled == 0 {
            continue;
        }
        let Some((lp_sym, per_lot)) = o["symbol"].as_str().and_then(|s| map.get(s)) else {
            continue;
        };
        let lp = o["lp"].as_str().unwrap_or(primary).to_string();
        let lots = Fixed::from_raw(filled);
        let Some(q) = lots.checked_mul(*per_lot) else {
            continue;
        };
        let signed = if o["side"] == "Buy" { q } else { -q };
        let e = out
            .entry((lp, lp_sym.clone()))
            .or_insert(Fixed::from_int(0));
        *e = *e + signed;
    }
    Some((out, written_ms))
}

/// `resetAt` (ms) of the console's zero-point button, if any.
fn reset_at(dir: &Path) -> Option<u64> {
    let v: Value = serde_json::from_slice(&std::fs::read(dir.join("ayar.json")).ok()?).ok()?;
    v["resetAt"].as_u64()
}

fn settings(dir: &Path) -> (bool, Fixed, usize) {
    let v: Value = std::fs::read(dir.join("ayar.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!({}));
    let max_lots = v["maxLots"]
        .as_f64()
        .map(|f| Fixed::from_raw((f * 1e8) as i64))
        .unwrap_or(Fixed::from_int(5));
    (
        v["autoheal"].as_bool().unwrap_or(false),
        max_lots,
        v["maxPerMin"].as_u64().unwrap_or(5) as usize,
    )
}

fn append(path: &Path, line: &Value) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let env = |k: &str, d: &str| {
        std::env::var(k)
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| d.to_string())
    };
    let url = env("NATS_URL", "nats://127.0.0.1:4222");
    let data = PathBuf::from(env("DENETCI_DATA", "/var/lib/core-engine"));
    let dir = data.join("denetim");
    std::fs::create_dir_all(&dir)?;
    let files: Vec<PathBuf> = env("DENETCI_LP_FILES", "/var/lib/fix-gateway/lp.json")
        .split(',')
        .map(|s| PathBuf::from(s.trim()))
        .collect();
    let (map, by_id) = symbol_map(&files);
    let lp_account = std::env::var("DENETCI_LP_ACCOUNT")
        .ok()
        .filter(|v| !v.is_empty());
    let cfg = AuditCfg {
        primary: env("DENETCI_PRIMARY", "LMAX"),
        staging: env("DENETCI_STAGING", "SIM")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        ..AuditCfg::default()
    };
    let primary = cfg.primary.clone();
    let status_file = std::env::var("DENETCI_STATUS_FILE").ok().map(PathBuf::from);
    tracing::info!(%url, symbols = map.len(), %primary, staging = ?cfg.staging, "denetçi starting");

    let client = async_nats::connect(&url).await?;
    let mut orders = client.subscribe("fx.lp.*.orders").await?;
    let mut events = client.subscribe("fx.lp.*.events").await?;
    let mut a = Auditor::new(cfg, now_ns() / 1_000_000);
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let mut sent: VecDeque<u64> = VecDeque::new(); // correction timestamps (rate limit)
    let mut seq = 0u64;
    // Wire ExecIDs survive restarts (else the LP's list re-reports our own
    // older fills as unseen), as does the end of the last trade-list window.
    let seen_file = dir.join("exec_gorulen.txt");
    // The held-net baseline survives restarts too: re-baselining on the core's
    // record would turn an open LP mismatch into "0 olay" (seen 8 Oct: LMAX
    // kept +1 XAU/USD after a deploy restarted the auditor).
    let held_file = dir.join("tutulan.json");
    if let Ok(b) = std::fs::read(&held_file) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Map<String, Value>>(&b) {
            let mut held = BTreeMap::new();
            for (k, q) in v {
                if let (Some((lp, sym)), Some(q)) = (k.split_once('|'), q.as_str().and_then(|s| s.parse::<Fixed>().ok())) {
                    held.insert((lp.to_string(), sym.to_string()), q);
                }
            }
            tracing::info!(symbols = held.len(), "held net restored");
            a.restore_held(held);
        }
    }
    let cutoff = now_ns() / 1_000_000 - 72 * 3_600_000;
    let mut kept = Vec::new();
    for line in std::fs::read_to_string(&seen_file)
        .unwrap_or_default()
        .lines()
    {
        if let Some((ts, key)) = line.split_once(' ') {
            if ts.parse::<u64>().is_ok_and(|t| t >= cutoff) {
                a.mark_seen(key.to_string());
                kept.push(line.to_string());
            }
        }
    }
    let _ = std::fs::write(
        &seen_file,
        kept.join("\n") + if kept.is_empty() { "" } else { "\n" },
    );
    tracing::info!(seen = kept.len(), "wire ExecIDs restored");
    // LP trade list: window end of the last completed request; an open request
    let mut trades_to_ms: Option<u64> = std::fs::read(dir.join("durum.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["lpTradesTo"].as_u64());
    let mut trades_open: Option<(String, u64)> = None; // (req id, window end)
    let mut trades_tick = tokio::time::interval(TRADES_EVERY);
    trades_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // a zero point taken earlier is re-applied quietly at start-up: from then
    // on the primary LP holds what the core booked at the primary and at the
    // staging LPs together (the staging legs were squared by hand)
    let mut applied_reset: Option<u64> = None;
    let mut first_apply = true;
    let lp_of = |subject: &str| subject.split('.').nth(2).unwrap_or("").to_string();

    loop {
        tokio::select! {
            m = orders.next() => {
                let Some(m) = m else { break };
                if let Ok(OrderCommand::Submit(o)) = serde_json::from_slice::<OrderCommand>(&m.payload) {
                    if !o.cl_ord_id.starts_with("AUD-") {
                        a.on_order(&lp_of(&m.subject), &o, now_ns());
                    }
                }
            }
            m = events.next() => {
                let Some(m) = m else { break };
                match serde_json::from_slice::<GatewayEvent>(&m.payload) {
                    Ok(GatewayEvent::Execution(x)) => {
                        let lp = lp_of(&m.subject);
                        if x.cl_ord_id.as_deref().is_some_and(|c| c.starts_with("AUD-")) {
                            if let (Some(q), Some(px)) = (x.last_qty.filter(|q| q.is_positive()), x.last_px) {
                                append(&dir.join("duzeltmeler.jsonl"), &json!({
                                    "ts": now_ns() / 1_000_000, "event": "fill", "lp": lp, "symbol": x.symbol,
                                    "side": format!("{:?}", x.side), "qty": q.to_string(), "price": px.to_string(),
                                    "cl_ord_id": x.cl_ord_id,
                                }));
                            }
                        }
                        if matches!(x.exec_type, domain::ExecType::Trade) && x.last_qty.is_some_and(|q| q.is_positive()) {
                            use std::io::Write;
                            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&seen_file) {
                                let _ = writeln!(f, "{} {lp}:{}", now_ns() / 1_000_000, x.exec_id);
                            }
                        }
                        a.on_exec(&lp, &x, now_ns());
                    }
                    Ok(GatewayEvent::CommandRejected { cl_ord_id, reason }) => {
                        if trades_open.as_ref().is_some_and(|(id, _)| *id == cl_ord_id) {
                            tracing::error!(%reason, "trade list request refused");
                            trades_open = None;
                        } else {
                            a.on_command_rejected(&cl_ord_id);
                        }
                    }
                    Ok(GatewayEvent::LpMessage { msg_type, fields }) => {
                        let lp = lp_of(&m.subject);
                        let get = |t: u32| fields.iter().find(|(k, _)| *k == t).map(|(_, v)| v.as_str());
                        let ours = trades_open.as_ref().is_some_and(|(id, _)| get(568) == Some(id.as_str()));
                        match msg_type.as_str() {
                            "AQ" if ours => {
                                let ok = get(749) == Some("0") && get(750) == Some("0");
                                if !ok {
                                    tracing::error!(result = ?get(749), status = ?get(750), text = ?get(58), "trade list request rejected by the LP");
                                    trades_open = None;
                                } else if get(748) == Some("0") {
                                    trades_to_ms = trades_open.take().map(|(_, to)| to);
                                }
                            }
                            "AE" if ours => {
                                if let Some((exec, symbol, side, qty, px, last)) = parse_ae(&fields, &by_id) {
                                    a.on_lp_trade(&lp, &exec, &symbol, side, qty, px, now_ns() / 1_000_000);
                                    if last {
                                        trades_to_ms = trades_open.take().map(|(_, to)| to);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            _ = trades_tick.tick(), if lp_account.is_some() => {
                let now_ms = now_ns() / 1_000_000;
                if trades_open.as_ref().is_some_and(|(_, to)| now_ms.saturating_sub(*to) > 2 * TRADES_EVERY.as_millis() as u64) {
                    tracing::warn!("trade list request without an answer; retrying");
                    trades_open = None;
                }
                if trades_open.is_none() {
                    let from = trades_to_ms
                        .map(|t| t.saturating_sub(TRADES_OVERLAP_MS))
                        .or_else(|| reset_at(&dir))
                        .unwrap_or(a.started_ms);
                    let to = now_ms.saturating_sub(2_000); // the LP's own clock may lag slightly
                    if to > from {
                        seq += 1;
                        let id = format!("T{}", now_ms % 100_000_000 * 10 + seq % 10);
                        let cmd = OrderCommand::Trades { req_id: id.clone(), from: fix_ts(from), to: fix_ts(to), account: lp_account.clone() };
                        match client.request(format!("fx.lp.{primary}.orders"), serde_json::to_vec(&cmd)?.into()).await {
                            Ok(r) if &r.payload[..] == b"ok" => trades_open = Some((id, to)),
                            Ok(r) => tracing::error!(reply = %String::from_utf8_lossy(&r.payload), "trade list request refused"),
                            Err(e) => tracing::error!(%e, "trade list request failed"),
                        }
                    }
                }
            }
            _ = tick.tick() => {
                let now = now_ns();
                let now_ms = now / 1_000_000;
                a.tick(now);
                let mut corrections: Vec<Correction> = Vec::new();
                if let Some((core, written_ms)) = engine_net(&data, &map, &primary) {
                    let r = reset_at(&dir);
                    if r.is_some() && r != applied_reset {
                        applied_reset = r;
                        a.reset_with(&core, now_ms, first_apply);
                        if first_apply {
                            a.reset_ms = r;
                        } else {
                            append(&dir.join("duzeltmeler.jsonl"), &json!({"ts": now_ms, "event": "zero_point"}));
                        }
                    }
                    first_apply = false;
                    corrections = a.compare_at(&core, written_ms, now_ms);
                }
                let (autoheal, max_lots, per_min) = settings(&dir);
                while sent.front().is_some_and(|t| now_ms.saturating_sub(*t) > 60_000) {
                    sent.pop_front();
                }
                for c in corrections {
                    let per_lot = map.get(&c.symbol.replace('/', "")).map(|(_, f)| *f).unwrap_or(Fixed::from_int(1));
                    let abs = if c.qty.is_positive() { c.qty } else { -c.qty };
                    let max_qty = max_lots.checked_mul(per_lot).unwrap_or(Fixed::from_int(0));
                    let why = if !autoheal {
                        Some("autoheal off")
                    } else if max_qty < abs {
                        Some("above the size limit: needs a person")
                    } else if sent.len() >= per_min {
                        Some("rate limit")
                    } else {
                        None
                    };
                    if let Some(why) = why {
                        tracing::warn!(symbol = %c.symbol, qty = %c.qty, why, "correction not sent");
                        continue;
                    }
                    seq += 1;
                    let cl = correction_cl_ord_id(now_ms, seq);
                    let order = Order {
                        cl_ord_id: cl.clone(),
                        symbol: c.symbol.clone(),
                        side: if c.qty.is_positive() { Side::Buy } else { Side::Sell },
                        qty: abs,
                        ord_type: OrderType::Market,
                        limit_price: None,
                        tif: TimeInForce::ImmediateOrCancel,
                    };
                    let body = serde_json::to_vec(&OrderCommand::Submit(order))?;
                    match client.request(format!("fx.lp.{}.orders", c.lp), body.into()).await {
                        Ok(r) if &r.payload[..] == b"ok" => {
                            a.on_correction(&c, &cl, now);
                            sent.push_back(now_ms);
                            append(&dir.join("duzeltmeler.jsonl"), &json!({
                                "ts": now_ms, "event": "sent", "lp": c.lp, "symbol": c.symbol,
                                "qty": c.qty.to_string(), "cl_ord_id": cl,
                            }));
                            tracing::warn!(symbol = %c.symbol, qty = %c.qty, %cl, "correction sent");
                        }
                        Ok(r) => tracing::error!(reply = %String::from_utf8_lossy(&r.payload), "correction refused by the gateway"),
                        Err(e) => tracing::error!(%e, "correction request failed"),
                    }
                }
                if let Some(held) = a.held() {
                    let m: serde_json::Map<String, Value> = held
                        .iter()
                        .map(|((lp, s), q)| (format!("{lp}|{s}"), json!(q.to_string())))
                        .collect();
                    let tmp = dir.join("tutulan.json.tmp");
                    if std::fs::write(&tmp, Value::Object(m).to_string()).is_ok() {
                        let _ = std::fs::rename(&tmp, &held_file);
                    }
                }
                let mut st = a.status(now_ms);
                st["autoheal"] = json!(autoheal);
                st["lpTradesTo"] = json!(trades_to_ms);
                st["maxLots"] = json!(max_lots.to_string());
                let tmp = dir.join("durum.json.tmp");
                if std::fs::write(&tmp, st.to_string()).is_ok() {
                    let _ = std::fs::rename(&tmp, dir.join("durum.json"));
                }
                if let Some(f) = &status_file {
                    let ok = st["ok"].as_bool().unwrap_or(true);
                    let line = if ok {
                        format!("OK denetim: {} olay, {} düzeltme\n", a.total_incidents, a.corrections_sent)
                    } else {
                        format!("HATA denetim: açık fark {}\n", st["openMismatches"])
                    };
                    let _ = std::fs::write(f, line);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::correction_cl_ord_id;

    #[test]
    fn correction_id_fits_lmax_limit() {
        let id = correction_cl_ord_id(1_791_492_994_926, 373);
        assert_eq!(id, "AUD-1791492994-373");
        assert!(id.len() <= 20);
        assert!(correction_cl_ord_id(4_102_444_800_000, 99_999).len() <= 20);
    }
}
