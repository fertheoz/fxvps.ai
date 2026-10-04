use super::*;
const USD: money::Currency = money::Currency::USD;
const EUR: money::Currency = money::Currency::EUR;
use proptest::prelude::*;

const BANK: AccountId = AccountId(1);
const LP: AccountId = AccountId(2);
const OMNI: AccountId = AccountId(10);
const BOOK: AccountId = AccountId(11);

fn usd(minor: i128) -> Money {
    Money::new(minor, USD)
}

fn setup(clients: u64) -> Ledger {
    let mut l = Ledger::new();
    l.open_account(BANK, AccountKind::External, None, "bank")
        .unwrap();
    l.open_account(LP, AccountKind::External, None, "lp-cpty")
        .unwrap();
    l.open_account(OMNI, AccountKind::LpOmnibus, None, "omnibus")
        .unwrap();
    l.open_account(BOOK, AccountKind::BrokerBook, Some(OMNI), "book")
        .unwrap();
    for i in 0..clients {
        l.open_account(
            AccountId(100 + i),
            AccountKind::Client,
            Some(BOOK),
            format!("c{i}"),
        )
        .unwrap();
    }
    l
}

fn req(key: &str, kind: TxnKind, p: Vec<(AccountId, Money)>) -> TxnRequest {
    TxnRequest {
        idempotency_key: key.into(),
        kind,
        postings: p.into_iter().map(|(a, m)| Posting::new(a, m)).collect(),
        memo: String::new(),
        ts: 0,
    }
}

#[test]
fn deposit_and_hierarchy() {
    let mut l = setup(2);
    l.post(req(
        "d1",
        TxnKind::Deposit,
        vec![(BANK, usd(-1000)), (AccountId(100), usd(1000))],
    ))
    .unwrap();
    l.post(req(
        "d2",
        TxnKind::Deposit,
        vec![(BANK, usd(-500)), (AccountId(101), usd(500))],
    ))
    .unwrap();
    l.post(req(
        "c1",
        TxnKind::Commission,
        vec![(AccountId(100), usd(-7)), (BOOK, usd(7))],
    ))
    .unwrap();
    assert_eq!(l.balance(AccountId(100), USD), usd(993));
    assert_eq!(l.subtree_balance(BOOK, USD), usd(1500));
    assert_eq!(l.subtree_balance(OMNI, USD), usd(1500));
    assert_eq!(
        l.subtree_balance(OMNI, USD).minor,
        -l.balance(BANK, USD).minor
    );
    l.check_invariants().unwrap();
}

#[test]
fn rejects_bad_transactions() {
    let mut l = setup(1);
    let c = AccountId(100);
    assert_eq!(
        l.post(req("x", TxnKind::Adjustment, vec![(c, usd(5))])),
        Err(LedgerError::TooFewPostings)
    );
    assert_eq!(
        l.post(req(
            "x",
            TxnKind::Adjustment,
            vec![(c, usd(5)), (BOOK, usd(-4))]
        )),
        Err(LedgerError::Unbalanced(USD, 1))
    );
    assert_eq!(
        l.post(req(
            "x",
            TxnKind::Adjustment,
            vec![(c, usd(0)), (BOOK, usd(0))]
        )),
        Err(LedgerError::ZeroPosting)
    );
    assert_eq!(
        l.post(req(
            "x",
            TxnKind::Adjustment,
            vec![(c, usd(5)), (BOOK, Money::new(-5, EUR))]
        )),
        Err(LedgerError::Unbalanced(EUR, -5))
    );
    assert_eq!(
        l.post(req(
            "x",
            TxnKind::Adjustment,
            vec![(AccountId(999), usd(5)), (BOOK, usd(-5))]
        )),
        Err(LedgerError::UnknownAccount(AccountId(999)))
    );
    assert_eq!(
        l.post(req(
            "w",
            TxnKind::Withdrawal,
            vec![(c, usd(-5)), (BANK, usd(5))]
        )),
        Err(LedgerError::InsufficientFunds(c))
    );
    // P&L losses may take a client negative (NBP handles it later).
    l.post(req(
        "p",
        TxnKind::RealizedPnl,
        vec![(c, usd(-5)), (BOOK, usd(5))],
    ))
    .unwrap();
    assert!(l.balance(c, USD).is_negative());
    assert!(l
        .open_account(AccountId(5), AccountKind::Client, Some(OMNI), "bad")
        .is_err());
    assert!(l
        .open_account(c, AccountKind::Client, Some(BOOK), "dup")
        .is_err());
    assert_eq!(l.seq(), 6);
}

