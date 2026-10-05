//! In-memory OHLC aggregation (M1..D1) of mid prices, fed by the quote stream.
//! [`CandleStore::dump`] / [`CandleStore::restore`] carry the history across restarts.

use std::collections::{HashMap, VecDeque};

use client_proto::{Candle, Decimal, Timeframe};
use domain::Fixed;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    pub open_time_ns: u64,
    pub open: Fixed,
    pub high: Fixed,
    pub low: Fixed,
    pub close: Fixed,
    pub ticks: u64,
}

impl Bar {
    pub fn to_proto(self) -> Candle {
        Candle {
            open_time_ns: self.open_time_ns,
            open: Some(Decimal::from_fixed(self.open)),
            high: Some(Decimal::from_fixed(self.high)),
            low: Some(Decimal::from_fixed(self.low)),
            close: Some(Decimal::from_fixed(self.close)),
            ticks: self.ticks,
        }
    }
}

pub struct CandleStore {
    capacity: usize,
    series: HashMap<(String, Timeframe), VecDeque<Bar>>,
}

impl CandleStore {
    pub fn new(capacity: usize) -> Self {
        CandleStore {
            capacity: capacity.max(1),
            series: HashMap::new(),
        }
    }

    /// Adds one price observation to every timeframe of `symbol`.
    pub fn on_price(&mut self, symbol: &str, px: Fixed, ts_ns: u64) {
        for tf in Timeframe::ALL {
            let Some(secs) = tf.seconds() else { continue };
            let len = secs * 1_000_000_000;
            let open_time = ts_ns - ts_ns % len;
            let s = self.series.entry((symbol.to_string(), tf)).or_default();
            match s.back_mut() {
                Some(b) if b.open_time_ns == open_time => {
                    b.high = b.high.max(px);
                    b.low = b.low.min(px);
                    b.close = px;
                    b.ticks += 1;
                }
                // Out-of-order observation for an older bucket: ignore.
                Some(b) if b.open_time_ns > open_time => {}
                _ => {
                    s.push_back(Bar {
                        open_time_ns: open_time,
                        open: px,
                        high: px,
                        low: px,
                        close: px,
                        ticks: 1,
                    });
                    while s.len() > self.capacity {
                        s.pop_front();
                    }
                }
            }
        }
    }

    /// Bars with `from <= open_time <= to` (0 = unbounded), most recent `limit`.
    pub fn query(&self, symbol: &str, tf: Timeframe, from: u64, to: u64, limit: usize) -> Vec<Bar> {
        let Some(s) = self.series.get(&(symbol.to_string(), tf)) else {
            return Vec::new();
        };
        let to = if to == 0 { u64::MAX } else { to };
        let v: Vec<Bar> = s
            .iter()
            .filter(|b| b.open_time_ns >= from && b.open_time_ns <= to)
            .copied()
            .collect();
        let skip = v.len().saturating_sub(limit.max(1));
        v[skip..].to_vec()
    }
}

impl CandleStore {
    /// Text snapshot, one bar per line, oldest first per series:
    /// `SYMBOL <tf seconds> <open_time_ns> <open> <high> <low> <close> <ticks>` (raw fixed).
    pub fn dump(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        for ((symbol, tf), bars) in &self.series {
            let Some(secs) = tf.seconds() else { continue };
            for b in bars {
                let _ = writeln!(
                    out,
                    "{symbol} {secs} {} {} {} {} {} {}",
                    b.open_time_ns,
                    b.open.raw(),
                    b.high.raw(),
                    b.low.raw(),
                    b.close.raw(),
                    b.ticks
                );
            }
        }
        out
    }

    /// Loads a [`CandleStore::dump`] snapshot (malformed lines are skipped); returns the
    /// number of bars taken. Bars older than `capacity` per series are dropped.
    pub fn restore(&mut self, text: &str) -> usize {
        let mut n = 0;
        for line in text.lines() {
            let f: Vec<&str> = line.split(' ').collect();
            let [symbol, secs, t, o, h, l, c, ticks] = f[..] else {
                continue;
            };
            let Some(tf) = secs
                .parse::<u64>()
                .ok()
                .and_then(|s| Timeframe::ALL.into_iter().find(|tf| tf.seconds() == Some(s)))
            else {
                continue;
            };
            let (Ok(t), Ok(o), Ok(h), Ok(l), Ok(c), Ok(ticks)) = (
                t.parse::<u64>(),
                o.parse::<i64>(),
                h.parse::<i64>(),
                l.parse::<i64>(),
                c.parse::<i64>(),
                ticks.parse::<u64>(),
            ) else {
                continue;
            };
            let s = self.series.entry((symbol.to_string(), tf)).or_default();
            if s.back().is_some_and(|b| b.open_time_ns >= t) {
                continue;
            }
            s.push_back(Bar {
                open_time_ns: t,
                open: Fixed::from_raw(o),
                high: Fixed::from_raw(h),
                low: Fixed::from_raw(l),
                close: Fixed::from_raw(c),
                ticks,
            });
            while s.len() > self.capacity {
                s.pop_front();
            }
            n += 1;
        }
        n
    }
}

