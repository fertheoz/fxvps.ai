//! End-to-end: lp-simulator + fix-gateway in-process over real TCP.

use std::time::Duration;

use domain::{ExecType, Fixed, Instrument, Order, OrderStatus, OrderType, Side, TimeInForce};
use fix_gateway::{GatewayConfig, GatewayEvent, OrderCommand, SessionEndpoint, SessionKind};
use tokio::sync::broadcast;

fn gateway_config(sim: &lp_simulator::SimHandle) -> GatewayConfig {
    let ep = |addr: String, sender: &str, target: &str| SessionEndpoint {
        addr,
        sender_comp_id: sender.into(),
        target_comp_id: target.into(),
        username: Some("demo".into()),
        password: Some("demo".into()),
        reset_on_logon: true,
        tls: None,
    };
    GatewayConfig {
        lp: "SIM".into(),
        heartbeat_secs: 30,
        market_depth: 3,
        reconnect_delay_ms: 100,
        max_logon_failures: 3,
        enabled: true,
        security_id_source: "8".into(),
        store_dir: None,
        md: ep(sim.md_addr.to_string(), "FXVPS-MD", "LMXBDM"),
        trade: ep(sim.trade_addr.to_string(), "FXVPS-TRD", "LMXBD"),
        instruments: vec![Instrument {
            symbol: "EUR/USD".into(),
            security_id: "4001".into(),
            tick_size: Fixed::from_parts(1, 5),
            qty_step: Fixed::from_int(1),
            contract_size: 1,
        }],
        nats: None,
    }
}

async fn next<F, T>(rx: &mut broadcast::Receiver<GatewayEvent>, mut f: F) -> T
where
    F: FnMut(GatewayEvent) -> Option<T>,
{
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Some(t) = f(ev) {
                        return t;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("event channel: {e}"),
            }
        }
    })
    .await
    .expect("timed out waiting for gateway event")
}

fn order(
    id: &str,
    side: Side,
    qty: i64,
    ord_type: OrderType,
    px: Option<Fixed>,
    tif: TimeInForce,
) -> Order {
    Order {
        cl_ord_id: id.into(),
        symbol: "EUR/USD".into(),
        side,
        qty: Fixed::from_int(qty),
        ord_type,
        limit_price: px,
        tif,
    }
}

async fn setup() -> (
    lp_simulator::SimHandle,
    fix_gateway::GatewayHandle,
    broadcast::Receiver<GatewayEvent>,
) {
    let sim = lp_simulator::start(lp_simulator::SimConfig::for_tests())
        .await
        .unwrap();
    let gw = fix_gateway::start(gateway_config(&sim)).unwrap();
    let mut rx = gw.subscribe();
    let mut md = false;
    let mut tr = false;
    next(&mut rx, |e| {
        if let GatewayEvent::SessionUp { session } = e {
            match session {
                SessionKind::MarketData => md = true,
                SessionKind::Trading => tr = true,
            }
        }
        (md && tr).then_some(())
    })
    .await;
    (sim, gw, rx)
}

#[tokio::test]
async fn logon_quotes_market_order_fill() {
    let (sim, gw, mut rx) = setup().await;

    // Quotes stream in and are normalized.
    let mut quotes = Vec::new();
    while quotes.len() < 3 {
        quotes.push(
            next(&mut rx, |e| match e {
                GatewayEvent::Quote(q) => Some(q),
                _ => None,
            })
            .await,
        );
    }
    for q in &quotes {
        assert_eq!(q.symbol, "EUR/USD");
        assert_eq!(q.lp, "SIM");
        assert_eq!(q.bids.len(), 3);
        assert!(q.best_bid().unwrap().price < q.best_ask().unwrap().price);
    }

    // Market order -> New + Trade(Filled).
    gw.orders()
        .send(OrderCommand::Submit(order(
            "e2e-1",
            Side::Buy,
            100_000,
            OrderType::Market,
            None,
            TimeInForce::ImmediateOrCancel,
        )))
        .await
        .unwrap();
    let new = next(&mut rx, |e| match e {
        GatewayEvent::Execution(x) if x.cl_ord_id.as_deref() == Some("e2e-1") => Some(x),
        _ => None,
    })
    .await;
    assert_eq!(new.exec_type, ExecType::New);
    let fill = next(&mut rx, |e| match e {
        GatewayEvent::Execution(x) if x.cl_ord_id.as_deref() == Some("e2e-1") => Some(x),
        _ => None,
    })
    .await;
    assert_eq!(fill.exec_type, ExecType::Trade);
    assert_eq!(fill.status, OrderStatus::Filled);
    assert_eq!(fill.symbol, "EUR/USD");
    assert_eq!(fill.cum_qty, Fixed::from_int(100_000));
    assert_eq!(fill.leaves_qty, Fixed::ZERO);
    let px = fill.last_px.unwrap();
    assert!(
        px > Fixed::from_int(1) && px < Fixed::from_int(2),
        "price {px}"
    );
    assert_eq!(fill.avg_px, Some(px));

    gw.shutdown().await;
    sim.shutdown().await;
}

