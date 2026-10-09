//! Multi-LP aggregation (bridge stage 6).
//!
//! Every LP bridge pushes its raw book (core symbol, prices, sizes in lots)
//! into one shared [`Aggregator`]. The aggregator keeps the per-LP books,
//! applies the per-LP policy (kill switch, symbol allow-list, size limits,
//! price-deviation guard) and derives
//!
//! * the aggregated top of book that the engine journals (`Command::Quote`),
//! * the merged depth shown to clients, and
//! * the LP candidates for an outgoing omnibus order, in the order the
//!   router should try them (best price / VWAP / priority / round-robin).
//!
//! The engine itself stays single-book: which LP took an order arrives back
//! as the journaled `Command::LpRouted`, so replay reproduces the LP tag
//! without talking to anybody.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

use money::{Price, Qty};
use risk::Side;
use serde::{Deserialize, Serialize};

/// How an order picks its LP among the eligible ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AggMode {
    /// Best top-of-book price for the side (ties: priority).
    #[default]
    BestPrice,
    /// Best volume-weighted price over the LP's depth for the order size.
    Vwap,
    /// Lowest `priority` number first; the others are fail-over only.
    Priority,
    /// Spread orders evenly over the eligible LPs.
    RoundRobin,
}

/// Per-LP policy (console: LP page -> Aggregation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LpPolicy {
    /// `GatewayConfig.lp` of the session.
    pub name: String,
    /// Kill switch: `false` = no orders and no contribution to prices.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// `false` = quote-only: contributes to prices and backs up the feed but
    /// never takes an order (a simulator, a reference feed). With no LP left
    /// to take an order the engine rejects it instead of filling it here.
    #[serde(default = "yes")]
    pub orders: bool,
    /// 1 = preferred. Also the tie-breaker of the other modes.
    #[serde(default = "one")]
    pub priority: u32,
    /// Smallest / largest omnibus order (lots) this LP takes; `None` = no limit.
    #[serde(default)]
    pub min_lots: Option<Qty>,
    #[serde(default)]
    pub max_lots: Option<Qty>,
    /// Core symbols this LP may trade / quote; empty = all it quotes.
    #[serde(default)]
    pub symbols: Vec<String>,
}

fn yes() -> bool {
    true
}
fn one() -> u32 {
    1
}

impl LpPolicy {
    pub fn new(name: impl Into<String>, priority: u32) -> LpPolicy {
        LpPolicy {
            name: name.into(),
            enabled: true,
            orders: true,
            priority,
            min_lots: None,
            max_lots: None,
            symbols: Vec::new(),
        }
    }

    fn allows_symbol(&self, symbol: &str) -> bool {
        self.symbols.is_empty() || self.symbols.iter().any(|s| s == symbol)
    }

