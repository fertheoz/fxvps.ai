//! Denetçi (auditor): an independent check of the core against its LPs.
//!
//! Inputs (both observed on the wire, never asked from the core):
//! - order commands the core sends to an LP (`fx.lp.<lp>.orders`),
//! - execution reports the LP sends back (`fx.lp.<lp>.events`),
//! - periodically, the core's own record of LP fills (snapshot `lp_orders`).
//!
//! Checks: every order gets a final answer in time; fills never exceed the
//! order or breach its limit; LP fills always belong to an order we sent;
//! and, per LP and symbol, the net the LP filled equals the net the core
//! believes it holds. Latency (send -> ack, send -> fill) is measured per LP.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use domain::{ExecType, Execution, Fixed, Order, OrderStatus, Side};
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct AuditCfg {
    /// Prefix of the core's LP order ids (`cl_ord_id = <prefix><lp_order_id>`).
    pub prefix: String,
    /// No final execution report within this time = `no_answer`.
    pub answer_timeout_ms: u64,
    /// Net mismatch must persist this long (with nothing in flight) to alert.
    pub settle_ms: u64,
    /// The LP that must actually hold the exposure (e.g. LMAX).
    pub primary: String,
    /// Test / disabled LPs (e.g. SIM): whatever the core booked there must be
    /// held at the primary LP instead.
    pub staging: Vec<String>,
}

impl Default for AuditCfg {
    fn default() -> Self {
        AuditCfg {
            prefix: "LP-".into(),
            answer_timeout_ms: 5_000,
            settle_ms: 6_000,
            primary: "LMAX".into(),
            staging: vec!["SIM".into()],
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// No final execution report for an order within the timeout.
    NoAnswer,
    /// An LP fill for an order the core never sent (manual trade, other system).
    ForeignFill,
    /// Filled more than ordered.
    Overfill,
    /// Fill price outside the order's limit.
    LimitBreach,
    /// Per LP/symbol net of LP fills differs from the core's record.
    NetMismatch,
    /// The LP's trade list has a trade we never saw on our FIX session
    /// (web / other channel). It is added to the LP net.
    LpTradeUnseen,
}

#[derive(Clone, Debug, Serialize)]
pub struct Incident {
    pub ts_ms: u64,
    pub kind: Kind,
    pub lp: String,
    pub symbol: String,
    pub cl_ord_id: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug)]
struct Track {
    lp: String,
    symbol: String,
    side: Side,
    qty: Fixed,
    limit: Option<Fixed>,
    sent_ns: u64,
    acked: bool,
    filled: Fixed,
    done: bool,
    /// Rests at the LP (GTC): answered by its ack, in flight only until then.
    gtc: bool,
}

impl Track {
    /// Still waiting for the LP: not done, and (for a resting order) not acked yet.
    fn open(&self) -> bool {
        !(self.done || (self.gtc && self.acked))
    }
}

#[derive(Default, Clone, Debug, Serialize)]
pub struct LpStats {
    pub orders: u64,
    pub fills: u64,
    pub rejects: u64,
    pub partial: u64,
    pub ack_ms_p50: f64,
    pub ack_ms_p99: f64,
    pub fill_ms_p50: f64,
    pub fill_ms_p99: f64,
    #[serde(skip)]
    ack: VecDeque<f64>,
    #[serde(skip)]
    fill: VecDeque<f64>,
}

const SAMPLES: usize = 2_000;

fn push(v: &mut VecDeque<f64>, x: f64) {
    if v.len() == SAMPLES {
        v.pop_front();
    }
    v.push_back(x);
}

fn pct(v: &VecDeque<f64>, p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f64> = v.iter().copied().collect();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let i = ((s.len() as f64 - 1.0) * p).round() as usize;
    (s[i] * 10.0).round() / 10.0
}

/// Key: (LP name, LP symbol).
pub type NetKey = (String, String);

/// A trade at the primary LP that closes a net mismatch (signed: + = buy).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Correction {
    pub lp: String,
    pub symbol: String,
    pub qty: Fixed,
}

