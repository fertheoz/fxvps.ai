//! Operational alerts (bridge stage 9): a background evaluator derives a set
//! of active conditions from the engine, the FIX session table and the LP
//! aggregator every few seconds. A condition that appears raises an alert
//! (audited, optionally POSTed to `CORE_ALERT_WEBHOOK_URL`); one that clears
//! resolves it. The console shows the active list and a short history.

use super::{store::AdminCmd, views, AdminCtx};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub id: String,
    /// `lp_down` | `fill_rate` | `latency` | `exposure` | `lp_deviation` | `stop_out`
    pub kind: String,
    pub target: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    /// ns since epoch
    pub raised_at: u64,
    pub resolved_at: Option<u64>,
    pub acked: bool,
}

/// One evaluated condition; `key` identifies it across evaluations.
#[derive(Clone, Debug)]
pub struct Condition {
    pub kind: &'static str,
    pub target: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
}

impl Condition {
    fn key(&self) -> String {
        format!("{}:{}", self.kind, self.target)
    }
}

#[derive(Default)]
struct Book {
    active: BTreeMap<String, Alert>,
    history: Vec<Alert>,
    seq: u64,
}

/// Shared alert state of the admin API.
#[derive(Default)]
pub struct AlertBook(Mutex<Book>);

impl AlertBook {
    /// Applies the current condition set; returns the newly raised alerts.
    pub fn apply(&self, now_ns: u64, conditions: Vec<Condition>) -> Vec<Alert> {
        let mut b = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut raised = Vec::new();
        let keys: std::collections::BTreeSet<String> =
            conditions.iter().map(Condition::key).collect();
        // resolve what disappeared
        let gone: Vec<String> = b
            .active
            .keys()
            .filter(|k| !keys.contains(*k))
            .cloned()
            .collect();
        for k in gone {
            if let Some(mut a) = b.active.remove(&k) {
                a.resolved_at = Some(now_ns);
                b.history.push(a);
            }
        }
        for c in conditions {
            let key = c.key();
            match b.active.get_mut(&key) {
                Some(a) => {
                    // keep the alert, refresh the text
                    a.detail = c.detail;
                    a.severity = c.severity;
                }
                None => {
                    b.seq += 1;
                    let a = Alert {
                        id: format!("al-{}", b.seq),
                        kind: c.kind.to_string(),
                        target: c.target,
                        severity: c.severity,
                        title: c.title,
                        detail: c.detail,
                        raised_at: now_ns,
                        resolved_at: None,
                        acked: false,
                    };
                    raised.push(a.clone());
                    b.active.insert(key, a);
                }
            }
        }
        if b.history.len() > 200 {
            let cut = b.history.len() - 200;
            b.history.drain(..cut);
        }
        raised
    }

    pub fn ack(&self, id: &str) -> bool {
        let mut b = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match b.active.values_mut().find(|a| a.id == id) {
            Some(a) => {
                a.acked = true;
                true
            }
            None => false,
        }
    }

    pub fn snapshot(&self) -> Value {
        let b = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut active: Vec<&Alert> = b.active.values().collect();
        active.sort_by_key(|a| std::cmp::Reverse(a.raised_at));
        let recent: Vec<&Alert> = b.history.iter().rev().take(50).collect();
        json!({ "active": active, "recent": recent })
    }
}

/// Thresholds of the evaluator (fixed for now; a settings page can follow).
const LP_DOWN_GRACE_MS: u64 = 60_000;
const FILL_RATE_MIN_ORDERS: u32 = 10;
const FILL_RATE_FLOOR: f64 = 0.90;
const LATENCY_MIN_SAMPLES: usize = 5;
const LATENCY_FLOOR_MS: f64 = 500.0;