    fn allows_size(&self, lots: Qty) -> bool {
        self.min_lots.is_none_or(|m| lots >= m) && self.max_lots.is_none_or(|m| lots <= m)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggConfig {
    #[serde(default)]
    pub mode: AggMode,
    /// Unknown LPs (a session without a policy) behave as enabled, priority 100.
    #[serde(default)]
    pub lps: Vec<LpPolicy>,
    /// Price-sanity guard: an LP whose mid deviates from the reference LP's
    /// mid by more than this many points is ignored for the symbol (quotes
    /// and orders) until it comes back. 0 = off. Reference = enabled LP with
    /// the lowest priority number that quotes the symbol. Default 100 points
    /// (10 pips on 5-digit FX): a stale or broken LP can never price clients
    /// off-market just because nobody saved a policy.
    #[serde(default = "default_deviation")]
    pub max_deviation_points: i64,
    /// Silent-LP guard: an LP that has sent nothing for this long while
    /// another LP keeps quoting is left out (quotes and orders) until it
    /// quotes again; a dropped session can no longer price clients with its
    /// last book. Measured against the freshest LP. The deviation guard still
    /// measures every remaining LP against the highest-priority LP's last
    /// price, so a backup LP can never move prices further than
    /// `max_deviation_points` from the last real price. 0 = off (default):
    /// turn it on only when the backup LPs are real markets with the same
    /// trading hours; a 24/7 simulator next to a real LP must not price
    /// clients while the real market is closed.
    #[serde(default = "default_quote_age")]
    pub max_quote_age_ms: u64,
    /// Feed QoS guard: an LP whose market-data latency (receive −
    /// `SendingTime`, smoothed in the gateway) is above this is left out
    /// (quotes and orders) until it recovers. 0 = off (default).
    #[serde(default)]
    pub max_latency_ms: u64,
}

pub const DEFAULT_QUOTE_AGE_MS: u64 = 0;

fn default_quote_age() -> u64 {
    DEFAULT_QUOTE_AGE_MS
}

pub const DEFAULT_DEVIATION_POINTS: i64 = 100;

fn default_deviation() -> i64 {
    DEFAULT_DEVIATION_POINTS
}

impl Default for AggConfig {
    fn default() -> AggConfig {
        AggConfig {
            mode: AggMode::BestPrice,
            lps: Vec::new(),
            max_deviation_points: DEFAULT_DEVIATION_POINTS,
            max_quote_age_ms: DEFAULT_QUOTE_AGE_MS,
            max_latency_ms: 0,
        }
    }
}

impl AggConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.lps.len() > 32 {
            return Err("too many LPs".into());
        }
        if !(0..=1_000_000).contains(&self.max_deviation_points) {
            return Err("maxDeviationPoints out of range".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for p in &self.lps {
            let n = p.name.trim();
            if n.is_empty() || n.len() > 32 {
                return Err("LP name must be 1..32 characters".into());
            }
            if !names.insert(n) {
                return Err(format!("duplicate LP {n}"));
            }
            if p.priority == 0 || p.priority > 1000 {
                return Err(format!("{n}: priority must be 1..1000"));
            }
            if let (Some(a), Some(b)) = (p.min_lots, p.max_lots) {
                if a > b {
                    return Err(format!("{n}: minLots > maxLots"));
                }
            }
            if p.min_lots.is_some_and(|m| m.raw() < 0) || p.max_lots.is_some_and(|m| m.raw() <= 0) {
                return Err(format!("{n}: lot limits must be positive"));
            }
            if p.symbols.len() > 500 {
                return Err(format!("{n}: too many symbols"));
            }
        }
        Ok(())
    }

    fn policy(&self, lp: &str) -> LpPolicy {
        self.lps
            .iter()
            .find(|p| p.name == lp)
            .cloned()
            .unwrap_or_else(|| LpPolicy::new(lp, 100))
    }
}

/// One LP's book for one symbol, in core prices and lots (best first).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LpBook {
    pub bids: Vec<(Price, Qty)>,
    pub asks: Vec<(Price, Qty)>,
    pub ts_ns: u64,
}

impl LpBook {
    fn best(&self, side: Side) -> Option<Price> {
        match side {
            Side::Buy => self.asks.first().map(|l| l.0),
            Side::Sell => self.bids.first().map(|l| l.0),
        }
    }

    fn mid(&self) -> Option<i128> {
        Some((self.bids.first()?.0.raw() as i128 + self.asks.first()?.0.raw() as i128) / 2)
    }

    /// Volume-weighted price to fill `lots` on `side` against this book;
    /// `None` when the depth cannot fill it.
    fn vwap(&self, side: Side, lots: Qty) -> Option<Price> {
        let levels = match side {
            Side::Buy => &self.asks,
            Side::Sell => &self.bids,
        };
        if lots.raw() <= 0 {
            return levels.first().map(|l| l.0);
        }
        let (mut left, mut notional) = (lots.raw() as i128, 0i128);
        for (p, q) in levels {
            let take = (q.raw() as i128).min(left);
            notional += take * p.raw() as i128;
            left -= take;
            if left == 0 {
                return Some(Price::from_raw((notional / lots.raw() as i128) as i64));
            }
        }
        None
    }
}

/// Runtime view of one LP for the console.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LpRuntime {
    pub name: String,
    pub policy: LpPolicy,
    /// Symbols with a current book.
    pub quoting: usize,
    /// Symbols currently excluded by the deviation guard.
    pub deviating: Vec<String>,
    /// Left out by the silent-LP guard (no quote for `max_quote_age_ms`).
    #[serde(default)]
    pub silent: bool,
    pub last_quote_ns: u64,
    /// Market-data latency reported by the gateway, ms (0 = unknown).
    #[serde(default)]
    pub latency_ms: u64,
    /// Left out by the feed-QoS guard (`latency_ms` > `max_latency_ms`).
    #[serde(default)]
    pub slow: bool,
}

