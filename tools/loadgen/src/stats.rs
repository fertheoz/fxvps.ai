//! Latency histogram (exact samples, microsecond resolution) and report types.

use serde::Serialize;

/// Collects latency samples in microseconds.
#[derive(Clone, Debug, Default)]
pub struct Hist {
    samples: Vec<u64>,
}

/// Percentile summary in milliseconds.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub count: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
}

impl Hist {
    pub fn record_us(&mut self, us: u64) {
        self.samples.push(us);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn merge(&mut self, other: Hist) {
        self.samples.extend(other.samples);
    }

    /// Nearest-rank percentile (`q` in 0..=100), in microseconds.
    pub fn percentile_us(&self, q: f64) -> Option<u64> {
        let mut s = self.samples.clone();
        s.sort_unstable();
        percentile_sorted(&s, q)
    }

    pub fn summary(&self) -> Summary {
        let mut s = self.samples.clone();
        if s.is_empty() {
            return Summary::default();
        }
        s.sort_unstable();
        let ms = |v: Option<u64>| v.unwrap_or(0) as f64 / 1000.0;
        let sum: u128 = s.iter().map(|&v| v as u128).sum();
        Summary {
            count: s.len() as u64,
            p50_ms: ms(percentile_sorted(&s, 50.0)),
            p95_ms: ms(percentile_sorted(&s, 95.0)),
            p99_ms: ms(percentile_sorted(&s, 99.0)),
            max_ms: ms(s.last().copied()),
            mean_ms: (sum as f64 / s.len() as f64) / 1000.0,
        }
    }
}

fn percentile_sorted(s: &[u64], q: f64) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let q = q.clamp(0.0, 100.0);
    let rank = ((q / 100.0) * s.len() as f64).ceil() as usize;
    Some(s[rank.saturating_sub(1).min(s.len() - 1)])
}

/// Per-client counters and histograms, merged into a [`Report`].
#[derive(Clone, Debug, Default)]
pub struct ClientStats {
    pub connected: bool,
    pub quotes: u64,
    pub quote_latency: Hist,
    pub orders_sent: u64,
    pub acks: u64,
    pub fills: u64,
    pub rejects: u64,
    pub ack_latency: Hist,
    pub fill_latency: Hist,
    pub errors: std::collections::BTreeMap<String, u64>,
}

impl ClientStats {
    pub fn error(&mut self, kind: impl Into<String>) {
        *self.errors.entry(kind.into()).or_default() += 1;
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub clients: u64,
    pub connected: u64,
    pub duration_s: f64,
    pub quotes: u64,
    pub quotes_per_s: f64,
    pub quote_latency: Summary,
    pub orders_sent: u64,
    pub acks: u64,
    pub fills: u64,
    pub rejects: u64,
    pub ack_latency: Summary,
    pub fill_latency: Summary,
    pub errors: std::collections::BTreeMap<String, u64>,
}

impl Report {
    pub fn from_clients(all: Vec<ClientStats>, duration_s: f64) -> Report {
        let mut r = Report {
            clients: all.len() as u64,
            duration_s,
            ..Default::default()
        };
        let (mut q, mut a, mut f) = (Hist::default(), Hist::default(), Hist::default());
        for c in all {
            r.connected += u64::from(c.connected);
            r.quotes += c.quotes;
            r.orders_sent += c.orders_sent;
            r.acks += c.acks;
            r.fills += c.fills;
            r.rejects += c.rejects;
            for (k, v) in c.errors {
                *r.errors.entry(k).or_default() += v;
            }
            q.merge(c.quote_latency);
            a.merge(c.ack_latency);
            f.merge(c.fill_latency);
        }
        r.quotes_per_s = if duration_s > 0.0 {
            r.quotes as f64 / duration_s
        } else {
            0.0
        };
        r.quote_latency = q.summary();
        r.ack_latency = a.summary();
        r.fill_latency = f.summary();
        r
    }

    /// Human-readable table.
    pub fn table(&self) -> String {
        let mut s = format!(
            "clients {}/{} connected, {:.1}s, quotes {} ({:.0}/s), orders {} (acks {}, fills {}, rejects {})\n",
            self.connected,
            self.clients,
            self.duration_s,
            self.quotes,
            self.quotes_per_s,
            self.orders_sent,
            self.acks,
            self.fills,
            self.rejects
        );
        s.push_str(&format!(
            "{:<16}{:>9}{:>10}{:>10}{:>10}{:>10}{:>10}\n",
            "metric (ms)", "count", "p50", "p95", "p99", "max", "mean"
        ));
        for (name, m) in [
            ("quote fan-out", &self.quote_latency),
            ("order ack", &self.ack_latency),
            ("order fill", &self.fill_latency),
        ] {
            s.push_str(&format!(
                "{:<16}{:>9}{:>10.3}{:>10.3}{:>10.3}{:>10.3}{:>10.3}\n",
                name, m.count, m.p50_ms, m.p95_ms, m.p99_ms, m.max_ms, m.mean_ms
            ));
        }
        if self.errors.is_empty() {
            s.push_str("errors: none\n");
        } else {
            for (k, v) in &self.errors {
                s.push_str(&format!("error {k}: {v}\n"));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_nearest_rank() {
        let mut h = Hist::default();
        for v in 1..=100 {
            h.record_us(v * 1000);
        }
        assert_eq!(h.percentile_us(50.0), Some(50_000));
        assert_eq!(h.percentile_us(95.0), Some(95_000));
        assert_eq!(h.percentile_us(99.0), Some(99_000));
        assert_eq!(h.percentile_us(100.0), Some(100_000));
        assert_eq!(h.percentile_us(0.0), Some(1_000));
        let s = h.summary();
        assert_eq!(s.count, 100);
        assert!((s.mean_ms - 50.5).abs() < 1e-9);
        assert!((s.max_ms - 100.0).abs() < 1e-9);
    }

    #[test]
    fn empty_and_merge() {
        assert_eq!(Hist::default().percentile_us(50.0), None);
        assert_eq!(Hist::default().summary(), Summary::default());
        let mut a = ClientStats {
            connected: true,
            quotes: 3,
            ..Default::default()
        };
        a.quote_latency.record_us(1000);
        a.error("timeout");
        let mut b = ClientStats::default();
        b.quote_latency.record_us(3000);
        b.error("timeout");
        let r = Report::from_clients(vec![a, b], 2.0);
        assert_eq!(r.clients, 2);
        assert_eq!(r.connected, 1);
        assert_eq!(r.quote_latency.count, 2);
        assert_eq!(r.errors["timeout"], 2);
        assert!((r.quotes_per_s - 1.5).abs() < 1e-9);
        assert!(r.table().contains("quote fan-out"));
        assert!(serde_json::to_string(&r).unwrap().contains("\"p99_ms\""));
    }
}
