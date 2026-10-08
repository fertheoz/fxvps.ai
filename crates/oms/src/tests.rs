use super::*;
use money::{px, qty, Currency, Money, Price, Qty};
use proptest::prelude::*;
use risk::{GroupConfig, MarginMode, Routing, Side, SymbolSpec};

const USD: Currency = Currency::USD;

fn usd(s: &str) -> Money {
    Money::parse(s, USD).unwrap()
}

struct H {
    e: Engine,
    seq: u64,
    ts: u64,
    router: RecordingRouter,
    journal: Vec<Envelope>,
    config: EngineConfig,
}

impl H {
    fn new(config: EngineConfig) -> H {
        let router = RecordingRouter::default();
        let mut h = H {
            e: Engine::new(config.clone(), Box::new(router.clone())),
            seq: 0,
            ts: 1_000,
            router,
            journal: Vec::new(),
            config,
        };
        let mut eu = SymbolSpec::fx("EURUSD", Currency::EUR, USD, 5);
        eu.swap_long = px("-7");
        eu.swap_short = px("2");
        h.cmd(Command::AddSymbol(eu));
        h.cmd(Command::AddSymbol(SymbolSpec::fx(
            "USDJPY",
            USD,
            Currency::JPY,
            3,
        )));
        for (name, routing, mode) in [
            ("b", Routing::BBook, MarginMode::Hedging),
            ("a", Routing::ABook, MarginMode::Hedging),
            ("n", Routing::BBook, MarginMode::Netting),
        ] {
            let mut g = GroupConfig::retail(name, USD, routing);
            g.esma = None;
            g.leverage = 100;
            g.margin_mode = mode;
            h.cmd(Command::SetGroup(g));
        }
        h.quote("EURUSD", "1.10000", "1.10010");
        h.quote("USDJPY", "150.000", "150.010");
        h
    }
    fn cmd(&mut self, cmd: Command) -> Vec<Event> {
        self.seq += 1;
        self.ts += 1_000;
        let env = Envelope {
            seq: self.seq,
            ts: self.ts,
            cmd,
        };
        self.journal.push(env.clone());
        let ev = self.e.apply(&env);
        self.e.check_invariants().unwrap();
        ev
    }
    fn quote(&mut self, s: &str, b: &str, a: &str) -> Vec<Event> {
        self.cmd(Command::Quote {
            symbol: s.into(),
            bid: px(b),
            ask: px(a),
        })
    }
    fn account(&mut self, id: u64, group: &str, dep: &str) {
        self.cmd(Command::OpenAccount {
            account: id,
            group: group.into(),
        });
        self.cmd(Command::Deposit {
            account: id,
            amount: usd(dep),
            key: format!("d{id}"),
        });
    }
    fn order(&mut self, o: NewOrder) -> (Vec<Event>, OrderId) {
        let (acc, clid) = (o.account, o.client_order_id.clone());
        let ev = self.cmd(Command::PlaceOrder(o));
        let id = self
            .e
            .order_by_client_id(acc, &clid)
            .map(|o| o.id)
            .unwrap_or(0);
        (ev, id)
    }
    fn market(&mut self, acc: u64, clid: &str, side: Side, v: &str) -> OrderId {
        self.order(NewOrder::market(acc, clid, "EURUSD", side, qty(v)))
            .1
    }
    fn pending(
        &mut self,
        acc: u64,
        clid: &str,
        side: Side,
        t: OrderType,
        lim: Option<&str>,
        stop: Option<&str>,
    ) -> NewOrder {
        let mut o = NewOrder::market(acc, clid, "EURUSD", side, qty("1"));
        o.order_type = t;
        o.limit_price = lim.map(px);
        o.stop_price = stop.map(px);
        o
    }
    fn pos(&self, acc: u64) -> Vec<Position> {
        self.e.positions_of(acc).into_iter().cloned().collect()
    }
    fn bal(&self, acc: u64) -> Money {
        self.e.balance(acc).unwrap()
    }
}

fn b() -> H {
    let mut h = H::new(EngineConfig::default());
    h.account(1, "b", "10000");
    h
}

#[test]
fn status_transitions() {
    use OrderStatus::*;
    assert!(New.can_transition(Accepted));
    assert!(Accepted.can_transition(PartiallyFilled));
    assert!(PartiallyFilled.can_transition(Filled));
    assert!(!Filled.can_transition(Cancelled));
    assert!(!Rejected.can_transition(Accepted));
    assert!(!PartiallyFilled.can_transition(Expired));
    assert!(!New.can_transition(Filled));
    for s in [Filled, Cancelled, Rejected, Expired] {
        assert!(s.is_terminal());
    }
}

#[test]
fn allocation_modes() {
    let c = [(1u64, 100), (2, 200), (3, 100)];
    assert_eq!(
        allocate(200, &c, AllocationMode::ProRata),
        vec![(1, 50), (2, 100), (3, 50)]
    );
    assert_eq!(
        allocate(3, &c, AllocationMode::ProRata),
        vec![(1, 1), (2, 1), (3, 1)]
    );
    assert_eq!(
        allocate(250, &c, AllocationMode::Fifo),
        vec![(1, 100), (2, 150)]
    );
    assert_eq!(
        allocate(1000, &c, AllocationMode::Fifo)
            .iter()
            .map(|a| a.1)
            .sum::<i64>(),
        400
    );
    assert!(allocate(0, &c, AllocationMode::ProRata).is_empty());
}

proptest! {
    #[test]
    fn allocation_conserves(fill in 0i64..10_000_000, rems in prop::collection::vec(0i64..1_000_000, 1..10), fifo in any::<bool>()) {
        let c: Vec<(usize, i64)> = rems.iter().copied().enumerate().collect();
        let mode = if fifo { AllocationMode::Fifo } else { AllocationMode::ProRata };
        let a = allocate(fill, &c, mode);
        let total: i64 = rems.iter().sum();
        prop_assert_eq!(a.iter().map(|x| x.1).sum::<i64>(), fill.min(total));
        for (id, q) in a {
            prop_assert!(q > 0 && q <= rems[id]);
        }
    }
}

#[test]
fn market_open_close_bbook() {
    let mut h = b();
    let id = h.market(1, "c1", Side::Buy, "1");
    let o = h.e.order(id).unwrap();
    assert_eq!(o.status, OrderStatus::Filled);
    assert_eq!(o.avg_price, px("1.1001"));
    let p = h.pos(1)[0].clone();
    assert_eq!(p.volume, qty("1"));
    h.quote("EURUSD", "1.10110", "1.10120");
    let r = h.e.account_risk(1).unwrap();
    assert_eq!(r.floating, usd("100"));
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: p.id,
        volume: None,
        client_order_id: "x1".into(),
    });
    assert!(h.pos(1).is_empty());
    assert_eq!(h.bal(1), usd("10100"));
    assert_eq!(h.e.ledger().balance(BROKER_BOOK, USD), usd("-100"));
}

#[test]
fn idempotent_client_order_id() {
    let mut h = b();
    let a = h.market(1, "same", Side::Buy, "1");
    let (ev, b2) = h.order(NewOrder::market(1, "same", "EURUSD", Side::Buy, qty("1")));
    assert_eq!(a, b2);
    assert_eq!(ev, vec![Event::DuplicateOrder { order_id: a }]);
    assert_eq!(h.pos(1).len(), 1);
    // same clid on another account is independent
    h.account(2, "b", "10000");
    h.market(2, "same", Side::Buy, "1");
    assert_eq!(h.pos(2).len(), 1);
}

#[test]
fn pre_trade_rejections_and_withdraw() {
    let mut h = b();
    let id = h.market(1, "big", Side::Buy, "10");
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Rejected);
    let mut o = NewOrder::market(1, "slip", "EURUSD", Side::Buy, qty("1"));
    o.requested_price = Some(px("1.2"));
    let (_, id) = h.order(o);
    assert!(h
        .e
        .order(id)
        .unwrap()
        .reject_reason
        .as_deref()
        .unwrap()
        .contains("tolerance"));
    let mut o = NewOrder::market(1, "badsl", "EURUSD", Side::Buy, qty("1"));
    o.sl = Some(px("1.2"));
    let (_, id) = h.order(o);
    assert_eq!(
        h.e.order(id).unwrap().reject_reason.as_deref(),
        Some("invalid SL/TP")
    );
    h.market(1, "ok", Side::Buy, "5"); // margin 5*110010/100 = 5500.5
    let ev = h.cmd(Command::Withdraw {
        account: 1,
        amount: usd("5000"),
        key: "w".into(),
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
    h.cmd(Command::Withdraw {
        account: 1,
        amount: usd("1000"),
        key: "w2".into(),
    });
    assert_eq!(h.bal(1), usd("9000"));
}

#[test]
fn partial_close_and_commission() {
    let mut h = H::new(EngineConfig::default());
    let mut eu = SymbolSpec::fx("EURUSD", Currency::EUR, USD, 5);
    eu.commission_per_lot = usd("3.5");
    h.cmd(Command::AddSymbol(eu));
    h.account(1, "b", "10000");
    h.market(1, "o", Side::Sell, "2");
    assert_eq!(h.bal(1), usd("9993"));
    let pid = h.pos(1)[0].id;
    h.quote("EURUSD", "1.09900", "1.09910");
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: pid,
        volume: Some(qty("0.5")),
        client_order_id: "pc".into(),
    });
    let p = h.e.position(pid).unwrap();
    assert_eq!(p.volume, qty("1.5"));
    // (1.10000-1.09910)*0.5*100000 = 45 profit, minus 1.75 commission
    assert_eq!(h.bal(1), usd("10036.25"));
    let ev = h.cmd(Command::ClosePosition {
        account: 1,
        position_id: pid,
        volume: Some(qty("2")),
        client_order_id: "pc2".into(),
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
    assert_eq!(
        h.e.ledger().balance(BROKER_BOOK, USD),
        usd("-45").checked_add(usd("8.75")).unwrap()
    );
}

#[test]
fn netting_reduce_and_flip() {
    let mut h = H::new(EngineConfig::default());
    h.account(1, "n", "10000");
    h.market(1, "a", Side::Buy, "1");
    h.market(1, "b", Side::Buy, "1");
    assert_eq!(h.pos(1).len(), 1);
    assert_eq!(h.pos(1)[0].volume, qty("2"));
    h.market(1, "c", Side::Sell, "0.5");
    assert_eq!(h.pos(1)[0].volume, qty("1.5"));
    h.market(1, "d", Side::Sell, "2");
    let p = &h.pos(1)[0];
    assert_eq!((p.side, p.volume), (Side::Sell, qty("0.5")));
    assert_eq!(p.open_price, px("1.1"));
    // realized: 2 lots bought at 1.1001, sold at 1.1 => -20
    assert_eq!(h.bal(1), usd("9980"));
}

#[test]
fn hedging_margin_and_positions() {
    let mut h = b();
    h.market(1, "a", Side::Buy, "1");
    let m1 = h.e.account_risk(1).unwrap().margin;
    h.market(1, "b", Side::Sell, "1");
    assert_eq!(h.pos(1).len(), 2);
    let m2 = h.e.account_risk(1).unwrap().margin;
    assert_eq!(m1, m2, "fully hedged at 50% per leg");
}

#[test]
fn limit_stop_stoplimit_expiry_cancel() {
    let mut h = b();
    let o = h.pending(1, "lim", Side::Buy, OrderType::Limit, Some("1.09950"), None);
    let (_, lim) = h.order(o);
    let o = h.pending(1, "stp", Side::Sell, OrderType::Stop, None, Some("1.09800"));
    let (_, stp) = h.order(o);
    let o = h.pending(
        1,
        "sl",
        Side::Buy,
        OrderType::StopLimit,
        Some("1.10300"),
        Some("1.10200"),
    );
    let (_, sl) = h.order(o);
    let mut o = h.pending(1, "exp", Side::Buy, OrderType::Limit, Some("1.0"), None);
    o.expire_at = Some(h.ts + 5_000);
    let (_, exp) = h.order(o);
    let o = h.pending(1, "can", Side::Buy, OrderType::Limit, Some("1.0"), None);
    let (_, can) = h.order(o);
    for id in [lim, stp, sl, exp, can] {
        assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Accepted);
    }
    h.cmd(Command::CancelOrder {
        account: 1,
        order_id: can,
    });
    assert_eq!(h.e.order(can).unwrap().status, OrderStatus::Cancelled);
    h.quote("EURUSD", "1.09930", "1.09940");
    assert_eq!(h.e.order(lim).unwrap().status, OrderStatus::Filled);
    assert_eq!(h.e.order(lim).unwrap().avg_price, px("1.0994"));
    h.quote("EURUSD", "1.10250", "1.10260"); // stop triggers, limit 1.103 >= ask -> fills
    assert_eq!(h.e.order(sl).unwrap().status, OrderStatus::Filled);
    h.cmd(Command::Tick);
    h.cmd(Command::Tick);
    assert_eq!(h.e.order(exp).unwrap().status, OrderStatus::Expired);
    h.quote("EURUSD", "1.09790", "1.09800");
    assert_eq!(h.e.order(stp).unwrap().status, OrderStatus::Filled);
    let ev = h.cmd(Command::CancelOrder {
        account: 1,
        order_id: stp,
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
}

#[test]
fn stop_limit_triggers_without_fill() {
    let mut h = b();
    let o = h.pending(
        1,
        "sl",
        Side::Buy,
        OrderType::StopLimit,
        Some("1.10100"),
        Some("1.10200"),
    );
    let (_, id) = h.order(o);
    let ev = h.quote("EURUSD", "1.10300", "1.10310");
    assert!(ev.contains(&Event::OrderTriggered { order_id: id }));
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Accepted);
    h.quote("EURUSD", "1.10080", "1.10090");
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Filled);
}

