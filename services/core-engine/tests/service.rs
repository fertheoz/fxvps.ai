use axum::body::Body;
use axum::http::{Request, StatusCode};
use core_engine::{recover, router, spawn, Settings};
use http_body_util::BodyExt;
use money::{px, qty, Currency, Money};
use oms::{Command, Event, NewOrder};
use risk::{GroupConfig, Routing, Side, SymbolSpec};
use serde_json::Value;
use tower::ServiceExt;

fn setup_cmds() -> Vec<Command> {
    vec![
        Command::AddSymbol(SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5)),
        Command::SetGroup(GroupConfig::retail("a", Currency::USD, Routing::ABook)),
        Command::SetGroup(GroupConfig::retail("b", Currency::USD, Routing::BBook)),
        Command::Quote {
            symbol: "EURUSD".into(),
            bid: px("1.1"),
            ask: px("1.1001"),
        },
        Command::OpenAccount {
            account: 7,
            group: "a".into(),
        },
        Command::Deposit {
            account: 7,
            amount: Money::parse("10000", Currency::USD).unwrap(),
            key: "d".into(),
        },
        Command::OpenAccount {
            account: 8,
            group: "b".into(),
        },
        Command::Deposit {
            account: 8,
            amount: Money::parse("10000", Currency::USD).unwrap(),
            key: "d".into(),
        },
    ]
}

#[test]
fn journal_recovery_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Settings::new(dir.path());
    s.snapshot_every = 5; // exercise snapshot + journal tail
    let (h, join) = spawn(s.clone()).unwrap();
    for c in setup_cmds() {
        h.command_blocking(c).unwrap();
    }
    for i in 0..6 {
        let side = if i % 2 == 0 { Side::Buy } else { Side::Sell };
        let ev = h
            .command_blocking(Command::PlaceOrder(NewOrder::market(
                7,
                &format!("o{i}"),
                "EURUSD",
                side,
                qty("0.5"),
            )))
            .unwrap();
        // simulated LP fills the A-book order within the same request
        assert!(
            ev.iter().any(|e| matches!(e, Event::OrderFilled { .. })),
            "{ev:?}"
        );
        h.command_blocking(Command::PlaceOrder(NewOrder::market(
            8,
            &format!("o{i}"),
            "EURUSD",
            side,
            qty("0.2"),
        )))
        .unwrap();
    }
    let digest = h
        .query_blocking(|e| Value::String(e.state_digest()))
        .unwrap();
    h.shutdown();
    join.join().unwrap();
    assert!(dir.path().join("snapshot.json").exists());
    let (e, _) = recover(&s).unwrap();
    assert_eq!(Value::String(e.state_digest()), digest);
    e.check_invariants().unwrap();
    // pure journal replay (no snapshot) gives the same state
    std::fs::remove_file(dir.path().join("snapshot.json")).unwrap();
    let (e2, _) = recover(&s).unwrap();
    assert_eq!(Value::String(e2.state_digest()), digest);
    // restart continues sequence numbers
    let (h, join) = spawn(s).unwrap();
    let ev = h.command_blocking(Command::Tick).unwrap();
    assert!(ev.is_empty());
    h.shutdown();
    join.join().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_http() {
    let dir = tempfile::tempdir().unwrap();
    let (h, _join) = spawn(Settings::new(dir.path())).unwrap();
    for c in setup_cmds() {
        h.command(c).await.unwrap();
    }
    let app = router(h.clone());
    let order = serde_json::to_string(&Command::PlaceOrder(NewOrder::market(
        8,
        "web1",
        "EURUSD",
        Side::Buy,
        qty("1"),
    )))
    .unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::post("/commands")
                .header("content-type", "application/json")
                .body(Body::from(order))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .clone()
        .oneshot(Request::get("/accounts/8").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v["positions"], 1);
    assert_eq!(v["risk"]["balance"]["minor"], 1_000_000);
    let res = app
        .clone()
        .oneshot(
            Request::get("/accounts/8/positions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let v: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    let res = app
        .clone()
        .oneshot(Request::get("/accounts/99").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = app
        .oneshot(Request::get("/accounts").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let v: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    h.shutdown();
}

#[test]
fn verify_and_compact_keep_the_digest() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Settings::new(dir.path());
    s.snapshot_every = 1_000;
    let (h, join) = spawn(s.clone()).unwrap();
    for c in setup_cmds() {
        h.command_blocking(c).unwrap();
    }
    h.shutdown();
    join.join().unwrap();
    let before = core_engine::verify(dir.path()).unwrap();
    assert!(before.ok, "{:?}", before.error);
    assert!(before.journal_lines >= 8);
    let c = core_engine::compact(dir.path()).unwrap();
    assert!(c.archived.is_some());
    assert_eq!(
        std::fs::metadata(dir.path().join("journal.jsonl"))
            .unwrap()
            .len(),
        0
    );
    let after = core_engine::verify(dir.path()).unwrap();
    assert!(after.ok);
    assert_eq!(after.digest, before.digest);
    assert_eq!(after.seq, before.seq);
    assert_eq!(after.journal_lines, 0);
    // the engine keeps working on the compacted directory
    let (h, join) = spawn(s).unwrap();
    h.command_blocking(Command::Quote {
        symbol: "EURUSD".into(),
        bid: px("1.2"),
        ask: px("1.2001"),
    })
    .unwrap();
    h.shutdown();
    join.join().unwrap();
    let again = core_engine::verify(dir.path()).unwrap();
    assert!(again.ok);
    assert!(again.seq > before.seq);
    assert_ne!(again.digest, before.digest);
}

#[test]
fn writer_lock_is_exclusive_and_warm_start_matches() {
    let dir = tempfile::tempdir().unwrap();
    let lock = core_engine::try_writer_lock(dir.path()).unwrap();
    assert!(lock.is_some());
    assert!(
        core_engine::try_writer_lock(dir.path()).unwrap().is_none(),
        "second holder refused"
    );
    let mut s = Settings::new(dir.path());
    s.snapshot_every = 3; // snapshot + tail exercises the fast skip
    let (h, join) = spawn(s.clone()).unwrap();
    for c in setup_cmds() {
        h.command_blocking(c).unwrap();
    }
    h.shutdown();
    join.join().unwrap();
    let before = core_engine::verify(dir.path()).unwrap();
    // standby: replica catches up, then hands its engine to the writer
    let rep = core_engine::replica::Replica::open(dir.path()).unwrap();
    let (engine, seq) = rep.into_parts();
    assert_eq!(seq, before.seq);
    let (h, join) =
        core_engine::spawn_with_state(s, core_engine::LpMode::Simulated, None, Some((engine, seq)))
            .unwrap();
    h.shutdown();
    join.join().unwrap();
    assert_eq!(
        core_engine::verify(dir.path()).unwrap().digest,
        before.digest
    );
    drop(lock);
    assert!(
        core_engine::try_writer_lock(dir.path()).unwrap().is_some(),
        "released on drop"
    );
}
