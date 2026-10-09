//! Tick warehouse (parça 13).
//!
//! The aggregated top of book of every symbol is sampled (at most one tick
//! per `sample_ms`, default 1000) into append-only binary files
//! `<dir>/<SYMBOL>/<YYYY-MM-DD>.tick`, 40 bytes a tick, little endian:
//! `ts_ns u64, bid i64, ask i64, bid_lots i64, ask_lots i64` (prices and
//! lots raw 1e8 fixed-point). A writer thread buffers and flushes once a
//! second; files older than `keep_days` (default 30) are removed at the
//! day roll. Readers (`read`, `hourly`, `price_at`) are plain functions over
//! the files, so the admin API needs no handle, only the directory.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

pub const TICK_BYTES: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    pub ts_ns: u64,
    pub bid: i64,
    pub ask: i64,
    pub bid_lots: i64,
    pub ask_lots: i64,
}

impl Tick {
    fn to_bytes(self) -> [u8; TICK_BYTES] {
        let mut b = [0u8; TICK_BYTES];
        b[0..8].copy_from_slice(&self.ts_ns.to_le_bytes());
        b[8..16].copy_from_slice(&self.bid.to_le_bytes());
        b[16..24].copy_from_slice(&self.ask.to_le_bytes());
        b[24..32].copy_from_slice(&self.bid_lots.to_le_bytes());
        b[32..40].copy_from_slice(&self.ask_lots.to_le_bytes());
        b
    }

    fn from_bytes(b: &[u8]) -> Tick {
        let n = |r: std::ops::Range<usize>| i64::from_le_bytes(b[r].try_into().unwrap_or([0; 8]));
        Tick {
            ts_ns: u64::from_le_bytes(b[0..8].try_into().unwrap_or([0; 8])),
            bid: n(8..16),
            ask: n(16..24),
            bid_lots: n(24..32),
            ask_lots: n(32..40),
        }
    }
}

/// `YYYY-MM-DD` (UTC) of a ns timestamp.
pub fn day_of(ts_ns: u64) -> String {
    let days = (ts_ns / 1_000_000_000 / 86_400) as i64;
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

struct Sample {
    symbol: String,
    tick: Tick,
}

/// Writer handle: `record` never blocks the quote path.
pub struct TickStore {
    tx: mpsc::SyncSender<Sample>,
    sample_ns: u64,
    last: std::sync::Mutex<BTreeMap<String, u64>>,
}

impl std::fmt::Debug for TickStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TickStore").finish()
    }
}

impl TickStore {
    /// Opens the warehouse under `dir` and starts the writer thread.
    /// `CORE_TICK_SAMPLE_MS` (default 1000) and `CORE_TICK_KEEP_DAYS` (30).
    pub fn open(dir: impl Into<PathBuf>) -> Arc<TickStore> {
        let dir = dir.into();
        let sample_ms = std::env::var("CORE_TICK_SAMPLE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1000);
        let keep_days = std::env::var("CORE_TICK_KEEP_DAYS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);
        let (tx, rx) = mpsc::sync_channel::<Sample>(8192);
        std::thread::Builder::new()
            .name("tick-store".into())
            .spawn(move || writer(dir, rx, keep_days))
            .ok();
        Arc::new(TickStore {
            tx,
            sample_ns: sample_ms.max(1) * 1_000_000,
            last: std::sync::Mutex::new(BTreeMap::new()),
        })
    }

    /// Samples one top of book (raw prices / lots); dropped when the writer
    /// is behind or the symbol was sampled less than `sample_ms` ago.
    pub fn record(
        &self,
        symbol: &str,
        ts_ns: u64,
        bid: i64,
        ask: i64,
        bid_lots: i64,
        ask_lots: i64,
    ) {
        {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            let prev = last.get(symbol).copied().unwrap_or(0);
            if ts_ns < prev.saturating_add(self.sample_ns) {
                return;
            }
            last.insert(symbol.to_string(), ts_ns);
        }
        let _ = self.tx.try_send(Sample {
            symbol: symbol.to_string(),
            tick: Tick {
                ts_ns,
                bid,
                ask,
                bid_lots,
                ask_lots,
            },
        });
    }
}

fn writer(dir: PathBuf, rx: mpsc::Receiver<Sample>, keep_days: u64) {
    let mut files: BTreeMap<(String, String), BufWriter<File>> = BTreeMap::new();
    let mut today = String::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(s) => {
                let day = day_of(s.tick.ts_ns);
                if day != today {
                    today = day.clone();
                    files.clear();
                    prune(&dir, keep_days, s.tick.ts_ns);
                }
                let key = (s.symbol.clone(), day.clone());
                let w = match files.get_mut(&key) {
                    Some(w) => w,
                    None => {
                        let d = dir.join(&s.symbol);
                        if fs::create_dir_all(&d).is_err() {
                            continue;
                        }
                        let Ok(f) = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(d.join(format!("{day}.tick")))
                        else {
                            continue;
                        };
                        files
                            .entry(key)
                            .or_insert_with(|| BufWriter::with_capacity(16 * 1024, f))
                    }
                };
                let _ = w.write_all(&s.tick.to_bytes());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                for w in files.values_mut() {
                    let _ = w.flush();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                for w in files.values_mut() {
                    let _ = w.flush();
                }
                return;
            }
        }
    }
}

/// Removes day files older than `keep_days` before `now_ns`.
fn prune(dir: &Path, keep_days: u64, now_ns: u64) {
    let cutoff = day_of(now_ns.saturating_sub(keep_days * 86_400 * 1_000_000_000));
    let Ok(symbols) = fs::read_dir(dir) else {
        return;
    };
    for s in symbols.flatten() {
        let Ok(days) = fs::read_dir(s.path()) else {
            continue;
        };
        for f in days.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if name.len() >= 10 && &name[..10] < cutoff.as_str() {
                let _ = fs::remove_file(f.path());
            }
        }
    }
}