#[test]
fn oco_group() {
    let mut h = b();
    let mut o1 = h.pending(1, "tp", Side::Sell, OrderType::Limit, Some("1.10500"), None);
    o1.oco_group = Some(7);
    let mut o2 = h.pending(1, "sl", Side::Sell, OrderType::Stop, None, Some("1.09500"));
    o2.oco_group = Some(7);
    let (_, a) = h.order(o1);
    let (_, b2) = h.order(o2);
    h.quote("EURUSD", "1.10500", "1.10510");
    assert_eq!(h.e.order(a).unwrap().status, OrderStatus::Filled);
    assert_eq!(h.e.order(b2).unwrap().status, OrderStatus::Cancelled);
}

#[test]
fn sl_tp_and_trailing_stop() {
    let mut h = b();
    let mut o = NewOrder::market(1, "t", "EURUSD", Side::Buy, qty("1"));
    o.tp = Some(px("1.10500"));
    o.sl = Some(px("1.09000"));
    h.order(o);
    let mut o = NewOrder::market(1, "tr", "EURUSD", Side::Buy, qty("1"));
    o.trailing_points = Some(100); // 0.00100
    let (_, tr) = h.order(o);
    let trp = h.e.order(tr).unwrap().position.unwrap();
    h.quote("EURUSD", "1.10300", "1.10310");
    assert_eq!(h.e.position(trp).unwrap().sl, Some(px("1.102")));
    h.quote("EURUSD", "1.10250", "1.10260"); // trailing never moves back
    assert_eq!(h.e.position(trp).unwrap().sl, Some(px("1.102")));
    h.quote("EURUSD", "1.10400", "1.10410");
    assert_eq!(h.e.position(trp).unwrap().sl, Some(px("1.103")));
    h.quote("EURUSD", "1.10290", "1.10300"); // trailing SL hit
    assert!(h.e.position(trp).is_none());
    assert_eq!(h.pos(1).len(), 1);
    h.quote("EURUSD", "1.10500", "1.10510"); // TP hit
    assert!(h.pos(1).is_empty());
    // trailing: +190 (1.1029-1.1001)... tp: +490
    assert_eq!(h.bal(1), usd("10770"));
}

#[test]
fn modify_position_sl_closes() {
    let mut h = b();
    h.market(1, "a", Side::Sell, "1");
    let pid = h.pos(1)[0].id;
    // SL already through the market (ask 1.10010) is rejected
    let ev = h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: Some(px("1.10005")),
        tp: None,
        trailing_points: None,
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
    let ev = h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: None,
        tp: Some(px("1.10020")),
        trailing_points: None,
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
    h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: Some(px("1.10050")),
        tp: Some(px("1.09000")),
        trailing_points: None,
    });
    assert_eq!(h.pos(1)[0].sl, Some(px("1.10050")));
    h.quote("EURUSD", "1.10045", "1.10055");
    assert!(h.pos(1).is_empty());
    let d = h.e.deals().last().unwrap();
    assert_eq!((d.entry, d.reason), (DealEntry::Out, OrderOrigin::StopLoss));
}

#[test]
fn stop_out_largest_loss_first_and_nbp() {
    let mut h = H::new(EngineConfig::default());
    h.account(1, "b", "1000");
    h.market(1, "eu", Side::Buy, "0.5"); // margin ~550
    h.order(NewOrder::market(1, "uj", "USDJPY", Side::Sell, qty("0.1"))); // margin 100
    let eu = h.e.order_by_client_id(1, "eu").unwrap().position.unwrap();
    let uj = h.e.order_by_client_id(1, "uj").unwrap().position.unwrap();
    let ev = h.quote("EURUSD", "1.09000", "1.09010");
    // equity ~ 1000 - 500.5 = 499.5 < 650 margin but > 50% -> margin call only
    assert!(ev.contains(&Event::MarginCall { account: 1 }));
    assert!(!ev.iter().any(|e| matches!(e, Event::StopOut { .. })));
    let ev = h.quote("EURUSD", "1.08500", "1.08510");
    let so: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            Event::StopOut { position_id, .. } => Some(*position_id),
            _ => None,
        })
        .collect();
    assert_eq!(so[0], eu, "largest loss first");
    assert!(h.e.position(eu).is_none());
    assert!(
        h.e.position(uj).is_some(),
        "remaining position kept once above level"
    );
    // gap: price jumps hugely, account goes negative -> NBP restores zero
    let ev = h.quote("USDJPY", "250.000", "250.010");
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::NegativeBalanceCompensated { .. })));
    assert!(h.pos(1).is_empty());
    assert_eq!(h.bal(1), usd("0"));
    // the compensation is kept for the client statement
    let m = h.e.cash_moves().last().expect("nbp cash move");
    assert_eq!(
        (m.account, m.kind),
        (1, CashMoveKind::NegativeBalanceCompensation)
    );
    assert!(m.amount.minor > 0);
}

#[test]
fn abook_partial_fills_markup_and_omnibus() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 5;
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "10000");
    let id = h.market(1, "x", Side::Buy, "1");
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].volume, qty("1"));
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Accepted);
    let lp = sent[0].lp_order_id;
    h.cmd(Command::LpFill {
        lp_order_id: lp,
        exec_id: "e1".into(),
        volume: qty("0.4"),
        price: px("1.10010"),
    });
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::PartiallyFilled);
    assert_eq!(h.e.omnibus_net("EURUSD"), qty("0.4").raw());
    // duplicate exec id is ignored
    h.cmd(Command::LpFill {
        lp_order_id: lp,
        exec_id: "e1".into(),
        volume: qty("0.4"),
        price: px("1.10010"),
    });
    assert_eq!(h.e.order(id).unwrap().filled, qty("0.4"));
    h.cmd(Command::LpFill {
        lp_order_id: lp,
        exec_id: "e2".into(),
        volume: qty("0.6"),
        price: px("1.10020"),
    });
    let o = h.e.order(id).unwrap();
    assert_eq!(o.status, OrderStatus::Filled);
    assert_eq!(o.avg_price, px("1.10021")); // vwap 1.100160 + 5 points markup
    let p = h.pos(1)[0].clone();
    assert_eq!(p.volume, qty("1"));
    assert_eq!(p.lp_open_price, px("1.10016"));
    // close: routed to LP again
    h.quote("EURUSD", "1.10500", "1.10510");
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: p.id,
        volume: None,
        client_order_id: "c".into(),
    });
    let sent = h.router.take();
    assert_eq!(sent[0].side, Side::Sell);
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "e3".into(),
        volume: qty("1"),
        price: px("1.10500"),
    });
    assert!(h.pos(1).is_empty());
    assert_eq!(h.e.omnibus_net("EURUSD"), 0);
    // client: (1.10495 - 1.10021) * 100000 = 474 ; LP: (1.105-1.10016) = 484 ; broker +10
    assert_eq!(h.bal(1), usd("10474"));
    assert_eq!(h.e.ledger().balance(BROKER_BOOK, USD), usd("10"));
    assert_eq!(h.e.ledger().balance(LP_COUNTERPARTY, USD), usd("-484"));
}

#[test]
fn revenue_levers_markup_sides_symbol_override_improvement_and_cap() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 5;
    g.markup_ask_points = Some(8);
    g.symbol_markup_points.insert("GBPUSD".into(), 20);
    g.pass_price_improvement = false;
    g.max_slippage_points = Some(10);
    h.cmd(Command::SetGroup(g.clone()));
    h.account(1, "a", "10000");
    // EURUSD 1.10000/1.10010 -> client bid 1.09995 (5), ask 1.10018 (8)
    let q = h.e.group_quote("a", "EURUSD").unwrap();
    assert_eq!((q.bid, q.ask), (px("1.09995"), px("1.10018")));
    // buy at the requested client ask with a 10-point cap: IOC limit at 1.10028 - 8 = 1.10020
    let mut o = h.pending(1, "m", Side::Buy, OrderType::Market, None, None);
    o.requested_price = Some(px("1.10018"));
    let (_, id) = h.order(o);
    let sent = h.router.take();
    assert_eq!(sent[0].limit, Some(px("1.10020")));
    // LP fills better (1.10000): client would get 1.10008, but improvement is kept -> 1.10018
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "x".into(),
        volume: qty("1"),
        price: px("1.10000"),
    });
    assert_eq!(h.e.order(id).unwrap().avg_price, px("1.10018"));
}

#[test]
fn group_commission_per_lot_and_per_million() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::BBook);
    g.esma = None;
    g.leverage = 100;
    g.commission = Some(risk::GroupCommission::PerLot { minor: 700 });
    h.cmd(Command::SetGroup(g.clone()));
    h.account(1, "a", "10000");
    h.market(1, "c1", Side::Buy, "0.5");
    assert_eq!(h.bal(1), usd("9996.50")); // 0.5 lot × $7
    g.commission = Some(risk::GroupCommission::PerMillion { minor: 3000 }); // $30 per million
    h.cmd(Command::SetGroup(g));
    h.market(1, "c2", Side::Buy, "1"); // 100,000 × ~1.1001 = $110,010 notional -> $3.30
    assert_eq!(h.bal(1), usd("9993.20"));
}

#[test]
fn routing_rules_decide_book_and_override_markup() {
    use risk::{OrderKindFilter, RoutingRule};
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::BBook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 5;
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "10000");
    let rule = |id: &str,
                symbols: Vec<&str>,
                routing: Option<Routing>,
                pct: Option<u8>,
                markup: Option<i64>| RoutingRule {
        id: id.into(),
        name: id.into(),
        enabled: true,
        groups: vec![],
        accounts: vec![],
        symbols: symbols.into_iter().map(String::from).collect(),
        min_centilots: None,
        max_centilots: None,
        kind: OrderKindFilter::Any,
        hours_utc: None,
        routing,
        a_book_pct: pct,
        markup_points: markup,
        max_slippage_points: None,
        partial_fill: None,
        min_toxicity: None,
        max_toxicity: None,
    };
    // EURUSD -> A-book with a 20-point markup; everything else stays in the B-book group.
    h.cmd(Command::SetRules(vec![rule(
        "eur-a",
        vec!["EURUSD"],
        Some(Routing::ABook),
        None,
        Some(20),
    )]));
    let id = h.market(1, "r1", Side::Buy, "1");
    let sent = h.router.take();
    assert_eq!(sent.len(), 1, "routed to the LP by the rule");
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "f".into(),
        volume: qty("1"),
        price: px("1.10010"),
    });
    let o = h.e.order(id).unwrap();
    assert_eq!(o.rule.as_deref(), Some("eur-a"));
    assert_eq!(o.routing, Routing::ABook);
    assert_eq!(o.avg_price, px("1.10030")); // LP + 20 points (rule), not 5 (group)
                                            // A disabled rule and a non-matching symbol fall back to the group (B-book, no LP order).
    let mut off = rule("off", vec!["EURUSD"], Some(Routing::ABook), None, None);
    off.enabled = false;
    h.cmd(Command::SetRules(vec![off]));
    h.market(1, "r2", Side::Buy, "1");
    assert!(h.router.take().is_empty());
    // Hybrid split: 100% A sends to the LP, 0% keeps B.
    h.cmd(Command::SetRules(vec![rule(
        "all-a",
        vec![],
        None,
        Some(100),
        None,
    )]));
    h.market(1, "r3", Side::Buy, "1");
    assert_eq!(h.router.take().len(), 1);
    h.cmd(Command::SetRules(vec![rule(
        "all-b",
        vec![],
        None,
        Some(0),
        None,
    )]));
    h.market(1, "r4", Side::Buy, "1");
    assert!(h.router.take().is_empty());
}

#[test]
fn partial_fill_policies() {
    use risk::PartialFill;
    // Retry: the remainder goes to the LP again, up to max_attempts LP orders.
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.partial_fill = PartialFill::Retry { max_attempts: 2 };
    h.cmd(Command::SetGroup(g.clone()));
    h.account(1, "a", "10000");
    let id = h.market(1, "r", Side::Buy, "1");
    let lp = h.router.take()[0].lp_order_id;
    h.cmd(Command::LpFill {
        lp_order_id: lp,
        exec_id: "r1".into(),
        volume: qty("0.3"),
        price: px("1.1001"),
    });
    h.cmd(Command::LpReject {
        lp_order_id: lp,
        reason: "ioc remainder".into(),
    });
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].volume, qty("0.7"));
    assert!(!sent[0].all_or_none);
    assert_eq!(h.e.order(id).unwrap().lp_attempts, 2);
    h.cmd(Command::LpReject {
        lp_order_id: sent[0].lp_order_id,
        reason: "ioc remainder".into(),
    });
    // second attempt was the last: remainder cancelled
    assert!(h.router.take().is_empty());
    let o = h.e.order(id).unwrap();
    assert_eq!((o.status, o.filled), (OrderStatus::Cancelled, qty("0.3")));

    // AllOrNone: the LP order goes out fill-or-kill.
    g.partial_fill = PartialFill::AllOrNone;
    h.cmd(Command::SetGroup(g));
    h.market(1, "k", Side::Buy, "1");
    assert!(h.router.take()[0].all_or_none);
}

