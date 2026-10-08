//! USDT (TRC-20) deposits through hazine.io, the same way MT5ForexVPS takes
//! crypto payments: a deposit request first opens a hazine payment link
//! (`POST /api/paylink-api`; hazine binds it to its collection address and
//! tags the amount so open invoices never collide) and is journaled only once
//! the link exists. The client pays on the hazine `/pay` card (wallet connect,
//! QR, mobile wallets), and this task asks hazine every minute
//! (`POST /api/paylink-api/<id>/check`) about every linked deposit of the
//! last four days, whatever its status. A paid link records the tx hash and
//! the amount that actually arrived; crediting still goes through staff
//! approval, the watcher never moves money. A payment that arrives after
//! staff decided the request is flagged (console + alert) until reviewed.
//!
//! hazine marks a link paid for any unbound transfer between 1x and 2x the
//! invoice amount, so only an exact match (received == the tagged amount)
//! lets a single approval credit the deposit; anything else keeps four-eyes.
//!
//! `HAZINE_PAYLINK_KEY` (server-side secret) turns it on; `HAZINE_BASE_URL`
//! defaults to `https://hazine.io`. Without a key crypto deposits stay manual.

use super::store::{AdminCmd, FundingKind, FundingMethod, FundingRequest, FundingStatus};
use super::AdminCtx;
use crate::admin::auth::Actor;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// A link stays payable a day; hazine still binds payments that arrive up to
/// 72 h after expiry (plus its ~1 min finality buffer), so linked requests
/// are checked for four days and an hour.
const LINK_LIFETIME_MIN: u64 = 24 * 60;
const CHECK_WINDOW_NS: u64 = (4 * 24 + 1) * 3_600_000_000_000;
/// One cent in micro-USDT; hazine's uniqueness tag stays below one USDT.
const MICRO_PER_CENT: i64 = 10_000;
const MAX_TAG_MICRO: i64 = 1_000_000;

/// hazine.io paylink API access of this server.
#[derive(Clone)]
pub struct Hazine {
    /// e.g. `https://hazine.io` (no trailing slash).
    pub base_url: String,
    pub key: String,
    /// How often the watcher asks about linked deposits.
    pub poll: Duration,
}

// the key never goes to logs
impl std::fmt::Debug for Hazine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hazine")
            .field("base_url", &self.base_url)
            .field("poll", &self.poll)
            .finish_non_exhaustive()
    }
}

/// A hazine payment link for a deposit request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayLink {
    pub id: String,
    pub url: String,
    /// Tagged amount the card asks for (micro-USDT).
    pub micro: i64,
}

/// hazine's answer for a paid link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paid {
    pub tx: String,
    /// Invoice (tagged) amount, micro-USDT.
    pub invoice: Option<i64>,
    /// What actually arrived (`receivedAmount`), micro-USDT.
    pub received: Option<i64>,
}

impl Hazine {
    pub fn new(base_url: &str, key: &str) -> Hazine {
        Hazine {
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
            poll: Duration::from_secs(60),
        }
    }