#[derive(Default)]
pub struct Auditor {
    cfg: AuditCfg,
    orders: HashMap<String, Track>,
    /// Net LP quantity filled since start, signed (buy +).
    net: BTreeMap<NetKey, Fixed>,
    /// Core net at start (baseline) and since when a mismatch has persisted.
    engine0: Option<BTreeMap<NetKey, Fixed>>,
    mismatch_since: HashMap<NetKey, u64>,
    /// ExecIDs seen on the wire or in the LP's trade list.
    seen_exec: HashSet<String>,
    pub lp_trades_checked: u64,
    /// Last LP execution per (LP, symbol), ms: the core's record must be
    /// newer than this before the symbol is compared.
    last_exec_ms: HashMap<NetKey, u64>,
    reported: HashMap<NetKey, Fixed>,
    stats: BTreeMap<String, LpStats>,
    incidents: VecDeque<Incident>,
    pub total_incidents: u64,
    pub corrections_sent: u64,
    pub started_ms: u64,
    pub reset_ms: Option<u64>,
}

impl Auditor {
    pub fn new(cfg: AuditCfg, now_ms: u64) -> Auditor {
        Auditor {
            cfg,
            started_ms: now_ms,
            ..Default::default()
        }
    }

    fn incident(
        &mut self,
        now_ms: u64,
        kind: Kind,
        lp: &str,
        symbol: &str,
        cl: Option<&str>,
        detail: String,
    ) {
        tracing::warn!(?kind, lp, symbol, cl_ord_id = cl, %detail, "denetim olayı");
        if self.incidents.len() == 500 {
            self.incidents.pop_front();
        }
        self.total_incidents += 1;
        self.incidents.push_back(Incident {
            ts_ms: now_ms,
            kind,
            lp: lp.into(),
            symbol: symbol.into(),
            cl_ord_id: cl.map(Into::into),
            detail,
        });
    }

    /// An order command observed on its way to `lp`.
    pub fn on_order(&mut self, lp: &str, o: &Order, now_ns: u64) {
        self.stats.entry(lp.into()).or_default().orders += 1;
        self.orders.insert(
            o.cl_ord_id.clone(),
            Track {
                lp: lp.into(),
                symbol: o.symbol.clone(),
                side: o.side,
                qty: o.qty,
                limit: o.limit_price,
                sent_ns: now_ns,
                acked: false,
                filled: Fixed::from_int(0),
                done: false,
                gtc: o.tif == domain::TimeInForce::GoodTillCancel,
            },
        );
    }

    /// Cancel/replace of a resting order: the new ClOrdID carries on (fills
    /// report under it), the old one is finished here.
    pub fn on_replace(&mut self, lp: &str, orig: &str, o: &Order, now_ns: u64) {
        let filled = self
            .orders
            .get(orig)
            .map_or(Fixed::from_int(0), |t| t.filled);
        if let Some(t) = self.orders.get_mut(orig) {
            t.done = true;
        }
        self.on_order(lp, o, now_ns);
        if let Some(t) = self.orders.get_mut(&o.cl_ord_id) {
            t.filled = filled;
            t.acked = true; // not a new round trip to judge
        }
    }

    /// Cancel of a resting order: its "Canceled" report comes under the
    /// cancel's ClOrdID; the resting one is finished here.
    pub fn on_cancel(&mut self, lp: &str, cl: &str, orig: &str, now_ns: u64) {
        if let Some(t) = self.orders.remove(orig) {
            self.orders.insert(
                cl.to_string(),
                Track {
                    sent_ns: now_ns,
                    acked: true,
                    done: true,
                    ..t
                },
            );
        } else {
            let _ = lp;
        }
    }