/// Conditions from the engine state (orders, LP orders, exposure, margin).
pub fn engine_conditions(
    e: &oms::Engine,
    admin: &super::store::AdminState,
    now_ns: u64,
) -> Vec<Condition> {
    let mut out = Vec::new();
    const MIN: u64 = 60_000_000_000;
    // fill rate over the last hour
    let since = now_ns.saturating_sub(60 * MIN);
    let (mut orders, mut filled) = (0u32, 0u32);
    for o in e
        .orders()
        .filter(|o| o.created_ts >= since && o.status.is_terminal())
    {
        orders += 1;
        if o.filled.raw() > 0 {
            filled += 1;
        }
    }
    if orders >= FILL_RATE_MIN_ORDERS {
        let rate = f64::from(filled) / f64::from(orders);
        if rate < FILL_RATE_FLOOR {
            out.push(Condition {
                kind: "fill_rate",
                target: "1h".into(),
                severity: Severity::Warning,
                title: "Fill rate dropped".into(),
                detail: format!(
                    "{filled}/{orders} orders filled in the last hour ({:.0}%)",
                    rate * 100.0
                ),
            });
        }
    }
    // LP latency: p95 of the last 15 min vs the previous hour
    let lat = |from: u64, to: u64| -> Vec<f64> {
        let mut v: Vec<f64> = e
            .lp_orders()
            .filter(|l| l.created_ts >= from && l.created_ts < to)
            .filter_map(|l| {
                l.fills
                    .iter()
                    .map(|f| f.ts)
                    .min()
                    .map(|t| t.saturating_sub(l.created_ts) as f64 / 1e6)
            })
            .collect();
        v.sort_by(|a, b| a.total_cmp(b));
        v
    };
    let p95 = |v: &[f64]| {
        if v.is_empty() {
            0.0
        } else {
            v[((v.len() - 1) as f64 * 0.95).round() as usize]
        }
    };
    let recent = lat(now_ns.saturating_sub(15 * MIN), now_ns + 1);
    let base = lat(
        now_ns.saturating_sub(75 * MIN),
        now_ns.saturating_sub(15 * MIN),
    );
    if recent.len() >= LATENCY_MIN_SAMPLES {
        let (r, b) = (p95(&recent), p95(&base));
        if r > LATENCY_FLOOR_MS.max(3.0 * b) {
            out.push(Condition {
                kind: "latency",
                target: "lp".into(),
                severity: Severity::Warning,
                title: "LP latency spike".into(),
                detail: format!("p95 {r:.0} ms in the last 15 min (previous hour {b:.0} ms)"),
            });
        }
    }
    // B-book exposure over its limit
    if let Value::Array(rows) = views::exposure(e) {
        for r in rows {
            if r["overLimit"].as_bool() == Some(true) {
                out.push(Condition {
                    kind: "exposure",
                    target: r["symbol"].as_str().unwrap_or("?").to_string(),
                    severity: Severity::Critical,
                    title: "B-book exposure over limit".into(),
                    detail: format!(
                        "{}: B-book {} lots, limit {} lots, hedge {} lots",
                        r["symbol"].as_str().unwrap_or("?"),
                        r["bBookLots"],
                        r["limitLots"],
                        r["hedgeLots"]
                    ),
                });
            }
        }
    }
    // accounts in stop-out
    if let Value::Array(rows) = views::margin_calls(e, admin) {
        let so = rows.iter().filter(|r| r["state"] == "stop_out").count();
        if so > 0 {
            out.push(Condition {
                kind: "stop_out",
                target: "accounts".into(),
                severity: Severity::Critical,
                title: "Accounts at stop-out".into(),
                detail: format!("{so} account(s) at or below the stop-out level"),
            });
        }
    }
    out
}