#[test]
fn abook_reject_and_partial_cancel() {
    let mut h = H::new(EngineConfig::default());
    h.account(1, "a", "10000");
    let id = h.market(1, "x", Side::Buy, "1");
    let lp = h.router.take()[0].lp_order_id;
    h.cmd(Command::LpReject {
        lp_order_id: lp,
        reason: "no liquidity".into(),
    });
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Rejected);
    let id = h.market(1, "y", Side::Buy, "1");
    let lp = h.router.take()[0].lp_order_id;
    h.cmd(Command::LpFill {
        lp_order_id: lp,
        exec_id: "f".into(),
        volume: qty("0.3"),
        price: px("1.1001"),
    });
    h.cmd(Command::LpReject {
        lp_order_id: lp,
        reason: "ioc remainder".into(),
    });
    let o = h.e.order(id).unwrap();
    assert_eq!((o.status, o.filled), (OrderStatus::Cancelled, qty("0.3")));
    assert_eq!(h.pos(1)[0].volume, qty("0.3"));
}

#[test]
fn abook_limit_goes_to_lp_as_limit_and_waits_when_unfilled() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 5;
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "10000");
    // market 1.10000/1.10010, client ask 1.10015: buy limit 1.09950 waits
    let o = h.pending(1, "lim", Side::Buy, OrderType::Limit, Some("1.09950"), None);
    let (_, id) = h.order(o);
    assert!(h.router.take().is_empty());
    // client ask reaches the limit: an IOC limit at the client limit net of markup
    h.quote("EURUSD", "1.09935", "1.09945");
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].limit, Some(px("1.09945")));
    assert!(h.e.order(id).unwrap().working);
    // LP fills 0.4 and cancels the rest (IOC): the remainder keeps waiting
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "l1".into(),
        volume: qty("0.4"),
        price: px("1.09940"),
    });
    h.cmd(Command::LpReject {
        lp_order_id: sent[0].lp_order_id,
        reason: "ioc remainder".into(),
    });
    let o = h.e.order(id).unwrap();
    assert_eq!(
        (o.status, o.filled),
        (OrderStatus::PartiallyFilled, qty("0.4"))
    );
    assert!(o.is_pending() && !o.working);
    assert_eq!(o.avg_price, px("1.09945")); // LP 1.09940 + 5 points: never worse than the limit
                                            // same price again: no new attempt until the market improves
    h.quote("EURUSD", "1.09935", "1.09945");
    assert!(h.router.take().is_empty());
    h.quote("EURUSD", "1.09930", "1.09940");
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].volume, qty("0.6"));
    assert_eq!(sent[0].limit, Some(px("1.09945")));
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "l2".into(),
        volume: qty("0.6"),
        price: px("1.09935"),
    });
    let o = h.e.order(id).unwrap();
    assert_eq!(o.status, OrderStatus::Filled);
    assert_eq!(h.pos(1)[0].volume, qty("1"));
    // a modified price re-arms at once
    let o2 = h.pending(
        1,
        "lim2",
        Side::Sell,
        OrderType::Limit,
        Some("1.09920"),
        None,
    );
    let (_, id2) = h.order(o2);
    let sent = h.router.take();
    assert_eq!(sent[0].limit, Some(px("1.09925")));
    h.cmd(Command::LpReject {
        lp_order_id: sent[0].lp_order_id,
        reason: "ioc".into(),
    });
    assert!(h.e.order(id2).unwrap().rearm_px.is_some());
    let mut c = change(h.e.order(id2).unwrap());
    c.limit_price = Some(px("1.09910"));
    h.cmd(Command::ModifyOrder {
        account: 1,
        order_id: id2,
        change: c,
    });
    assert_eq!(h.router.take().len(), 1);
}

#[test]
fn account_group_change_needs_a_flat_account() {
    let mut h = b();
    // account 1 is in the hedging B-book group "b"; "n" is netting
    let id = h.market(1, "x", Side::Buy, "1");
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Filled);
    let ev = h.cmd(Command::SetAccountGroup {
        account: 1,
        group: "n".into(),
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
    assert_eq!(h.e.account(1).unwrap().group, "b");
    let pid = h.pos(1)[0].id;
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: pid,
        volume: None,
        client_order_id: "c".into(),
    });
    let ev = h.cmd(Command::SetAccountGroup {
        account: 1,
        group: "n".into(),
    });
    assert_eq!(
        ev,
        vec![Event::AccountGroupChanged {
            account: 1,
            group: "n".into()
        }]
    );
    assert_eq!(h.e.account(1).unwrap().group, "n");
    let ev = h.cmd(Command::SetAccountGroup {
        account: 1,
        group: "nope".into(),
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
}

#[test]
fn abook_aggregated_pro_rata_allocation() {
    let mut h = H::new(EngineConfig {
        allocation: AllocationMode::ProRata,
        aggregate_a_book: true,
    });
    h.account(1, "a", "10000");
    h.account(2, "a", "10000");
    let a = h.market(1, "x", Side::Buy, "1");
    let b2 = h.market(2, "x", Side::Buy, "3");
    assert!(h.router.take().is_empty());
    h.cmd(Command::FlushLp);
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].volume, qty("4"));
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "p1".into(),
        volume: qty("2"),
        price: px("1.1001"),
    });
    assert_eq!(h.e.order(a).unwrap().filled, qty("0.5"));
    assert_eq!(h.e.order(b2).unwrap().filled, qty("1.5"));
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "p2".into(),
        volume: qty("2"),
        price: px("1.1002"),
    });
    assert_eq!(h.e.order(a).unwrap().status, OrderStatus::Filled);
    assert_eq!(h.e.order(b2).unwrap().status, OrderStatus::Filled);
    assert_eq!(h.e.omnibus_net("EURUSD"), qty("4").raw());
    assert!(h.e.lp_order(sent[0].lp_order_id).unwrap().done);
}

#[test]
fn swap_rollover() {
    let mut h = b();
    h.market(1, "l", Side::Buy, "2");
    h.market(1, "s", Side::Sell, "1");
    h.cmd(Command::Rollover);
    // long: -7 * 2 = -14 ; short: +2 * 1 = +2  -> -12, minus spread on nothing
    assert_eq!(h.bal(1), usd("9988"));
    // same UTC day: idempotent
    h.cmd(Command::Rollover);
    assert_eq!(h.bal(1), usd("9988"));
    // swap sits on the positions and travels into the closing deals
    let long = h.pos(1).into_iter().find(|p| p.side == Side::Buy).unwrap();
    assert_eq!(long.swap_minor, -1400);
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: long.id,
        volume: Some(qty("1")),
        client_order_id: "half".into(),
    });
    let d = h.e.deals().last().unwrap();
    assert_eq!(d.swap, -700);
    assert_eq!(
        h.pos(1)
            .into_iter()
            .find(|p| p.side == Side::Buy)
            .unwrap()
            .swap_minor,
        -700
    );
}

#[test]
fn rollover_triple_day_weekend_and_multiplier() {
    use risk::SwapConfig;
    let mut h = b();
    // 1970-01-01 (ts ~1 µs) is a Thursday; make Thursday the triple day of EURUSD
    let mut eu = h.e.symbol_spec("EURUSD").unwrap().clone();
    eu.triple_swap_day = 4;
    h.cmd(Command::AddSymbol(eu));
    let mut g = h.e.group("b").unwrap().clone();
    g.swap_multiplier_pct = 50;
    h.cmd(Command::SetGroup(g));
    h.market(1, "l", Side::Buy, "2");
    h.cmd(Command::Rollover);
    // -7 × 2 lots × 50 % × 3 days = -21
    assert_eq!(h.bal(1), usd("9979"));
    // next day is Friday (1 day), then Saturday: skipped
    h.ts += 86_400_000_000_000;
    h.cmd(Command::Rollover);
    assert_eq!(h.bal(1), usd("9972"));
    h.ts += 86_400_000_000_000;
    let ev = h.cmd(Command::Rollover);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Rollover { applied: false, .. })));
    assert_eq!(h.bal(1), usd("9972"));
    // disabled schedule: nothing, even on a weekday
    h.ts += 2 * 86_400_000_000_000;
    h.cmd(Command::SetSwapConfig(SwapConfig {
        enabled: false,
        ..SwapConfig::default()
    }));
    h.cmd(Command::Rollover);
    assert_eq!(h.bal(1), usd("9972"));
}

fn scenario(h: &mut H) {
    h.account(1, "b", "10000");
    h.account(2, "a", "5000");
    h.account(3, "n", "3000");
    let prices = [
        "1.10000", "1.10120", "1.09870", "1.10300", "1.09500", "1.10050",
    ];
    for (i, p) in prices.iter().enumerate() {
        let bid: Price = px(p);
        let ask = Price::from_raw(bid.raw() + 10 * 1_000);
        h.cmd(Command::Quote {
            symbol: "EURUSD".into(),
            bid,
            ask,
        });
        let side = if i % 2 == 0 { Side::Buy } else { Side::Sell };
        let mut o = NewOrder::market(1, &format!("m{i}"), "EURUSD", side, qty("0.3"));
        o.trailing_points = Some(50);
        h.order(o);
        h.market(3, &format!("n{i}"), side.opposite(), "0.2");
        h.market(2, &format!("a{i}"), side, "0.1");
        for r in h.router.take() {
            h.cmd(Command::LpFill {
                lp_order_id: r.lp_order_id,
                exec_id: format!("x{}", r.lp_order_id),
                volume: r.volume,
                price: bid,
            });
        }
        if i == 3 {
            h.cmd(Command::Rollover);
        }
    }
}

#[test]
fn deterministic_replay_and_snapshot() {
    let mut h = H::new(EngineConfig::default());
    scenario(&mut h);
    let digest = h.e.state_digest();
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), digest);
    // snapshot halfway + tail
    let mid = h.journal.len() / 2;
    let half = Engine::replay(h.config.clone(), &h.journal[..mid]);
    let snap = half.snapshot();
    let json = serde_json::to_string(&snap).unwrap();
    let snap: EngineSnapshot = serde_json::from_str(&json).unwrap();
    let mut restored = Engine::restore(&snap, Box::new(NullRouter)).unwrap();
    for env in &h.journal[mid..] {
        restored.apply(env);
    }
    assert_eq!(restored.state_digest(), digest);
    restored.check_invariants().unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn random_flow_keeps_invariants(ops in prop::collection::vec((0u8..6, 0u64..3, -300i64..300, 1i64..5), 1..60)) {
        let mut h = H::new(EngineConfig::default());
        for a in 1..=3 {
            h.account(a, ["b", "a", "n"][(a - 1) as usize], "2000");
        }
        let mut mid = 110_000_000i64;
        for (i, (op, acc, dp, lots)) in ops.into_iter().enumerate() {
            let acc = acc + 1;
            match op {
                0 | 1 => {
                    let side = if op == 0 { Side::Buy } else { Side::Sell };
                    h.order(NewOrder::market(acc, &format!("o{i}"), "EURUSD", side, Qty::from_raw(lots * 10_000_000)));
                }
                2 => {
                    mid = (mid + dp * 1_000).max(50_000_000);
                    h.cmd(Command::Quote { symbol: "EURUSD".into(), bid: Price::from_raw(mid), ask: Price::from_raw(mid + 10_000) });
                }
                3 => {
                    if let Some(p) = h.pos(acc).first() {
                        let v = Qty::from_raw((p.free_volume().raw() / 2 / 1_000_000).max(1) * 1_000_000);
                        h.cmd(Command::ClosePosition { account: acc, position_id: p.id, volume: Some(v.min(p.free_volume())), client_order_id: format!("c{i}") });
                    }
                }
                4 => {
                    for r in h.router.take() {
                        let part = Qty::from_raw(r.volume.raw() / 2);
                        let price = Price::from_raw(mid);
                        if part.is_positive() {
                            h.cmd(Command::LpFill { lp_order_id: r.lp_order_id, exec_id: format!("p{i}-{}", r.lp_order_id), volume: part, price });
                        }
                        h.cmd(Command::LpFill { lp_order_id: r.lp_order_id, exec_id: format!("f{i}-{}", r.lp_order_id), volume: r.volume, price });
                    }
                }
                _ => { h.cmd(Command::Rollover); }
            }
        }
        let replayed = Engine::replay(h.config.clone(), &h.journal);
        prop_assert_eq!(replayed.state_digest(), h.e.state_digest());
        for a in 1..=3u64 {
            prop_assert!(h.bal(a).minor >= 0 || !h.pos(a).is_empty(), "NBP: flat accounts are never negative");
        }
    }
}