    /// An execution report from `lp`.
    pub fn on_exec(&mut self, lp: &str, x: &Execution, now_ns: u64) {
        let now_ms = now_ns / 1_000_000;
        self.last_exec_ms
            .insert((lp.to_string(), x.symbol.clone()), now_ms);
        let trade = matches!(x.exec_type, ExecType::Trade);
        let last = x.last_qty.filter(|q| q.is_positive());
        if trade && last.is_some() {
            self.seen_exec.insert(format!("{lp}:{}", x.exec_id));
        }
        if trade {
            if let Some(q) = last {
                let k = (lp.to_string(), x.symbol.clone());
                let signed = if x.side == Side::Buy { q } else { -q };
                let e = self.net.entry(k).or_insert(Fixed::from_int(0));
                *e = *e + signed;
            }
        }
        let cl = x.cl_ord_id.clone().unwrap_or_default();
        let Some(t) = self.orders.get_mut(&cl) else {
            // orders the core sent before this auditor started are ours, not foreign
            let ours_at_start =
                cl.starts_with(&self.cfg.prefix) && now_ms.saturating_sub(self.started_ms) < 60_000;
            if trade && last.is_some() && !ours_at_start {
                let detail = format!(
                    "{:?} {} @ {} (cl_ord_id {:?}, order {})",
                    x.side,
                    last.map(|q| q.to_string()).unwrap_or_default(),
                    x.last_px.map(|p| p.to_string()).unwrap_or_default(),
                    x.cl_ord_id,
                    x.order_id
                );
                let sym = x.symbol.clone();
                self.incident(
                    now_ms,
                    Kind::ForeignFill,
                    lp,
                    &sym,
                    x.cl_ord_id.as_deref(),
                    detail,
                );
            }
            return;
        };
        let lat_ms = now_ns.saturating_sub(t.sent_ns) as f64 / 1e6;
        let st = self.stats.entry(lp.into()).or_default();
        if !t.acked {
            t.acked = true;
            push(&mut st.ack, lat_ms);
        }
        let mut found = Vec::new();
        if trade {
            if let Some(q) = last {
                if t.filled.is_zero() {
                    push(&mut st.fill, lat_ms);
                }
                t.filled = t.filled + q;
                st.fills += 1;
                if t.qty < t.filled {
                    found.push((Kind::Overfill, format!("filled {} of {}", t.filled, t.qty)));
                }
                if let (Some(l), Some(px)) = (t.limit, x.last_px) {
                    let bad = match t.side {
                        Side::Buy => l < px,
                        Side::Sell => px < l,
                    };
                    if bad {
                        found.push((
                            Kind::LimitBreach,
                            format!("{:?} limit {l} filled at {px}", t.side),
                        ));
                    }
                }
            }
        }
        let fin = matches!(
            x.status,
            OrderStatus::Filled
                | OrderStatus::Canceled
                | OrderStatus::Rejected
                | OrderStatus::Expired
        );
        if fin && !t.done {
            t.done = true;
            if matches!(x.status, OrderStatus::Rejected) || (t.filled.is_zero() && fin) {
                st.rejects += 1;
            } else if t.filled < t.qty {
                st.partial += 1;
            }
        }
        let (tlp, tsym) = (t.lp.clone(), t.symbol.clone());
        for (k, d) in found {
            self.incident(now_ms, k, &tlp, &tsym, Some(&cl), d);
        }
    }

    /// One trade from the LP's own list (TradeCaptureReport). A trade we never
    /// saw on the wire (web channel, another system) is an incident and joins
    /// the LP net, so the net comparison and the corrections cover it.
    pub fn on_lp_trade(
        &mut self,
        lp: &str,
        exec_id: &str,
        symbol: &str,
        side: Side,
        qty: Fixed,
        px: Fixed,
        now_ms: u64,
    ) {
        self.lp_trades_checked += 1;
        let key = format!("{lp}:{exec_id}");
        if !self.seen_exec.insert(key) {
            return;
        }
        let k = (lp.to_string(), symbol.to_string());
        let signed = if side == Side::Buy { qty } else { -qty };
        let e = self.net.entry(k).or_insert(Fixed::from_int(0));
        *e = *e + signed;
        self.last_exec_ms
            .insert((lp.to_string(), symbol.to_string()), now_ms);
        let d = format!("{side:?} {qty} @ {px} (exec {exec_id}) not seen on our FIX session");
        self.incident(now_ms, Kind::LpTradeUnseen, lp, symbol, None, d);
    }

    /// Marks an ExecID (`lp:exec`) as seen on the wire (restored from disk at start-up).
    pub fn mark_seen(&mut self, key: String) {
        self.seen_exec.insert(key);
    }

    /// What the primary LP holds by our count: baseline + wire fills since.
    /// `None` before the first comparison (no baseline yet). Persisted by the
    /// binary so that a restart does not re-baseline on the core's record and
    /// hide an open mismatch.
    pub fn held(&self) -> Option<BTreeMap<NetKey, Fixed>> {
        let base = self.engine0.as_ref()?;
        let mut out = base.clone();
        for (k, v) in &self.net {
            let e = out.entry(k.clone()).or_insert(Fixed::from_int(0));
            *e = *e + *v;
        }
        Some(out)
    }

    /// Restores [`Self::held`] from a previous run as the baseline.
    pub fn restore_held(&mut self, held: BTreeMap<NetKey, Fixed>) {
        self.engine0 = Some(held);
        self.net.clear();
    }

    /// The core rejected an order before it reached the LP (gateway / session).
    pub fn on_command_rejected(&mut self, cl: &str) {
        if let Some(t) = self.orders.get_mut(cl) {
            t.done = true;
            self.stats.entry(t.lp.clone()).or_default().rejects += 1;
        }
    }