    /// `HAZINE_PAYLINK_KEY` (unset or empty: off) and `HAZINE_BASE_URL`.
    pub fn from_env() -> Option<Hazine> {
        let key = std::env::var("HAZINE_PAYLINK_KEY")
            .ok()
            .filter(|k| !k.is_empty())?;
        let base = std::env::var("HAZINE_BASE_URL")
            .ok()
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "https://hazine.io".into());
        Some(Hazine::new(&base, &key))
    }

    async fn call(&self, http: &reqwest::Client, path: &str, body: Value) -> Result<Value, String> {
        let r = http
            .post(format!("{}/api{path}", self.base_url))
            .bearer_auth(&self.key)
            .json(&body)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| format!("hazine unreachable: {e}"))?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(format!(
                "hazine {path} HTTP {status}: {}",
                v["message"].as_str().unwrap_or("")
            ));
        }
        Ok(v)
    }

    /// Opens a payment link for `amount` minor units (USD cents) of `account`.
    pub async fn open_link(
        &self,
        http: &reqwest::Client,
        account: u64,
        amount: i64,
    ) -> Result<PayLink, String> {
        let v = self
            .call(
                http,
                "/paylink-api",
                json!({
                    "amount": format!("{}.{:02}", amount / 100, amount % 100),
                    "reference": format!("fxvps.ai deposit #{account}"),
                    "expiresInMin": LINK_LIFETIME_MIN,
                }),
            )
            .await?;
        let id = match &v["id"] {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
        if !plain_token(&id) {
            return Err("hazine returned no usable link id".into());
        }
        let micro = amount_of(&v["amount"]).ok_or("hazine returned no amount")?;
        if !covers(micro, amount) {
            return Err(format!(
                "hazine asks {} USDT for a {amount} cent deposit",
                fmt_micro(micro)
            ));
        }
        Ok(PayLink {
            url: format!("{}/pay?id={id}", self.base_url),
            micro,
            id,
        })
    }

    /// `Some` once hazine has bound a transfer to the link.
    pub async fn check(
        &self,
        http: &reqwest::Client,
        pay_id: &str,
    ) -> Result<Option<Paid>, String> {
        if !plain_token(pay_id) {
            return Err(format!("bad link id {pay_id:?}"));
        }
        let v = self
            .call(http, &format!("/paylink-api/{pay_id}/check"), json!({}))
            .await?;
        Ok(parse_check(&v))
    }
}

/// Link ids and tx hashes end up in URLs: letters, digits and dashes only.
fn plain_token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The tagged amount `micro` covers `cents` and adds at most the tag.
fn covers(micro: i64, cents: i64) -> bool {
    cents
        .checked_mul(MICRO_PER_CENT)
        .is_some_and(|base| micro >= base && micro - base < MAX_TAG_MICRO)
}

/// "100.000001" -> 100_000_001 micro-USDT. `None` for anything but a plain
/// non-negative decimal; digits past the sixth decimal must be zeros (a
/// rounded value must never pass as an exact one).
pub fn str_to_micro(s: &str) -> Option<i64> {
    let s = s.trim();
    let (a, b) = s.split_once('.').unwrap_or((s, ""));
    if a.is_empty()
        || !a.bytes().all(|c| c.is_ascii_digit())
        || !b.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    if b.len() > 6 && b.bytes().skip(6).any(|c| c != b'0') {
        return None;
    }
    let frac: String = format!("{b:0<6}").chars().take(6).collect();
    a.parse::<i64>()
        .ok()?
        .checked_mul(1_000_000)?
        .checked_add(frac.parse().ok()?)
}

fn amount_of(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => str_to_micro(s),
        Value::Number(n) => str_to_micro(&n.to_string()),
        _ => None,
    }
}

/// 100_000_001 -> "100.000001".
pub fn fmt_micro(m: i64) -> String {
    let sign = if m < 0 { "-" } else { "" };
    let a = m.unsigned_abs();
    format!("{sign}{}.{:06}", a / 1_000_000, a % 1_000_000)
}

/// hazine's `/check` (public view of the link): paid with a tx hash, or `None`.
pub fn parse_check(v: &Value) -> Option<Paid> {
    if v["status"].as_str() != Some("paid") {
        return None;
    }
    let tx = v["txHash"].as_str().filter(|t| plain_token(t))?;
    Some(Paid {
        tx: tx.to_string(),
        invoice: amount_of(&v["amount"]),
        received: amount_of(&v["receivedAmount"]),
    })
}

fn is_card_deposit(f: &FundingRequest) -> bool {
    f.kind == FundingKind::Deposit && f.method == FundingMethod::UsdtTrc20 && f.pay_id.is_some()
}

/// Linked USDT deposits hazine has not reported paid yet, whatever their
/// status: the card stays payable after staff reject or approve a request.
pub fn links_to_check(
    funding: &BTreeMap<String, FundingRequest>,
    now: u64,
) -> Vec<(String, String)> {
    funding
        .values()
        .filter(|f| {
            is_card_deposit(f)
                && f.tx_hash.is_none()
                && now.saturating_sub(f.requested_at) < CHECK_WINDOW_NS
        })
        .filter_map(|f| Some((f.id.clone(), f.pay_id.clone()?)))
        .collect()
}