#[tokio::test]
async fn partial_fills_fok_reject_and_cancel() {
    let (sim, gw, mut rx) = setup().await;
    let orders = gw.orders();
    let execs_for = |id: &'static str| {
        move |e: GatewayEvent| match e {
            GatewayEvent::Execution(x) if x.cl_ord_id.as_deref() == Some(id) => Some(x),
            _ => None,
        }
    };

    // 2.5M sweeps two levels (1M + 2M available): partial then filled.
    orders
        .send(OrderCommand::Submit(order(
            "sweep",
            Side::Sell,
            2_500_000,
            OrderType::Market,
            None,
            TimeInForce::ImmediateOrCancel,
        )))
        .await
        .unwrap();
    assert_eq!(
        next(&mut rx, execs_for("sweep")).await.exec_type,
        ExecType::New
    );
    let p1 = next(&mut rx, execs_for("sweep")).await;
    assert_eq!(p1.status, OrderStatus::PartiallyFilled);
    let p2 = next(&mut rx, execs_for("sweep")).await;
    assert_eq!(p2.status, OrderStatus::Filled);
    assert_eq!(p2.cum_qty, Fixed::from_int(2_500_000));

    // FOK larger than the whole book is killed.
    orders
        .send(OrderCommand::Submit(order(
            "fok",
            Side::Buy,
            50_000_000,
            OrderType::Market,
            None,
            TimeInForce::FillOrKill,
        )))
        .await
        .unwrap();
    let k = next(&mut rx, execs_for("fok")).await;
    assert_eq!(
        (k.exec_type, k.status, k.cum_qty),
        (ExecType::Canceled, OrderStatus::Canceled, Fixed::ZERO)
    );

    // Resting limit far from market, then cancel.
    orders
        .send(OrderCommand::Submit(order(
            "rest",
            Side::Buy,
            1_000,
            OrderType::Limit,
            Some(Fixed::from_parts(5, 1)),
            TimeInForce::Day,
        )))
        .await
        .unwrap();
    assert_eq!(
        next(&mut rx, execs_for("rest")).await.status,
        OrderStatus::New
    );
    orders
        .send(OrderCommand::Cancel {
            cl_ord_id: "rest-c".into(),
            orig_cl_ord_id: "rest".into(),
            symbol: "EUR/USD".into(),
            side: Side::Buy,
        })
        .await
        .unwrap();
    let c = next(&mut rx, execs_for("rest-c")).await;
    assert_eq!(c.status, OrderStatus::Canceled);
    assert_eq!(c.orig_cl_ord_id.as_deref(), Some("rest"));

    // Cancel of unknown order -> OrderCancelReject.
    orders
        .send(OrderCommand::Cancel {
            cl_ord_id: "x-c".into(),
            orig_cl_ord_id: "nope".into(),
            symbol: "EUR/USD".into(),
            side: Side::Buy,
        })
        .await
        .unwrap();
    next(&mut rx, |e| {
        matches!(e, GatewayEvent::CancelRejected { ref cl_ord_id, .. } if cl_ord_id == "x-c")
            .then_some(())
    })
    .await;

    // Duplicate ClOrdID rejected by the LP.
    orders
        .send(OrderCommand::Submit(order(
            "sweep",
            Side::Buy,
            1,
            OrderType::Market,
            None,
            TimeInForce::ImmediateOrCancel,
        )))
        .await
        .unwrap();
    let r = next(&mut rx, |e| match e {
        GatewayEvent::Execution(x) if x.exec_type == ExecType::Rejected => Some(x),
        _ => None,
    })
    .await;
    assert_eq!(r.status, OrderStatus::Rejected);

    // Unknown symbol never leaves the gateway.
    let mut bad = order(
        "bad",
        Side::Buy,
        1,
        OrderType::Market,
        None,
        TimeInForce::ImmediateOrCancel,
    );
    bad.symbol = "XAU/USD".into();
    orders.send(OrderCommand::Submit(bad)).await.unwrap();
    next(&mut rx, |e| {
        matches!(e, GatewayEvent::CommandRejected { ref cl_ord_id, .. } if cl_ord_id == "bad")
            .then_some(())
    })
    .await;

    gw.shutdown().await;
    sim.shutdown().await;
}

#[tokio::test]
async fn reconnects_after_lp_restart() {
    let mut cfg = lp_simulator::SimConfig::for_tests();
    let (sim, gw, mut rx) = setup().await;
    // Restart the simulator on the same ports.
    cfg.md.listen = sim.md_addr.to_string();
    cfg.trade.listen = sim.trade_addr.to_string();
    sim.shutdown().await;
    next(&mut rx, |e| {
        matches!(
            e,
            GatewayEvent::SessionDown {
                session: SessionKind::Trading,
                ..
            }
        )
        .then_some(())
    })
    .await;
    let sim = lp_simulator::start(cfg).await.unwrap();
    next(&mut rx, |e| {
        matches!(
            e,
            GatewayEvent::SessionUp {
                session: SessionKind::Trading
            }
        )
        .then_some(())
    })
    .await;
    next(&mut rx, |e| {
        matches!(e, GatewayEvent::Quote(_)).then_some(())
    })
    .await;
    gw.shutdown().await;
    sim.shutdown().await;
}