    fn in_flight(&self, lp: &str, symbol: &str) -> bool {
        self.orders
            .values()
            .any(|t| t.open() && t.lp == lp && t.symbol == symbol)
    }

    /// Timeouts and housekeeping.
    pub fn tick(&mut self, now_ns: u64) {
        let now_ms = now_ns / 1_000_000;
        let limit = self.cfg.answer_timeout_ms * 1_000_000;
        let late: Vec<(String, String, String)> = self
            .orders
            .iter_mut()
            .filter(|(_, t)| t.open() && now_ns.saturating_sub(t.sent_ns) > limit)
            .map(|(cl, t)| {
                t.done = true;
                (cl.clone(), t.lp.clone(), t.symbol.clone())
            })
            .collect();
        for (cl, lp, sym) in late {
            let d = format!(
                "no final execution report in {} ms",
                self.cfg.answer_timeout_ms
            );
            self.incident(now_ms, Kind::NoAnswer, &lp, &sym, Some(&cl), d);
        }
        // forget answered orders after a minute
        let keep = 60_000 * 1_000_000;
        self.orders
            .retain(|_, t| !t.done || now_ns.saturating_sub(t.sent_ns) < keep);
        // a GTC order marks itself done on its final report, so the retain
        // above never drops a live resting order
    }

    /// Compares what the primary LP must hold with what it holds.
    ///
    /// `engine`: the core's cumulative net per (LP, LP symbol). Expected at the
    /// primary = core's primary net + everything booked at staging LPs. Held
    /// at the primary = the core's primary net at start (baseline) + the fills
    /// observed on the wire since. Returns the corrections (signed LP qty to
    /// trade at the primary) for mismatches that settled.
    pub fn compare(&mut self, engine: &BTreeMap<NetKey, Fixed>, now_ms: u64) -> Vec<Correction> {
        self.compare_at(engine, u64::MAX, now_ms)
    }

    /// [`Auditor::compare`] with the time the core's record was written
    /// (`engine_ms`): symbols with LP activity after it are not judged yet.
    pub fn compare_at(
        &mut self,
        engine: &BTreeMap<NetKey, Fixed>,
        engine_ms: u64,
        now_ms: u64,
    ) -> Vec<Correction> {
        let zero = Fixed::from_int(0);
        let primary = self.cfg.primary.clone();
        let base = self
            .engine0
            .get_or_insert_with(|| {
                engine
                    .iter()
                    .filter(|((lp, _), _)| *lp == primary)
                    .map(|(k, v)| (k.clone(), *v))
                    .collect()
            })
            .clone();
        let mut symbols: Vec<String> = engine
            .keys()
            .chain(self.net.keys())
            .map(|(_, s)| s.clone())
            .collect();
        symbols.sort();
        symbols.dedup();
        let mut out = Vec::new();
        for sym in symbols {
            let k = (primary.clone(), sym.clone());
            let mut expected = engine.get(&k).copied().unwrap_or(zero);
            for st in &self.cfg.staging {
                expected = expected
                    + engine
                        .get(&(st.clone(), sym.clone()))
                        .copied()
                        .unwrap_or(zero);
            }
            let held =
                base.get(&k).copied().unwrap_or(zero) + self.net.get(&k).copied().unwrap_or(zero);
            let stale = self.last_exec_ms.get(&k).is_some_and(|t| *t > engine_ms);
            if expected == held || stale || self.in_flight(&primary, &sym) {
                self.mismatch_since.remove(&k);
                if expected == held {
                    self.reported.remove(&k);
                }
                continue;
            }
            let since = *self.mismatch_since.entry(k.clone()).or_insert(now_ms);
            if now_ms.saturating_sub(since) < self.cfg.settle_ms {
                continue;
            }
            let diff = expected - held;
            if self.reported.get(&k) != Some(&diff) {
                self.reported.insert(k.clone(), diff);
                let d =
                    format!("{primary} must hold {expected}, holds {held}: trade {diff} to fix");
                self.incident(now_ms, Kind::NetMismatch, &primary, &sym, None, d);
            }
            out.push(Correction {
                lp: primary.clone(),
                symbol: sym,
                qty: diff,
            });
        }
        out
    }

    /// Zero point: from now on the primary LP is taken to hold exactly what
    /// the core expects (both sides were flattened by hand); earlier
    /// history is closed. Counters, latency and the incident log are kept.
    pub fn reset(&mut self, engine: &BTreeMap<NetKey, Fixed>, now_ms: u64) {
        self.reset_with(engine, now_ms, false)
    }