#[test]
fn idempotency() {
    let mut l = setup(1);
    let r = req(
        "dep-1",
        TxnKind::Deposit,
        vec![(BANK, usd(-10)), (AccountId(100), usd(10))],
    );
    let a = l.post(r.clone()).unwrap();
    let b = l.post(r).unwrap();
    assert_eq!(a, PostOutcome::Posted(1));
    assert_eq!(b, PostOutcome::Duplicate(1));
    assert_eq!(l.balance(AccountId(100), USD), usd(10));
    let conflict = req(
        "dep-1",
        TxnKind::Deposit,
        vec![(BANK, usd(-11)), (AccountId(100), usd(11))],
    );
    assert!(matches!(
        l.post(conflict),
        Err(LedgerError::IdempotencyConflict(_))
    ));
}

#[test]
fn snapshot_and_replay() {
    let mut l = setup(1);
    l.post(req(
        "a",
        TxnKind::Deposit,
        vec![(BANK, usd(-10)), (AccountId(100), usd(10))],
    ))
    .unwrap();
    let snap = l.snapshot();
    l.post(req(
        "b",
        TxnKind::Swap,
        vec![(LP, usd(-3)), (AccountId(100), usd(3))],
    ))
    .unwrap();
    let tail: Vec<_> = l.events_since(snap.seq).to_vec();
    let restored = Ledger::restore(&snap, &tail).unwrap();
    assert_eq!(restored.state_digest(), l.state_digest());
    assert_eq!(restored.seq(), l.seq());
    let full = Ledger::replay(l.events_since(0)).unwrap();
    assert_eq!(full.state_digest(), l.state_digest());
    // duplicates still detected after restore
    let mut r2 = restored;
    assert_eq!(
        r2.post(req(
            "b",
            TxnKind::Swap,
            vec![(LP, usd(-3)), (AccountId(100), usd(3))]
        ))
        .unwrap(),
        PostOutcome::Duplicate(2)
    );
    let json = serde_json::to_string(&snap).unwrap();
    assert_eq!(serde_json::from_str::<LedgerSnapshot>(&json).unwrap(), snap);
}

fn arb_op() -> impl Strategy<Value = (u8, u8, u8, i64, bool)> {
    (0u8..8, 0u64..6, 0u64..6, 1i64..1_000_000, any::<bool>())
        .prop_map(|(k, a, b, amt, eur)| (k, a as u8, b as u8, amt, eur))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn invariants_hold_under_random_ops(ops in prop::collection::vec(arb_op(), 1..80), snap_at in 0usize..80) {
        let mut l = setup(4);
        let all = [BANK, LP, BOOK, AccountId(100), AccountId(101), AccountId(102)];
        let kinds = [TxnKind::Deposit, TxnKind::Withdrawal, TxnKind::Adjustment, TxnKind::RealizedPnl,
            TxnKind::Commission, TxnKind::Swap, TxnKind::NegativeBalanceProtection, TxnKind::Transfer];
        let mut snap = None;
        for (i, (k, a, b, amt, eur)) in ops.iter().enumerate() {
            if i == snap_at { snap = Some(l.snapshot()); }
            let ccy = if *eur { EUR } else { USD };
            let (a, b) = (all[*a as usize % 6], all[*b as usize % 6]);
            let r = req(&format!("k{}", i % 50), kinds[*k as usize],
                vec![(a, Money::new(-(*amt as i128), ccy)), (b, Money::new(*amt as i128, ccy))]);
            let before = l.state_digest();
            match l.post(r) {
                Ok(PostOutcome::Duplicate(_)) | Err(_) => prop_assert_eq!(before, l.state_digest()),
                Ok(PostOutcome::Posted(_)) => {}
            }
            prop_assert!(l.check_invariants().is_ok());
            for c in [USD, EUR] {
                // master = sum of subtree; internal tree mirrors the externals
                let ext = l.balance(BANK, c).minor + l.balance(LP, c).minor;
                prop_assert_eq!(l.subtree_balance(OMNI, c).minor, -ext);
            }
        }
        let full = Ledger::replay(l.events_since(0)).unwrap();
        prop_assert_eq!(full.state_digest(), l.state_digest());
        if let Some(s) = snap {
            let r = Ledger::restore(&s, l.events_since(s.seq)).unwrap();
            prop_assert_eq!(r.state_digest(), l.state_digest());
        }
    }
}