/// hazine saw exactly the tagged amount of this request (which covers the
/// amount to credit): one internal approval is enough, hazine's on-chain
/// match is the second pair of eyes. Over- or odd payments and unknown
/// amounts keep the four-eyes rule.
pub fn exact_payment(f: &FundingRequest) -> bool {
    let (Some(expected), Some(received)) = (f.expected_micro, f.received_micro) else {
        return false;
    };
    is_card_deposit(f)
        && f.currency == "USD"
        && f.tx_hash.is_some()
        && received == expected
        && covers(expected, f.amount)
}

/// A payment that arrived after the decision and nobody has reviewed yet.
pub fn late_unhandled(f: &FundingRequest) -> bool {
    f.paid_after_decision && f.late_handled_by.is_none()
}

/// Reason line of the balance operation an approval creates: what hazine
/// has seen so far, so the second approver does not have to look it up.
pub fn approval_note(f: &FundingRequest) -> Option<String> {
    let pay = f.pay_id.as_ref()?;
    let usdt = |m: Option<i64>| m.map_or_else(|| "?".to_string(), fmt_micro);
    Some(match &f.tx_hash {
        Some(tx) => format!(
            "hazine {pay}: received {} USDT for {} USDT, tx {tx}",
            usdt(f.received_micro),
            usdt(f.expected_micro)
        ),
        None => format!(
            "hazine {pay}: {} USDT not seen on chain yet",
            usdt(f.expected_micro)
        ),
    })
}