    /// [`Auditor::reset`]; `quiet` = re-applying a zero point taken earlier
    /// (process restart): no log line, no incident for open exposure.
    pub fn reset_with(&mut self, engine: &BTreeMap<NetKey, Fixed>, now_ms: u64, quiet: bool) {
        let zero = Fixed::from_int(0);
        let mut base: BTreeMap<NetKey, Fixed> = BTreeMap::new();
        for ((lp, sym), v) in engine {
            if *lp == self.cfg.primary || self.cfg.staging.contains(lp) {
                let e = base
                    .entry((self.cfg.primary.clone(), sym.clone()))
                    .or_insert(zero);
                *e = *e + *v;
            }
        }
        let open: Vec<String> = base
            .iter()
            .filter(|(_, v)| !v.is_zero())
            .map(|((_, s), v)| format!("{s} {v}"))
            .collect();
        self.engine0 = Some(base);
        self.net.clear();
        self.mismatch_since.clear();
        self.reported.clear();
        if quiet {
            return;
        }
        self.reset_ms = Some(now_ms);
        tracing::warn!(?open, "denetim sıfır noktası");
        if !open.is_empty() {
            // not flat: still a valid zero point, but say so
            self.incident(
                now_ms,
                Kind::NetMismatch,
                &self.cfg.primary.clone(),
                "*",
                None,
                format!(
                    "zero point taken with open core exposure: {}",
                    open.join(", ")
                ),
            );
        }
    }

    /// Records a correction order the auditor sent (tracked like any order).
    pub fn on_correction(&mut self, c: &Correction, cl_ord_id: &str, now_ns: u64) {
        let side = if c.qty.is_positive() {
            Side::Buy
        } else {
            Side::Sell
        };
        let qty = if c.qty.is_positive() { c.qty } else { -c.qty };
        self.corrections_sent += 1;
        self.on_order(
            &c.lp,
            &Order {
                cl_ord_id: cl_ord_id.into(),
                symbol: c.symbol.clone(),
                side,
                qty,
                ord_type: domain::OrderType::Market,
                limit_price: None,
                stop_price: None,
                tif: domain::TimeInForce::ImmediateOrCancel,
            },
            now_ns,
        );
        self.mismatch_since
            .remove(&(c.lp.clone(), c.symbol.clone()));
    }

