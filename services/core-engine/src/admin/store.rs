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
    },
    AccountOpened {
        account: u64,
        group: String,
        profile: ClientProfile,
    },
    GroupSaved {
        group: String,
        details: String,
    },
    RulesSaved {
        count: usize,
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
        self.audit.push(AuditRec {
            id: format!("a{}", r.seq),
            at: r.ts,
            actor: r.actor.name.clone(),
            role: r.actor.role,
            action,
            target,
            details,
        });
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
            AdminCmd::KycSet { account, kyc } => {
                let old = self.kyc_of(*account).to_string();
                self.kyc.insert(*account, kyc.clone());
                self.audit(
                    r,
                    "kyc.update".into(),
                    format!("#{account}"),
                    format!("{old} → {kyc}"),
                );
            }
            AdminCmd::LpConfigSaved { details } => {
                self.audit(r, "lp.config".into(), "fix-gateway".into(), details.clone())
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
            AdminCmd::RulesSaved { count } => self.audit(
                r,
                "rules.update".into(),
                "routing".into(),
                format!("{count} rules"),
            ),
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