/// Days with a file for `symbol`, newest first.
pub fn days(dir: &Path, symbol: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir.join(symbol))
        .map(|rd| {
            rd.flatten()
                .filter_map(|f| {
                    let n = f.file_name().to_string_lossy().to_string();
                    n.strip_suffix(".tick").map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

/// Symbols with any file.
pub fn symbols(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|f| f.path().is_dir())
                .map(|f| f.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// All ticks of `symbol` on `day`, in file order (time order).
pub fn read(dir: &Path, symbol: &str, day: &str) -> Vec<Tick> {
    let Ok(mut f) = File::open(dir.join(symbol).join(format!("{day}.tick"))) else {
        return Vec::new();
    };
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    buf.as_chunks::<TICK_BYTES>()
        .0
        .iter()
        .map(|c| Tick::from_bytes(c))
        .collect()
}

/// Last tick at or before `ts_ns` (ticks in time order).
pub fn price_at(ticks: &[Tick], ts_ns: u64) -> Option<Tick> {
    let i = ticks.partition_point(|t| t.ts_ns <= ts_ns);
    (i > 0).then(|| ticks[i - 1])
}

/// Per-hour liquidity of a day: ticks, spread (raw price units) avg / min /
/// max, average top-of-book lots (raw).
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourStat {
    pub hour: u8,
    pub ticks: u32,
    pub avg_spread: f64,
    pub min_spread: i64,
    pub max_spread: i64,
    pub avg_bid_lots: f64,
    pub avg_ask_lots: f64,
}

pub fn hourly(ticks: &[Tick]) -> Vec<HourStat> {
    let mut out: Vec<HourStat> = (0..24)
        .map(|h| HourStat {
            hour: h,
            min_spread: i64::MAX,
            ..HourStat::default()
        })
        .collect();
    for t in ticks {
        let h = ((t.ts_ns / 1_000_000_000) % 86_400 / 3_600) as usize;
        let s = &mut out[h];
        let spread = t.ask - t.bid;
        s.ticks += 1;
        s.avg_spread += spread as f64;
        s.min_spread = s.min_spread.min(spread);
        s.max_spread = s.max_spread.max(spread);
        s.avg_bid_lots += t.bid_lots as f64;
        s.avg_ask_lots += t.ask_lots as f64;
    }
    for s in &mut out {
        if s.ticks > 0 {
            let n = f64::from(s.ticks);
            s.avg_spread /= n;
            s.avg_bid_lots /= n;
            s.avg_ask_lots /= n;
        } else {
            s.min_spread = 0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_of_matches_civil_dates() {
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(day_of(1_709_294_400 * 1_000_000_000), "2024-03-01");
        assert_eq!(day_of(1_791_531_015 * 1_000_000_000), "2026-10-09");
    }

    #[test]
    fn ticks_round_trip_and_hourly_stats() {
        let dir = std::env::temp_dir().join(format!("ticks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = TickStore::open(&dir);
        let base = 1_791_504_000u64 * 1_000_000_000; // 2026-10-09 00:00 UTC
        store.record(
            "EURUSD",
            base + 1,
            110_000_000,
            110_010_000,
            100_000_000,
            200_000_000,
        );
        // sampled away: same second
        store.record("EURUSD", base + 500_000_000, 1, 2, 3, 4);
        store.record(
            "EURUSD",
            base + 3_600_000_000_000 + 5,
            110_000_000,
            110_030_000,
            300_000_000,
            100_000_000,
        );
        drop(store);
        std::thread::sleep(Duration::from_millis(300));
        let ticks = read(&dir, "EURUSD", "2026-10-09");
        assert_eq!(ticks.len(), 2, "{ticks:?}");
        assert_eq!(ticks[0].ask - ticks[0].bid, 10_000);
        let h = hourly(&ticks);
        assert_eq!((h[0].ticks, h[1].ticks, h[2].ticks), (1, 1, 0));
        assert_eq!(h[1].min_spread, 30_000);
        assert_eq!(
            price_at(&ticks, base + 10).map(|t| t.ask),
            Some(110_010_000)
        );
        assert_eq!(price_at(&ticks, base), None);
        assert_eq!(days(&dir, "EURUSD"), vec!["2026-10-09".to_string()]);
        assert_eq!(symbols(&dir), vec!["EURUSD".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }
}
