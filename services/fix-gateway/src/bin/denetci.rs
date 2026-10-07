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

fn now_ns() -> u64 {
    domain::now_ns()
}

/// Core symbol (EURUSD) -> (LP symbol, LP qty per engine lot).
fn symbol_map(files: &[PathBuf]) -> HashMap<String, (String, Fixed)> {
    let mut m = HashMap::new();
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
        }
    }
    m
}

/// The core's cumulative LP net per (LP, LP symbol) from the engine snapshot.
fn engine_net(
    data: &Path,
    map: &HashMap<String, (String, Fixed)>,
    primary: &str,
) -> Option<BTreeMap<NetKey, Fixed>> {
    let b = std::fs::read(data.join("snapshot.json")).ok()?;
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
    Some(out)
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
    let map = symbol_map(&files);
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
                        a.on_exec(&lp, &x, now_ns());
                    }
                    Ok(GatewayEvent::CommandRejected { cl_ord_id, .. }) => a.on_command_rejected(&cl_ord_id),
                    _ => {}
                }
            }
            _ = tick.tick() => {
                let now = now_ns();
                let now_ms = now / 1_000_000;
                a.tick(now);
                let mut corrections: Vec<Correction> = Vec::new();
                if let Some(core) = engine_net(&data, &map, &primary) {
                    corrections = a.compare(&core, now_ms);
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
                    let cl = format!("AUD-{}-{seq}", now_ms);
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
                let mut st = a.status(now_ms);
                st["autoheal"] = json!(autoheal);
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
