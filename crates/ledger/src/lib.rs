//! Double-entry, append-only, event-sourced ledger.
//!
//! Account hierarchy: `LpOmnibus` (master account held at the LP) ->
//! `BrokerBook` -> `Client` sub-accounts. `External` accounts (bank/PSP, LP
//! counterparty) sit outside the tree and absorb the other side of cash
//! movements and LP-settled P&L.
//!
//! Every transaction is a set of [`Posting`]s whose amounts sum to zero per
//! currency. State is derived solely from the [`JournalEvent`] stream, so a
//! ledger can be rebuilt by replay, or from a [`LedgerSnapshot`] plus the
//! events recorded after it.
//!
//! Sign convention: a positive balance is what the system owes the account
//! holder (client equity is positive after a deposit; the external bank
//! account goes negative by the same amount).

use money::{Currency, Money, MoneyError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Debug)]
#[serde(transparent)]
pub struct AccountId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum AccountKind {
    /// Master account at a liquidity provider (omnibus).
    LpOmnibus,
    /// The broker's own book (revenue, B-book counterparty).
    BrokerBook,
    /// Client trading sub-account.
    Client,
    /// Outside world: bank/PSP, LP counterparty.
    External,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct AccountInfo {
    pub id: AccountId,
    pub kind: AccountKind,
    pub parent: Option<AccountId>,
    pub name: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum TxnKind {
    Deposit,
    Withdrawal,
    Adjustment,
    RealizedPnl,
    Commission,
    Swap,
    NegativeBalanceProtection,
    Transfer,
}

impl TxnKind {
    /// Kinds that must not drive a client account below zero.
    pub fn enforces_funds(self) -> bool {
        matches!(self, TxnKind::Withdrawal | TxnKind::Transfer)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct Posting {
    pub account: AccountId,
    pub amount: Money,
}

impl Posting {
    pub fn new(account: AccountId, amount: Money) -> Posting {
        Posting { account, amount }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct TxnRequest {
    pub idempotency_key: String,
    pub kind: TxnKind,
    pub postings: Vec<Posting>,
    pub memo: String,
    /// Caller-supplied timestamp (ns); the ledger never reads a clock.
    pub ts: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct Txn {
    pub id: u64,
    pub fingerprint: u64,
    pub req: TxnRequest,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum JournalEvent {
    AccountOpened(AccountInfo),
    Posted(Txn),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    #[error("unknown account {0:?}")]
    UnknownAccount(AccountId),
    #[error("account {0:?} already exists")]
    DuplicateAccount(AccountId),
    #[error("invalid parent for {0:?}")]
    InvalidParent(AccountId),
    #[error("transaction needs at least two postings")]
    TooFewPostings,
    #[error("zero-amount posting")]
    ZeroPosting,
    #[error("unbalanced transaction in {0}: {1}")]
    Unbalanced(Currency, i128),
    #[error("insufficient funds on {0:?}")]
    InsufficientFunds(AccountId),
    #[error("idempotency key reused with different content: {0}")]
    IdempotencyConflict(String),
    #[error("money error: {0}")]
    Money(#[from] MoneyError),
    #[error("journal sequence mismatch: expected {expected}, got {got}")]
    SeqMismatch { expected: u64, got: u64 },
}

/// Result of a post: either a new transaction or a replayed duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostOutcome {
    Posted(u64),
    Duplicate(u64),
}

impl PostOutcome {
    pub fn txn_id(self) -> u64 {
        match self {
            PostOutcome::Posted(id) | PostOutcome::Duplicate(id) => id,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct LedgerSnapshot {
    /// Number of journal events folded into this snapshot.
    pub seq: u64,
    pub accounts: Vec<AccountInfo>,
    pub balances: Vec<(AccountId, Currency, i128)>,
    pub idempotency: Vec<(String, u64, u64)>,
    pub next_txn: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Ledger {
    accounts: BTreeMap<AccountId, AccountInfo>,
    balances: BTreeMap<(AccountId, Currency), i128>,
    /// key -> (txn id, fingerprint)
    idem: BTreeMap<String, (u64, u64)>,
    next_txn: u64,
    /// Sequence number of `journal[0]`.
    base_seq: u64,
    journal: Vec<JournalEvent>,
}

/// FNV-1a over the canonical JSON of (kind, postings) -- stable across runs.
fn fingerprint(req: &TxnRequest) -> u64 {
    let bytes = serde_json::to_vec(&(&req.kind, &req.postings)).unwrap_or_default();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl Ledger {
    pub fn new() -> Ledger {
        Ledger {
            next_txn: 1,
            ..Default::default()
        }
    }

    /// Next journal sequence number (= total number of events ever applied).
    pub fn seq(&self) -> u64 {
        self.base_seq + self.journal.len() as u64
    }

    pub fn account(&self, id: AccountId) -> Option<&AccountInfo> {
        self.accounts.get(&id)
    }

    pub fn accounts(&self) -> impl Iterator<Item = &AccountInfo> {
        self.accounts.values()
    }

    pub fn balance(&self, id: AccountId, ccy: Currency) -> Money {
        Money::new(*self.balances.get(&(id, ccy)).unwrap_or(&0), ccy)
    }

    /// All non-zero balances of an account.
    pub fn balances_of(&self, id: AccountId) -> Vec<Money> {
        self.balances
            .range((id, Currency::new([0; 3]))..=(id, Currency::new([255; 3])))
            .filter(|(_, v)| **v != 0)
            .map(|((_, c), v)| Money::new(*v, *c))
            .collect()
    }

    /// Balance of an account plus all of its descendants.
    pub fn subtree_balance(&self, id: AccountId, ccy: Currency) -> Money {
        let mut total = self.balance(id, ccy).minor;
        for a in self.accounts.values() {
            if a.parent == Some(id) {
                total += self.subtree_balance(a.id, ccy).minor;
            }
        }
        Money::new(total, ccy)
    }

    /// Journal events with sequence number `>= seq` still held in memory.
    pub fn events_since(&self, seq: u64) -> &[JournalEvent] {
        let start = seq.saturating_sub(self.base_seq) as usize;
        &self.journal[start.min(self.journal.len())..]
    }

    pub fn open_account(
        &mut self,
        id: AccountId,
        kind: AccountKind,
        parent: Option<AccountId>,
        name: impl Into<String>,
    ) -> Result<(), LedgerError> {
        if self.accounts.contains_key(&id) {
            return Err(LedgerError::DuplicateAccount(id));
        }
        let parent_ok = match (kind, parent.and_then(|p| self.accounts.get(&p))) {
            (AccountKind::LpOmnibus | AccountKind::External, None) => parent.is_none(),
            (AccountKind::BrokerBook, Some(p)) => p.kind == AccountKind::LpOmnibus,
            (AccountKind::BrokerBook, None) => parent.is_none(),
            (AccountKind::Client, Some(p)) => p.kind == AccountKind::BrokerBook,
            _ => false,
        };
        if !parent_ok {
            return Err(LedgerError::InvalidParent(id));
        }
        let ev = JournalEvent::AccountOpened(AccountInfo {
            id,
            kind,
            parent,
            name: name.into(),
        });
        self.apply_event(ev);
        Ok(())
    }

    /// Validates and appends a transaction. Idempotent on `idempotency_key`.
    pub fn post(&mut self, req: TxnRequest) -> Result<PostOutcome, LedgerError> {
        let fp = fingerprint(&req);
        if let Some(&(id, f)) = self.idem.get(&req.idempotency_key) {
            return if f == fp {
                Ok(PostOutcome::Duplicate(id))
            } else {
                Err(LedgerError::IdempotencyConflict(req.idempotency_key))
            };
        }
        self.validate(&req)?;
        let id = self.next_txn;
        self.apply_event(JournalEvent::Posted(Txn {
            id,
            fingerprint: fp,
            req,
        }));
        Ok(PostOutcome::Posted(id))
    }

    fn validate(&self, req: &TxnRequest) -> Result<(), LedgerError> {
        if req.postings.len() < 2 {
            return Err(LedgerError::TooFewPostings);
        }
        let mut sums: BTreeMap<Currency, i128> = BTreeMap::new();
        let mut deltas: BTreeMap<(AccountId, Currency), i128> = BTreeMap::new();
        for p in &req.postings {
            if !self.accounts.contains_key(&p.account) {
                return Err(LedgerError::UnknownAccount(p.account));
            }
            if p.amount.is_zero() {
                return Err(LedgerError::ZeroPosting);
            }
            let s = sums.entry(p.amount.currency).or_default();
            *s = s.checked_add(p.amount.minor).ok_or(MoneyError::Overflow)?;
            let d = deltas.entry((p.account, p.amount.currency)).or_default();
            *d = d.checked_add(p.amount.minor).ok_or(MoneyError::Overflow)?;
        }
        if let Some((c, s)) = sums.into_iter().find(|(_, s)| *s != 0) {
            return Err(LedgerError::Unbalanced(c, s));
        }
        for ((acc, ccy), d) in deltas {
            let cur = *self.balances.get(&(acc, ccy)).unwrap_or(&0);
            let new = cur.checked_add(d).ok_or(MoneyError::Overflow)?;
            let kind = self.accounts[&acc].kind;
            if req.kind.enforces_funds() && kind == AccountKind::Client && d < 0 && new < 0 {
                return Err(LedgerError::InsufficientFunds(acc));
            }
        }
        Ok(())
    }

    fn apply_event(&mut self, ev: JournalEvent) {
        match &ev {
            JournalEvent::AccountOpened(info) => {
                self.accounts.insert(info.id, info.clone());
            }
            JournalEvent::Posted(txn) => {
                for p in &txn.req.postings {
                    *self
                        .balances
                        .entry((p.account, p.amount.currency))
                        .or_default() += p.amount.minor;
                }
                self.idem
                    .insert(txn.req.idempotency_key.clone(), (txn.id, txn.fingerprint));
                self.next_txn = self.next_txn.max(txn.id + 1);
            }
        }
        self.journal.push(ev);
    }

    /// Rebuilds a ledger from a full event stream (re-validating each txn).
    pub fn replay<'a>(
        events: impl IntoIterator<Item = &'a JournalEvent>,
    ) -> Result<Ledger, LedgerError> {
        let mut l = Ledger::new();
        for ev in events {
            l.replay_one(ev)?;
        }
        Ok(l)
    }

    fn replay_one(&mut self, ev: &JournalEvent) -> Result<(), LedgerError> {
        match ev {
            JournalEvent::AccountOpened(a) => {
                self.open_account(a.id, a.kind, a.parent, a.name.clone())
            }
            JournalEvent::Posted(t) => {
                self.validate(&t.req)?;
                self.apply_event(ev.clone());
                Ok(())
            }
        }
    }

    pub fn snapshot(&self) -> LedgerSnapshot {
        LedgerSnapshot {
            seq: self.seq(),
            accounts: self.accounts.values().cloned().collect(),
            balances: self
                .balances
                .iter()
                .map(|((a, c), v)| (*a, *c, *v))
                .collect(),
            idempotency: self
                .idem
                .iter()
                .map(|(k, (id, f))| (k.clone(), *id, *f))
                .collect(),
            next_txn: self.next_txn,
        }
    }

    /// Restores from a snapshot and applies the events recorded after it.
    pub fn restore<'a>(
        snap: &LedgerSnapshot,
        tail: impl IntoIterator<Item = &'a JournalEvent>,
    ) -> Result<Ledger, LedgerError> {
        let mut l = Ledger {
            accounts: snap.accounts.iter().map(|a| (a.id, a.clone())).collect(),
            balances: snap
                .balances
                .iter()
                .map(|(a, c, v)| ((*a, *c), *v))
                .collect(),
            idem: snap
                .idempotency
                .iter()
                .map(|(k, id, f)| (k.clone(), (*id, *f)))
                .collect(),
            next_txn: snap.next_txn,
            base_seq: snap.seq,
            journal: Vec::new(),
        };
        for ev in tail {
            l.replay_one(ev)?;
        }
        Ok(l)
    }

    /// Checks global invariants: every currency sums to zero across all
    /// accounts, and the in-memory journal tail is consistent.
    pub fn check_invariants(&self) -> Result<(), String> {
        let mut sums: BTreeMap<Currency, i128> = BTreeMap::new();
        for ((_, c), v) in &self.balances {
            *sums.entry(*c).or_default() += v;
        }
        for (c, s) in sums {
            if s != 0 {
                return Err(format!("currency {c} sums to {s}"));
            }
        }
        for ev in &self.journal {
            if let JournalEvent::Posted(t) = ev {
                let mut per: BTreeMap<Currency, i128> = BTreeMap::new();
                for p in &t.req.postings {
                    *per.entry(p.amount.currency).or_default() += p.amount.minor;
                }
                if per.values().any(|v| *v != 0) {
                    return Err(format!("txn {} unbalanced", t.id));
                }
            }
        }
        Ok(())
    }

    /// Canonical, comparable view of state (for replay determinism checks).
    pub fn state_digest(&self) -> String {
        let s = self.snapshot();
        serde_json::to_string(&(s.accounts, s.balances, s.idempotency, s.next_txn))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
