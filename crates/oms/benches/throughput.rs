//! Order throughput of the single-writer engine (B-book market orders,
//! open + close, ledger postings included). Reports, does not assert.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use money::{px, qty, Currency, Money};
use oms::{Command, Engine, EngineConfig, Envelope, NewOrder, NullRouter};
use risk::{GroupConfig, Routing, Side, SymbolSpec};
use std::hint::black_box;

fn setup(accounts: u64) -> (Engine, u64) {
    let mut e = Engine::new(EngineConfig::default(), Box::new(NullRouter));
    let mut seq = 0;
    let mut run = |e: &mut Engine, cmd| {
        seq += 1;
        e.apply(&Envelope { seq, ts: seq, cmd });
    };
    run(
        &mut e,
        Command::AddSymbol(SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5)),
    );
    run(
        &mut e,
        Command::SetGroup(GroupConfig::retail("b", Currency::USD, Routing::BBook)),
    );
    run(
        &mut e,
        Command::Quote {
            symbol: "EURUSD".into(),
            bid: px("1.1"),
            ask: px("1.1001"),
        },
    );
    for a in 0..accounts {
        run(
            &mut e,
            Command::OpenAccount {
                account: a,
                group: "b".into(),
            },
        );
        run(
            &mut e,
            Command::Deposit {
                account: a,
                amount: Money::parse("100000", Currency::USD).unwrap(),
                key: "d".into(),
            },
        );
    }
    (e, seq)
}

fn bench(c: &mut Criterion) {
    let accounts = 100;
    let (mut e, mut seq) = setup(accounts);
    let mut n: u64 = 0;
    let mut g = c.benchmark_group("engine");
    // one iteration = 2 orders (open + close)
    g.throughput(Throughput::Elements(2));
    g.bench_function("market_open_close_bbook", |b| {
        b.iter(|| {
            n += 1;
            let acc = n % accounts;
            seq += 1;
            let ev = e.apply(&Envelope {
                seq,
                ts: seq,
                cmd: Command::PlaceOrder(NewOrder::market(
                    acc,
                    &format!("o{n}"),
                    "EURUSD",
                    Side::Buy,
                    qty("0.1"),
                )),
            });
            let pid = ev.iter().find_map(|x| match x {
                oms::Event::PositionOpened { position_id } => Some(*position_id),
                _ => None,
            });
            if let Some(pid) = pid {
                seq += 1;
                black_box(e.apply(&Envelope {
                    seq,
                    ts: seq,
                    cmd: Command::ClosePosition {
                        account: acc,
                        position_id: pid,
                        volume: None,
                        client_order_id: format!("c{n}"),
                    },
                }));
            }
        })
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