#[derive(Default)]
struct Books {
    /// core symbol -> lp -> book
    by_symbol: BTreeMap<String, BTreeMap<String, LpBook>>,
    /// lp -> last quote time
    last: BTreeMap<String, u64>,
    /// lp -> market-data latency reported by the gateway (ms)
    latency: BTreeMap<String, u64>,
    /// symbol -> point (raw price units), for the deviation guard
    points: BTreeMap<String, i64>,
    /// last aggregated top of book sent to the engine, per symbol
    sent: BTreeMap<String, (Price, Price)>,
}

/// Shared between the LP bridges, the router and the admin API.
pub struct Aggregator {
    cfg: RwLock<AggConfig>,
    books: RwLock<Books>,
    rr: AtomicUsize,
}

impl std::fmt::Debug for Aggregator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aggregator")
            .field("cfg", &self.config())
            .finish_non_exhaustive()
    }
}

impl Default for Aggregator {
    fn default() -> Aggregator {
        Aggregator::new(AggConfig::default())
    }
}

impl Aggregator {
    pub fn new(cfg: AggConfig) -> Aggregator {
        Aggregator {
            cfg: RwLock::new(cfg),
            books: RwLock::new(Books::default()),
            rr: AtomicUsize::new(0),
        }
    }

    pub fn config(&self) -> AggConfig {
        self.cfg.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replaces the policy; the next quote of every symbol re-aggregates
    /// (a symbol whose aggregate changes is pushed on its next LP tick).
    pub fn set_config(&self, cfg: AggConfig) {
        *self.cfg.write().unwrap_or_else(|e| e.into_inner()) = cfg;
        self.books
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .sent
            .clear();
    }

    /// Market-data latency of an LP as the gateway measured it (ms).
    pub fn set_latency(&self, lp: &str, latency_ms: u64) {
        self.books
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .latency
            .insert(lp.to_string(), latency_ms);
    }

    /// Point size of a symbol (raw price units), needed by the deviation guard.
    pub fn set_point(&self, symbol: &str, point: Price) {
        self.books
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .points
            .insert(symbol.to_string(), point.raw());
    }

    /// Stores an LP book; returns the new aggregated top of book when it
    /// differs from the last one returned for the symbol (`None` = unchanged
    /// or no eligible quote).
    pub fn update(&self, lp: &str, symbol: &str, book: LpBook) -> Option<(Price, Price)> {
        let cfg = self.config();
        let mut b = self.books.write().unwrap_or_else(|e| e.into_inner());
        // monotonic: a republished old quote (NATS re-sends each symbol's last
        // quote with its original timestamp) must never make a live LP look silent
        let last = b.last.entry(lp.to_string()).or_insert(0);
        *last = (*last).max(book.ts_ns);
        b.by_symbol
            .entry(symbol.to_string())
            .or_default()
            .insert(lp.to_string(), book);
        let eligible = eligible(&cfg, &b, symbol, None);
        let bid = eligible
            .iter()
            .filter_map(|(_, bk)| bk.best(Side::Sell))
            .max()?;
        let ask = eligible
            .iter()
            .filter_map(|(_, bk)| bk.best(Side::Buy))
            .min()?;
        // Two LPs can cross each other for a moment; the engine refuses a
        // crossed quote, so fall back to the reference LP alone.
        let (bid, ask) = if bid >= ask {
            let r = &eligible.first()?.1;
            (r.best(Side::Sell)?, r.best(Side::Buy)?)
        } else {
            (bid, ask)
        };
        if b.sent.get(symbol) == Some(&(bid, ask)) {
            return None;
        }
        b.sent.insert(symbol.to_string(), (bid, ask));
        Some((bid, ask))
    }

    /// Merged depth of the eligible LPs (bids best first, asks best first),
    /// levels of equal price combined.
    pub fn merged(&self, symbol: &str) -> LpBook {
        let cfg = self.config();
        let b = self.books.read().unwrap_or_else(|e| e.into_inner());
        let eligible = eligible(&cfg, &b, symbol, None);
        let mut bids: BTreeMap<i64, i64> = BTreeMap::new();
        let mut asks: BTreeMap<i64, i64> = BTreeMap::new();
        let mut ts = 0;
        for (_, bk) in &eligible {
            ts = ts.max(bk.ts_ns);
            for (p, q) in &bk.bids {
                *bids.entry(p.raw()).or_default() += q.raw();
            }
            for (p, q) in &bk.asks {
                *asks.entry(p.raw()).or_default() += q.raw();
            }
        }
        LpBook {
            bids: bids
                .into_iter()
                .rev()
                .map(|(p, q)| (Price::from_raw(p), Qty::from_raw(q)))
                .collect(),
            asks: asks
                .into_iter()
                .map(|(p, q)| (Price::from_raw(p), Qty::from_raw(q)))
                .collect(),
            ts_ns: ts,
        }
    }

    /// The LP a resting (GTC) order of `symbol` goes to: the enabled,
    /// order-taking policy LP of highest priority that allows the symbol —
    /// whether or not it has quoted yet (a fresh process). `None` = nobody.
    pub fn resting_lp(&self, symbol: &str) -> Option<String> {
        let cfg = self.config();
        let mut lps: Vec<&LpPolicy> = cfg
            .lps
            .iter()
            .filter(|p| p.enabled && p.orders && p.allows_symbol(symbol))
            .collect();
        lps.sort_by_key(|p| (p.priority, p.name.clone()));
        lps.first().map(|p| p.name.clone())
    }

    /// LPs to try for an order, best first. Empty = nobody eligible.
    pub fn choose(&self, symbol: &str, side: Side, lots: Qty) -> Vec<String> {
        let cfg = self.config();
        let b = self.books.read().unwrap_or_else(|e| e.into_inner());
        let mut cands = eligible(&cfg, &b, symbol, Some(lots));
        cands.retain(|(lp, _)| cfg.policy(lp).orders);
        if cands.is_empty() {
            return Vec::new();
        }
        let prio = |lp: &str| cfg.policy(lp).priority;
        let better = |a: Price, b: Price| match side {
            Side::Buy => a.cmp(&b),
            Side::Sell => b.cmp(&a),
        };
        match cfg.mode {
            AggMode::Priority => cands.sort_by_key(|(lp, _)| (prio(lp), lp.clone())),
            AggMode::BestPrice => cands.sort_by(|(la, a), (lb, b)| {
                let (pa, pb) = (a.best(side), b.best(side));
                match (pa, pb) {
                    (Some(pa), Some(pb)) => better(pa, pb),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
                .then_with(|| (prio(la), la).cmp(&(prio(lb), lb)))
            }),
            AggMode::Vwap => cands.sort_by(|(la, a), (lb, b)| {
                // A book that cannot fill the size loses to one that can.
                let (pa, pb) = (a.vwap(side, lots), b.vwap(side, lots));
                match (pa, pb) {
                    (Some(pa), Some(pb)) => better(pa, pb),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
                .then_with(|| (prio(la), la).cmp(&(prio(lb), lb)))
            }),
            AggMode::RoundRobin => {
                cands.sort_by_key(|(lp, _)| (prio(lp), lp.clone()));
                let n = self.rr.fetch_add(1, Ordering::Relaxed) % cands.len();
                cands.rotate_left(n);
            }
        }
        cands.into_iter().map(|(lp, _)| lp).collect()
    }

    /// Console view: every LP that has a policy or has quoted.
    pub fn runtime(&self) -> Vec<LpRuntime> {
        let cfg = self.config();
        let b = self.books.read().unwrap_or_else(|e| e.into_inner());
        let mut names: Vec<String> = cfg.lps.iter().map(|p| p.name.clone()).collect();
        for n in b.last.keys() {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
        names
            .into_iter()
            .map(|name| {
                let mut quoting = 0;
                let mut deviating = Vec::new();
                for (sym, lps) in &b.by_symbol {
                    // an empty book (LP sends no levels for the symbol) is "not
                    // quoting", not a price deviation
                    let Some(bk) = lps.get(&name) else {
                        continue;
                    };
                    if bk.bids.is_empty() && bk.asks.is_empty() {
                        continue;
                    }
                    quoting += 1;
                    let ok = eligible(&cfg, &b, sym, None)
                        .iter()
                        .any(|(lp, _)| *lp == name);
                    let p = cfg.policy(&name);
                    if !ok && p.enabled && p.allows_symbol(sym) && is_fresh(&cfg, &b, &name) {
                        deviating.push(sym.clone());
                    }
                }
                LpRuntime {
                    policy: cfg.policy(&name),
                    quoting,
                    deviating,
                    silent: !is_fresh(&cfg, &b, &name),
                    last_quote_ns: b.last.get(&name).copied().unwrap_or(0),
                    latency_ms: b.latency.get(&name).copied().unwrap_or(0),
                    slow: is_slow(&cfg, &b, &name),
                    name,
                }
            })
            .collect()
    }
}

/// Eligible (lp, book) pairs of a symbol: enabled, symbol allowed, size within
/// limits (when `lots` is given), within the deviation guard. The reference
/// LP comes first.
/// Silent-LP guard: has `lp` quoted (anything) within `max_quote_age_ms` of
/// the freshest LP?
fn is_fresh(cfg: &AggConfig, b: &Books, lp: &str) -> bool {
    let age_ns = cfg.max_quote_age_ms.saturating_mul(1_000_000);
    if age_ns == 0 {
        return true;
    }
    let newest = b.last.values().copied().max().unwrap_or(0);
    newest.saturating_sub(b.last.get(lp).copied().unwrap_or(0)) <= age_ns
}

/// Feed-QoS guard: is `lp`'s market-data latency above `max_latency_ms`?
fn is_slow(cfg: &AggConfig, b: &Books, lp: &str) -> bool {
    cfg.max_latency_ms > 0 && b.latency.get(lp).copied().unwrap_or(0) > cfg.max_latency_ms
}

fn eligible(cfg: &AggConfig, b: &Books, symbol: &str, lots: Option<Qty>) -> Vec<(String, LpBook)> {
    let Some(lps) = b.by_symbol.get(symbol) else {
        return Vec::new();
    };
    let quoting = |lp: &str, bk: &LpBook| {
        let p = cfg.policy(lp);
        p.enabled && p.allows_symbol(symbol) && !(bk.bids.is_empty() && bk.asks.is_empty())
    };
    // The deviation reference is the highest-priority quoting LP's last mid,
    // chosen BEFORE the silent-LP filter: a backup LP that keeps ticking while
    // the primary is quiet stays within the guard of the last real price.
    let mut all: Vec<(&String, &LpBook)> = lps.iter().filter(|(lp, bk)| quoting(lp, bk)).collect();
    all.sort_by_key(|(lp, _)| (cfg.policy(lp).priority, (*lp).clone()));
    let reference = all.first().and_then(|(_, bk)| bk.mid());
    let mut v: Vec<(String, LpBook)> = all
        .into_iter()
        .filter(|(lp, _)| {
            is_fresh(cfg, b, lp)
                && !is_slow(cfg, b, lp)
                && lots.is_none_or(|l| cfg.policy(lp).allows_size(l))
        })
        .map(|(lp, bk)| (lp.clone(), bk.clone()))
        .collect();
    if cfg.max_deviation_points > 0 {
        if let Some(reference) = reference {
            let point = b.points.get(symbol).copied().unwrap_or(0) as i128;
            let limit = point * cfg.max_deviation_points as i128;
            if limit > 0 {
                v.retain(|(_, bk)| bk.mid().is_some_and(|m| (m - reference).abs() <= limit));
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use money::{px, qty};

    fn book(bid: &str, ask: &str, lots: &str) -> LpBook {
        LpBook {
            bids: vec![(px(bid), qty(lots))],
            asks: vec![(px(ask), qty(lots))],
            ts_ns: 1,
        }
    }

    fn two_lps(mode: AggMode) -> Aggregator {
        let a = Aggregator::new(AggConfig {
            mode,
            lps: vec![LpPolicy::new("LMAX", 1), LpPolicy::new("SIM", 2)],
            max_deviation_points: 0,
            ..AggConfig::default()
        });
        a.update("LMAX", "EURUSD", book("1.10000", "1.10020", "5"));
        a.update("SIM", "EURUSD", book("1.10005", "1.10025", "10"));
        a
    }

    #[test]
    fn aggregates_best_bid_and_ask_and_dedups() {
        let a = Aggregator::default();
        assert_eq!(
            a.update("LMAX", "EURUSD", book("1.10000", "1.10020", "5")),
            Some((px("1.10000"), px("1.10020")))
        );
        // SIM improves the bid only
        assert_eq!(
            a.update("SIM", "EURUSD", book("1.10005", "1.10030", "5")),
            Some((px("1.10005"), px("1.10020")))
        );
        // same aggregate again: nothing to journal
        assert_eq!(
            a.update("SIM", "EURUSD", book("1.10005", "1.10030", "5")),
            None
        );
        let m = a.merged("EURUSD");
        assert_eq!(m.bids[0], (px("1.10005"), qty("5")));
        assert_eq!(m.bids[1], (px("1.10000"), qty("5")));
        assert_eq!(m.asks.len(), 2);
    }

    #[test]
    fn best_price_per_side_and_kill_switch() {
        let a = two_lps(AggMode::BestPrice);
        // buying: SIM ask 1.10025 is worse than LMAX 1.10020
        assert_eq!(a.choose("EURUSD", Side::Buy, qty("1")), vec!["LMAX", "SIM"]);
        // selling: SIM bid 1.10005 is better
        assert_eq!(
            a.choose("EURUSD", Side::Sell, qty("1")),
            vec!["SIM", "LMAX"]
        );
        let mut cfg = a.config();
        cfg.lps[1].enabled = false;
        a.set_config(cfg);
        assert_eq!(a.choose("EURUSD", Side::Sell, qty("1")), vec!["LMAX"]);
        // the aggregate drops SIM too
        assert_eq!(
            a.update("LMAX", "EURUSD", book("1.10000", "1.10020", "5")),
            Some((px("1.10000"), px("1.10020")))
        );
    }

    #[test]
    fn size_limits_vwap_priority_and_round_robin() {
        let a = two_lps(AggMode::Vwap);
        // 8 lots: LMAX depth (5) cannot fill it, SIM (10) can
        assert_eq!(a.choose("EURUSD", Side::Buy, qty("8")), vec!["SIM", "LMAX"]);
        let mut cfg = a.config();
        cfg.lps[1].max_lots = Some(qty("3"));
        a.set_config(cfg.clone());
        assert_eq!(a.choose("EURUSD", Side::Buy, qty("8")), vec!["LMAX"]);
        cfg.lps[1].max_lots = None;
        cfg.mode = AggMode::Priority;
        a.set_config(cfg.clone());
        assert_eq!(
            a.choose("EURUSD", Side::Sell, qty("1")),
            vec!["LMAX", "SIM"]
        );
        cfg.mode = AggMode::RoundRobin;
        a.set_config(cfg);
        let first = a.choose("EURUSD", Side::Buy, qty("1"))[0].clone();
        let second = a.choose("EURUSD", Side::Buy, qty("1"))[0].clone();
        assert_ne!(first, second);
    }

    #[test]
    fn resting_lp_is_the_primary_order_taker_even_before_it_quotes() {
        let mut sim = LpPolicy::new("SIM", 2);
        sim.orders = false;
        let a = Aggregator::new(AggConfig {
            mode: AggMode::BestPrice,
            lps: vec![sim, LpPolicy::new("LMAX", 1)],
            max_deviation_points: 0,
            ..AggConfig::default()
        });
        assert_eq!(a.resting_lp("EURUSD").as_deref(), Some("LMAX"));
        let mut sym = LpPolicy::new("LMAX", 1);
        sym.symbols = vec!["XAUUSD".into()];
        a.set_config(AggConfig {
            lps: vec![sym],
            ..AggConfig::default()
        });
        assert_eq!(a.resting_lp("EURUSD"), None);
    }

    #[test]
    fn quote_only_lp_prices_but_never_takes_an_order() {
        let mut sim = LpPolicy::new("SIM", 2);
        sim.orders = false;
        let a = Aggregator::new(AggConfig {
            mode: AggMode::BestPrice,
            lps: vec![LpPolicy::new("LMAX", 1), sim],
            max_deviation_points: 0,
            ..AggConfig::default()
        });
        // the simulator quotes first (a fresh process after a take-over)
        a.update("SIM", "EURUSD", book("1.10005", "1.10025", "10"));
        assert!(!a.merged("EURUSD").bids.is_empty());
        // nobody may take the order: the engine rejects instead of filling here
        assert!(a.choose("EURUSD", Side::Sell, qty("1")).is_empty());
        a.update("LMAX", "EURUSD", book("1.10000", "1.10020", "5"));
        assert_eq!(a.choose("EURUSD", Side::Sell, qty("1")), vec!["LMAX"]);
        assert_eq!(a.choose("EURUSD", Side::Buy, qty("1")), vec!["LMAX"]);
    }

    #[test]
    fn deviation_guard_drops_outlier_until_it_returns() {
        let a = Aggregator::new(AggConfig {
            mode: AggMode::BestPrice,
            lps: vec![LpPolicy::new("LMAX", 1), LpPolicy::new("SIM", 2)],
            max_deviation_points: 100,
            ..AggConfig::default()
        });
        a.set_point("EURUSD", px("0.00001"));
        a.update("LMAX", "EURUSD", book("1.10000", "1.10020", "5"));
        // 50 pips away: ignored (would otherwise cross the book)
        assert_eq!(
            a.update("SIM", "EURUSD", book("1.09500", "1.09520", "5")),
            None
        );
        assert_eq!(a.choose("EURUSD", Side::Buy, qty("1")), vec!["LMAX"]);
        assert_eq!(a.runtime()[1].deviating, vec!["EURUSD"]);
        assert_eq!(
            a.update("SIM", "EURUSD", book("1.09990", "1.10010", "5")),
            Some((px("1.10000"), px("1.10010")))
        );
    }

    #[test]
    fn crossed_lps_fall_back_to_reference() {
        let a = two_lps(AggMode::BestPrice);
        // SIM bid above LMAX ask
        assert_eq!(
            a.update("SIM", "EURUSD", book("1.10030", "1.10050", "5")),
            Some((px("1.10000"), px("1.10020")))
        );
    }

    #[test]
    fn config_validation() {
        let mut c = AggConfig::default();
        c.lps.push(LpPolicy::new("A", 1));
        c.lps.push(LpPolicy::new("A", 2));
        assert!(c.validate().is_err());
        c.lps.pop();
        c.lps[0].min_lots = Some(qty("5"));
        c.lps[0].max_lots = Some(qty("1"));
        assert!(c.validate().is_err());
        c.lps[0].max_lots = None;
        assert!(c.validate().is_ok());
        let j = serde_json::to_string(&c).unwrap();
        assert!(j.contains("\"mode\":\"best_price\""));
    }

    #[test]
    fn silent_lp_drops_out_until_it_quotes_again() {
        let agg = Aggregator::new(AggConfig {
            max_quote_age_ms: 30_000,
            ..AggConfig::default()
        });
        agg.set_point("EURUSD", px("0.00001"));
        let at = |bid: &str, ask: &str, ts: u64| LpBook {
            bids: vec![(px(bid), qty("10"))],
            asks: vec![(px(ask), qty("10"))],
            ts_ns: ts,
        };
        let s = 1_000_000_000u64;
        // LMAX is the better ask; SIM is worse
        agg.update("LMAX", "EURUSD", at("1.10000", "1.10010", s));
        agg.update("SIM", "EURUSD", at("1.09995", "1.10015", s));
        assert_eq!(agg.choose("EURUSD", Side::Buy, qty("1"))[0], "LMAX");
        // LMAX goes silent; SIM keeps quoting for 31 s
        let q = agg.update("SIM", "EURUSD", at("1.09995", "1.10015", 32 * s));
        assert_eq!(q, Some((px("1.09995"), px("1.10015"))));
        assert_eq!(
            agg.choose("EURUSD", Side::Buy, qty("1")),
            vec!["SIM".to_string()]
        );
        // LMAX is back
        agg.update("LMAX", "EURUSD", at("1.10000", "1.10010", 33 * s));
        assert_eq!(agg.choose("EURUSD", Side::Buy, qty("1"))[0], "LMAX");
    }

    #[test]
    fn slow_lp_is_suspended_until_its_latency_recovers() {
        let agg = Aggregator::new(AggConfig {
            max_latency_ms: 500,
            ..AggConfig::default()
        });
        agg.set_point("EURUSD", px("0.00001"));
        agg.set_config(AggConfig {
            max_latency_ms: 500,
            lps: vec![LpPolicy::new("LMAX", 1), LpPolicy::new("SIM", 2)],
            ..AggConfig::default()
        });
        agg.update("LMAX", "EURUSD", at("1.10000", "1.10010", 1));
        agg.update("SIM", "EURUSD", at("1.09995", "1.10020", 1));
        assert_eq!(agg.choose("EURUSD", Side::Buy, qty("1"))[0], "LMAX");
        agg.set_latency("LMAX", 800);
        assert_eq!(
            agg.choose("EURUSD", Side::Buy, qty("1")),
            vec!["SIM".to_string()]
        );
        let r = agg.runtime();
        let lmax = r.iter().find(|r| r.name == "LMAX").unwrap();
        assert!(lmax.slow && lmax.latency_ms == 800);
        agg.set_latency("LMAX", 120);
        assert_eq!(agg.choose("EURUSD", Side::Buy, qty("1"))[0], "LMAX");
        assert!(!agg.runtime().iter().any(|r| r.slow));
    }

    #[test]
    fn republished_old_quote_never_makes_a_live_lp_silent() {
        let agg = Aggregator::new(AggConfig {
            max_quote_age_ms: 30_000,
            ..AggConfig::default()
        });
        agg.set_point("EURUSD", px("0.00001"));
        let at = |bid: &str, ask: &str, ts: u64| LpBook {
            bids: vec![(px(bid), qty("10"))],
            asks: vec![(px(ask), qty("10"))],
            ts_ns: ts,
        };
        let s = 1_000_000_000u64;
        agg.update("LMAX", "EURUSD", at("1.10000", "1.10010", 40 * s));
        agg.update("SIM", "EURUSD", at("1.09995", "1.10015", 40 * s));
        // NATS republishes LMAX's quiet XAU book with its 5 s old timestamp
        agg.update("LMAX", "XAUUSD", at("2000.00", "2000.50", 5 * s));
        agg.update("SIM", "EURUSD", at("1.09995", "1.10015", 41 * s));
        assert_eq!(agg.choose("EURUSD", Side::Buy, qty("1"))[0], "LMAX");
        assert!(!agg.runtime().iter().any(|r| r.silent));
    }

    #[test]
    fn backup_lp_stays_within_the_guard_of_the_silent_primary() {
        let agg = Aggregator::new(AggConfig {
            mode: AggMode::BestPrice,
            lps: vec![LpPolicy::new("LMAX", 1), LpPolicy::new("SIM", 2)],
            max_deviation_points: 100,
            max_quote_age_ms: 30_000,
        });
        agg.set_point("EURUSD", px("0.00001"));
        let at = |bid: &str, ask: &str, ts: u64| LpBook {
            bids: vec![(px(bid), qty("10"))],
            asks: vec![(px(ask), qty("10"))],
            ts_ns: ts,
        };
        let s = 1_000_000_000u64;
        agg.update("LMAX", "EURUSD", at("1.10000", "1.10010", s));
        // weekend: LMAX silent, the simulator drifts 300 points away and keeps ticking
        let q = agg.update("SIM", "EURUSD", at("1.10300", "1.10310", 60 * s));
        assert_eq!(q, None, "a drifting backup must not price clients");
        assert!(agg.choose("EURUSD", Side::Buy, qty("1")).is_empty());
        let rt = agg.runtime();
        let lmax = rt.iter().find(|r| r.name == "LMAX").unwrap();
        assert!(lmax.silent && lmax.deviating.is_empty());
    }

    #[test]
    fn silent_guard_is_off_by_default() {
        assert_eq!(AggConfig::default().max_quote_age_ms, 0);
    }
}
