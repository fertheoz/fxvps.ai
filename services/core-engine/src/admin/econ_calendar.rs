//! Economic calendar (plan item 10): release events entered in the console or
//! imported from the free weekly ForexFactory feed, shown to clients in the
//! terminal's calendar and as pins on the chart.
//!
//! Pure part: the event type, the feed parser and the idempotent merge. The
//! HTTP handlers live in `econ_admin`; the events themselves in the admin
//! journal (`AdminCmd::EconEvent*`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Impact {
    Low,
    Medium,
    High,
}

impl Impact {
    pub fn as_str(self) -> &'static str {
        match self {
            Impact::Low => "low",
            Impact::Medium => "medium",
            Impact::High => "high",
        }
    }

    /// Feed / form spelling (`High`, `medium`, ...). Holidays and other
    /// non-economic rows count as low impact.
    pub fn parse_lenient(s: &str) -> Impact {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" => Impact::High,
            "medium" | "med" => Impact::Medium,
            _ => Impact::Low,
        }
    }
}

pub const MAX_TITLE: usize = 120;
pub const MAX_VALUE: usize = 32;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EconEvent {
    pub id: String,
    /// Release time, ns since epoch (UTC).
    pub time_ns: u64,
    /// ISO 4217 code of the affected currency (`ALL` = every currency).
    pub currency: String,
    pub title: String,
    pub impact: Impact,
    #[serde(default)]
    pub actual: Option<String>,
    #[serde(default)]
    pub forecast: Option<String>,
    #[serde(default)]
    pub previous: Option<String>,
}

/// Identity of a release for the import: same title (any case), currency and time.
pub type EventKey = (String, String, u64);

impl EconEvent {
    pub fn key(&self) -> EventKey {
        (
            self.title.to_lowercase(),
            self.currency.clone(),
            self.time_ns,
        )
    }

    /// Trims the text fields, upper-cases the currency and drops empty values.
    pub fn normalized(mut self) -> EconEvent {
        self.currency = self.currency.trim().to_ascii_uppercase();
        self.title = self.title.trim().to_string();
        self.actual = clean(self.actual);
        self.forecast = clean(self.forecast);
        self.previous = clean(self.previous);
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.time_ns == 0 {
            return Err("time is required".into());
        }
        if self.currency.len() != 3 || !self.currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(format!(
                "currency must be a 3-letter code, got {:?}",
                self.currency
            ));
        }
        if self.title.is_empty() || self.title.chars().count() > MAX_TITLE {
            return Err(format!("title must be 1..{MAX_TITLE} characters"));
        }
        for v in [&self.actual, &self.forecast, &self.previous]
            .into_iter()
            .flatten()
        {
            if v.chars().count() > MAX_VALUE {
                return Err(format!(
                    "actual / forecast / previous up to {MAX_VALUE} characters"
                ));
            }
        }
        Ok(())
    }

    /// Does the event concern one of `currencies` (`ALL` concerns every one)?
    pub fn touches(&self, currencies: &[String]) -> bool {
        self.currency == "ALL" || currencies.contains(&self.currency)
    }
}