    pub fn status(&mut self, now_ms: u64) -> serde_json::Value {
        for s in self.stats.values_mut() {
            s.ack_ms_p50 = pct(&s.ack, 0.5);
            s.ack_ms_p99 = pct(&s.ack, 0.99);
            s.fill_ms_p50 = pct(&s.fill, 0.5);
            s.fill_ms_p99 = pct(&s.fill, 0.99);
        }
        let open_mismatch: Vec<serde_json::Value> = self
            .reported
            .iter()
            .map(|(k, d)| serde_json::json!({"lp": k.0, "symbol": k.1, "diff": d.to_string()}))
            .collect();
        let in_flight = self.orders.values().filter(|t| !t.done).count();
        serde_json::json!({
            "at": now_ms,
            "startedMs": self.started_ms,
            "ok": open_mismatch.is_empty(),
            "inFlight": in_flight,
            "lps": self.stats,
            "net": self.net.iter().map(|(k, v)| serde_json::json!({"lp": k.0, "symbol": k.1, "qty": v.to_string()})).collect::<Vec<_>>(),
            "openMismatches": open_mismatch,
            "incidentsTotal": self.total_incidents,
            "lpTradesChecked": self.lp_trades_checked,
            "correctionsSent": self.corrections_sent,
            "resetMs": self.reset_ms,
            "incidents": self.incidents.iter().rev().take(100).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{OrderType, TimeInForce};

    fn px(s: &str) -> Fixed {
        s.parse().unwrap()
    }
    fn order(cl: &str, side: Side, qty: &str, limit: Option<&str>) -> Order {
        Order {
            cl_ord_id: cl.into(),
            symbol: "EUR/USD".into(),
            side,
            qty: px(qty),
            ord_type: if limit.is_some() {
                OrderType::Limit
            } else {
                OrderType::Market
            },
            limit_price: limit.map(px),
            stop_price: None,
            tif: TimeInForce::ImmediateOrCancel,
        }
    }
    fn exec(
        cl: Option<&str>,
        side: Side,
        last: Option<(&str, &str)>,
        status: OrderStatus,
    ) -> Execution {
        Execution {
            lp: "LMAX".into(),
            symbol: "EUR/USD".into(),
            order_id: "o1".into(),
            cl_ord_id: cl.map(Into::into),
            orig_cl_ord_id: None,
            exec_id: "e".into(),
            exec_type: if last.is_some() {
                ExecType::Trade
            } else {
                ExecType::New
            },
            status,
            side,
            last_qty: last.map(|(q, _)| px(q)),
            last_px: last.map(|(_, p)| px(p)),
            cum_qty: px("0"),
            leaves_qty: px("0"),
            avg_px: None,
            text: None,
            ts_recv_ns: 0,
        }
    }
    const MS: u64 = 1_000_000;

    fn key(lp: &str, sym: &str) -> NetKey {
        (lp.into(), sym.into())
    }

    #[test]
    fn healthy_flow_measures_latency_and_matches_the_core() {
        let mut a = Auditor::new(AuditCfg::default(), 0);
        let mut core = BTreeMap::new();
        core.insert(key("LMAX", "EUR/USD"), px("3")); // held before start
        assert!(a.compare(&core, 0).is_empty()); // baseline
        a.on_order("LMAX", &order("LP-1", Side::Buy, "1", Some("1.10020")), 0);
        a.on_exec(
            "LMAX",
            &exec(Some("LP-1"), Side::Buy, None, OrderStatus::New),
            40 * MS,
        );
        a.on_exec(
            "LMAX",
            &exec(
                Some("LP-1"),
                Side::Buy,
                Some(("1", "1.10010")),
                OrderStatus::Filled,
            ),
            72 * MS,
        );
        core.insert(key("LMAX", "EUR/USD"), px("4"));
        a.tick(10_000 * MS);
        assert!(a.compare(&core, 10_000).is_empty());
        assert!(a.compare(&core, 20_000).is_empty());
        let s = a.status(20_000);
        assert_eq!(s["ok"], true);
        assert_eq!(a.total_incidents, 0);
        assert_eq!(s["lps"]["LMAX"]["ack_ms_p50"], 40.0);
        assert_eq!(s["lps"]["LMAX"]["fill_ms_p50"], 72.0);
    }

    #[test]
    fn held_net_survives_a_restart_so_the_mismatch_is_not_hidden() {
        let mut a = Auditor::new(AuditCfg::default(), 0);
        let mut core = BTreeMap::new();
        core.insert(key("LMAX", "EUR/USD"), px("0"));
        assert!(a.compare(&core, 0).is_empty()); // baseline 0
                                                 // the LP filled a buy the core never booked at LMAX (booked at SIM)
        a.on_order("LMAX", &order("LP-9", Side::Buy, "1", None), 0);
        a.on_exec(
            "LMAX",
            &exec(
                Some("LP-9"),
                Side::Buy,
                Some(("1", "4140")),
                OrderStatus::Filled,
            ),
            MS,
        );
        core.insert(key("SIM", "EUR/USD"), px("-1"));
        core.insert(key("LMAX", "EUR/USD"), px("1"));
        a.tick(10_000 * MS);
        assert!(a.compare(&core, 10_000).is_empty(), "first look settles");
        assert_eq!(
            a.compare(&core, 20_000).len(),
            1,
            "LMAX holds 1, must hold 0"
        );
        let held = a.held().expect("baseline");
        assert_eq!(held[&key("LMAX", "EUR/USD")], px("1"));
        // restart: a fresh auditor with the persisted held-net keeps seeing it
        let mut b = Auditor::new(AuditCfg::default(), 30_000);
        b.restore_held(held);
        assert!(b.compare(&core, 30_000).is_empty(), "first look settles");
        b.tick(40_000 * MS);
        let c = b.compare(&core, 40_000);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].qty, px("-1"));
    }

    #[test]
    fn staging_exposure_is_moved_to_the_primary_and_the_correction_clears_it() {
        // today: the core booked GBP/USD -1 at SIM; LMAX never saw it
        let mut a = Auditor::new(AuditCfg::default(), 0);
        let mut core = BTreeMap::new();
        core.insert(key("LMAX", "GBP/USD"), px("-3"));
        core.insert(key("SIM", "GBP/USD"), px("-1"));
        assert!(a.compare(&core, 0).is_empty(), "first look: wait to settle");
        assert!(a.compare(&core, 3_000).is_empty());
        let c = a.compare(&core, 7_000);
        assert_eq!(
            c,
            vec![Correction {
                lp: "LMAX".into(),
                symbol: "GBP/USD".into(),
                qty: px("-1")
            }]
        );
        assert_eq!(a.total_incidents, 1);
        assert_eq!(a.status(7_000)["ok"], false);
        // the auditor sells 1 at LMAX; while in flight nothing new is proposed
        a.on_correction(&c[0], "AUD-1", 7_000 * MS);
        assert!(a.compare(&core, 8_000).is_empty());
        let mut x = exec(
            Some("AUD-1"),
            Side::Sell,
            Some(("1", "1.32300")),
            OrderStatus::Filled,
        );
        x.symbol = "GBP/USD".into();
        a.on_exec("LMAX", &x, 8_100 * MS);
        assert!(a.compare(&core, 9_000).is_empty());
        assert!(a.compare(&core, 20_000).is_empty());
        assert_eq!(a.status(20_000)["ok"], true);
        assert_eq!(
            a.total_incidents, 1,
            "no foreign fill: the correction was ours"
        );
    }

    #[test]
    fn foreign_fill_overfill_limit_breach_and_no_answer() {
        let mut a = Auditor::new(AuditCfg::default(), 0);
        // a fill nobody here ordered (manual trade, another system)
        a.on_exec(
            "LMAX",
            &exec(
                Some("YUK1-0"),
                Side::Buy,
                Some(("0.1", "1.1")),
                OrderStatus::Filled,
            ),
            MS,
        );
        // limit breached and overfilled
        a.on_order("LMAX", &order("LP-2", Side::Sell, "1", Some("1.10000")), 0);
        a.on_exec(
            "LMAX",
            &exec(
                Some("LP-2"),
                Side::Sell,
                Some(("1.5", "1.09990")),
                OrderStatus::Filled,
            ),
            2 * MS,
        );
        // never answered
        a.on_order("LMAX", &order("LP-3", Side::Buy, "1", None), 0);
        a.tick(6_000 * MS);
        let kinds: Vec<Kind> = a.incidents.iter().map(|i| i.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                Kind::ForeignFill,
                Kind::Overfill,
                Kind::LimitBreach,
                Kind::NoAnswer
            ]
        );
    }

