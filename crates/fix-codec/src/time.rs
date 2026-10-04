//! UTCTimestamp formatting (`YYYYMMDD-HH:MM:SS.sss`) without external crates.

use std::time::{SystemTime, UNIX_EPOCH};

/// Formats `t` as a FIX UTCTimestamp with millisecond precision.
pub fn utc_timestamp(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let (y, m, day) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{day:02}-{:02}:{:02}:{:02}.{millis:03}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Current time as a FIX UTCTimestamp.
pub fn now_timestamp() -> String {
    utc_timestamp(SystemTime::now())
}

// Howard Hinnant's days-from-civil inverse.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn formats_known_instants() {
        assert_eq!(utc_timestamp(UNIX_EPOCH), "19700101-00:00:00.000");
        let t = UNIX_EPOCH + Duration::from_millis(1_790_000_000_123);
        assert_eq!(utc_timestamp(t), "20260921-14:13:20.123");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400); // 2000-02-29
        assert_eq!(utc_timestamp(leap), "20000229-00:00:00.000");
    }
}
