//! Admin journal: every mutating back-office command is appended (with its
//! actor) to `admin.jsonl` in the data directory before the response is
//! sent. Replaying the journal rebuilds the admin state (audit log, 4-eyes
//! queue, idempotency keys, credit, KYC, users, settings) deterministically.
//! Engine-affecting effects (deposits, group/symbol changes, closes) are
//! journaled as engine commands in the engine journal.

use super::auth::{Actor, Role};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    /// IB payout: the UTC days `[from, to)` (days since epoch) whose accrual
    /// this deposit pays; applying the op moves the IB's paid-through mark to `to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ib_days: Option<(u64, u64)>,
}

/// Introducing-broker terms (the commission share lives in `ib_share`).
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IbPlan {
    /// Rebate per closed lot (round turn), minor units of the IB's currency.
    #[serde(default)]
    pub per_lot_cents: i64,
    /// Share of a sub-IB's clients' revenue paid to this IB (percent, up to 3 levels).
    #[serde(default)]
    pub override_pct: u8,
    /// Referral code (`?ref=CODE`); empty = none.
    #[serde(default)]
    pub code: String,
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
    /// Tenant scope (stage 14): the user sees only this tenant's groups/accounts; None = all.
    #[serde(default)]
    pub tenant: Option<String>,
}

/// A brand / white-label broker sharing this core (stage 14).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantRec {
    pub id: String,
    pub name: String,
    /// Engine groups that belong to the tenant.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Public hostnames (terminal / console) for routing and branding.
    #[serde(default)]
    pub hostnames: Vec<String>,
    /// White label: accent colour (`#rrggbb`), logo (https URL), support e-mail.
    #[serde(default)]
    pub brand_color: String,
    #[serde(default)]
    pub logo_url: String,
    #[serde(default)]
    pub support_email: String,
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

/// Account-behaviour thresholds (parça 10a; Settings → Alerts). 0 turns a
/// flag off.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BehaviorThresholds {
    /// Hours of history scored (1..168).
    pub window_h: u32,
    /// A close held shorter than this is a scalp.
    pub scalper_hold_s: u32,
    pub scalper_min_closes: u32,
    /// Percent of closes that are scalps.
    pub scalper_pct: u8,
    /// Client orders in any 60 s.
    pub burst_per_min: u32,
    /// Connects in the window.
    pub churn_connects: u32,
    /// Failed authentications in the window (per account / per IP).
    pub auth_fails: u32,
    /// Distinct IPs in the window.
    pub ip_count: u32,
}

impl Default for BehaviorThresholds {
    fn default() -> BehaviorThresholds {
        BehaviorThresholds {
            window_h: 24,
            scalper_hold_s: 60,
            scalper_min_closes: 10,
            scalper_pct: 50,
            burst_per_min: 30,
            churn_connects: 30,
            auth_fails: 10,
            ip_count: 5,
        }
    }
}

/// A user of a connected trading platform (MT5 server behind the bridge),
/// as the platform reports it; keyed `institution:login`. Fields the
/// platform does not know stay empty; anything beyond the schema lands in
/// `extra` (name → value) so nothing the plugin sends is dropped.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlatformUser {
    pub institution: String,
    pub login: u64,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub country: String,
    pub city: String,
    pub address: String,
    pub group: String,
    pub server: String,
    pub leverage: u32,
    pub balance: f64,
    pub currency: String,
    pub registered_at: String,
    pub last_login_at: String,
    pub last_ip: String,
    pub status: String,
    pub comment: String,
    pub extra: BTreeMap<String, String>,
    /// Set by the store when the record was last upserted.
    pub updated_ms: u64,
}

impl PlatformUser {
    pub fn key(&self) -> String {
        format!("{}:{}", self.institution, self.login)
    }

    /// Length limits so a plugin cannot grow the journal without bound.
    pub fn validate(&self) -> Result<(), String> {
        if self.institution.is_empty() || self.institution.len() > 32 {
            return Err("institution 1-32 chars".into());
        }
        if self.login == 0 {
            return Err("login > 0".into());
        }
        let texts = [
            &self.name,
            &self.email,
            &self.phone,
            &self.country,
            &self.city,
            &self.address,
            &self.group,
            &self.server,
            &self.currency,
            &self.registered_at,
            &self.last_login_at,
            &self.last_ip,
            &self.status,
            &self.comment,
        ];
        if texts.iter().any(|t| t.len() > 200) {
            return Err("text fields ≤ 200 chars".into());
        }
        if self.extra.len() > 40
            || self
                .extra
                .iter()
                .any(|(k, v)| k.len() > 40 || v.len() > 200)
        {
            return Err("extra: ≤ 40 entries, key ≤ 40, value ≤ 200 chars".into());
        }
        Ok(())
    }
}