    #[test]
    fn a_mismatch_is_reported_once_until_it_changes() {
        let mut a = Auditor::new(AuditCfg::default(), 0);
        let mut core = BTreeMap::new();
        a.compare(&core, 0);
        core.insert(key("LMAX", "EUR/USD"), px("1")); // core booked, LP never filled
        a.compare(&core, 1_000);
        assert_eq!(a.total_incidents, 0, "not before it settles");
        assert_eq!(a.compare(&core, 8_000).len(), 1);
        assert_eq!(a.compare(&core, 9_000).len(), 1, "still proposed");
        assert_eq!(a.total_incidents, 1, "reported once");
    }

    #[test]
    fn zero_point_closes_history_and_tracks_from_there() {
        let mut a = Auditor::new(AuditCfg::default(), 0);
        let mut core = BTreeMap::new();
        core.insert(key("LMAX", "GBP/USD"), px("-3"));
        core.insert(key("SIM", "GBP/USD"), px("-1"));
        a.compare(&core, 0);
        assert_eq!(a.compare(&core, 7_000).len(), 1, "old mismatch");
        // everything was closed by hand on both sides; the core shows flat
        // nets per LP that still sum the old legs (closes went to LMAX)
        core.insert(key("LMAX", "GBP/USD"), px("1"));
        a.reset(&core, 8_000);
        assert!(
            a.compare(&core, 20_000).is_empty(),
            "flat after the zero point"
        );
        assert_eq!(a.status(20_000)["ok"], true);
        // a new order the LP never fills shows up again
        core.insert(key("LMAX", "GBP/USD"), px("2"));
        a.compare(&core, 21_000);
        assert_eq!(a.compare(&core, 28_000).len(), 1);
    }

    #[test]
    fn a_stale_core_record_is_not_judged() {
        let mut a = Auditor::new(AuditCfg::default(), 0);
        let core = BTreeMap::new();
        a.compare_at(&core, 0, 0);
        // LP filled at 1 s; the core's snapshot is still from 0.5 s
        let mut x = exec(None, Side::Sell, Some(("1", "1.32")), OrderStatus::Filled);
        x.symbol = "GBP/USD".into();
        a.on_exec("LMAX", &x, 1_000 * MS);
        a.compare_at(&core, 500, 2_000);
        a.compare_at(&core, 500, 20_000);
        assert_eq!(
            a.total_incidents, 1,
            "only the foreign fill, no net mismatch on a stale record"
        );
        // the core's record catches up with the fill
        let mut c2 = BTreeMap::new();
        c2.insert(key("LMAX", "GBP/USD"), px("-1"));
        a.compare_at(&c2, 21_000, 21_000);
        assert_eq!(a.status(21_000)["ok"], true);
    }