fn change(o: &Order) -> OrderChange {
    OrderChange {
        volume: o.req.volume,
        limit_price: o.req.limit_price,
        stop_price: o.req.stop_price,
        sl: o.req.sl,
        tp: o.req.tp,
        trailing_points: o.req.trailing_points,
        expire_at: o.req.expire_at,
    }
}

#[test]
fn modify_pending_order_in_place() {
    let mut h = b();
    let o = h.pending(1, "lim", Side::Buy, OrderType::Limit, Some("1.09900"), None);
    let (_, id) = h.order(o);
    let mut c = change(h.e.order(id).unwrap());
    c.volume = qty("2");
    c.limit_price = Some(px("1.09950"));
    c.sl = Some(px("1.09000"));
    c.tp = Some(px("1.11000"));
    let ev = h.cmd(Command::ModifyOrder {
        account: 1,
        order_id: id,
        change: c.clone(),
    });
    assert_eq!(ev, vec![Event::OrderModified { order_id: id }]);
    let o = h.e.order(id).unwrap();
    assert_eq!(o.req.client_order_id, "lim");
    assert_eq!(o.req.volume, qty("2"));
    assert_eq!(o.req.limit_price, Some(px("1.09950")));
    // invalid change (SL above a buy entry) is rejected and nothing changes
    let mut bad = c.clone();
    bad.sl = Some(px("1.20000"));
    let ev = h.cmd(Command::ModifyOrder {
        account: 1,
        order_id: id,
        change: bad,
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
    assert_eq!(h.e.order(id).unwrap().req.sl, Some(px("1.09000")));
    // someone else's order
    h.account(2, "b", "1000");
    let ev = h.cmd(Command::ModifyOrder {
        account: 2,
        order_id: id,
        change: c.clone(),
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
    // moving the limit through the market fills it, SL/TP go to the position
    c.limit_price = Some(px("1.10050"));
    h.cmd(Command::ModifyOrder {
        account: 1,
        order_id: id,
        change: c.clone(),
    });
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Filled);
    let p = &h.pos(1)[0];
    assert_eq!((p.volume, p.sl, p.tp), (qty("2"), c.sl, c.tp));
    // filled order is no longer modifiable
    let ev = h.cmd(Command::ModifyOrder {
        account: 1,
        order_id: id,
        change: c,
    });
    assert!(matches!(ev[0], Event::CommandRejected { .. }));
}

#[test]
fn deal_history_records_entries_pnl_and_reasons() {
    let mut h = H::new(EngineConfig::default());
    let mut eu = SymbolSpec::fx("EURUSD", Currency::EUR, USD, 5);
    eu.commission_per_lot = usd("3.5");
    h.cmd(Command::AddSymbol(eu));
    h.account(1, "b", "10000");
    let mut o = NewOrder::market(1, "o", "EURUSD", Side::Buy, qty("2"));
    o.sl = Some(px("1.09000"));
    let (ev, _) = h.order(o);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::DealAdded { deal_id: 1 })));
    let pid = h.pos(1)[0].id;
    h.quote("EURUSD", "1.10100", "1.10110");
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: pid,
        volume: Some(qty("0.5")),
        client_order_id: "pc".into(),
    });
    h.quote("EURUSD", "1.08900", "1.08910");
    assert!(h.pos(1).is_empty(), "SL closed the rest");
    let d = h.e.deals();
    assert_eq!(d.len(), 3);
    assert_eq!(
        (d[0].entry, d[0].volume, d[0].pnl),
        (DealEntry::In, qty("2"), usd("0"))
    );
    assert_eq!(d[0].commission, usd("-7"));
    // (1.10100 - 1.10010) * 0.5 lot = 45
    assert_eq!(
        (d[1].entry, d[1].pnl, d[1].reason),
        (DealEntry::Out, usd("45"), OrderOrigin::Client)
    );
    assert_eq!(d[1].commission, usd("-1.75"));
    assert_eq!(d[2].reason, OrderOrigin::StopLoss);
    assert_eq!(d[2].volume, qty("1.5"));
    assert!(d[2].pnl.minor < 0);
    assert!(d.iter().all(|x| x.position_id == pid && x.account == 1));
    assert_eq!(h.e.deal(2).unwrap().id, 2);
    assert!(h.e.deal(0).is_none() && h.e.deal(4).is_none());
}

#[test]
fn netting_flip_yields_out_and_in_deals() {
    let mut h = H::new(EngineConfig::default());
    h.account(1, "n", "10000");
    h.market(1, "a", Side::Buy, "1");
    h.market(1, "b", Side::Sell, "1.5");
    let d = h.e.deals();
    assert_eq!(d.len(), 3);
    assert_eq!((d[1].entry, d[1].volume), (DealEntry::Out, qty("1")));
    assert_eq!((d[2].entry, d[2].volume), (DealEntry::In, qty("0.5")));
    assert_ne!(d[1].position_id, d[2].position_id);
    assert_eq!(h.pos(1)[0].id, d[2].position_id);
}

#[test]
fn hedge_switch_to_a_book_when_b_book_limit_exceeded() {
    use risk::{HedgeMode, HedgePolicy};
    let mut h = H::new(EngineConfig::default());
    h.account(1, "b", "100000");
    h.cmd(Command::SetHedge(HedgePolicy {
        enabled: true,
        mode: HedgeMode::SwitchToABook,
        default_symbol_limit: Some(qty("1")),
        ..HedgePolicy::default()
    }));
    let a = h.market(1, "h1", Side::Buy, "0.8");
    assert!(h.router.take().is_empty(), "within the limit: B-book");
    assert_eq!(h.e.order(a).unwrap().routing, Routing::BBook);
    let b = h.market(1, "h2", Side::Buy, "0.5");
    assert_eq!(h.router.take().len(), 1, "over the limit: goes to the LP");
    let o = h.e.order(b).unwrap();
    assert_eq!(o.routing, Routing::ABook);
    assert_eq!(o.rule.as_deref(), Some("hedge:limit"));
    // risk-reducing flow stays B-book even over the limit
    let c = h.market(1, "h3", Side::Sell, "0.3");
    assert!(h.router.take().is_empty());
    assert_eq!(h.e.order(c).unwrap().routing, Routing::BBook);
}

#[test]
fn hedge_excess_opens_and_unwinds_lp_hedge() {
    use risk::{HedgeMode, HedgePolicy};
    let mut h = H::new(EngineConfig::default());
    h.account(1, "b", "100000");
    h.cmd(Command::SetHedge(HedgePolicy {
        enabled: true,
        mode: HedgeMode::HedgeExcess,
        default_symbol_limit: Some(qty("1")),
        ..HedgePolicy::default()
    }));
    h.market(1, "x1", Side::Buy, "1.5");
    let sent = h.router.take();
    assert_eq!(sent.len(), 1, "excess 0.5 lot hedged at the LP");
    assert_eq!(sent[0].side, Side::Sell);
    assert_eq!(sent[0].volume, qty("0.5"));
    assert!(h.e.lp_order(sent[0].lp_order_id).unwrap().hedge);
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "hx1".into(),
        volume: qty("0.5"),
        price: px("1.10000"),
    });
    assert_eq!(h.e.hedge_net("EURUSD"), -qty("0.5").raw());
    assert_eq!(h.e.omnibus_net("EURUSD"), h.e.hedge_net("EURUSD"));
    // a tiny add stays under one hedge step: nothing new goes out
    h.market(1, "x2", Side::Buy, "0.001");
    assert!(h.router.take().is_empty());
    // client closes the big one: exposure falls under the release level, hedge unwinds
    let pid = h.pos(1)[0].id;
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: pid,
        volume: None,
        client_order_id: "c1".into(),
    });
    let sent = h.router.take();
    let unwind = sent
        .iter()
        .find(|r| r.side == Side::Buy)
        .expect("unwind hedge");
    h.quote("EURUSD", "1.09900", "1.09910");
    h.cmd(Command::LpFill {
        lp_order_id: unwind.lp_order_id,
        exec_id: "hx2".into(),
        volume: unwind.volume,
        price: px("1.09910"),
    });
    assert_eq!(h.e.hedge_pending("EURUSD"), 0);
    assert!(h.e.hedge_net("EURUSD").abs() < qty("0.02").raw());
    // sold 1.10000, bought back 1.09910: a gain on the hedge book
    assert!(h.e.hedge_realized("EURUSD") > 0);
    h.e.check_invariants().unwrap();
}

#[test]
fn toxicity_feeds_rules_and_profile() {
    use risk::{OrderKindFilter, RoutingRule};
    let mut h = H::new(EngineConfig::default());
    h.account(1, "b", "100000");
    // a rule sending toxic flow (score >= 50) to the A-book
    h.cmd(Command::SetRules(vec![RoutingRule {
        id: "toxic".into(),
        name: "toxic".into(),
        enabled: true,
        groups: vec![],
        accounts: vec![],
        symbols: vec![],
        min_centilots: None,
        max_centilots: None,
        kind: OrderKindFilter::Any,
        hours_utc: None,
        routing: Some(Routing::ABook),
        a_book_pct: None,
        markup_points: None,
        max_slippage_points: None,
        partial_fill: None,
        min_toxicity: Some(50),
        max_toxicity: None,
    }]));
    // five scalps: open and close within the same second, all winners
    for i in 0..5 {
        h.market(1, &format!("o{i}"), Side::Buy, "0.1");
        assert!(h.router.take().is_empty(), "not toxic yet: B-book");
        let pid = h.pos(1)[0].id;
        h.quote("EURUSD", "1.10100", "1.10110");
        h.cmd(Command::ClosePosition {
            account: 1,
            position_id: pid,
            volume: None,
            client_order_id: format!("c{i}"),
        });
        h.quote("EURUSD", "1.10000", "1.10010");
    }
    let f = h.e.flow(1).unwrap();
    assert_eq!(f.trades, 5);
    assert_eq!(f.short_holds, 5);
    assert!(h.e.toxicity(1) >= 50, "score {}", h.e.toxicity(1));
    h.market(1, "o9", Side::Buy, "0.1");
    assert_eq!(h.router.take().len(), 1, "toxic flow now routed to the LP");
}

#[test]
fn market_hours_and_holidays_reject_new_orders_only() {
    use risk::{TradingCalendar, TradingSession};
    let mut h = b();
    h.account(1, "b", "100000");
    // open a position while the market is open (no sessions yet)
    let id = h.market(1, "o1", Side::Buy, "0.1");
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Filled);
    // 1970-01-01 is a Thursday; sessions only on Friday -> closed now
    let mut eu = h.e.symbol_spec("EURUSD").unwrap().clone();
    eu.sessions = vec![TradingSession {
        day: 5,
        open_min: 0,
        close_min: 1440,
    }];
    h.cmd(Command::AddSymbol(eu));
    let id2 = h.market(1, "o2", Side::Buy, "0.1");
    assert_eq!(h.e.order(id2).unwrap().status, OrderStatus::Rejected);
    assert!(h
        .e
        .order(id2)
        .unwrap()
        .reject_reason
        .as_deref()
        .unwrap_or("")
        .contains("closed"));
    // closing the open position is still allowed
    let pid = h.pos(1)[0].id;
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: pid,
        volume: None,
        client_order_id: "c1".into(),
    });
    assert!(h.pos(1).is_empty());
    // holiday blocks new orders even with open sessions
    let mut eu = h.e.symbol_spec("EURUSD").unwrap().clone();
    eu.sessions.clear();
    h.cmd(Command::AddSymbol(eu));
    h.cmd(Command::SetCalendar(TradingCalendar {
        holidays: vec!["1970-01-01".into()],
    }));
    let id3 = h.market(1, "o3", Side::Buy, "0.1");
    assert_eq!(h.e.order(id3).unwrap().status, OrderStatus::Rejected);
    let ev = h.cmd(Command::Rollover);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Rollover { applied: false, .. })));
}

#[test]
fn same_exec_id_on_different_lp_orders_is_not_a_duplicate() {
    let mut h = H::new(EngineConfig::default());
    h.account(1, "a", "100000");
    h.market(1, "x1", Side::Buy, "0.1");
    let first = h.router.take()[0].lp_order_id;
    h.cmd(Command::LpFill {
        lp_order_id: first,
        exec_id: "E1".into(),
        volume: qty("0.1"),
        price: px("1.1001"),
    });
    // a replay of the same fill is ignored
    h.cmd(Command::LpFill {
        lp_order_id: first,
        exec_id: "E1".into(),
        volume: qty("0.1"),
        price: px("1.1001"),
    });
    h.market(1, "x2", Side::Buy, "0.1");
    let second = h.router.take()[0].lp_order_id;
    // simulator restarted: its exec ids begin at E1 again
    h.cmd(Command::LpFill {
        lp_order_id: second,
        exec_id: "E1".into(),
        volume: qty("0.1"),
        price: px("1.1002"),
    });
    let lots: i64 = h.pos(1).iter().map(|p| p.volume.raw()).sum();
    assert_eq!(lots, qty("0.2").raw());
}

