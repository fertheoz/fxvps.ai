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