    #[test]
    fn lp_trade_list_finds_web_trades_and_ignores_what_we_saw() {
        let mut a = Auditor::new(AuditCfg::default(), 0);
        let core = BTreeMap::new();
        a.compare(&core, 0);
        a.on_order("LMAX", &order("LP-1", Side::Buy, "1", None), 0);
        let mut x = exec(
            Some("LP-1"),
            Side::Buy,
            Some(("1", "1.1")),
            OrderStatus::Filled,
        );
        x.exec_id = "E1".into();
        a.on_exec("LMAX", &x, MS);
        // the LP's list: our fill (known) + a web trade (unseen)
        a.on_lp_trade(
            "LMAX",
            "E1",
            "EUR/USD",
            Side::Buy,
            px("1"),
            px("1.1"),
            2_000,
        );
        a.on_lp_trade(
            "LMAX",
            "W7",
            "EUR/USD",
            Side::Sell,
            px("2"),
            px("1.1"),
            2_000,
        );
        a.on_lp_trade(
            "LMAX",
            "W7",
            "EUR/USD",
            Side::Sell,
            px("2"),
            px("1.1"),
            3_000,
        ); // repeated list: once
        assert_eq!(a.total_incidents, 1);
        assert_eq!(a.incidents[0].kind, Kind::LpTradeUnseen);
        // the web trade is now part of the LP net: core (+1) vs LP (1 - 2 = -1)
        let mut c = BTreeMap::new();
        c.insert(key("LMAX", "EUR/USD"), px("1"));
        a.compare_at(&c, 5_000, 5_000);
        let fix = a.compare_at(&c, 12_000, 12_000);
        assert_eq!(
            fix,
            vec![Correction {
                lp: "LMAX".into(),
                symbol: "EUR/USD".into(),
                qty: px("2")
            }]
        );
    }

    #[test]
    fn resting_gtc_order_is_answered_by_its_ack_and_not_in_flight() {
        let mut a = Auditor::new(AuditCfg::default(), 0);
        let mut core = BTreeMap::new();
        a.compare(&core, 0);
        let mut o = order("LP-5", Side::Sell, "1", Some("1.10105"));
        o.tif = TimeInForce::GoodTillCancel;
        a.on_order("LMAX", &o, 0);
        a.on_exec(
            "LMAX",
            &exec(Some("LP-5"), Side::Sell, None, OrderStatus::New),
            30 * MS,
        );
        // no "no answer" after the timeout: it rests
        a.tick(20_000 * MS);
        assert_eq!(a.total_incidents, 0);
        // and it does not hold the net comparison of the symbol
        core.insert(key("LMAX", "EUR/USD"), px("1"));
        assert!(a.compare(&core, 7_000).is_empty()); // settle
        assert_eq!(a.compare(&core, 20_000).len(), 1, "mismatch still judged");
        // replace: fills under the new ClOrdID are ours, not foreign
        let mut r = order("LP-5-r1", Side::Sell, "1", Some("1.10205"));
        r.tif = TimeInForce::GoodTillCancel;
        a.on_replace("LMAX", "LP-5", &r, 40 * MS);
        let mut x = exec(
            Some("LP-5-r1"),
            Side::Sell,
            Some(("1", "1.10205")),
            OrderStatus::Filled,
        );
        x.exec_id = "fill-r1".into();
        a.on_exec("LMAX", &x, 50 * MS);
        assert_eq!(a.total_incidents, 1, "no ForeignFill incident");
        assert_eq!(a.status(60_000)["net"][0]["qty"], "-1");
    }

    #[test]
    fn in_flight_orders_hold_the_comparison() {
        let mut a = Auditor::new(AuditCfg::default(), 0);
        let mut core = BTreeMap::new();
        a.compare(&core, 0);
        a.on_order("LMAX", &order("LP-9", Side::Buy, "1", None), 0);
        core.insert(key("LMAX", "EUR/USD"), px("1")); // core booked, LP not yet
        assert!(a.compare(&core, 7_000).is_empty());
        assert!(a.compare(&core, 14_000).is_empty());
        assert_eq!(a.total_incidents, 0);
    }
}