#[test]
fn retry_keeps_the_slippage_cap() {
    use risk::PartialFill;
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 5;
    g.markup_ask_points = Some(8);
    g.max_slippage_points = Some(10);
    g.partial_fill = PartialFill::Retry { max_attempts: 3 };
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "10000");
    let mut o = h.pending(1, "m", Side::Buy, OrderType::Market, None, None);
    o.requested_price = Some(px("1.10018"));
    let (_, id) = h.order(o);
    let first = h.router.take();
    assert_eq!(first[0].limit, Some(px("1.10020")));
    h.cmd(Command::LpFill {
        lp_order_id: first[0].lp_order_id,
        exec_id: "x1".into(),
        volume: qty("0.4"),
        price: px("1.10010"),
    });
    h.cmd(Command::LpReject {
        lp_order_id: first[0].lp_order_id,
        reason: "ioc remainder".into(),
    });
    // the remainder goes out again, still bounded by the client's cap
    let again = h.router.take();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].volume, qty("0.6"));
    assert_eq!(again[0].limit, Some(px("1.10020")));
    assert_eq!(h.e.order(id).unwrap().lp_attempts, 2);
}

#[test]
fn market_order_without_cap_gets_the_circuit_breaker() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 0;
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "10000");
    let mut o = h.pending(1, "m", Side::Sell, OrderType::Market, None, None);
    o.requested_price = Some(px("1.10000"));
    h.order(o);
    // 50 bps of 1.10000 = 550 points: the LP may not fill a sell below 1.09450
    assert_eq!(h.router.take()[0].limit, Some(px("1.09450")));
    // no requested price: the client quote at execution is the reference
    // (EURUSD bid 1.10000 -> 1.09450)
    h.market(1, "n", Side::Sell, "0.1");
    assert_eq!(h.router.take()[0].limit, Some(px("1.09450")));
}

#[test]
fn client_deviation_tightens_the_cap() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 0;
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "10000");
    // client allows 30 points from the bid at execution (1.10000)
    let mut o = h.pending(1, "d", Side::Sell, OrderType::Market, None, None);
    o.max_deviation_points = Some(30);
    h.order(o);
    assert_eq!(h.router.take()[0].limit, Some(px("1.09970")));
    // a looser client value cannot widen the circuit breaker (550 points)
    let mut o = h.pending(1, "w", Side::Sell, OrderType::Market, None, None);
    o.max_deviation_points = Some(5_000);
    h.order(o);
    assert_eq!(h.router.take()[0].limit, Some(px("1.09450")));
}

/// A-book gap: the stop-out close fills at the LP even worse than the quote;
/// the balance goes negative only after that fill, and NBP restores zero
/// (the broker book carries the shortfall).
#[test]
fn abook_gap_stop_out_and_nbp() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 0;
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "1000");
    h.market(1, "x", Side::Buy, "0.5"); // margin 550
    let open = h.router.take();
    h.cmd(Command::LpFill {
        lp_order_id: open[0].lp_order_id,
        exec_id: "o1".into(),
        volume: qty("0.5"),
        price: px("1.10010"),
    });
    assert_eq!(h.pos(1).len(), 1);
    // weekend gap: 200 pips down -> equity 1000 - 1000.5 < 0, far below stop-out
    let ev = h.quote("EURUSD", "1.08000", "1.08010");
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::StopOut { account: 1, .. })));
    let close = h.router.take();
    assert_eq!(close.len(), 1, "stop-out goes to the LP");
    assert!(
        close[0].limit.is_none(),
        "stop-out is never bounded by the slippage guard"
    );
    // no compensation before the LP fill
    assert!(!ev
        .iter()
        .any(|e| matches!(e, Event::NegativeBalanceCompensated { .. })));
    // LP fills even lower (thin market after the gap)
    let ev = h.cmd(Command::LpFill {
        lp_order_id: close[0].lp_order_id,
        exec_id: "c1".into(),
        volume: qty("0.5"),
        price: px("1.07900"),
    });
    assert!(h.pos(1).is_empty());
    let comp = ev.iter().find_map(|e| match e {
        Event::NegativeBalanceCompensated { account: 1, amount } => Some(*amount),
        _ => None,
    });
    // loss (1.10010 - 1.07900) * 50 000 = 1055 -> balance -55 -> compensated 55
    assert_eq!(comp, Some(usd("55")));
    assert_eq!(h.bal(1), usd("0"));
}

#[test]
fn abook_stop_out_rejected_by_lp_is_retried() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 0;
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "1000");
    h.market(1, "x", Side::Buy, "0.5");
    let open = h.router.take();
    h.cmd(Command::LpFill {
        lp_order_id: open[0].lp_order_id,
        exec_id: "o1".into(),
        volume: qty("0.5"),
        price: px("1.10010"),
    });
    h.quote("EURUSD", "1.08200", "1.08210");
    let close = h.router.take();
    assert_eq!(close.len(), 1);
    // LP has no liquidity right after the gap
    h.cmd(Command::LpReject {
        lp_order_id: close[0].lp_order_id,
        reason: "no liquidity".into(),
    });
    assert_eq!(h.pos(1).len(), 1, "position still open");
    // next tick: stop-out fires again
    let ev = h.quote("EURUSD", "1.08190", "1.08200");
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::StopOut { account: 1, .. })));
    assert_eq!(h.router.take().len(), 1, "a new LP close order");
}

/// Weekend leverage cap: Friday 20:00 UTC the margin of open positions rises
/// (500:1 -> 100:1); an account that cannot carry it gets the margin call
/// before the gap, not after.
#[test]
fn weekend_leverage_raises_margin_before_the_gap() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("w", USD, Routing::BBook);
    g.esma = None;
    g.leverage = 500;
    g.weekend_leverage = Some(100);
    h.cmd(Command::SetGroup(g));
    h.account(1, "w", "1000");
    let friday = 1_791_504_000u64 * 1_000_000_000; // 2026-10-09 00:00 UTC
    h.ts = friday + 19 * 3_600 * 1_000_000_000;
    h.market(1, "x", Side::Buy, "1"); // 500:1 -> margin 220
    let r = h.e.account_risk(1).unwrap();
    assert!(!r.is_margin_call(h.e.group("w").unwrap()));
    h.ts = friday + 20 * 3_600 * 1_000_000_000;
    let ev = h.quote("EURUSD", "1.10000", "1.10010");
    // 100:1 -> margin 1100 > equity ~1000: margin call before the weekend
    assert!(ev.contains(&Event::MarginCall { account: 1 }));
    // new orders are checked against the weekend leverage too
    let (ev, _) = h.order(NewOrder::market(1, "y", "EURUSD", Side::Buy, qty("1")));
    assert!(ev.iter().any(|e| matches!(e, Event::OrderRejected { .. })));
}

#[test]
fn rollover_swap_free_fee_after_grace() {
    let mut h = b();
    let mut eu = h.e.symbol_spec("EURUSD").unwrap().clone();
    eu.triple_swap_day = 9; // never triple
    h.cmd(Command::AddSymbol(eu));
    let mut g = h.e.group("b").unwrap().clone();
    g.swap_multiplier_pct = 0;
    g.swap_free_fee_per_lot = 500; // $5 per lot per night
    g.swap_free_grace_days = 1;
    h.cmd(Command::SetGroup(g));
    h.market(1, "l", Side::Buy, "2");
    // day 0: inside the grace period, no charge
    h.cmd(Command::Rollover);
    assert_eq!(h.bal(1), usd("10000"));
    // day 1: 2 lots × $5
    h.ts += 86_400_000_000_000;
    h.cmd(Command::Rollover);
    assert_eq!(h.bal(1), usd("9990"));
}

#[test]
fn copy_trading_mirrors_scales_and_charges_hwm_fee() {
    let mut h = b();
    h.account(2, "b", "10000");
    // follower 2 copies provider 1 at 50 %, 20 % performance fee
    let ev = h.cmd(Command::CopySubscribe {
        follower: 2,
        provider: 1,
        ratio_bps: 5_000,
        equity_stop_pct: 0,
        perf_fee_bps: 2_000,
    });
    assert!(
        !ev.iter()
            .any(|e| matches!(e, Event::CommandRejected { .. })),
        "{ev:?}"
    );
    h.market(1, "p1", Side::Buy, "2");
    let copies: Vec<_> = h.e.positions_of(2).into_iter().cloned().collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].volume, qty("1"));
    assert_eq!(copies[0].side, Side::Buy);
    // provider closes half: the copy follows
    let pp = h.e.positions_of(1)[0].id;
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: pp,
        volume: Some(qty("1")),
        client_order_id: "c1".into(),
    });
    assert_eq!(h.e.positions_of(2)[0].volume, qty("0.5"));
    // price up, provider closes the rest: the copy closes in profit
    h.quote("EURUSD", "1.10200", "1.10210");
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: pp,
        volume: None,
        client_order_id: "c2".into(),
    });
    assert!(h.e.positions_of(2).is_empty());
    let sub = h.e.copy_subscriptions()[0].clone();
    assert!(sub.realized > 0, "{sub:?}");
    let before = h.bal(2);
    h.cmd(Command::CopySettle { provider: 1 });
    let settled_at = h.e.now_ns();
    let fee = sub.realized * 2_000 / 10_000;
    assert_eq!(h.bal(2).minor, before.minor - fee);
    // settling again charges nothing (high-water mark)
    h.cmd(Command::CopySettle { provider: 1 });
    assert_eq!(h.bal(2).minor, before.minor - fee);
    // both sides of the fee are kept for the statements
    let moves: Vec<_> =
        h.e.cash_moves()
            .iter()
            .map(|m| (m.account, m.kind, m.amount.minor, m.counterparty))
            .collect();
    assert_eq!(
        moves,
        vec![
            (2, CashMoveKind::CopyFee, -fee, Some(1)),
            (1, CashMoveKind::CopyFeeIncome, fee, Some(2)),
        ]
    );
    assert!(h.e.cash_moves().iter().all(|m| m.ts == settled_at));
    // replay reproduces the same state
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

#[test]
fn copy_trading_never_reopens_a_followers_manual_close() {
    let mut h = b();
    h.account(2, "b", "10000");
    h.cmd(Command::CopySubscribe {
        follower: 2,
        provider: 1,
        ratio_bps: 10_000,
        equity_stop_pct: 0,
        perf_fee_bps: 0,
    });
    h.market(1, "p1", Side::Sell, "1");
    let fp = h.e.positions_of(2)[0].id;
    h.cmd(Command::ClosePosition {
        account: 2,
        position_id: fp,
        volume: None,
        client_order_id: "mine".into(),
    });
    h.cmd(Command::Tick);
    h.cmd(Command::Tick);
    assert!(h.e.positions_of(2).is_empty());
    // unsubscribe stops new copies
    h.cmd(Command::CopyUnsubscribe {
        follower: 2,
        provider: 1,
        close: true,
    });
    h.market(1, "p2", Side::Buy, "1");
    assert!(h.e.positions_of(2).is_empty());
}

fn subscribe(h: &mut H, follower: u64, provider: u64, ratio_bps: u32, fee_bps: u32) -> Vec<Event> {
    h.cmd(Command::CopySubscribe {
        follower,
        provider,
        ratio_bps,
        equity_stop_pct: 0,
        perf_fee_bps: fee_bps,
    })
}

fn copy_sub(h: &H, follower: u64, provider: u64) -> CopySubscription {
    h.e.copy_subscriptions()
        .into_iter()
        .find(|s| s.follower == follower && s.provider == provider)
        .cloned()
        .unwrap()
}

fn rejected(ev: &[Event]) -> bool {
    ev.iter()
        .any(|e| matches!(e, Event::CommandRejected { .. }))
}

fn close_all(h: &mut H, acc: u64, clid: &str) {
    for p in h.pos(acc) {
        h.cmd(Command::ClosePosition {
            account: acc,
            position_id: p.id,
            volume: None,
            client_order_id: format!("{clid}-{}", p.id),
        });
    }
}

fn lp_fill(h: &mut H, lp_order_id: LpOrderId, exec: &str, v: &str, price: &str) {
    h.cmd(Command::LpFill {
        lp_order_id,
        exec_id: exec.into(),
        volume: qty(v),
        price: px(price),
    });
}

#[test]
fn copy_result_counts_every_close_and_the_opening_commission() {
    let mut h = b();
    let mut g = h.e.group("b").unwrap().clone();
    g.commission = Some(risk::GroupCommission::PerLot { minor: 350 }); // $3.50 per lot and side
    h.cmd(Command::SetGroup(g));
    h.account(2, "b", "10000");
    assert!(!rejected(&subscribe(&mut h, 2, 1, 10_000, 2_000)));
    // copy 1 hits the follower's own stop loss: -120
    h.market(1, "p1", Side::Buy, "1");
    let c1 = h.pos(2)[0].id;
    h.cmd(Command::ModifyPosition {
        account: 2,
        position_id: c1,
        sl: Some(px("1.09900")),
        tp: None,
        trailing_points: None,
    });
    h.quote("EURUSD", "1.09890", "1.09900");
    assert!(h.pos(2).is_empty(), "stop loss closed the copy");
    // copy 2 is closed by the provider: +200
    h.market(1, "p2", Side::Buy, "1");
    h.quote("EURUSD", "1.10100", "1.10110");
    let p2 = h.pos(1).iter().map(|p| p.id).max().unwrap();
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: p2,
        volume: None,
        client_order_id: "c2".into(),
    });
    assert!(h.pos(2).is_empty());
    // copy 3 is closed in full by the follower: +40
    h.market(1, "p3", Side::Buy, "1");
    let c3 = h.pos(2)[0].id;
    h.quote("EURUSD", "1.10150", "1.10160");
    h.cmd(Command::ClosePosition {
        account: 2,
        position_id: c3,
        volume: None,
        client_order_id: "mine".into(),
    });
    assert!(h.pos(2).is_empty());
    // -120 + 200 + 40, less 6 fills x $3.50 commission (opens included) = $99:
    // exactly what the follower's balance made
    assert_eq!(copy_sub(&h, 2, 1).realized, usd("99").minor);
    assert_eq!(h.bal(2), usd("10099"));
    h.cmd(Command::CopySettle { provider: 1 });
    assert_eq!(h.bal(2), usd("10079.20"));
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