pub fn spawn(ctx: AdminCtx) {
    let Some(hz) = ctx.hazine.clone() else {
        return;
    };
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(hz.poll);
        loop {
            iv.tick().await;
            let links = links_to_check(&ctx.store.lock().await.state.funding, domain::now_ns());
            let mut changed = false;
            for (id, pay_id) in links {
                let paid = match hz.check(&ctx.http, &pay_id).await {
                    Ok(Some(p)) => p,
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::warn!(request = %id, error = %e, "hazine check failed");
                        continue;
                    }
                };
                let mut store = ctx.store.lock().await;
                let Some(f) = store.state.funding.get(&id) else {
                    continue;
                };
                if f.tx_hash.is_some() {
                    continue;
                }
                let decided = f.status != FundingStatus::Requested;
                let micro = paid.invoice.or(f.expected_micro).unwrap_or(0);
                if let Err(e) = store.append(
                    &Actor::system(),
                    AdminCmd::FundingMatched {
                        id: id.clone(),
                        tx: paid.tx.clone(),
                        micro,
                        received: paid.received,
                    },
                ) {
                    tracing::error!(error = %e, "hazine: journal write failed");
                    break;
                }
                if decided {
                    tracing::warn!(request = %id, tx = %paid.tx, "usdt paid after the request was decided");
                } else {
                    tracing::info!(request = %id, tx = %paid.tx, "usdt deposit paid on hazine");
                }
                changed = true;
            }
            if changed {
                ctx.notify(&["listFunding", "listAudit"]);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str, pay: Option<&str>, status: FundingStatus, at: u64) -> FundingRequest {
        FundingRequest {
            id: id.into(),
            account: 1,
            kind: FundingKind::Deposit,
            method: FundingMethod::UsdtTrc20,
            amount: 10_000,
            currency: "USD".into(),
            details: String::new(),
            requested_by: "c".into(),
            requested_at: at,
            status,
            decided_by: None,
            decided_at: None,
            note: None,
            op_id: None,
            expected_micro: Some(100_000_001),
            tx_hash: None,
            pay_id: pay.map(str::to_string),
            pay_url: None,
            received_micro: None,
            paid_after_decision: false,
            late_handled_by: None,
        }
    }

    #[test]
    fn amounts_to_micro() {
        assert_eq!(str_to_micro("100.000001"), Some(100_000_001));
        assert_eq!(str_to_micro("12"), Some(12_000_000));
        assert_eq!(str_to_micro("0.5"), Some(500_000));
        assert_eq!(str_to_micro("150.50000000"), Some(150_500_000));
        // a value hazine rounded or printed oddly never passes as exact
        assert_eq!(str_to_micro("100.0000019"), None);
        assert_eq!(str_to_micro("1e-7"), None);
        assert_eq!(str_to_micro("-5"), None);
        assert_eq!(str_to_micro(".5"), None);
        assert_eq!(str_to_micro("x"), None);
        assert_eq!(fmt_micro(100_000_001), "100.000001");
        assert_eq!(fmt_micro(0), "0.000000");
    }

    #[test]
    fn check_reads_the_received_amount() {
        // hazine publicView: `amount` is the invoice, `receivedAmount` what came
        let v = json!({"id": "a1", "status": "paid", "amount": "100.000001",
                       "receivedAmount": "150", "txHash": "abc123"});
        assert_eq!(
            parse_check(&v),
            Some(Paid {
                tx: "abc123".into(),
                invoice: Some(100_000_001),
                received: Some(150_000_000),
            })
        );
        let pending =
            json!({"status": "pending", "amount": "100", "receivedAmount": "0", "txHash": ""});
        assert_eq!(parse_check(&pending), None);
        let expired = json!({"status": "expired", "amount": "100"});
        assert_eq!(parse_check(&expired), None);
        // a paid view without a usable hash is not a match
        let odd =
            json!({"status": "paid", "amount": "100", "receivedAmount": "100", "txHash": "<x>"});
        assert_eq!(parse_check(&odd), None);
    }

    #[test]
    fn every_linked_unpaid_deposit_is_checked_whatever_its_status() {
        let now = 10 * 86_400_000_000_000;
        let mut m = BTreeMap::new();
        let mut paid = req("e", Some("p5"), FundingStatus::Requested, now - 1);
        paid.tx_hash = Some("t".into());
        let mut bank = req("f", Some("p6"), FundingStatus::Requested, now - 1);
        bank.method = FundingMethod::Bank;
        for f in [
            req("a", Some("p1"), FundingStatus::Requested, now - 1),
            req("b", None, FundingStatus::Requested, now - 1),
            req("c", Some("p3"), FundingStatus::Rejected, now - 1),
            req(
                "d",
                Some("p4"),
                FundingStatus::Approved,
                now - 96 * 3_600_000_000_000,
            ),
            paid,
            bank,
            req("g", Some("p7"), FundingStatus::Requested, 1),
        ] {
            m.insert(f.id.clone(), f);
        }
        assert_eq!(
            links_to_check(&m, now),
            vec![
                ("a".to_string(), "p1".to_string()),
                ("c".to_string(), "p3".to_string()),
                ("d".to_string(), "p4".to_string()),
            ]
        );
    }

    #[test]
    fn only_an_exact_payment_counts_as_the_second_approval() {
        let mut f = req("a", Some("p1"), FundingStatus::Requested, 1);
        f.tx_hash = Some("t".into());
        f.received_micro = Some(100_000_001);
        assert!(exact_payment(&f));
        // overpayment hazine still binds (1x..2x)
        f.received_micro = Some(150_000_000);
        assert!(!exact_payment(&f));
        // unknown received amount (older journal, odd format)
        f.received_micro = None;
        assert!(!exact_payment(&f));
        // the tagged amount must cover what gets credited
        f.received_micro = Some(100_000_001);
        f.amount = 20_000;
        assert!(!exact_payment(&f));
        f.amount = 10_000;
        f.currency = "EUR".into();
        assert!(!exact_payment(&f));
        f.currency = "USD".into();
        f.tx_hash = None;
        assert!(!exact_payment(&f));
    }

    #[test]
    fn link_amount_must_cover_the_deposit() {
        assert!(covers(100_000_000, 10_000));
        assert!(covers(100_000_999, 10_000));
        assert!(!covers(99_999_999, 10_000));
        assert!(!covers(101_000_000, 10_000));
        assert!(!covers(1, i64::MAX));
    }
}