fn clean(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn truncate(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

/// Days since 1970-01-01 of a civil date (H. Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn num(s: &str, from: usize, to: usize) -> Option<i64> {
    let part = s.get(from..to)?;
    if !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM[:SS[.fff]]` with `Z`, `±HH:MM`, `±HHMM` or
/// no offset (UTC) -> ns since epoch.
pub fn parse_rfc3339_ns(s: &str) -> Option<u64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d) = (num(s, 0, 4)?, num(s, 5, 7)?, num(s, 8, 10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut secs = days_from_civil(y, m, d) * 86_400;
    let mut nanos = 0u64;
    let rest = &s[10..];
    if !rest.is_empty() {
        let t = rest.strip_prefix(['T', 't', ' '])?;
        let tb = t.as_bytes();
        if tb.get(2) != Some(&b':') {
            return None;
        }
        let (hh, mm) = (num(t, 0, 2)?, num(t, 3, 5)?);
        let mut ss = 0;
        let mut i = 5;
        if tb.get(5) == Some(&b':') {
            ss = num(t, 6, 8)?;
            i = 8;
            if tb.get(8) == Some(&b'.') {
                let digits = t[9..].bytes().take_while(u8::is_ascii_digit).count();
                if digits == 0 {
                    return None;
                }
                let frac: String = t[9..9 + digits].chars().take(9).collect();
                nanos = format!("{frac:0<9}").parse().ok()?;
                i = 9 + digits;
            }
        }
        if hh > 23 || mm > 59 || ss > 60 {
            return None;
        }
        secs += hh * 3600 + mm * 60 + ss;
        let tz = &t[i..];
        let offset = match tz {
            "" | "Z" | "z" => 0,
            _ => {
                let sign = match tz.as_bytes()[0] {
                    b'+' => 1,
                    b'-' => -1,
                    _ => return None,
                };
                let z = tz[1..].replace(':', "");
                if z.len() != 4 {
                    return None;
                }
                let (oh, om) = (num(&z, 0, 2)?, num(&z, 2, 4)?);
                if oh > 23 || om > 59 {
                    return None;
                }
                sign * (oh * 3600 + om * 60)
            }
        };
        secs -= offset;
    }
    u64::try_from(secs)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(nanos)
}

/// A string-ish JSON field (strings as-is, numbers printed, anything else none).
fn text(row: &Value, key: &str) -> Option<String> {
    match row.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Parsed feed: the usable rows (without ids) and how many were skipped.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FeedParse {
    pub events: Vec<EconEvent>,
    pub skipped: usize,
}

/// Upper bound on rows read from one feed (the weekly file holds ~150).
pub const MAX_FEED_ROWS: usize = 5_000;

/// Parses the ForexFactory weekly JSON (`ff_calendar_thisweek.json`):
/// `[{ "title", "country": "USD", "date": "2026-10-09T08:30:00-04:00",
/// "impact": "High", "forecast", "previous", "actual"? }]`. Rows without a
/// title, a 3-letter currency or a valid date are skipped.
pub fn parse_ff_feed(body: &str) -> Result<FeedParse, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("feed is not JSON: {e}"))?;
    let rows = v.as_array().ok_or("feed is not a JSON array")?;
    let mut out = FeedParse::default();
    for row in rows.iter().take(MAX_FEED_ROWS) {
        match feed_row(row) {
            Some(e) => out.events.push(e),
            None => out.skipped += 1,
        }
    }
    out.skipped += rows.len().saturating_sub(MAX_FEED_ROWS);
    Ok(out)
}

fn feed_row(row: &Value) -> Option<EconEvent> {
    let value = |k: &str| text(row, k).map(|s| truncate(&s, MAX_VALUE));
    let ev = EconEvent {
        id: String::new(),
        time_ns: parse_rfc3339_ns(&text(row, "date")?)?,
        currency: text(row, "country")?,
        title: truncate(&text(row, "title")?, MAX_TITLE),
        impact: Impact::parse_lenient(&text(row, "impact").unwrap_or_default()),
        actual: value("actual"),
        forecast: value("forecast"),
        previous: value("previous"),
    }
    .normalized();
    ev.validate().ok().map(|_| ev)
}

/// Result of merging a feed into the stored events.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Merge {
    /// New or changed events (to journal); unchanged ones are left out.
    pub upserts: Vec<EconEvent>,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
}

/// Idempotent import: an incoming event matching a stored one by
/// [`EconEvent::key`] keeps the stored id (and a stored value the feed leaves
/// empty, e.g. an `actual` typed in by hand); new ones get `{id_prefix}.{n}`.
/// Duplicate rows within the feed count once.
pub fn merge_import(
    existing: &BTreeMap<String, EconEvent>,
    incoming: Vec<EconEvent>,
    id_prefix: &str,
) -> Merge {
    let index: BTreeMap<EventKey, &EconEvent> = existing.values().map(|e| (e.key(), e)).collect();
    let mut seen = BTreeSet::new();
    let mut m = Merge::default();
    for (n, mut ev) in incoming.into_iter().enumerate() {
        let key = ev.key();
        if !seen.insert(key.clone()) {
            continue;
        }
        match index.get(&key) {
            Some(old) => {
                ev.id = old.id.clone();
                ev.actual = ev.actual.or_else(|| old.actual.clone());
                ev.forecast = ev.forecast.or_else(|| old.forecast.clone());
                ev.previous = ev.previous.or_else(|| old.previous.clone());
                if ev == **old {
                    m.unchanged += 1;
                } else {
                    m.updated += 1;
                    m.upserts.push(ev);
                }
            }
            None => {
                ev.id = format!("{id_prefix}.{n}");
                m.added += 1;
                m.upserts.push(ev);
            }
        }
    }
    m
}

/// Events in `[from, to)` (ns), optionally only those touching `currencies`,
/// ordered by time, at most `limit`.
pub fn in_range(
    events: &BTreeMap<String, EconEvent>,
    from: u64,
    to: u64,
    currencies: Option<&[String]>,
    limit: usize,
) -> Vec<EconEvent> {
    let mut out: Vec<EconEvent> = events
        .values()
        .filter(|e| e.time_ns >= from && e.time_ns < to)
        .filter(|e| currencies.is_none_or(|c| e.touches(c)))
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        (a.time_ns, b.impact, &a.currency, &a.title).cmp(&(
            b.time_ns,
            a.impact,
            &b.currency,
            &b.title,
        ))
    });
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: u64 = 1_000_000_000;

    #[test]
    fn rfc3339_forms() {
        // 2026-10-09T12:30:00Z
        let base = 1_791_549_000 * NS;
        assert_eq!(parse_rfc3339_ns("2026-10-09T12:30:00Z"), Some(base));
        assert_eq!(parse_rfc3339_ns("2026-10-09T08:30:00-04:00"), Some(base));
        assert_eq!(parse_rfc3339_ns("2026-10-09T14:30:00+0200"), Some(base));
        assert_eq!(parse_rfc3339_ns("2026-10-09T12:30"), Some(base));
        assert_eq!(
            parse_rfc3339_ns("2026-10-09 12:30:00.250Z"),
            Some(base + NS / 4)
        );
        assert_eq!(
            parse_rfc3339_ns("2026-10-09"),
            Some(base - (12 * 3600 + 1800) * NS)
        );
        for bad in [
            "",
            "2026-13-01",
            "2026-10-09T25:00:00Z",
            "2026-10-09T12:30:00+5",
            "09-10-2026",
            "2026-10-09T12:30:00 EST",
            "2026-10-09T12:30:00.Z",
            "1960-01-01",
        ] {
            assert_eq!(parse_rfc3339_ns(bad), None, "{bad}");
        }
    }

    const FEED: &str = r#"[
      {"title":"Non-Farm Employment Change","country":"USD","date":"2026-10-09T08:30:00-04:00","impact":"High","forecast":"140K","previous":"22K"},
      {"title":"German Industrial Production m/m","country":"eur","date":"2026-10-08T02:00:00-04:00","impact":"Medium","forecast":"","previous":"1.3%"},
      {"title":"Bank Holiday","country":"JPY","date":"2026-10-12T00:00:00-04:00","impact":"Holiday","forecast":"","previous":""},
      {"title":"OPEC Meetings","country":"ALL","date":"2026-10-10T03:00:00-04:00","impact":"Low"},
      {"title":"","country":"USD","date":"2026-10-09T08:30:00-04:00","impact":"High"},
      {"title":"Broken date","country":"USD","date":"10-09-2026","impact":"High"},
      {"title":"Bad currency","country":"US","date":"2026-10-09T08:30:00-04:00","impact":"High"},
      {"title":"Unemployment Rate","country":"USD","date":"2026-10-09T08:30:00-04:00","impact":"High","forecast":4.3,"previous":null,"actual":"4.2%"}
    ]"#;

    #[test]
    fn parses_the_ff_feed() {
        let p = parse_ff_feed(FEED).unwrap();
        assert_eq!(p.skipped, 3);
        assert_eq!(p.events.len(), 5);
        let nfp = &p.events[0];
        assert_eq!(nfp.currency, "USD");
        assert_eq!(nfp.impact, Impact::High);
        assert_eq!(nfp.time_ns, 1_791_549_000 * NS);
        assert_eq!(nfp.forecast.as_deref(), Some("140K"));
        assert_eq!(nfp.actual, None);
        let ip = &p.events[1];
        assert_eq!(
            (ip.currency.as_str(), ip.impact, ip.forecast.as_deref()),
            ("EUR", Impact::Medium, None)
        );
        assert_eq!(p.events[2].impact, Impact::Low); // holiday
        assert_eq!(p.events[3].currency, "ALL");
        let ur = &p.events[4];
        assert_eq!(
            (
                ur.forecast.as_deref(),
                ur.previous.as_deref(),
                ur.actual.as_deref()
            ),
            (Some("4.3"), None, Some("4.2%"))
        );
        assert!(parse_ff_feed("{}").is_err());
        assert!(parse_ff_feed("<html>rate limited</html>").is_err());
        assert_eq!(parse_ff_feed("[]").unwrap(), FeedParse::default());
    }

    #[test]
    fn merge_is_idempotent_by_title_currency_time() {
        let feed = parse_ff_feed(FEED).unwrap().events;
        let mut store = BTreeMap::new();
        let m = merge_import(&store, feed.clone(), "ev7");
        assert_eq!((m.added, m.updated, m.unchanged), (5, 0, 0));
        assert_eq!(m.upserts[0].id, "ev7.0");
        for e in m.upserts {
            store.insert(e.id.clone(), e);
        }
        // the same feed again: nothing to journal
        let again = merge_import(&store, feed.clone(), "ev9");
        assert_eq!((again.added, again.updated, again.unchanged), (0, 0, 5));
        assert!(again.upserts.is_empty());
        // actual published later (and a title in another case): an update of the same id
        let mut next = feed.clone();
        next[0].actual = Some("254K".into());
        next[0].title = next[0].title.to_uppercase();
        next.push(next[1].clone()); // duplicate row
        let up = merge_import(&store, next, "ev11");
        assert_eq!((up.added, up.updated, up.unchanged), (0, 1, 4));
        assert_eq!(up.upserts[0].id, "ev7.0");
        assert_eq!(up.upserts[0].actual.as_deref(), Some("254K"));
        // a value typed in by hand survives a feed row that leaves it empty
        store.get_mut("ev7.1").unwrap().actual = Some("-0.3%".into());
        let keep = merge_import(&store, feed, "ev12");
        assert_eq!((keep.added, keep.updated, keep.unchanged), (0, 0, 5));
    }

    #[test]
    fn range_and_currency_filter() {
        let mut store = BTreeMap::new();
        for e in merge_import(&BTreeMap::new(), parse_ff_feed(FEED).unwrap().events, "e").upserts {
            store.insert(e.id.clone(), e);
        }
        let all = in_range(&store, 0, u64::MAX, None, 100);
        assert_eq!(all.len(), 5);
        assert!(all.windows(2).all(|w| w[0].time_ns <= w[1].time_ns));
        // same time: higher impact first
        let usd = in_range(&store, 0, u64::MAX, Some(&["USD".to_string()]), 100);
        let titles: Vec<&str> = usd.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Non-Farm Employment Change",
                "Unemployment Rate",
                "OPEC Meetings"
            ]
        );
        let nfp = 1_791_549_000 * NS;
        assert_eq!(in_range(&store, nfp, nfp + 1, None, 100).len(), 2);
        assert_eq!(in_range(&store, nfp, nfp + 1, None, 1).len(), 1);
        assert!(in_range(&store, nfp + 1, nfp + 2, None, 100).is_empty());
    }

    #[test]
    fn validation() {
        let ok = EconEvent {
            id: "x".into(),
            time_ns: 1,
            currency: " usd ".into(),
            title: " CPI ".into(),
            impact: Impact::High,
            actual: Some("  ".into()),
            forecast: None,
            previous: Some("3.1%".into()),
        }
        .normalized();
        assert_eq!(
            (ok.currency.as_str(), ok.title.as_str(), ok.actual.clone()),
            ("USD", "CPI", None)
        );
        assert!(ok.validate().is_ok());
        assert!(EconEvent {
            currency: "US1".into(),
            ..ok.clone()
        }
        .validate()
        .is_err());
        assert!(EconEvent {
            title: String::new(),
            ..ok.clone()
        }
        .validate()
        .is_err());
        assert!(EconEvent {
            time_ns: 0,
            ..ok.clone()
        }
        .validate()
        .is_err());
        assert!(EconEvent {
            forecast: Some("x".repeat(MAX_VALUE + 1)),
            ..ok
        }
        .validate()
        .is_err());
    }
}