#[test]
fn copy_resubscribe_keeps_the_copies() {
    let mut h = b();
    h.account(2, "b", "10000");
    subscribe(&mut h, 2, 1, 10_000, 0);
    h.market(1, "p1", Side::Buy, "1");
    let held = |h: &H| h.pos(2).iter().map(|p| p.volume.raw()).sum::<i64>();
    assert_eq!(held(&h), qty("1").raw());
    // a higher ratio is no top-up in the middle of a trade ...
    assert!(!rejected(&subscribe(&mut h, 2, 1, 20_000, 0)));
    h.cmd(Command::Tick);
    assert_eq!(held(&h), qty("1").raw());
    // ... a lower one brings the copy down to it
    subscribe(&mut h, 2, 1, 5_000, 0);
    assert_eq!(held(&h), qty("0.5").raw());
    // unsubscribed keeping the copies, subscribed again: still there, a
    // provider position opened in between is not copied
    h.cmd(Command::CopyUnsubscribe {
        follower: 2,
        provider: 1,
        close: false,
    });
    h.market(1, "p2", Side::Buy, "1");
    subscribe(&mut h, 2, 1, 5_000, 0);
    h.cmd(Command::Tick);
    assert_eq!(h.pos(2).len(), 1);
    assert_eq!(held(&h), qty("0.5").raw());
    // and still managed: the provider's close closes it
    close_all(&mut h, 1, "pc");
    assert!(h.pos(2).is_empty());
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

#[test]
fn copy_open_in_flight_when_the_provider_closes_is_closed_once_filled() {
    let mut h = b(); // provider 1: B-book, closes at once
    h.account(2, "a", "10000"); // follower 2: A-book
    subscribe(&mut h, 2, 1, 10_000, 0);
    h.market(1, "p1", Side::Buy, "1");
    let open = h.router.take();
    assert_eq!(open.len(), 1, "copy open at the LP");
    close_all(&mut h, 1, "pc");
    assert!(h.router.take().is_empty());
    // the LP fills after the provider is flat: the copy is closed right away
    lp_fill(&mut h, open[0].lp_order_id, "f1", "1", "1.10010");
    let close = h.router.take();
    assert_eq!(close.len(), 1);
    assert_eq!(close[0].side, Side::Sell);
    lp_fill(&mut h, close[0].lp_order_id, "f2", "1", "1.10000");
    assert!(h.pos(2).is_empty());
    assert!(copy_sub(&h, 2, 1).copied.is_empty());

    // the same at an unsubscribe with close: the open in flight is closed once filled
    h.market(1, "p2", Side::Buy, "1");
    let open = h.router.take();
    h.cmd(Command::CopyUnsubscribe {
        follower: 2,
        provider: 1,
        close: true,
    });
    assert!(copy_sub(&h, 2, 1).closing);
    assert!(
        rejected(&subscribe(&mut h, 2, 1, 10_000, 0)),
        "not while the copies are being closed"
    );
    lp_fill(&mut h, open[0].lp_order_id, "f3", "1", "1.10010");
    let close = h.router.take();
    assert_eq!(close.len(), 1);
    lp_fill(&mut h, close[0].lp_order_id, "f4", "1", "1.10000");
    assert!(h.pos(2).is_empty());
    assert!(!copy_sub(&h, 2, 1).closing);
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

#[test]
fn copy_open_rejected_or_cut_at_the_lp_is_sent_again() {
    let mut h = b();
    h.account(2, "a", "10000");
    subscribe(&mut h, 2, 1, 10_000, 0);
    h.market(1, "p1", Side::Buy, "1");
    let p1 = h.pos(1)[0].id;
    let lp = h.router.take()[0].lp_order_id;
    h.cmd(Command::LpReject {
        lp_order_id: lp,
        reason: "no liquidity".into(),
    });
    assert!(h.router.take().is_empty(), "backs off");
    assert_eq!(copy_sub(&h, 2, 1).copied[&p1], 0);
    h.ts += 5_000_000_000;
    h.cmd(Command::Tick);
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].volume, qty("1"));
    // IOC: 0.4 filled, the rest cancelled -> the 0.6 goes out again
    lp_fill(&mut h, sent[0].lp_order_id, "f1", "0.4", "1.10010");
    h.cmd(Command::LpReject {
        lp_order_id: sent[0].lp_order_id,
        reason: "ioc remainder".into(),
    });
    assert!(h.router.take().is_empty());
    assert_eq!(copy_sub(&h, 2, 1).copied[&p1], qty("0.4").raw());
    h.ts += 5_000_000_000;
    h.cmd(Command::Tick);
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].volume, qty("0.6"));
    lp_fill(&mut h, sent[0].lp_order_id, "f2", "0.6", "1.10010");
    let held: i64 = h.pos(2).iter().map(|p| p.volume.raw()).sum();
    assert_eq!(held, qty("1").raw());
    // the provider closes: both copies follow
    close_all(&mut h, 1, "pc");
    assert_eq!(h.router.take().len(), 2);
}