/// Conditions from the FIX session table and the LP aggregator.
pub fn infra_conditions(ctx: &AdminCtx, now_ms: u64) -> Vec<Condition> {
    let mut out = Vec::new();
    if let Some(rows) = ctx
        .lp_status
        .as_ref()
        .and_then(|s| s.read().ok().map(|t| t.clone()))
    {
        for r in rows {
            if r.logged_on {
                continue;
            }
            // never connected yet (since_ms == 0) counts once the grace period has passed
            let down_for = if r.since_ms == 0 {
                LP_DOWN_GRACE_MS
            } else {
                now_ms.saturating_sub(r.since_ms)
            };
            if down_for >= LP_DOWN_GRACE_MS {
                let lp = if r.lp.is_empty() {
                    r.target_comp_id.clone()
                } else {
                    r.lp.clone()
                };
                let kind = match r.kind {
                    fix_gateway::SessionKind::MarketData => "MD",
                    fix_gateway::SessionKind::Trading => "TRADING",
                };
                out.push(Condition {
                    kind: "lp_down",
                    target: format!("{lp}-{kind}"),
                    severity: Severity::Critical,
                    title: format!("{lp} {kind} session down"),
                    detail: r
                        .last_down_reason
                        .clone()
                        .unwrap_or_else(|| "not logged on".into()),
                });
            }
        }
    }
    // Backup drill status file written by deploy/lmax-demo/yedek.sh
    if let Ok(path) = std::env::var("CORE_BACKUP_STATUS_FILE") {
        match std::fs::read_to_string(&path) {
            Ok(s) => {
                let fresh = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|d| d.as_secs() < 36 * 3_600);
                if !s.starts_with("OK") {
                    out.push(Condition {
                        kind: "backup",
                        target: "drill".into(),
                        severity: Severity::Critical,
                        title: "Backup / restore drill failed".into(),
                        detail: s.trim().chars().take(200).collect(),
                    });
                } else if !fresh {
                    out.push(Condition {
                        kind: "backup",
                        target: "stale".into(),
                        severity: Severity::Warning,
                        title: "No successful backup in 36 h".into(),
                        detail: s.trim().chars().take(200).collect(),
                    });
                }
            }
            Err(_) => out.push(Condition {
                kind: "backup",
                target: "missing".into(),
                severity: Severity::Warning,
                title: "Backup status file missing".into(),
                detail: path,
            }),
        }
    }
    if let Some(agg) = &ctx.agg {
        for r in agg.runtime() {
            if r.policy.enabled && !r.deviating.is_empty() {
                out.push(Condition {
                    kind: "lp_deviation",
                    target: r.name.clone(),
                    severity: Severity::Warning,
                    title: format!("{} prices excluded by the deviation guard", r.name),
                    detail: r.deviating.join(", "),
                });
            }
        }
    }
    out
}

/// Background evaluator: every 15 s.
pub fn spawn(ctx: AdminCtx) {
    let webhook = std::env::var("CORE_ALERT_WEBHOOK_URL")
        .ok()
        .filter(|v| !v.is_empty());
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_secs(15));
        loop {
            iv.tick().await;
            let now_ns = domain::now_ns();
            let st = ctx.view_state().await;
            let Ok(mut conditions) = ctx
                .engine
                .read(move |e| engine_conditions(e, &st, now_ns))
                .await
            else {
                break;
            };
            conditions.extend(infra_conditions(&ctx, now_ns / 1_000_000));
            let raised = ctx.alerts.apply(now_ns, conditions);
            if raised.is_empty() {
                continue;
            }
            for a in &raised {
                tracing::warn!(kind = %a.kind, target = %a.target, detail = %a.detail, "alert raised");
                let cmd = AdminCmd::AlertRaised {
                    kind: a.kind.clone(),
                    target: a.target.clone(),
                    detail: a.detail.clone(),
                };
                if let Err(e) = ctx
                    .store
                    .lock()
                    .await
                    .append(&super::auth::Actor::system(), cmd)
                {
                    tracing::warn!(error = %e, "alert not audited");
                }
                if let Some(url) = &webhook {
                    let body = json!({
                        "kind": a.kind, "severity": a.severity, "title": a.title,
                        "detail": a.detail, "target": a.target, "at": views::iso(a.raised_at),
                    });
                    if let Err(e) = ctx.http.post(url).json(&body).send().await {
                        tracing::warn!(error = %e, "alert webhook failed");
                    }
                }
            }
            ctx.notify(&["listAlerts", "listAudit"]);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond(kind: &'static str, target: &str) -> Condition {
        Condition {
            kind,
            target: target.into(),
            severity: Severity::Warning,
            title: kind.into(),
            detail: String::new(),
        }
    }

    #[test]
    fn raises_once_then_resolves() {
        let book = AlertBook::default();
        let raised = book.apply(1, vec![cond("lp_down", "LMAX-MD")]);
        assert_eq!(raised.len(), 1);
        // same condition again: nothing new
        assert!(book.apply(2, vec![cond("lp_down", "LMAX-MD")]).is_empty());
        assert!(book.ack(&raised[0].id));
        // cleared: moves to history
        assert!(book.apply(3, vec![]).is_empty());
        let s = book.snapshot();
        assert_eq!(s["active"].as_array().unwrap().len(), 0);
        assert_eq!(s["recent"][0]["resolvedAt"], json!(3));
        assert_eq!(s["recent"][0]["acked"], json!(true));
    }
}