/// A dealer-defined alert (parça 10b, Settings → Alert rules): `metric`
/// read for `target` (symbol / currency / LP, "" = whole book) compared with
/// `threshold`; evaluated every 15 s next to the built-in conditions.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertRule {
    pub id: String,
    #[serde(default = "d_rule_on")]
    pub enabled: bool,
    /// One of [`super::alerts::RULE_METRICS`].
    pub metric: String,
    #[serde(default)]
    pub target: String,
    /// `gt` | `lt`
    #[serde(default = "d_rule_gt")]
    pub op: String,
    pub threshold: f64,
    /// `info` | `warning` | `critical`
    #[serde(default = "d_rule_warning")]
    pub severity: String,
    #[serde(default)]
    pub title: String,
}

fn d_rule_on() -> bool {
    true
}
fn d_rule_gt() -> String {
    "gt".into()
}
fn d_rule_warning() -> String {
    "warning".into()
}

/// Alert thresholds, channels and the daily operations report (stage 13).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertSettings {
    #[serde(default = "d_lp_down")]
    pub lp_down_grace_s: u64,
    /// Feed QoS: warn when an LP's market-data latency (ms) stays above this. 0 = off.
    #[serde(default = "d_lp_slow")]
    pub lp_slow_ms: u64,
    /// Account-behaviour flags (parça 10a).
    #[serde(default)]
    pub behavior: BehaviorThresholds,
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