#[test]
fn copy_open_back_off_never_holds_back_the_providers_close() {
    let mut h = b();
    h.account(2, "a", "10000");
    subscribe(&mut h, 2, 1, 10_000, 0);
    h.market(1, "p1", Side::Buy, "1");
    let sent = h.router.take();
    // IOC: 0.4 filled, the rest cancelled -> the open backs off
    lp_fill(&mut h, sent[0].lp_order_id, "f1", "0.4", "1.10010");
    h.cmd(Command::LpReject {
        lp_order_id: sent[0].lp_order_id,
        reason: "ioc remainder".into(),
    });
    assert!(h.router.take().is_empty());
    // the provider closes during the open's back-off: the 0.4 follows at once
    close_all(&mut h, 1, "pc");
    let close = h.router.take();
    assert_eq!(close.len(), 1);
    assert_eq!((close[0].side, close[0].volume), (Side::Sell, qty("0.4")));
    lp_fill(&mut h, close[0].lp_order_id, "f2", "0.4", "1.10000");
    assert!(h.pos(2).is_empty());
    assert!(copy_sub(&h, 2, 1).copied.is_empty());
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

#[test]
fn copy_fee_change_sets_the_equity_stop_from_after_the_fee() {
    let mut h = b();
    h.account(2, "b", "10000");
    let sub = |h: &mut H, fee| {
        h.cmd(Command::CopySubscribe {
            follower: 2,
            provider: 1,
            ratio_bps: 10_000,
            equity_stop_pct: 3,
            perf_fee_bps: fee,
        })
    };
    assert!(!rejected(&sub(&mut h, 5_000)));
    h.market(1, "p1", Side::Buy, "1"); // 1.10010
    h.quote("EURUSD", "1.11010", "1.11020");
    close_all(&mut h, 1, "c1"); // copy +1000
    h.market(1, "p2", Side::Buy, "1");
    assert_eq!(h.pos(2).len(), 1);
    // the fee change settles $500 (5 % of equity, over the 3 % stop): the
    // stop counts from the equity after it, the copy stays
    assert!(!rejected(&sub(&mut h, 0)));
    assert_eq!(h.bal(2), usd("10500"));
    h.cmd(Command::Tick);
    let s = copy_sub(&h, 2, 1);
    assert!(s.active, "{:?}", s.stopped_reason);
    assert_eq!(h.pos(2).len(), 1);
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

#[test]
fn copy_fee_change_waives_what_the_follower_cannot_pay() {
    let mut h = b();
    h.account(2, "b", "10000");
    subscribe(&mut h, 2, 1, 10_000, 5_000); // 50 %
    h.market(1, "p1", Side::Buy, "1");
    h.quote("EURUSD", "1.10110", "1.10120");
    close_all(&mut h, 1, "c1"); // copy +100, fee due $50
    h.cmd(Command::Withdraw {
        account: 2,
        amount: usd("10080"),
        key: "w".into(),
    });
    // $20 paid of the old $50; the rest is never charged at the new fee
    subscribe(&mut h, 2, 1, 10_000, 2_000);
    let s = copy_sub(&h, 2, 1);
    assert_eq!((s.hwm, s.fees_paid), (usd("100").minor, usd("20").minor));
    h.cmd(Command::CopySettle { provider: 1 });
    assert_eq!(h.bal(2), usd("0"));
}

#[test]
fn copy_close_rejected_at_the_lp_backs_off() {
    let mut h = b();
    h.account(2, "a", "10000");
    subscribe(&mut h, 2, 1, 10_000, 0);
    h.market(1, "p1", Side::Buy, "1");
    let open = h.router.take();
    lp_fill(&mut h, open[0].lp_order_id, "f1", "1", "1.10010");
    close_all(&mut h, 1, "pc");
    let close = h.router.take();
    assert_eq!(close.len(), 1);
    // the LP refuses every close: no new close in the same command (no
    // reject / resend loop), one per back-off step (5 s, 10 s, 20 s)
    let mut lp = close[0].lp_order_id;
    for wait in [5u64, 10, 20] {
        h.cmd(Command::LpReject {
            lp_order_id: lp,
            reason: "instrument halted".into(),
        });
        assert!(h.router.take().is_empty());
        h.ts += (wait - 1) * 1_000_000_000;
        h.cmd(Command::Tick);
        assert!(h.router.take().is_empty(), "still backing off");
        h.ts += 1_000_000_000;
        h.cmd(Command::Tick);
        let again = h.router.take();
        assert_eq!(again.len(), 1);
        lp = again[0].lp_order_id;
    }
    lp_fill(&mut h, lp, "f2", "1", "1.10000");
    assert!(h.pos(2).is_empty());
    assert!(copy_sub(&h, 2, 1).copied.is_empty());
}

#[test]
fn copy_open_that_keeps_failing_gives_up() {
    let mut h = b();
    h.account(2, "b", "100"); // never enough margin for 1 lot
    subscribe(&mut h, 2, 1, 10_000, 0);
    h.market(1, "p1", Side::Buy, "1");
    let p1 = h.pos(1)[0].id;
    for _ in 0..400 {
        h.ts += 10_000_000_000;
        h.cmd(Command::Tick);
    }
    // over 4000 s: attempts after doubling waits, then the position is skipped
    let tries = h.e.orders().filter(|o| o.req.account == 2).count();
    assert_eq!(tries, 8);
    assert_eq!(copy_sub(&h, 2, 1).copied[&p1], qty("1").raw());
    // the provider's close still clears it
    close_all(&mut h, 1, "pc");
    assert!(copy_sub(&h, 2, 1).copied.is_empty());
}

#[test]
fn copy_below_the_minimum_lot_sends_nothing() {
    let mut h = b();
    let mut eu = h.e.symbol_spec("EURUSD").unwrap().clone();
    eu.min_lot = qty("0.1");
    h.cmd(Command::AddSymbol(eu));
    h.account(2, "b", "10000");
    subscribe(&mut h, 2, 1, 500, 0); // 5 %
    h.market(1, "p1", Side::Buy, "1"); // 0.05 lot
    h.cmd(Command::Tick);
    assert_eq!(h.e.orders().filter(|o| o.req.account == 2).count(), 0);
    h.market(1, "p2", Side::Buy, "2"); // 0.1 lot
    assert_eq!(h.pos(2).len(), 1);
}

#[test]
fn copy_opens_respect_disabled_symbols() {
    let mut h = b();
    h.account(2, "b", "100"); // too small for 1 lot
    subscribe(&mut h, 2, 1, 10_000, 0);
    h.market(1, "p1", Side::Buy, "1");
    assert!(h.pos(2).is_empty());
    // the symbol is switched off, then the follower funds the account
    let mut eu = h.e.symbol_spec("EURUSD").unwrap().clone();
    eu.enabled = false;
    h.cmd(Command::AddSymbol(eu.clone()));
    h.cmd(Command::Deposit {
        account: 2,
        amount: usd("10000"),
        key: "d2b".into(),
    });
    h.ts += 5_000_000_000;
    h.cmd(Command::Tick);
    assert!(h.pos(2).is_empty());
    let last = h.e.orders().filter(|o| o.req.account == 2).last().unwrap();
    assert_eq!(last.reject_reason.as_deref(), Some("symbol disabled"));
    // switched on again: copied after the back-off
    eu.enabled = true;
    h.cmd(Command::AddSymbol(eu));
    h.ts += 10_000_000_000;
    h.cmd(Command::Tick);
    assert_eq!(h.pos(2).len(), 1);
}

#[test]
fn copy_accounts_keep_their_group_while_subscribed() {
    let mut h = b();
    h.account(2, "b", "10000");
    subscribe(&mut h, 2, 1, 10_000, 0);
    for acc in [1, 2] {
        let ev = h.cmd(Command::SetAccountGroup {
            account: acc,
            group: "a".into(),
        });
        assert!(rejected(&ev), "{ev:?}");
    }
    h.cmd(Command::CopyUnsubscribe {
        follower: 2,
        provider: 1,
        close: true,
    });
    let ev = h.cmd(Command::SetAccountGroup {
        account: 2,
        group: "a".into(),
    });
    assert!(!rejected(&ev), "{ev:?}");
}

#[test]
fn copy_no_chains_in_either_direction() {
    let mut h = b();
    h.account(2, "b", "10000");
    h.account(3, "b", "10000");
    assert!(!rejected(&subscribe(&mut h, 2, 1, 10_000, 0)));
    // 1 has a follower: it cannot start copying 3
    assert!(rejected(&subscribe(&mut h, 1, 3, 10_000, 0)));
    // 2 copies 1: nobody can copy 2
    assert!(rejected(&subscribe(&mut h, 3, 2, 10_000, 0)));
}

#[test]
fn copy_fee_mark_moves_at_zero_fee_and_on_a_fee_change() {
    let mut h = b();
    h.account(2, "b", "10000");
    subscribe(&mut h, 2, 1, 10_000, 0);
    h.market(1, "p1", Side::Buy, "1"); // 1.10010
    h.quote("EURUSD", "1.10110", "1.10120");
    close_all(&mut h, 1, "c1"); // copy +100
    assert_eq!(copy_sub(&h, 2, 1).realized, usd("100").minor);
    // settled at a zero fee: nothing charged, the mark still moves
    h.cmd(Command::CopySettle { provider: 1 });
    assert_eq!(copy_sub(&h, 2, 1).hwm, usd("100").minor);
    // +50 more under 0 %, then the fee is raised: the old result is settled
    // at the old fee first, the new fee only applies from here on
    h.market(1, "p2", Side::Buy, "1"); // 1.10120
    h.quote("EURUSD", "1.10170", "1.10180");
    close_all(&mut h, 1, "c2");
    subscribe(&mut h, 2, 1, 10_000, 2_000);
    let s = copy_sub(&h, 2, 1);
    assert_eq!((s.hwm, s.perf_fee_bps), (usd("150").minor, 2_000));
    h.cmd(Command::CopySettle { provider: 1 });
    assert_eq!(h.bal(2), usd("10150"));
}

#[test]
fn copy_fee_never_drives_the_follower_negative() {
    let mut h = b();
    h.account(2, "b", "10000");
    subscribe(&mut h, 2, 1, 10_000, 5_000); // 50 %
    h.market(1, "p1", Side::Buy, "1");
    h.quote("EURUSD", "1.10110", "1.10120");
    close_all(&mut h, 1, "c1"); // copy +100, fee due $50
                                // the follower takes everything out before the settlement
    h.cmd(Command::Withdraw {
        account: 2,
        amount: usd("10100"),
        key: "w".into(),
    });
    assert_eq!(h.bal(2), usd("0"));
    let provider = h.bal(1);
    h.cmd(Command::CopySettle { provider: 1 });
    assert_eq!(h.bal(2), usd("0"));
    let s = copy_sub(&h, 2, 1);
    assert_eq!((s.hwm, s.fees_paid), (0, 0));
    // partly funded: what it can pay now ($20 for $40 of the gain), the rest later
    h.cmd(Command::Deposit {
        account: 2,
        amount: usd("20"),
        key: "d3".into(),
    });
    h.cmd(Command::CopySettle { provider: 1 });
    assert_eq!(h.bal(2), usd("0"));
    let s = copy_sub(&h, 2, 1);
    assert_eq!((s.hwm, s.fees_paid), (usd("40").minor, usd("20").minor));
    h.cmd(Command::Deposit {
        account: 2,
        amount: usd("100"),
        key: "d4".into(),
    });
    h.cmd(Command::CopySettle { provider: 1 });
    assert_eq!(h.bal(2), usd("70"));
    let s = copy_sub(&h, 2, 1);
    assert_eq!((s.hwm, s.fees_paid), (usd("100").minor, usd("50").minor));
    assert_eq!(h.bal(1).minor - provider.minor, usd("50").minor);
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

#[test]
fn positions_iterates_the_whole_book_once() {
    let mut h = b();
    h.account(2, "b", "10000");
    h.market(1, "x1", Side::Buy, "1");
    h.market(2, "x2", Side::Sell, "0.5");
    h.market(1, "x3", Side::Sell, "0.2");
    let all: Vec<(u64, u64)> = h.e.positions().map(|p| (p.id, p.account)).collect();
    assert_eq!(all.len(), 3);
    assert!(all.windows(2).all(|w| w[0].0 < w[1].0), "id order");
    for acc in [1, 2] {
        let mut ids: Vec<u64> = all.iter().filter(|p| p.1 == acc).map(|p| p.0).collect();
        ids.sort_unstable();
        let of: Vec<u64> = h.e.positions_of(acc).iter().map(|p| p.id).collect();
        assert_eq!(ids, of);
    }
}

#[test]
fn markout_measures_mid_move_after_fills() {
    let mut h = b();
    h.market(1, "m1", Side::Buy, "1");
    // fill at the ask (1.10010) with the mid at 1.10005; 1 s later the mid is
    // 1.10035 -> +30 points (5 digits): measured from the mid, not the fill
    h.ts += 1_000_000_000;
    h.quote("EURUSD", "1.10030", "1.10040");
    let f = h.e.flow(1).unwrap().clone();
    assert_eq!(f.markout_count, [1, 0, 0]);
    assert_eq!(f.markout_points_sum[0], 30);
    // 60 s later all horizons are measured and the sample is gone
    h.ts += 60_000_000_000;
    h.quote("EURUSD", "1.09990", "1.10000");
    let f = h.e.flow(1).unwrap().clone();
    assert_eq!(f.markout_count, [1, 1, 1]);
    assert_eq!(f.markout_points_sum[2], -10);
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

#[test]
fn markout_ignores_spread_and_markup() {
    // regression: the sample stored the client fill price, so with the mid
    // unchanged every fill read -(half spread + markup) points and the
    // markout could never add to the toxicity score
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("m", USD, Routing::BBook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 12;
    h.cmd(Command::SetGroup(g));
    h.account(1, "m", "10000");
    h.market(1, "m1", Side::Buy, "1");
    let p = h.pos(1)[0].clone();
    assert!(p.open_price > px("1.10010"), "{p:?}"); // ask + markup
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: p.id,
        volume: None,
        client_order_id: "c".into(),
    });
    // 5 s later the mid has not moved: the open and the close both read 0
    h.ts += 5_000_000_000;
    h.quote("EURUSD", "1.10000", "1.10010");
    let f = h.e.flow(1).unwrap().clone();
    assert_eq!(f.markout_count, [2, 2, 0]);
    assert_eq!(f.markout_points_sum, [0, 0, 0]);
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

#[test]
fn markout_drops_samples_from_snapshots_without_a_mid() {
    // a snapshot of the previous engine holds samples priced at the client
    // fill and no mid: they are dropped, never measured against a zero mid
    let mut h = b();
    h.market(1, "m1", Side::Buy, "1");
    let mut v = serde_json::to_value(h.e.snapshot()).unwrap();
    let s = v["state"]["markouts"][0].as_object_mut().unwrap();
    s.remove("mid").unwrap();
    s.insert("price".into(), px("1.10010").raw().into());
    let snap: EngineSnapshot = serde_json::from_value(v).unwrap();
    let mut old = Engine::restore(&snap, Box::new(NullRouter)).unwrap();
    h.ts += 2_000_000_000;
    let quote = |seq: u64, ts: u64| Envelope {
        seq,
        ts,
        cmd: Command::Quote {
            symbol: "EURUSD".into(),
            bid: px("1.10000"),
            ask: px("1.10010"),
        },
    };
    old.apply(&quote(h.seq + 1, h.ts));
    old.apply(&quote(h.seq + 2, h.ts + 60_000_000_000));
    let count = |e: &Engine| e.flow(1).map(|f| f.markout_count).unwrap_or_default();
    assert_eq!(count(&old), [0, 0, 0]);
    // the current engine measures the same fill
    h.quote("EURUSD", "1.10000", "1.10010");
    assert_eq!(count(&h.e), [1, 0, 0]);
}

#[test]
fn markout_snapshots_stay_readable_by_the_previous_image() {
    // regression: the previous image requires `price` on every pending
    // sample; without it a rollback cannot read the snapshot and boots from
    // the journal tail alone
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct PrevSample {
        account: AccountNo,
        symbol: String,
        sign: i64,
        price: i64,
        ts: u64,
        done: u8,
    }
    let mut h = b();
    h.market(1, "m1", Side::Buy, "1");
    let v = serde_json::to_value(h.e.snapshot()).unwrap();
    let prev: Vec<PrevSample> = serde_json::from_value(v["state"]["markouts"].clone()).unwrap();
    assert_eq!(prev.len(), 1);
    // and it measures from there: the mid, not the client fill at the ask
    assert_eq!(prev[0].price, px("1.10005").raw());
}

#[test]
fn swap_free_fee_on_a_book_is_broker_revenue() {
    let mut h = H::new(EngineConfig::default());
    let mut g = h.e.group("a").unwrap().clone();
    g.swap_multiplier_pct = 0;
    g.swap_free_fee_per_lot = 500; // $5 per lot per night
    g.swap_free_grace_days = 0;
    h.cmd(Command::SetGroup(g.clone()));
    h.account(1, "a", "10000");
    let fill = |h: &mut H, exec: &str, v: &str| {
        let sent = h.router.take();
        assert_eq!(sent.len(), 1);
        h.cmd(Command::LpFill {
            lp_order_id: sent[0].lp_order_id,
            exec_id: exec.into(),
            volume: qty(v),
            price: px("1.10010"),
        });
    };
    h.market(1, "x", Side::Buy, "2");
    fill(&mut h, "e1", "2");
    let broker = h.e.ledger().balance(BROKER_BOOK, USD).minor;
    let lp = h.e.ledger().balance(LP_COUNTERPARTY, USD).minor;
    // 1970-01-01 is a Thursday: one night, 2 lots × $5
    h.cmd(Command::Rollover);
    assert_eq!(h.bal(1), usd("9990"));
    // the fee is ours, not the LP's: the LP counterparty is untouched
    assert_eq!(h.e.ledger().balance(BROKER_BOOK, USD).minor - broker, 1000);
    assert_eq!(h.e.ledger().balance(LP_COUNTERPARTY, USD).minor, lp);
    let p = h.pos(1)[0].clone();
    assert_eq!((p.swap_minor, p.swap_fee_minor), (-1000, -1000));
    // closing half takes half the fee into the deal, marked as fee
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: p.id,
        volume: Some(qty("1")),
        client_order_id: "c1".into(),
    });
    fill(&mut h, "e2", "1");
    let d = h.e.deals().last().unwrap().clone();
    assert_eq!((d.swap, d.swap_fee), (-500, -500));
    // a real swap on the next night still passes through to the LP
    g.swap_multiplier_pct = 100;
    h.cmd(Command::SetGroup(g));
    h.ts += 86_400_000_000_000;
    let broker = h.e.ledger().balance(BROKER_BOOK, USD).minor;
    let lp = h.e.ledger().balance(LP_COUNTERPARTY, USD).minor;
    h.cmd(Command::Rollover);
    assert_eq!(h.e.ledger().balance(BROKER_BOOK, USD).minor, broker);
    assert_eq!(h.e.ledger().balance(LP_COUNTERPARTY, USD).minor - lp, 700);
    let p = h.pos(1)[0].clone();
    assert_eq!((p.swap_minor, p.swap_fee_minor), (-1200, -500));
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: p.id,
        volume: None,
        client_order_id: "c2".into(),
    });
    fill(&mut h, "e3", "1");
    let d = h.e.deals().last().unwrap().clone();
    assert_eq!((d.swap, d.swap_fee), (-1200, -500));
    let replayed = Engine::replay(h.config.clone(), &h.journal);
    assert_eq!(replayed.state_digest(), h.e.state_digest());
}

// ---- LP-resting orders (GroupConfig::lp_resting) ---------------------------

fn resting_h() -> H {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 5;
    g.lp_resting = true;
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "10000");
    h
}

/// Opens 1 lot long for account 1 at the LP (1.10010) and returns the position id.
fn resting_long(h: &mut H) -> PositionId {
    h.market(1, "x", Side::Buy, "1");
    let lp = h.router.take()[0].lp_order_id;
    h.cmd(Command::LpFill {
        lp_order_id: lp,
        exec_id: "e1".into(),
        volume: qty("1"),
        price: px("1.10010"),
    });
    h.pos(1)[0].id
}