/// Mid price `(bid + ask) / 2`, truncated to 1e-8.
pub fn mid(bid: Fixed, ask: Fixed) -> Fixed {
    Fixed::from_raw(((i128::from(bid.raw()) + i128::from(ask.raw())) / 2) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000_000_000;

    fn px(s: &str) -> Fixed {
        s.parse().unwrap()
    }

    #[test]
    fn dump_and_restore_round_trip() {
        let mut c = CandleStore::new(10);
        c.on_price("EURUSD", px("1.1"), 60 * S);
        c.on_price("EURUSD", px("1.3"), 125 * S);
        c.on_price("XAUUSD", px("4100.5"), 125 * S);
        let mut r = CandleStore::new(10);
        assert_eq!(r.restore(&format!("garbage line
{}", c.dump())), 21);
        for tf in Timeframe::ALL {
            assert_eq!(r.query("EURUSD", tf, 0, 0, 100), c.query("EURUSD", tf, 0, 0, 100));
            assert_eq!(r.query("XAUUSD", tf, 0, 0, 100), c.query("XAUUSD", tf, 0, 0, 100));
        }
        // New prices continue the restored last bar.
        r.on_price("EURUSD", px("1.4"), 130 * S);
        let m1 = r.query("EURUSD", Timeframe::M1, 0, 0, 100);
        assert_eq!((m1.len(), m1[1].high, m1[1].ticks), (2, px("1.4"), 2));
        let mut small = CandleStore::new(1);
        small.restore(&c.dump());
        assert_eq!(small.query("EURUSD", Timeframe::M1, 0, 0, 100).len(), 1);
    }

    #[test]
    fn aggregates_ohlc_across_buckets() {
        let mut c = CandleStore::new(10);
        c.on_price("EURUSD", px("1.1"), 60 * S);
        c.on_price("EURUSD", px("1.3"), 61 * S);
        c.on_price("EURUSD", px("1.0"), 62 * S);
        c.on_price("EURUSD", px("1.2"), 119 * S);
        c.on_price("EURUSD", px("1.25"), 120 * S);
        let m1 = c.query("EURUSD", Timeframe::M1, 0, 0, 100);
        assert_eq!(m1.len(), 2);
        assert_eq!(
            m1[0],
            Bar {
                open_time_ns: 60 * S,
                open: px("1.1"),
                high: px("1.3"),
                low: px("1.0"),
                close: px("1.2"),
                ticks: 4
            }
        );
        assert_eq!(m1[1].open, px("1.25"));
        let m5 = c.query("EURUSD", Timeframe::M5, 0, 0, 100);
        assert_eq!(m5.len(), 1);
        assert_eq!(m5[0].ticks, 5);
        assert_eq!(m5[0].close, px("1.25"));
        assert_eq!(c.query("EURUSD", Timeframe::D1, 0, 0, 100).len(), 1);
        // limit and range
        assert_eq!(
            c.query("EURUSD", Timeframe::M1, 0, 0, 1)[0].open_time_ns,
            120 * S
        );
        assert_eq!(c.query("EURUSD", Timeframe::M1, 0, 60 * S, 10).len(), 1);
        assert!(c.query("GBPUSD", Timeframe::M1, 0, 0, 10).is_empty());
    }

    #[test]
    fn capacity_bounds_history() {
        let mut c = CandleStore::new(3);
        for i in 0..10 {
            c.on_price("X", px("1"), i * 60 * S);
        }
        let v = c.query("X", Timeframe::M1, 0, 0, 100);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].open_time_ns, 7 * 60 * S);
    }

    #[test]
    fn mid_price() {
        assert_eq!(mid(px("1.08501"), px("1.08503")), px("1.08502"));
    }
}
