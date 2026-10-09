//! Venue session hours (Solid FX: Sunday 17:05 ET – Friday 17:00 ET with a
//! 17:00–17:05 ET break Monday–Thursday). Outside the hours the gateway
//! neither connects nor alarms: the down is "scheduled".
//!
//! Times are New York local time; the US DST rule (second Sunday of March
//! 02:00 – first Sunday of November 02:00) is applied without a tz
//! database so the gateway stays dependency-free.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionHours {
    /// Weekly open: day (0 = Sunday … 6 = Saturday) and `HH:MM` New York time.
    pub open_day: u8,
    pub open_time: String,
    /// Weekly close.
    pub close_day: u8,
    pub close_time: String,
    /// Daily break inside the week (`HH:MM`–`HH:MM` New York time), e.g.
    /// `17:00`–`17:05`; absent = none.
    #[serde(default)]
    pub daily_break: Option<(String, String)>,
}

impl SessionHours {
    /// Solid FX / MAS Markets defaults.
    pub fn solid_fx() -> SessionHours {
        SessionHours {
            open_day: 0,
            open_time: "17:05".into(),
            close_day: 5,
            close_time: "17:00".into(),
            daily_break: Some(("17:00".into(), "17:05".into())),
        }
    }

    /// Is the venue open at `unix_s`? Also returns how many seconds until
    /// the state could change (for the gateway's wait), at most one hour.
    pub fn is_open(&self, unix_s: u64) -> (bool, u64) {
        let ny = unix_s as i64 + ny_offset_s(unix_s);
        let dow = ((ny.div_euclid(86_400) + 4).rem_euclid(7)) as u8; // 1970-01-01 = Thursday
        let tod = ny.rem_euclid(86_400) as u64;
        let week_s = u64::from(dow) * 86_400 + tod;
        let open = u64::from(self.open_day) * 86_400 + hm(&self.open_time);
        let close = u64::from(self.close_day) * 86_400 + hm(&self.close_time);
        let in_week = if open <= close {
            week_s >= open && week_s < close
        } else {
            week_s >= open || week_s < close
        };
        if !in_week {
            return (false, 60);
        }
        if let Some((from, to)) = &self.daily_break {
            let (f, t) = (hm(from), hm(to));
            if tod >= f && tod < t {
                return (false, (t - tod).clamp(1, 3600));
            }
        }
        (true, 60)
    }
}

fn hm(s: &str) -> u64 {
    let mut it = s.split(':');
    let h: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let m: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    (h * 60 + m) * 60
}

/// New York offset from UTC in seconds at `unix_s` (EST −5 h, EDT −4 h).
pub fn ny_offset_s(unix_s: u64) -> i64 {
    let days = (unix_s / 86_400) as i64;
    let (y, m, d) = civil(days);
    // DST: second Sunday of March 07:00 UTC .. first Sunday of November 06:00 UTC
    let dst_start = days_from_civil(y, 3, nth_sunday(y, 3, 2)) * 86_400 + 7 * 3600;
    let dst_end = days_from_civil(y, 11, nth_sunday(y, 11, 1)) * 86_400 + 6 * 3600;
    let _ = (m, d);
    let t = unix_s as i64;
    if t >= dst_start && t < dst_end {
        -4 * 3600
    } else {
        -5 * 3600
    }
}

fn nth_sunday(y: i64, m: i64, n: i64) -> i64 {
    let first = days_from_civil(y, m, 1);
    let dow = (first + 4).rem_euclid(7); // 0 = Sunday
    let first_sunday = 1 + (7 - dow) % 7;
    first_sunday + (n - 1) * 7
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i64, m: i64, d: i64, hh: i64, mm: i64) -> u64 {
        (days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60) as u64
    }

    #[test]
    fn new_york_offset_follows_us_dst() {
        assert_eq!(ny_offset_s(at(2026, 1, 15, 12, 0)), -5 * 3600);
        assert_eq!(ny_offset_s(at(2026, 7, 15, 12, 0)), -4 * 3600);
        // 2026: DST starts March 8, ends November 1
        assert_eq!(ny_offset_s(at(2026, 3, 8, 6, 59)), -5 * 3600);
        assert_eq!(ny_offset_s(at(2026, 3, 8, 7, 1)), -4 * 3600);
        assert_eq!(ny_offset_s(at(2026, 11, 1, 5, 59)), -4 * 3600);
        assert_eq!(ny_offset_s(at(2026, 11, 1, 6, 1)), -5 * 3600);
    }

    #[test]
    fn solid_fx_week_and_daily_break() {
        let h = SessionHours::solid_fx();
        // Friday 9 Oct 2026 (EDT): 16:59 ET open, 17:00 ET closed for the weekend
        assert!(h.is_open(at(2026, 10, 9, 20, 59)).0);
        assert!(!h.is_open(at(2026, 10, 9, 21, 0)).0);
        // Saturday closed, Sunday 17:04 closed, 17:05 open
        assert!(!h.is_open(at(2026, 10, 10, 12, 0)).0);
        assert!(!h.is_open(at(2026, 10, 11, 21, 4)).0);
        assert!(h.is_open(at(2026, 10, 11, 21, 5)).0);
        // Tuesday 17:02 ET: daily break, reopening in 3 minutes
        let (open, wait) = h.is_open(at(2026, 10, 13, 21, 2));
        assert!(!open);
        assert_eq!(wait, 180);
        assert!(h.is_open(at(2026, 10, 13, 21, 5)).0);
    }
}