#[test]
fn resting_tp_rests_at_the_lp_and_only_the_lp_fill_closes() {
    let mut h = resting_h();
    let pid = resting_long(&mut h);
    assert!(h.router.take().is_empty(), "no TP yet: nothing rests");
    h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: None,
        tp: Some(px("1.10100")),
        trailing_points: None,
    });
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    let r = &sent[0];
    assert!(r.resting);
    assert_eq!((r.side, r.volume), (Side::Sell, qty("1")));
    // the client sells at LP bid − 5 points: the LP must fill at TP + 5
    assert_eq!(r.limit, Some(px("1.10105")));
    assert_eq!(h.pos(1)[0].lp_tp, Some(r.lp_order_id));
    // our own quote through the TP closes nothing (the LP did not trade)
    h.quote("EURUSD", "1.10300", "1.10310");
    assert_eq!(h.pos(1).len(), 1);
    assert!(h.router.take().is_empty());
    // the LP fills the resting order: the position closes at LP − markup
    h.cmd(Command::LpFill {
        lp_order_id: r.lp_order_id,
        exec_id: "t1".into(),
        volume: qty("1"),
        price: px("1.10110"),
    });
    assert!(h.pos(1).is_empty());
    let d = h.e.deals().last().unwrap();
    assert_eq!(d.price, px("1.10105"));
    assert_eq!(h.e.omnibus_net("EURUSD"), 0);
    assert!(h.router.take().is_empty(), "no IOC close was sent");
}

#[test]
fn resting_tp_follows_modify_partial_close_and_removal() {
    let mut h = resting_h();
    let pid = resting_long(&mut h);
    h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: None,
        tp: Some(px("1.10100")),
        trailing_points: None,
    });
    let lp = h.router.take()[0].lp_order_id;
    // new TP → replace with the new limit, revision 1
    h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: None,
        tp: Some(px("1.10200")),
        trailing_points: None,
    });
    let rep = h.router.take_replaced();
    assert_eq!(rep.len(), 1);
    assert_eq!((rep[0].lp_order_id, rep[0].revision), (lp, 1));
    assert_eq!(rep[0].limit, Some(px("1.10205")));
    // manual partial close 0.4: the resting quantity drops to 0.6 at once
    h.cmd(Command::ClosePosition {
        account: 1,
        position_id: pid,
        volume: Some(qty("0.4")),
        client_order_id: "c1".into(),
    });
    let rep = h.router.take_replaced();
    assert_eq!(rep.len(), 1);
    assert_eq!((rep[0].volume, rep[0].revision), (qty("0.6"), 2));
    let close_lp = h.router.take()[0].lp_order_id; // the IOC close itself
    h.cmd(Command::LpFill {
        lp_order_id: close_lp,
        exec_id: "c".into(),
        volume: qty("0.4"),
        price: px("1.10050"),
    });
    assert!(h.router.take_replaced().is_empty(), "0.6 already rests");
    // TP removed → cancel at the LP; the LP's confirmation ends the order
    h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: None,
        tp: None,
        trailing_points: None,
    });
    assert_eq!(h.router.take_cancelled(), vec![(lp, 2)]);
    assert_eq!(h.pos(1)[0].lp_tp, None);
    h.cmd(Command::LpReject {
        lp_order_id: lp,
        reason: "Canceled".into(),
    });
    assert!(h.e.lp_orders().find(|l| l.id == lp).unwrap().done);
}

#[test]
fn resting_tp_reject_falls_back_to_our_trigger() {
    let mut h = resting_h();
    let pid = resting_long(&mut h);
    h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: None,
        tp: Some(px("1.10100")),
        trailing_points: None,
    });
    let lp = h.router.take()[0].lp_order_id;
    h.cmd(Command::LpReject {
        lp_order_id: lp,
        reason: "no eligible LP".into(),
    });
    assert_eq!(h.pos(1)[0].lp_tp, None);
    // trigger mode again: the client bid (LP bid − 5) through the TP sends an IOC close
    h.quote("EURUSD", "1.10110", "1.10120");
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert!(!sent[0].resting);
}

#[test]
fn resting_limit_entry_rests_at_the_lp() {
    let mut h = resting_h();
    let o = h.pending(1, "l1", Side::Buy, OrderType::Limit, Some("1.09900"), None);
    let (_, id) = h.order(o);
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].resting);
    // the client buys at LP ask + 5 points: the LP limit is 5 points lower
    assert_eq!(sent[0].limit, Some(px("1.09895")));
    assert_eq!(h.e.order(id).unwrap().lp_resting, Some(sent[0].lp_order_id));
    // our quote through the limit does not execute it
    h.quote("EURUSD", "1.09800", "1.09810");
    assert!(h.router.take().is_empty());
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Accepted);
    // modify the price → replace
    h.cmd(Command::ModifyOrder {
        account: 1,
        order_id: id,
        change: OrderChange {
            volume: qty("1"),
            limit_price: Some(px("1.09850")),
            stop_price: None,
            sl: None,
            tp: Some(px("1.10500")),
            trailing_points: None,
            expire_at: None,
        },
    });
    let rep = h.router.take_replaced();
    assert_eq!(rep.len(), 1);
    assert_eq!(rep[0].limit, Some(px("1.09845")));
    // the LP fills: position opens at LP + markup, and its TP rests at once
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "f".into(),
        volume: qty("1"),
        price: px("1.09840"),
    });
    let p = &h.pos(1)[0];
    assert_eq!(p.open_price, px("1.09845"));
    assert_eq!(h.e.order(id).unwrap().lp_resting, None);
    let tp = h.router.take();
    assert_eq!(tp.len(), 1);
    assert!(tp[0].resting);
    assert_eq!(tp[0].limit, Some(px("1.10505")));
    assert_eq!(p.lp_tp, Some(tp[0].lp_order_id));
    // a second entry cancelled by the client is cancelled at the LP
    let o = h.pending(1, "l2", Side::Buy, OrderType::Limit, Some("1.09000"), None);
    let (_, id2) = h.order(o);
    let lp2 = h.router.take()[0].lp_order_id;
    h.cmd(Command::CancelOrder {
        account: 1,
        order_id: id2,
    });
    assert_eq!(h.router.take_cancelled(), vec![(lp2, 0)]);
    assert_eq!(h.e.order(id2).unwrap().status, OrderStatus::Cancelled);
}

#[test]
fn resting_sl_rests_as_a_stop_and_the_lp_fill_closes_with_stop_loss() {
    let mut h = resting_h();
    let pid = resting_long(&mut h);
    h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: Some(px("1.09900")),
        tp: Some(px("1.10100")),
        trailing_points: None,
    });
    let sent = h.router.take();
    assert_eq!(sent.len(), 2);
    let tp = sent.iter().find(|r| r.limit.is_some()).expect("tp limit");
    let sl = sent.iter().find(|r| r.stop.is_some()).expect("sl stop");
    assert_eq!((sl.side, sl.limit), (Side::Sell, None));
    // the client sells at LP bid − 5 points: the stop sits 5 points above the SL
    assert_eq!(sl.stop, Some(px("1.09905")));
    assert_eq!(h.pos(1)[0].lp_sl, Some(sl.lp_order_id));
    // our own quote through the SL closes nothing: the LP's stop decides
    h.quote("EURUSD", "1.09800", "1.09810");
    assert_eq!(h.pos(1).len(), 1);
    assert!(h.router.take().is_empty());
    // the LP's stop fires and fills: closed as a stop loss at LP − markup;
    // the resting TP is cancelled with the position
    h.cmd(Command::LpFill {
        lp_order_id: sl.lp_order_id,
        exec_id: "s1".into(),
        volume: qty("1"),
        price: px("1.09890"),
    });
    assert!(h.pos(1).is_empty());
    let d = h.e.deals().last().unwrap();
    assert_eq!(d.price, px("1.09885"));
    assert_eq!(d.reason, OrderOrigin::StopLoss);
    assert_eq!(h.router.take_cancelled(), vec![(tp.lp_order_id, 0)]);
    assert_eq!(h.e.omnibus_net("EURUSD"), 0);
}

#[test]
fn resting_stop_entry_rests_at_the_lp() {
    let mut h = resting_h();
    let o = h.pending(1, "s1", Side::Buy, OrderType::Stop, None, Some("1.10500"));
    let (_, id) = h.order(o);
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].limit, sent[0].stop), (None, Some(px("1.10495"))));
    // our quote through the stop does not execute it
    h.quote("EURUSD", "1.10600", "1.10610");
    assert!(h.router.take().is_empty());
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Accepted);
    h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "f".into(),
        volume: qty("1"),
        price: px("1.10500"),
    });
    assert_eq!(h.pos(1)[0].open_price, px("1.10505"));
}

#[test]
fn trailing_stop_replaces_the_lp_stop_at_most_once_a_second() {
    let mut h = resting_h();
    let pid = resting_long(&mut h);
    h.cmd(Command::ModifyPosition {
        account: 1,
        position_id: pid,
        sl: Some(px("1.09900")),
        tp: None,
        trailing_points: Some(50),
    });
    h.router.take();
    // in profit by more than 50 points: the SL trails, the LP stop is replaced
    h.quote("EURUSD", "1.10200", "1.10210");
    assert_eq!(h.router.take_replaced().len(), 1);
    // a tick later (1 ms in this harness): trailed again here, not sent yet
    h.quote("EURUSD", "1.10210", "1.10220");
    assert!(h.router.take_replaced().is_empty());
    h.ts += 1_000_000_000;
    h.quote("EURUSD", "1.10220", "1.10230");
    let rep = h.router.take_replaced();
    assert_eq!(rep.len(), 1);
    assert_eq!(rep[0].stop, Some(px("1.10170")));
}

#[test]
fn retry_chain_reaches_the_client_as_one_fill() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.markup_points = 5;
    g.partial_fill = risk::PartialFill::Retry { max_attempts: 3 };
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "100000");
    let id = h.market(1, "x", Side::Buy, "5");
    let lp1 = h.router.take()[0].lp_order_id;
    // the LP fills 3 of 5 and cancels the rest (IOC)
    let ev = h.cmd(Command::LpFill {
        lp_order_id: lp1,
        exec_id: "a".into(),
        volume: qty("3"),
        price: px("1.10010"),
    });
    assert!(
        !ev.iter().any(|e| matches!(e, Event::OrderFilled { .. })),
        "nothing shown yet"
    );
    assert_eq!(h.e.order(id).unwrap().status, OrderStatus::Accepted);
    assert!(h.pos(1).is_empty());
    assert_eq!(h.e.omnibus_net("EURUSD"), qty("3").raw(), "omnibus is live");
    h.cmd(Command::LpReject {
        lp_order_id: lp1,
        reason: "Canceled".into(),
    });
    // the remainder (2, not 5) goes out at once
    let sent = h.router.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].volume, qty("2"));
    let ev = h.cmd(Command::LpFill {
        lp_order_id: sent[0].lp_order_id,
        exec_id: "b".into(),
        volume: qty("2"),
        price: px("1.10030"),
    });
    // one fill, one position, one deal: VWAP 1.10018 + 5 points
    let fills: Vec<_> = ev
        .iter()
        .filter(|e| matches!(e, Event::OrderFilled { .. }))
        .collect();
    assert_eq!(fills.len(), 1);
    assert!(
        matches!(fills[0], Event::OrderFilled { status: OrderStatus::Filled, volume, .. } if *volume == qty("5"))
    );
    let p = &h.pos(1)[0];
    assert_eq!((p.volume, p.open_price), (qty("5"), px("1.10023")));
    assert_eq!(
        h.e.deals().iter().filter(|d| d.position_id == p.id).count(),
        1
    );
    assert_eq!(h.e.omnibus_net("EURUSD"), qty("5").raw());
}

#[test]
fn retry_chain_exhausted_shows_the_partial_once_and_cancels_the_rest() {
    let mut h = H::new(EngineConfig::default());
    let mut g = GroupConfig::retail("a", USD, Routing::ABook);
    g.esma = None;
    g.leverage = 100;
    g.partial_fill = risk::PartialFill::Retry { max_attempts: 2 };
    h.cmd(Command::SetGroup(g));
    h.account(1, "a", "100000");
    let id = h.market(1, "x", Side::Buy, "5");
    let lp1 = h.router.take()[0].lp_order_id;
    h.cmd(Command::LpFill {
        lp_order_id: lp1,
        exec_id: "a".into(),
        volume: qty("3"),
        price: px("1.10010"),
    });
    h.cmd(Command::LpReject {
        lp_order_id: lp1,
        reason: "Canceled".into(),
    });
    let lp2 = h.router.take()[0].lp_order_id;
    h.cmd(Command::LpFill {
        lp_order_id: lp2,
        exec_id: "b".into(),
        volume: qty("1"),
        price: px("1.10020"),
    });
    let ev = h.cmd(Command::LpReject {
        lp_order_id: lp2,
        reason: "Canceled".into(),
    });
    // attempts exhausted: 4 lots reach the client as one fill, 1 lot is cancelled
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, Event::OrderFilled { .. }))
            .count(),
        1
    );
    let o = h.e.order(id).unwrap();
    assert_eq!((o.status, o.filled), (OrderStatus::Cancelled, qty("4")));
    assert_eq!(h.pos(1)[0].volume, qty("4"));
    assert!(h.router.take().is_empty());
}