fn d_lp_slow() -> u64 {
    2000
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
            lp_slow_ms: 2000,
            behavior: BehaviorThresholds::default(),
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
    /// USDT deposits: exact amount the hazine card asks for (micro-USDT).
    #[serde(default)]
    pub expected_micro: Option<i64>,
    /// On-chain transfer hazine reported for this request (`usdt_watch`).
    #[serde(default)]
    pub tx_hash: Option<String>,
    /// hazine.io payment link (id and the `/pay` card URL shown to the client).
    #[serde(default)]
    pub pay_id: Option<String>,
    #[serde(default)]
    pub pay_url: Option<String>,
    /// What actually arrived on chain (hazine `receivedAmount`, micro-USDT).
    /// hazine binds any transfer between 1x and 2x the invoice, so this can
    /// differ from `expected_micro`; only an exact match skips the 4-eyes step.
    #[serde(default)]
    pub received_micro: Option<i64>,
    /// hazine reported the payment after staff had already decided the
    /// request (rejected, or approved before the money came).
    #[serde(default)]
    pub paid_after_decision: bool,
    /// Staff member who reviewed such a late payment (clears the flag).
    #[serde(default)]
    pub late_handled_by: Option<String>,
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
    /// A hazine payment link was opened for a USDT deposit request. New
    /// requests carry the link in `FundingRequested`; kept for older journals.
    FundingPayLink {
        id: String,
        pay_id: String,
        pay_url: String,
        micro: Option<i64>,
    },
    /// hazine reported the payment link paid (no money moves). `micro` is the
    /// invoice amount hazine reports, `received` what actually arrived.
    FundingMatched {
        id: String,
        tx: String,
        micro: i64,
        #[serde(default)]
        received: Option<i64>,
    },
    /// Staff reviewed a payment that arrived after the request was decided.
    FundingLateHandled {
        id: String,
        note: Option<String>,
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
    /// Platform user records (bridge plugin or import), upserted by key.
    PlatformUsersUpserted {
        users: Vec<PlatformUser>,
    },
    /// Tenant registry (stage 14), full replacement.
    TenantsSaved {
        tenants: Vec<TenantRec>,
    },
    /// Copy trading strategy catalogue entry (created or edited).
    StrategySaved {
        strategy: super::copy_admin::StrategyRec,
    },
    /// Introducing-broker share of a parent account's children commission (percent).
    IbShareSet {
        account: u64,
        pct: u8,
    },
    /// IB rebate per lot, sub-IB override and referral code.
    IbPlanSet {
        account: u64,
        plan: IbPlan,
    },
    /// Which IB account a client belongs to (`None` = none).
    IbLinked {
        account: u64,
        ib: Option<u64>,
        /// The link counts for deals from this record's time on. Records
        /// written before link history existed lack it and count from 0.
        #[serde(default)]
        dated: bool,
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
    /// Manual broker hedge order (engine journal has the order itself).
    ManualHedge {
        details: String,
    },
    /// Dealer-defined alert rules, full replacement.
    AlertRulesSaved {
        rules: Vec<AlertRule>,
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
    /// Economic calendar (plan item 10): an event created or edited in the console.
    EconEventSaved {
        event: super::econ_calendar::EconEvent,
    },
    EconEventDeleted {
        id: String,
    },
    /// Feed import: only the new or changed events (by title, currency, time).
    EconEventsImported {
        events: Vec<super::econ_calendar::EconEvent>,
        added: usize,
        updated: usize,
        source: String,
    },
    /// Monthly statement e-mailed to the account's profile address
    /// (`month` = statement period `YYYY-MM`); the run's idempotency record.
    StatementMailed {
        month: String,
        account: u64,
    },
    /// One pass of a month's statement run finished.
    StatementPass {
        month: String,
        sent: u32,
        failed: u32,
    },
    /// A staff member mailed a statement to their own address (console test).
    StatementTestSent {
        month: String,
        account: u64,
    },
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct AdminRecord {
    pub seq: u64,
    pub ts: u64,
    pub actor: Actor,
    pub cmd: AdminCmd,
}

/// Monthly statement e-mail run of one period (`AdminState::statement_mail`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementMonth {
    /// Accounts already mailed for this month.
    #[serde(default)]
    pub sent: BTreeSet<u64>,
    #[serde(default)]
    pub passes: u32,
    /// Failures of the last pass.
    #[serde(default)]
    pub failed: u32,
    /// No more passes: everything went out, or the retries are used up.
    #[serde(default)]
    pub done: bool,
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
    /// Link history per client account, oldest first: (since ns, IB; `None` =
    /// unlinked). A deal belongs to the IB the client was linked to at its time.
    #[serde(default)]
    pub ib_links: BTreeMap<u64, Vec<(u64, Option<u64>)>>,
    /// IB account -> first UTC day (days since epoch) not paid out yet.
    #[serde(default)]
    pub ib_paid_to: BTreeMap<u64, u64>,
    /// IB terms beyond the share: per-lot rebate, override on sub-IBs, referral code.
    #[serde(default)]
    pub ib_plan: BTreeMap<u64, IbPlan>,
    /// Alert thresholds / channels (stage 13).
    #[serde(default)]
    pub alerts: AlertSettings,
    /// Routing-rule table history, oldest first (last 50).
    #[serde(default)]
    pub rule_versions: Vec<RuleVersion>,
    /// Tenants (stage 14).
    #[serde(default)]
    pub tenants: BTreeMap<String, TenantRec>,
    /// Platform (MT5) users reported by bridge plugins or imported, by `institution:login`.
    #[serde(default)]
    pub platform_users: BTreeMap<String, PlatformUser>,
    /// Dealer-defined alert rules (parça 10b).
    #[serde(default)]
    pub alert_rules: Vec<AlertRule>,
    /// Copy trading strategies by provider account.
    #[serde(default)]
    pub strategies: BTreeMap<u64, super::copy_admin::StrategyRec>,
    /// Economic calendar events by id.
    #[serde(default)]
    pub econ_events: BTreeMap<String, super::econ_calendar::EconEvent>,
    /// Monthly statement e-mail runs by period (`YYYY-MM`).
    #[serde(default)]
    pub statement_mail: BTreeMap<String, StatementMonth>,
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

    /// An introducing broker: has a share or plan, or linked clients.
    pub fn is_ib(&self, login: u64) -> bool {
        self.ib_share.contains_key(&login)
            || self.ib_plan.contains_key(&login)
            || self.ib_of.values().any(|i| *i == login)
    }

    /// The IB `account` was linked to at `ts`: the last link made at or
    /// before it. Without history (state built elsewhere) the current link.
    pub fn ib_at(&self, account: u64, ts: u64) -> Option<u64> {
        match self.ib_links.get(&account) {
            Some(h) => h.iter().rev().find(|(since, _)| *since <= ts)?.1,
            None => self.ib_of.get(&account).copied(),
        }
    }

    /// First UTC day (days since epoch) of an IB's unpaid accrual: the
    /// paid-through mark, or the day after the last payout booked before the
    /// mark existed (key `ibpay:<ib>:<from>:<to>`, `to` an inclusive date).
    pub fn ib_paid_from(&self, ib: u64) -> u64 {
        let prefix = format!("ibpay:{ib}:");
        let legacy = self
            .ops
            .values()
            .filter(|o| o.account == ib && o.ib_days.is_none() && o.status != OpStatus::Rejected)
            .filter_map(|o| {
                let to = o
                    .idempotency_key
                    .strip_prefix(&prefix)?
                    .rsplit(':')
                    .next()?;
                super::routes::parse_iso_ns(to).map(|ns| ns / 86_400_000_000_000 + 1)
            })
            .max()
            .unwrap_or(0);
        self.ib_paid_to.get(&ib).copied().unwrap_or(0).max(legacy)
    }

    /// An IB payout of `ib` waiting for its second approval.
    pub fn ib_payout_pending(&self, ib: u64) -> Option<&BalanceOp> {
        let prefix = format!("ibpay:{ib}:");
        self.ops.values().find(|o| {
            o.account == ib
                && o.status == OpStatus::PendingApproval
                && (o.ib_days.is_some() || o.idempotency_key.starts_with(&prefix))
        })
    }

    fn apply_ib_payout(&mut self, op: &BalanceOp) {
        if let Some((_, to)) = op.ib_days {
            let paid = self.ib_paid_to.entry(op.account).or_default();
            *paid = (*paid).max(to);
        }
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
                self.apply_ib_payout(op);
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
                self.apply_ib_payout(&op);
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
                    match (&req.pay_id, req.expected_micro) {
                        (Some(pay), Some(m)) => format!(
                            "{} {} {} via {:?}: hazine {pay}, {} USDT",
                            req.id,
                            req.amount,
                            req.currency,
                            req.method,
                            super::usdt_watch::fmt_micro(m)
                        ),
                        _ => format!(
                            "{} {} {} via {:?}: {}",
                            req.id, req.amount, req.currency, req.method, req.details
                        ),
                    },
                )
            }
            AdminCmd::FundingPayLink {
                id,
                pay_id,
                pay_url,
                micro,
            } => {
                let acc = self.funding.get(id).map(|f| f.account).unwrap_or(0);
                if let Some(f) = self.funding.get_mut(id) {
                    f.pay_id = Some(pay_id.clone());
                    f.pay_url = Some(pay_url.clone());
                    f.expected_micro = *micro;
                }
                self.audit(
                    r,
                    "funding.paylink".into(),
                    format!("#{acc}"),
                    format!("{id}: hazine {pay_id}"),
                )
            }
            AdminCmd::FundingMatched {
                id,
                tx,
                micro,
                received,
            } => {
                let mut acc = 0;
                let mut late = None;
                if let Some(f) = self.funding.get_mut(id) {
                    acc = f.account;
                    f.tx_hash = Some(tx.clone());
                    f.received_micro = *received;
                    // money that arrives after the decision needs a human look
                    if f.status != FundingStatus::Requested {
                        f.paid_after_decision = true;
                        late = Some(f.status);
                    }
                }
                let got = received.map_or("?".into(), super::usdt_watch::fmt_micro);
                self.audit(
                    r,
                    "funding.onchain".into(),
                    format!("#{acc}"),
                    format!(
                        "{id}: received {got} USDT for {} USDT, tx {tx}{}{}",
                        super::usdt_watch::fmt_micro(*micro),
                        if *received == Some(*micro) {
                            ""
                        } else {
                            " (amount differs)"
                        },
                        late.map(|s| format!(", after the request was {s:?}"))
                            .unwrap_or_default()
                    ),
                )
            }
            AdminCmd::FundingLateHandled { id, note } => {
                let mut acc = 0;
                if let Some(f) = self.funding.get_mut(id) {
                    acc = f.account;
                    f.late_handled_by = Some(r.actor.name.clone());
                }
                self.audit(
                    r,
                    "funding.late_handled".into(),
                    format!("#{acc}"),
                    format!(
                        "{id}{}",
                        note.as_deref()
                            .map(|n| format!(" ({n})"))
                            .unwrap_or_default()
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
            AdminCmd::IbLinked { account, ib, dated } => {
                let old = self.ib_of.get(account).copied();
                match ib {
                    Some(i) => {
                        self.ib_of.insert(*account, *i);
                    }
                    None => {
                        self.ib_of.remove(account);
                    }
                }
                let since = if *dated { r.ts } else { 0 };
                self.ib_links
                    .entry(*account)
                    .or_default()
                    .push((since, *ib));
                self.audit(
                    r,
                    "ib.link".into(),
                    format!("#{account}"),
                    format!("IB {:?} → {:?}", old, ib),
                )
            }
            AdminCmd::StrategySaved { strategy } => {
                self.strategies.insert(strategy.account, strategy.clone());
                self.audit(
                    r,
                    "copy.strategy".into(),
                    format!("#{}", strategy.account),
                    format!(
                        "{} fee {} bps{}",
                        strategy.name,
                        strategy.perf_fee_bps,
                        if strategy.public { ", public" } else { "" }
                    ),
                )
            }
            AdminCmd::IbPlanSet { account, plan } => {
                let old = self
                    .ib_plan
                    .insert(*account, plan.clone())
                    .unwrap_or_default();
                self.audit(
                    r,
                    "ib.plan".into(),
                    format!("#{account}"),
                    format!(
                        "lot {}→{} c, override {}→{}%, code {:?}→{:?}",
                        old.per_lot_cents,
                        plan.per_lot_cents,
                        old.override_pct,
                        plan.override_pct,
                        old.code,
                        plan.code
                    ),
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
            AdminCmd::ManualHedge { details } => {
                self.audit(r, "risk.manualHedge".into(), "lp".into(), details.clone())
            }
            AdminCmd::AlertRulesSaved { rules } => {
                self.alert_rules = rules.clone();
                self.audit(
                    r,
                    "alerts.rules".into(),
                    "alerts".into(),
                    format!("{} rule(s)", rules.len()),
                )
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
            AdminCmd::EconEventSaved { event } => {
                self.econ_events.insert(event.id.clone(), event.clone());
                self.audit(
                    r,
                    "calendar.event".into(),
                    event.id.clone(),
                    format!(
                        "{} {} {} ({})",
                        super::views::iso(event.time_ns),
                        event.currency,
                        event.title,
                        event.impact.as_str()
                    ),
                )
            }
            AdminCmd::EconEventDeleted { id } => {
                let old = self.econ_events.remove(id);
                self.audit(
                    r,
                    "calendar.delete".into(),
                    id.clone(),
                    old.map(|e| format!("{} {}", e.currency, e.title))
                        .unwrap_or_default(),
                )
            }
            AdminCmd::EconEventsImported {
                events,
                added,
                updated,
                source,
            } => {
                for e in events {
                    self.econ_events.insert(e.id.clone(), e.clone());
                }
                self.audit(
                    r,
                    "calendar.import".into(),
                    source.clone(),
                    format!("{added} added, {updated} updated"),
                )
            }
            AdminCmd::StatementMailed { month, account } => {
                self.statement_mail
                    .entry(month.clone())
                    .or_default()
                    .sent
                    .insert(*account);
                self.audit(
                    r,
                    "statement.email".into(),
                    format!("#{account}"),
                    format!("monthly statement {month}"),
                )
            }
            AdminCmd::StatementPass {
                month,
                sent,
                failed,
            } => {
                let m = self.statement_mail.entry(month.clone()).or_default();
                m.passes += 1;
                m.failed = *failed;
                m.done = *failed == 0 || m.passes >= super::statement::MAX_PASSES;
                let pass = m.passes;
                self.audit(
                    r,
                    "statement.run".into(),
                    month.clone(),
                    format!("pass {pass}: {sent} sent, {failed} failed"),
                )
            }
            AdminCmd::StatementTestSent { month, account } => self.audit(
                r,
                "statement.test".into(),
                format!("#{account}"),
                format!("statement {month} sent to the requester"),
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
            AdminCmd::PlatformUsersUpserted { users } => {
                for u in users {
                    let mut u = u.clone();
                    u.updated_ms = r.ts / 1_000_000;
                    self.platform_users.insert(u.key(), u);
                }
                self.audit(
                    r,
                    "platform.users".into(),
                    "platform".into(),
                    format!("{} user record(s) upserted", users.len()),
                )
            }
            AdminCmd::TenantsSaved { tenants } => {
                self.tenants = tenants.iter().map(|t| (t.id.clone(), t.clone())).collect();
                self.audit(
                    r,
                    "tenants.update".into(),
                    "tenants".into(),
                    tenants
                        .iter()
                        .map(|t| format!("{} ({} groups)", t.id, t.groups.len()))
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            }
            AdminCmd::AlertSettingsSaved { settings } => {
                self.alerts = settings.clone();
                self.audit(
                    r,
                    "settings.alerts".into(),
                    "alerts".into(),
                    format!(
                        "lp down {}s, feed slow >{}ms, fill ≥{}% (≥{} orders), latency >{}ms ×{}, webhook {}, telegram {}, quiet {:?}, daily report {:?}",
                        settings.lp_down_grace_s,
                        settings.lp_slow_ms,
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
