//! FIX wire log of the trading session: every order-related frame, both
//! directions, appended to `<dir>/YYYY-MM-DD.jsonl` with the tags the
//! back office needs to find it again (ClOrdID, OrigClOrdID, OrderID,
//! ExecID). The LP's "send us the FIX message you received" is answered from
//! here. Admin and market-data frames are not kept. Nothing is ever deleted.

use std::io::Write;
use std::path::PathBuf;

use fix_codec::RawMessage;
use fix_session::WireFrame;
use serde::Serialize;
use tokio::sync::mpsc;

/// One logged frame.
#[derive(Debug, Clone, Serialize)]
pub struct WireLine {
    pub ts_ms: u64,
    pub lp: String,
    pub dir: &'static str,
    pub msg_type: String,
    pub cl_ord_id: Option<String>,
    pub orig_cl_ord_id: Option<String>,
    pub order_id: Option<String>,
    pub exec_id: Option<String>,
    /// The frame with SOH shown as `|`.
    pub raw: String,
}

/// Session-level and market-data message types are noise here.
fn keep(msg_type: &[u8]) -> bool {
    !matches!(
        msg_type,
        b"0" | b"1" | b"2" | b"4" | b"5" | b"A" | b"V" | b"W" | b"X" | b"Y"
    )
}

/// Builds the log line of a frame, or `None` for frames that are not kept.
pub fn line(lp: &str, f: &WireFrame, ts_ms: u64) -> Option<WireLine> {
    let raw = RawMessage::parse(&f.bytes).ok()?;
    let mt = raw.msg_type();
    if !keep(mt) {
        return None;
    }
    let s = |tag: u32| {
        raw.view()
            .get(tag)
            .map(|v| String::from_utf8_lossy(v).into_owned())
    };
    Some(WireLine {
        ts_ms,
        lp: lp.to_string(),
        dir: if f.outbound { "out" } else { "in" },
        msg_type: String::from_utf8_lossy(mt).into_owned(),
        cl_ord_id: s(11),
        orig_cl_ord_id: s(41),
        order_id: s(37),
        exec_id: s(17),
        raw: String::from_utf8_lossy(&f.bytes).replace('\u{1}', "|"),
    })
}

/// Civil date (UTC) of a UNIX millisecond stamp: `YYYY-MM-DD`.
pub fn day(ts_ms: u64) -> String {
    // Howard Hinnant's days-from-civil inverse
    let z = (ts_ms / 86_400_000) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Starts the writer task; returns the tap to hand to the session.
pub fn spawn(dir: PathBuf, lp: String) -> mpsc::Sender<WireFrame> {
    let (tx, mut rx) = mpsc::channel::<WireFrame>(4096);
    tokio::spawn(async move {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(error = %e, dir = %dir.display(), "fix log dir");
        }
        while let Some(f) = rx.recv().await {
            let ts = now_ms();
            let Some(l) = line(&lp, &f, ts) else {
                continue;
            };
            let path = dir.join(format!("{}.jsonl", day(ts)));
            let r = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .and_then(|mut fh| {
                    let mut s = serde_json::to_string(&l).unwrap_or_default();
                    s.push('\n');
                    fh.write_all(s.as_bytes())
                });
            if let Err(e) = r {
                tracing::warn!(error = %e, path = %path.display(), "fix log write");
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_of_known_stamps() {
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(1_791_492_994_926), "2026-10-08");
        assert_eq!(day(951_782_400_000), "2000-02-29");
    }

    /// A complete frame around `body` (fields with `|` for SOH): BodyLength and CheckSum computed.
    fn frame(body: &str) -> Vec<u8> {
        let body = body.replace('|', "\u{1}");
        let mut m = format!("8=FIX.4.4\u{1}9={}\u{1}{body}", body.len()).into_bytes();
        let sum: u32 = m.iter().map(|b| u32::from(*b)).sum();
        m.extend_from_slice(format!("10={:03}\u{1}", sum % 256).as_bytes());
        m
    }

    #[test]
    fn order_frames_are_kept_with_their_tags_and_heartbeats_are_not() {
        let nos = frame(
            "35=D|49=A|56=B|34=7|52=20261008-20:36:03.000|11=LP-170-r2|41=LP-170-r1|55=XAU/USD|",
        );
        let l = line(
            "LMAX",
            &WireFrame {
                outbound: true,
                bytes: nos,
            },
            5,
        )
        .expect("kept");
        assert_eq!((l.dir, l.msg_type.as_str()), ("out", "D"));
        assert_eq!(l.cl_ord_id.as_deref(), Some("LP-170-r2"));
        assert_eq!(l.orig_cl_ord_id.as_deref(), Some("LP-170-r1"));
        assert!(l.raw.contains("|35=D|") && !l.raw.contains('\u{1}'));
        let hb = frame("35=0|49=A|56=B|34=8|52=20261008-20:36:03.000|");
        assert!(line(
            "LMAX",
            &WireFrame {
                outbound: false,
                bytes: hb
            },
            5
        )
        .is_none());
    }
}
