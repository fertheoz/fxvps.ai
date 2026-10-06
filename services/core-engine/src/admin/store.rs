//! Admin journal: every mutating back-office command is appended (with its
//! actor) to `admin.jsonl` in the data directory before the response is
//! sent. Replaying the journal rebuilds the admin state (audit log, 4-eyes
//! queue, idempotency keys, credit, KYC, users, settings) deterministically.
//! Engine-affecting effects (deposits, group/symbol changes, closes) are
//! journaled as engine commands in the engine journal.

use super::auth::{Actor, Role};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceKind {
    Deposit,
    Withdraw,
    Credit,
}

impl BalanceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BalanceKind::Deposit => "deposit",
            BalanceKind::Withdraw => "withdraw",
            BalanceKind::Credit => "credit",
        }
    }
    pub fn permission(self) -> &'static str {
        match self {
            BalanceKind::Deposit => "balance.deposit",
            BalanceKind::Withdraw => "balance.withdraw",
            BalanceKind::Credit => "balance.credit",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    Applied,
    PendingApproval,
    Rejected,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceOp {
    pub id: String,
    pub account: u64,
    pub kind: BalanceKind,
    /// Minor units, > 0.
    pub amount: i64,
    pub currency: String,
    pub reason: String,
    pub idempotency_key: String,
    pub requested_by: Actor,
    pub requested_at: u64,
    pub status: OpStatus,
    pub decided_by: Option<Actor>,
    pub decided_at: Option<u64>,
    pub decision_note: Option<String>,
    pub new_balance: i64,
    pub new_credit: i64,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserRec {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: Role,
    pub mfa: bool,
    pub active: bool,
    pub last_login: Option<String>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsRec {
    pub broker_name: String,
    pub base_currency: String,
    pub four_eyes_threshold: i64,
    pub session_timeout_min: u32,
    pub require_mfa: bool,
    pub default_book: String,
    /// Our own LEI, stamped as executing entity on transaction reports.
    #[serde(default)]
    pub broker_lei: String,
    /// Shown to clients in the terminal's funding dialog (stage 12).
    #[serde(default)]
    pub funding: FundingInstructions,
}

/// Alert thresholds, channels and the daily operations report (stage 13).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertSettings {
    #[serde(default = "d_lp_down")]
    pub lp_down_grace_s: u64,
    #[serde(default = "d_fr_orders")]
    pub fill_rate_min_orders: u32,
    /// Percent, e.g. 90.
    #[serde(default = "d_fr_floor")]
    pub fill_rate_floor_pct: u8,
    #[serde(default = "d_lat_floor")]
    pub latency_floor_ms: u32,
    /// p95 of the last 15 min vs the previous hour (× this).
    #[serde(default = "d_lat_mult")]
    pub latency_multiplier: u8,
    /// Outgoing webhook (JSON POST); overrides `CORE_ALERT_WEBHOOK_URL` when set.
    #[serde(default)]
    pub webhook_url: String,
    /// Telegram bot token (write-only in the API) and chat id.
    #[serde(default)]
    pub telegram_token: String,
    #[serde(default)]
    pub telegram_chat_id: String,
    /// UTC hours `[from, to)` in which only critical alerts are sent; None = always.
    #[serde(default)]
    pub quiet_hours_utc: Option<(u8, u8)>,
    /// UTC hour of the daily operations report (Telegram/webhook); None = off.
    #[serde(default)]
    pub daily_report_hour_utc: Option<u8>,
}

fn d_lp_down() -> u64 {
    60
}
fn d_fr_orders() -> u32 {
    10
}
fn d_fr_floor() -> u8 {
    90
}
fn d_lat_floor() -> u32 {
    500
}
fn d_lat_mult() -> u8 {
    3
}

impl Default for AlertSettings {
    fn default() -> AlertSettings {
        AlertSettings {
            lp_down_grace_s: 60,
            fill_rate_min_orders: 10,
            fill_rate_floor_pct: 90,
            latency_floor_ms: 500,
            latency_multiplier: 3,
            webhook_url: String::new(),
            telegram_token: String::new(),
            telegram_chat_id: String::new(),
            quiet_hours_utc: None,
            daily_report_hour_utc: None,
        }
    }
}

/// A saved routing-rule table (stage 13: versioning / restore).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleVersion {
    pub id: String,
    pub at: u64,
    pub actor: String,
    pub count: usize,
    /// JSON array of `RoutingRule`.
    pub rules_json: String,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingInstructions {
    /// Deposit address for USDT on TRON (TRC-20); empty = crypto deposits off.
    #[serde(default)]
    pub usdt_trc20_address: String,
    /// Free-text bank transfer instructions (beneficiary, IBAN, reference rule).
    #[serde(default)]
    pub bank_details: String,
    /// Minor units of the account currency; 0 = no minimum.
    #[serde(default)]
    pub min_deposit_minor: i64,
    #[serde(default)]
    pub min_withdraw_minor: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingKind {
    Deposit,
    Withdraw,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingMethod {
    UsdtTrc20,
    Bank,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingStatus {
    Requested,
    /// Deposit credited / withdrawal debited from the trading account.
    Approved,
    Rejected,
    /// Withdrawal money actually sent (after `Approved`).
    Paid,
}

/// A client's deposit / withdrawal request (self-service), decided by staff.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingRequest {
    pub id: String,
    pub account: u64,
    pub kind: FundingKind,
    pub method: FundingMethod,
    /// Minor units of the account currency.
    pub amount: i64,
    pub currency: String,
    /// Client-supplied reference: tx hash / sender name / destination address or IBAN.
    pub details: String,
    pub requested_by: String,
    pub requested_at: u64,
    pub status: FundingStatus,
    pub decided_by: Option<String>,
    pub decided_at: Option<u64>,
    pub note: Option<String>,
    /// Balance operation created on approval.
    pub op_id: Option<String>,
}

/// Metadata of an uploaded KYC document (bytes live under `<data_dir>/kyc/<account>/`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KycDoc {
    pub id: String,
    pub account: u64,
    /// `id_front` | `id_back` | `proof_of_address` | `selfie` | `other`
    pub kind: String,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
    pub uploaded_by: String,
    pub uploaded_at: u64,
}

impl Default for SettingsRec {
    fn default() -> SettingsRec {
        SettingsRec {
            broker_name: "fxvps.ai".into(),
            base_currency: "USD".into(),
            four_eyes_threshold: 1_000_000,
            session_timeout_min: 30,
            require_mfa: true,
            default_book: "A".into(),
            broker_lei: String::new(),
            funding: FundingInstructions::default(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AuditRec {
    pub id: String,
    /// ns since epoch.
    pub at: u64,
    pub actor: String,
    pub role: Role,
    pub action: String,
    pub target: String,
    pub details: String,
    /// Hash chain (stage 11): `hash = sha256(prev_hash | id | at | actor | role | action | target | details)`.
    /// Records written before the chain existed carry empty strings.
    #[serde(default)]
    pub prev_hash: String,
    #[serde(default)]
    pub hash: String,
}

/// Chain link of one audit record over its predecessor's hash.
pub fn audit_hash(
    prev: &str,
    id: &str,
    at: u64,
    actor: &str,
    role: Role,
    action: &str,
    target: &str,
    details: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for part in [
        prev,
        id,
        &at.to_string(),
        actor,
        &format!("{role:?}"),
        action,
        target,
        details,
    ] {
        h.update(part.as_bytes());
        h.update([0u8]);
    }
    format!("{:x}", h.finalize())
}

/// Mutating admin commands (the admin journal payload).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminCmd {
    /// Applied immediately (below the 4-eyes threshold).
    BalanceApplied {
        op: BalanceOp,
    },
    /// Queued for a second approver.
    BalanceQueued {
        op: BalanceOp,
    },
    BalanceApproved {
        id: String,
        new_balance: i64,
        new_credit: i64,
    },
    BalanceRejected {
        id: String,
        note: String,
    },
    KycSet {
        account: u64,
        kyc: String,
        #[serde(default)]
        note: Option<String>,
    },
    AccountOpened {
        account: u64,
        group: String,
        profile: ClientProfile,
    },
    /// Reporting identity of a client (stage 11).
    ProfileUpdated {
        account: u64,
        lei: Option<String>,
    },
    /// Client self-service (stage 12).
    FundingRequested {
        req: FundingRequest,
    },
    FundingDecided {
        id: String,
        status: FundingStatus,
        note: Option<String>,
        op_id: Option<String>,
    },
    KycDocAdded {
        doc: KycDoc,
    },
    /// Alert thresholds / channels (stage 13).
    AlertSettingsSaved {
        settings: AlertSettings,
    },
    /// Introducing-broker share of a parent account's children commission (percent).
    IbShareSet {
        account: u64,
        pct: u8,
    },
    /// Which IB account a client belongs to (`None` = none).
    IbLinked {
        account: u64,
        ib: Option<u64>,
    },
    GroupSaved {
        group: String,
        details: String,
    },
    RulesSaved {
        count: usize,
        /// Full table as JSON (stage 13 versioning); empty in older records.
        #[serde(default)]
        rules_json: String,
    },
    SymbolSaved {
        symbol: String,
        details: String,
    },
    PresetApplied {
        group: String,
        preset: String,
    },
    ForceClosed {
        positions: Vec<u64>,
        closed: u32,
    },
    UserSaved {
        user: AdminUserRec,
    },
    SettingsSaved {
        settings: SettingsRec,
    },
    /// LP connection settings stored on the fix-gateway (no secrets in `details`).
    LpConfigSaved {
        details: String,
    },
    /// Multi-LP aggregation policy (applied to the running aggregator too).
    AggregationSaved {
        cfg: crate::lp_agg::AggConfig,
    },
    /// B-book exposure / auto-hedge policy (lives in the engine journal).
    HedgeSaved {
        details: String,
    },
    /// Rollover schedule (engine journal).
    SwapConfigSaved {
        details: String,
    },
    /// Manual rollover run from the console.
    RolloverRun {
        details: String,
    },
    /// Operational alert raised by the evaluator (stage 9).
    AlertRaised {
        kind: String,
        target: String,
        detail: String,
    },
    AccountGroupSet {
        account: u64,
        group: String,
        old: String,
    },
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct AdminRecord {
    pub seq: u64,
    pub ts: u64,
    pub actor: Actor,
    pub cmd: AdminCmd,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientProfile {
    pub name: String,
    pub email: String,
    /// Legal Entity Identifier for transaction reporting (20 chars) when the client is a legal person.
    #[serde(default)]
    pub lei: Option<String>,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct AdminState {
    pub seq: u64,
    pub ops: BTreeMap<String, BalanceOp>,
    /// Idempotency key -> op id.
    pub keys: BTreeMap<String, String>,
    pub credit: BTreeMap<u64, i64>,
    pub kyc: BTreeMap<u64, String>,
    /// Client name / e-mail of accounts opened via `POST /v1/accounts`.
    #[serde(default)]
    pub profiles: BTreeMap<u64, ClientProfile>,
    pub users: BTreeMap<String, AdminUserRec>,
    pub settings: SettingsRec,
    /// Multi-LP aggregation policy (stage 6); `None` = engine default.
    #[serde(default)]
    pub aggregation: Option<crate::lp_agg::AggConfig>,
    /// Client funding requests and KYC documents (stage 12), IB shares (account -> pct).
    #[serde(default)]
    pub funding: BTreeMap<String, FundingRequest>,
    #[serde(default)]
    pub kyc_docs: Vec<KycDoc>,
    #[serde(default)]
    pub ib_share: BTreeMap<u64, u8>,
    /// client account -> IB account
    #[serde(default)]
    pub ib_of: BTreeMap<u64, u64>,
    /// Alert thresholds / channels (stage 13).
    #[serde(default)]
    pub alerts: AlertSettings,
    /// Routing-rule table history, oldest first (last 50).
    #[serde(default)]
    pub rule_versions: Vec<RuleVersion>,
    /// Oldest first.
    pub audit: Vec<AuditRec>,
}

fn money(minor: i64, ccy: &str) -> String {
    let d = money::Currency::new(
        ccy.as_bytes()
            .get(..3)
            .and_then(|b| <[u8; 3]>::try_from(b).ok())
            .unwrap_or(*b"USD"),
    )
    .minor_exponent();
    format!("{} {ccy}", money::format_scaled(minor as i128, d))
}

impl AdminState {
    pub fn credit_of(&self, account: u64) -> i64 {
        self.credit.get(&account).copied().unwrap_or(0)
    }
    pub fn kyc_of(&self, account: u64) -> &str {
        self.kyc.get(&account).map_or("none", |s| s.as_str())
    }

    fn audit(&mut self, r: &AdminRecord, action: String, target: String, details: String) {
        let prev_hash = self
            .audit
            .last()
            .map(|a| a.hash.clone())
            .unwrap_or_default();
        let id = format!("a{}", r.seq);
        let hash = audit_hash(
            &prev_hash,
            &id,
            r.ts,
            &r.actor.name,
            r.actor.role,
            &action,
            &target,
            &details,
        );
        self.audit.push(AuditRec {
            id,
            at: r.ts,
            actor: r.actor.name.clone(),
            role: r.actor.role,
            action,
            target,
            details,
            prev_hash,
            hash,
        });
    }

    /// Recomputes the whole chain: `Ok(head)` when every link matches, else the
    /// first broken record id.
    pub fn verify_chain(&self) -> Result<String, String> {
        let mut prev = String::new();
        for a in &self.audit {
            if a.hash.is_empty() {
                // pre-chain record: the chain starts after it
                prev.clear();
                continue;
            }
            let expect = audit_hash(
                &prev, &a.id, a.at, &a.actor, a.role, &a.action, &a.target, &a.details,
            );
            if a.prev_hash != prev || a.hash != expect {
                return Err(a.id.clone());
            }
            prev = a.hash.clone();
        }
        Ok(prev)
    }

    fn apply_credit(&mut self, op: &BalanceOp) {
        if op.kind == BalanceKind::Credit {
            *self.credit.entry(op.account).or_default() += op.amount;
        }
    }

    /// Deterministic state transition (live path and replay).
    pub fn apply(&mut self, r: &AdminRecord) {
        self.seq = r.seq;
        match &r.cmd {
            AdminCmd::BalanceApplied { op } => {
                self.apply_credit(op);
                self.keys.insert(op.idempotency_key.clone(), op.id.clone());
                self.ops.insert(op.id.clone(), op.clone());
                self.audit(
                    r,
                    format!("balance.{}", op.kind.as_str()),
                    format!("#{}", op.account),
                    format!(
                        "{} — {} [{}]",
                        money(op.amount, &op.currency),
                        op.reason,
                        op.idempotency_key
                    ),
                );
            }
            AdminCmd::BalanceQueued { op } => {
                self.keys.insert(op.idempotency_key.clone(), op.id.clone());
                self.ops.insert(op.id.clone(), op.clone());
                self.audit(
                    r,
                    format!("balance.{}.requested", op.kind.as_str()),
                    format!("#{}", op.account),
                    format!(
                        "{} — {} — awaiting 2nd approval ({}) [{}]",
                        money(op.amount, &op.currency),
                        op.reason,
                        op.id,
                        op.idempotency_key
                    ),
                );
            }
            AdminCmd::BalanceApproved {
                id,
                new_balance,
                new_credit,
            } => {
                let Some(mut op) = self.ops.get(id).cloned() else {
                    return;
                };
                op.status = OpStatus::Applied;
                op.decided_by = Some(r.actor.clone());
                op.decided_at = Some(r.ts);
                op.new_balance = *new_balance;
                op.new_credit = *new_credit;
                self.apply_credit(&op);
                self.ops.insert(id.clone(), op.clone());
                self.audit(
                    r,
                    format!("balance.{}.approved", op.kind.as_str()),
                    format!("#{}", op.account),
                    format!(
                        "{} ({}) requested by {}",
                        money(op.amount, &op.currency),
                        op.id,
                        op.requested_by.name
                    ),
                );
            }
            AdminCmd::BalanceRejected { id, note } => {
                let Some(op) = self.ops.get_mut(id) else {
                    return;
                };
                op.status = OpStatus::Rejected;
                op.decided_by = Some(r.actor.clone());
                op.decided_at = Some(r.ts);
                op.decision_note = Some(note.clone());
                let (kind, account, amount) = (op.kind, op.account, money(op.amount, &op.currency));
                self.audit(
                    r,
                    format!("balance.{}.rejected", kind.as_str()),
                    format!("#{account}"),
                    format!("{amount} ({id}) — {note}"),
                );
            }
            AdminCmd::AccountOpened {
                account,
                group,
                profile,
            } => {
                self.profiles.insert(*account, profile.clone());
                self.audit(
                    r,
                    "account.open".into(),
                    format!("#{account}"),
                    format!("{} <{}> in {group}", profile.name, profile.email),
                );
            }
            AdminCmd::KycSet { account, kyc, note } => {
                let old = self.kyc_of(*account).to_string();
                self.kyc.insert(*account, kyc.clone());
                self.audit(
                    r,
                    "kyc.update".into(),
                    format!("#{account}"),
                    format!(
                        "{old} → {kyc}{}",
                        note.as_deref()
                            .map(|n| format!(" ({n})"))
                            .unwrap_or_default()
                    ),
                );
            }
            AdminCmd::LpConfigSaved { details } => {
                self.audit(r, "lp.config".into(), "fix-gateway".into(), details.clone())
            }
            AdminCmd::SwapConfigSaved { details } => {
                self.audit(r, "settings.swap".into(), "engine".into(), details.clone())
            }
            AdminCmd::RolloverRun { details } => self.audit(
                r,
                "settings.rollover".into(),
                "engine".into(),
                details.clone(),
            ),
            AdminCmd::ProfileUpdated { account, lei } => {
                let p = self.profiles.entry(*account).or_default();
                let old = p.lei.clone().unwrap_or_default();
                p.lei = lei.clone();
                self.audit(
                    r,
                    "profile.update".into(),
                    format!("#{account}"),
                    format!("LEI {old} → {}", lei.clone().unwrap_or_default()),
                )
            }
            AdminCmd::FundingRequested { req } => {
                self.funding.insert(req.id.clone(), req.clone());
                self.audit(
                    r,
                    format!(
                        "funding.{}",
                        match req.kind {
                            FundingKind::Deposit => "deposit",
                            FundingKind::Withdraw => "withdraw",
                        }
                    ),
                    format!("#{}", req.account),
                    format!(
                        "{} {} {} via {:?}: {}",
                        req.id, req.amount, req.currency, req.method, req.details
                    ),
                )
            }
            AdminCmd::FundingDecided {
                id,
                status,
                note,
                op_id,
            } => {
                if let Some(f) = self.funding.get_mut(id) {
                    f.status = *status;
                    f.decided_by = Some(r.actor.name.clone());
                    f.decided_at = Some(r.ts);
                    f.note = note.clone();
                    if op_id.is_some() {
                        f.op_id = op_id.clone();
                    }
                }
                let acc = self.funding.get(id).map(|f| f.account).unwrap_or(0);
                self.audit(
                    r,
                    "funding.decide".into(),
                    format!("#{acc}"),
                    format!(
                        "{id} → {status:?}{}",
                        note.as_deref()
                            .map(|n| format!(" ({n})"))
                            .unwrap_or_default()
                    ),
                )
            }
            AdminCmd::KycDocAdded { doc } => {
                self.kyc_docs.push(doc.clone());
                self.audit(
                    r,
                    "kyc.document".into(),
                    format!("#{}", doc.account),
                    format!(
                        "{} {} ({} B, {})",
                        doc.kind,
                        doc.filename,
                        doc.size,
                        &doc.sha256[..12.min(doc.sha256.len())]
                    ),
                )
            }
            AdminCmd::IbLinked { account, ib } => {
                let old = self.ib_of.get(account).copied();
                match ib {
                    Some(i) => {
                        self.ib_of.insert(*account, *i);
                    }
                    None => {
                        self.ib_of.remove(account);
                    }
                }
                self.audit(
                    r,
                    "ib.link".into(),
                    format!("#{account}"),
                    format!("IB {:?} → {:?}", old, ib),
                )
            }
            AdminCmd::IbShareSet { account, pct } => {
                let old = self.ib_share.get(account).copied().unwrap_or(0);
                if *pct == 0 {
                    self.ib_share.remove(account);
                } else {
                    self.ib_share.insert(*account, *pct);
                }
                self.audit(
                    r,
                    "ib.share".into(),
                    format!("#{account}"),
                    format!("{old}% → {pct}%"),
                )
            }
            AdminCmd::AlertRaised {
                kind,
                target,
                detail,
            } => self.audit(r, format!("alert.{kind}"), target.clone(), detail.clone()),
            AdminCmd::HedgeSaved { details } => {
                self.audit(r, "risk.hedge".into(), "engine".into(), details.clone())
            }
            AdminCmd::AggregationSaved { cfg } => {
                self.aggregation = Some(cfg.clone());
                let lps: Vec<String> = cfg
                    .lps
                    .iter()
                    .map(|p| format!("{}{}", p.name, if p.enabled { "" } else { " (off)" }))
                    .collect();
                self.audit(
                    r,
                    "lp.aggregation".into(),
                    "aggregator".into(),
                    format!("{:?}; {}", cfg.mode, lps.join(", ")),
                )
            }
            AdminCmd::AccountGroupSet {
                account,
                group,
                old,
            } => self.audit(
                r,
                "account.group".into(),
                format!("#{account}"),
                format!("{old} → {group}"),
            ),
            AdminCmd::GroupSaved { group, details } => {
                self.audit(r, "group.update".into(), group.clone(), details.clone())
            }
            AdminCmd::RulesSaved { count, rules_json } => {
                if !rules_json.is_empty() {
                    self.rule_versions.push(RuleVersion {
                        id: format!("rv{}", r.seq),
                        at: r.ts,
                        actor: r.actor.name.clone(),
                        count: *count,
                        rules_json: rules_json.clone(),
                    });
                    if self.rule_versions.len() > 50 {
                        let cut = self.rule_versions.len() - 50;
                        self.rule_versions.drain(..cut);
                    }
                }
                self.audit(
                    r,
                    "rules.update".into(),
                    "routing".into(),
                    format!("{count} rules"),
                )
            }
            AdminCmd::AlertSettingsSaved { settings } => {
                self.alerts = settings.clone();
                self.audit(
                    r,
                    "settings.alerts".into(),
                    "alerts".into(),
                    format!(
                        "lp down {}s, fill ≥{}% (≥{} orders), latency >{}ms ×{}, webhook {}, telegram {}, quiet {:?}, daily report {:?}",
                        settings.lp_down_grace_s,
                        settings.fill_rate_floor_pct,
                        settings.fill_rate_min_orders,
                        settings.latency_floor_ms,
                        settings.latency_multiplier,
                        if settings.webhook_url.is_empty() { "off" } else { "on" },
                        if settings.telegram_chat_id.is_empty() { "off" } else { "on" },
                        settings.quiet_hours_utc,
                        settings.daily_report_hour_utc
                    ),
                )
            }
            AdminCmd::SymbolSaved { symbol, details } => {
                self.audit(r, "symbol.update".into(), symbol.clone(), details.clone())
            }
            AdminCmd::PresetApplied { group, preset } => {
                self.audit(r, "risk.applyPreset".into(), group.clone(), preset.clone())
            }
            AdminCmd::ForceClosed { positions, closed } => {
                let ids: Vec<String> = positions.iter().map(|p| format!("#{p}")).collect();
                self.audit(
                    r,
                    "position.forceClose".into(),
                    ids.join(","),
                    format!("Closed {closed} of {} positions", positions.len()),
                )
            }
            AdminCmd::UserSaved { user } => {
                self.users.insert(user.id.clone(), user.clone());
                self.audit(
                    r,
                    "user.update".into(),
                    user.email.clone(),
                    format!(
                        "role {}, mfa {}, active {}",
                        user.role.as_str(),
                        user.mfa,
                        user.active
                    ),
                );
            }
            AdminCmd::SettingsSaved { settings } => {
                self.settings = settings.clone();
                self.audit(
                    r,
                    "settings.update".into(),
                    "settings".into(),
                    format!("4-eyes threshold {}", settings.four_eyes_threshold),
                );
            }
        }
    }
}

pub fn journal_path(dir: &Path) -> PathBuf {
    dir.join("admin.jsonl")
}

/// Reads all records of the admin journal (a torn last line is ignored).
pub fn read_journal(dir: &Path) -> std::io::Result<Vec<AdminRecord>> {
    let mut out = Vec::new();
    let Ok(f) = File::open(journal_path(dir)) else {
        return Ok(out);
    };
    for line in BufReader::new(f).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(r) = serde_json::from_str::<AdminRecord>(&line) else {
            break;
        };
        out.push(r);
    }
    Ok(out)
}

/// Replays the admin journal into a fresh state.
pub fn replay(dir: &Path) -> std::io::Result<AdminState> {
    let mut st = AdminState::default();
    for r in read_journal(dir)? {
        st.apply(&r);
    }
    Ok(st)
}

pub struct AdminStore {
    pub state: AdminState,
    file: File,
    last_ts: u64,
}

impl AdminStore {
    pub fn open(dir: &Path) -> std::io::Result<AdminStore> {
        std::fs::create_dir_all(dir)?;
        let state = replay(dir)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(journal_path(dir))?;
        Ok(AdminStore {
            state,
            file,
            last_ts: 0,
        })
    }

    /// Next record sequence number (used to derive ids before appending).
    pub fn next_seq(&self) -> u64 {
        self.state.seq + 1
    }

    pub fn now(&mut self) -> u64 {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        self.last_ts = t.max(self.last_ts + 1);
        self.last_ts
    }

    /// Appends (fsync'd) and applies a record.
    pub fn append(&mut self, actor: &Actor, cmd: AdminCmd) -> std::io::Result<AdminRecord> {
        let r = AdminRecord {
            seq: self.next_seq(),
            ts: self.now(),
            actor: actor.clone(),
            cmd,
        };
        let mut line = serde_json::to_vec(&r).map_err(std::io::Error::other)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.sync_data()?;
        self.state.apply(&r);
        Ok(r)
    }
}
