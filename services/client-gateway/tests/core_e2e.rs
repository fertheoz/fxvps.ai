//! Full path: WebSocket client -> client-gateway -> core-engine (risk, OMS,
//! ledger) -> FixLpRouter -> fix-gateway -> lp-simulator and back.

use std::time::Duration;

use client_gateway::auth::{issue_hs256, Authenticator};
use client_gateway::demo::{Demo, DemoOptions};
use client_gateway::ClientGatewayConfig;
use client_proto::*;
use domain::Fixed;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const KEY: &[u8] = b"core-e2e-key";

struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    seq: u64,
    backlog: Vec<Body>,
}

impl Client {
    async fn login(addr: &str, account: &str) -> (Client, AccountSnapshot) {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .unwrap();
        let mut c = Client {
            ws,
            seq: 0,
            backlog: Vec::new(),
        };
        c.send(Body::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "core-e2e".into(),
            max_quote_hz: 0,
        }))
        .await;
        assert!(matches!(c.next().await, Body::Hello(_)));
        c.send(Body::Auth(Auth {
            token: issue_hs256(KEY, "trader", &[account], 300),
        }))
        .await;
        assert!(matches!(c.next().await, Body::AuthOk(_)));
        let snap = match c.next().await {
            Body::AccountSnapshot(s) => s,
            b => panic!("expected snapshot, got {b:?}"),
        };
        c.send(Body::Subscribe(Subscribe {
            request_id: "sub".into(),
            symbols: vec!["EURUSD".into()],
            depth_symbols: vec![],
        }))
        .await;
        // wait for live (marked-up) quotes before trading
        c.until(|b| matches!(b, Body::QuoteBatch(_)).then_some(()))
            .await;
        (c, snap)
    }

    async fn send(&mut self, body: Body) {
        self.seq += 1;
        self.ws
            .send(Message::Binary(
                Envelope::new(self.seq, body).to_protobuf().into(),
            ))
            .await
            .unwrap();
    }

    async fn next(&mut self) -> Body {
        loop {
            match tokio::time::timeout(Duration::from_secs(15), self.ws.next())
                .await
                .expect("timeout")
            {
                Some(Ok(Message::Binary(b))) => {
                    return Envelope::from_protobuf(&b).unwrap().body.unwrap()
                }
                Some(Ok(Message::Close(f))) => panic!("closed: {f:?}"),
                Some(Ok(_)) => continue,
                other => panic!("ws error: {other:?}"),
            }
        }
    }

    async fn until<T>(&mut self, mut f: impl FnMut(Body) -> Option<T>) -> T {
        // frames skipped while waiting for an Ack come first
        while !self.backlog.is_empty() {
            if let Some(t) = f(self.backlog.remove(0)) {
                return t;
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            assert!(tokio::time::Instant::now() < deadline, "until: timed out");
            if let Some(t) = f(self.next().await) {
                return t;
            }
        }
    }

    async fn market(&mut self, id: &str, account: &str, side: Side, units: i64) {
        self.send(Body::PlaceOrder(PlaceOrder {
            request_id: id.into(),
            account_id: account.into(),
            symbol: "EURUSD".into(),
            side: side as i32,
            order_type: OrderType::Market as i32,
            qty: Some(Fixed::from_int(units).into()),
            limit_price: None,
            tif: TimeInForce::Unspecified as i32,
            ..Default::default()
        }))
        .await;
    }

    /// Waits for the Filled update of `id` (fails on errors / rejects).
    async fn filled(&mut self, id: &str) -> OrderUpdate {
        self.until(|b| match b {
            Body::Error(e) if e.request_id == id => panic!("order error: {e:?}"),
            Body::OrderUpdate(u) if u.client_request_id == id => {
                assert_ne!(u.status, OrderStatus::Rejected as i32, "{u:?}");
                (u.status == OrderStatus::Filled as i32).then_some(u)
            }
            _ => None,
        })
        .await
    }
}

fn fx(d: Option<Decimal>) -> Fixed {
    d.and_then(|d| d.to_fixed()).expect("decimal")
}

async fn start() -> (Demo, String) {
    let cfg = ClientGatewayConfig {
        max_quote_hz: 50,
        ..Default::default()
    };
    let demo = Demo::start_with(
        cfg,
        Authenticator::hs256(KEY),
        DemoOptions {
            tick_ms: Some(20),
            volatility_ticks: Some(1),
            data_dir: None,
        },
    )
    .await
    .unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap().to_string();
    tokio::spawn(client_gateway::serve(demo.hub.clone(), l));
    (demo, addr)
}

/// Subscribing with `depth_symbols` streams the LP book (simulator: several
/// levels per side, sizes set) as `Depth` frames, marked up like the quotes.
#[tokio::test]
async fn depth_frames_follow_depth_subscription() {
    let (_demo, addr) = start().await;
    let (mut c, _) = Client::login(&addr, "DEMO-1").await;
    c.send(Body::Subscribe(Subscribe {
        request_id: "sub-depth".into(),
        symbols: vec![],
        depth_symbols: vec!["EURUSD".into()],
    }))
    .await;
    let d = c
        .until(|b| match b {
            Body::Depth(d) if d.symbol == "EURUSD" => Some(d),
            _ => None,
        })
        .await;
    assert!(d.bids.len() > 1 && d.asks.len() > 1, "{d:?}");
    let best_bid = fx(d.bids[0].price);
    let best_ask = fx(d.asks[0].price);
    assert!(best_bid < best_ask, "{d:?}");
    assert!(fx(d.bids[1].price) < best_bid, "bids best first: {d:?}");
    assert!(fx(d.bids[0].qty).is_positive(), "{d:?}");
    // lots, not LP contracts: the simulator quotes a few hundred lots per level at most
    assert!(fx(d.bids[0].qty) < Fixed::from_int(10_000), "{d:?}");
}

#[tokio::test]
async fn open_close_margin_reject_and_stop_out() {
    let (demo, addr) = start().await;

    // --- fill -> PositionUpdate with P&L, close -> realized P&L - commission ---
    let (mut c, snap) = Client::login(&addr, "DEMO-1").await;
    assert_eq!(snap.currency, "USD");
    let b0 = fx(snap.balance);
    assert_eq!(b0, Fixed::from_int(10_000));
    assert_eq!(fx(snap.free_margin), b0);

    c.market("o1", "DEMO-1", Side::Buy, 100_000).await;
    let open = c.filled("o1").await;
    assert_eq!(fx(open.filled_qty), Fixed::from_int(100_000));
    let open_px = fx(open.avg_price);
    let pos = c
        .until(|b| match b {
            Body::PositionUpdate(p) if p.account_id == "DEMO-1" => p.position,
            _ => None,
        })
        .await;
    assert_eq!(fx(pos.net_qty), Fixed::from_int(100_000));
    assert_eq!(fx(pos.avg_price), open_px);
    // bought at the marked-up ask: immediately under water by the spread
    assert!(fx(pos.unrealized_pnl).raw() < 0, "{pos:?}");
    let acc = c
        .until(|b| match b {
            Body::AccountSnapshot(a) if a.account_id == "DEMO-1" => Some(a),
            _ => None,
        })
        .await;
    assert!(fx(acc.margin_used).is_positive());
    assert_eq!(fx(acc.balance), "9996.5".parse().unwrap()); // commission 3.50

    c.market("c1", "DEMO-1", Side::Sell, 100_000).await;
    let close = c.filled("c1").await;
    let close_px = fx(close.avg_price);
    let acc = c
        .until(|b| match b {
            Body::AccountSnapshot(a) if a.account_id == "DEMO-1" => Some(a),
            _ => None,
        })
        .await;
    let realized = Fixed::from_raw((close_px.raw() - open_px.raw()) * 100_000);
    let commission: Fixed = "7".parse().unwrap();
    assert_eq!(fx(acc.balance) - b0, realized - commission);
    assert!(acc.positions.is_empty());
    assert_eq!(fx(acc.margin_used), Fixed::ZERO);

    // --- insufficient margin: 5 lots at 1:30 needs ~18k USD ---
    let (mut c2, _) = Client::login(&addr, "DEMO-2").await;
    c2.market("big", "DEMO-2", Side::Buy, 500_000).await;
    let err = c2
        .until(|b| match b {
            Body::Error(e) if e.request_id == "big" => Some(e),
            Body::Ack(a) if a.request_id == "big" => panic!("accepted"),
            _ => None,
        })
        .await;
    assert_eq!(err.code, ErrorCode::InsufficientMargin as i32);
    let snap2 = demo.hub.account_snapshot("DEMO-2").await.expect("snapshot");
    assert!(snap2.positions.is_empty());

    // --- stop-out after a price shock at the LP ---
    let (mut c3, _) = Client::login(&addr, "DEMO-3").await;
    c3.market("lev", "DEMO-3", Side::Buy, 200_000).await;
    c3.filled("lev").await;
    let mid = demo.sim.mid("EUR/USD").unwrap();
    // -4 big figures on 2 lots = -8000 USD: equity far below 50% of margin
    assert!(demo
        .sim
        .set_mid("EUR/USD", Fixed::from_raw(mid.raw() - 4_000_000)));
    let so = c3
        .until(|b| match b {
            Body::OrderUpdate(u)
                if u.client_request_id.starts_with("so-")
                    && u.status == OrderStatus::Filled as i32 =>
            {
                Some(u)
            }
            _ => None,
        })
        .await;
    assert_eq!(so.side, Side::Sell as i32);
    assert_eq!(fx(so.filled_qty), Fixed::from_int(200_000));
    let acc = c3
        .until(|b| match b {
            Body::AccountSnapshot(a) if a.account_id == "DEMO-3" => Some(a),
            _ => None,
        })
        .await;
    assert!(acc.positions.is_empty(), "{acc:?}");
    let bal = fx(acc.balance);
    assert!(
        bal < Fixed::from_int(3_000) && bal.raw() >= 0,
        "balance after stop-out: {bal}"
    );

    demo.shutdown().await;
}

// ---------------------------------------------------------------------------
// v1.2: order types, protection, position ids, partial close, history
// ---------------------------------------------------------------------------

fn dec(s: &str) -> Option<Decimal> {
    Some(s.parse::<Fixed>().unwrap().into())
}

fn px_off(mid: Fixed, off: &str) -> Fixed {
    mid + off.parse::<Fixed>().unwrap()
}

impl Client {
    /// Sends `body` and waits for its Ack (panics on Error).
    /// Sends `body` and waits for its Ack (panics on Error); other frames
    /// (e.g. OrderUpdates published before the Ack) stay available to `until`.
    async fn acked(&mut self, id: &str, body: Body) {
        self.send(body).await;
        let mut skipped = Vec::new();
        loop {
            match self.next().await {
                Body::Ack(a) if a.request_id == id => break,
                Body::Error(e) if e.request_id == id => panic!("{id} failed: {e:?}"),
                b => skipped.push(b),
            }
        }
        self.backlog.extend(skipped);
    }

    async fn order_update(&mut self, id: &str, status: OrderStatus) -> OrderUpdate {
        self.until(|b| match b {
            Body::OrderUpdate(u) if u.client_request_id == id && u.status == status as i32 => {
                Some(u)
            }
            _ => None,
        })
        .await
    }

    async fn position(&mut self, f: impl Fn(&Position) -> bool) -> Position {
        self.until(|b| match b {
            Body::PositionUpdate(p) => p.position.filter(|p| f(p)),
            _ => None,
        })
        .await
    }
}

fn pending(id: &str, account: &str, side: Side, t: OrderType, qty: i64) -> PlaceOrder {
    PlaceOrder {
        request_id: id.into(),
        account_id: account.into(),
        symbol: "EURUSD".into(),
        side: side as i32,
        order_type: t as i32,
        qty: Some(Fixed::from_int(qty).into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn order_types_protection_hedging_and_history() {
    let (demo, addr) = start().await;
    let mid = || demo.sim.mid("EUR/USD").unwrap();

    // --- hedging account: account info, several positions, partial close ---
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    let mut h = Client {
        ws,
        seq: 0,
        backlog: Vec::new(),
    };
    h.send(Body::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        ..Default::default()
    }))
    .await;
    assert!(matches!(h.next().await, Body::Hello(_)));
    h.send(Body::Auth(Auth {
        token: issue_hs256(KEY, "trader", &["DEMO-H1"], 300),
    }))
    .await;
    let ok = match h.next().await {
        Body::AuthOk(a) => a,
        b => panic!("{b:?}"),
    };
    let info = &ok.accounts[0];
    assert_eq!(info.account_id, "DEMO-H1");
    assert_eq!(info.margin_mode, MarginMode::Hedging as i32);
    assert_eq!(info.leverage, 30);
    assert_eq!(info.currency, "USD");
    let snap = match h.next().await {
        Body::AccountSnapshot(s) => s,
        b => panic!("{b:?}"),
    };
    assert_eq!(snap.margin_mode, MarginMode::Hedging as i32);
    assert_eq!(snap.leverage, 30);
    h.send(Body::Subscribe(Subscribe {
        request_id: "sub".into(),
        symbols: vec!["EURUSD".into()],
        depth_symbols: vec![],
    }))
    .await;

    h.market("h1", "DEMO-H1", Side::Buy, 100_000).await;
    let f1 = h.filled("h1").await;
    let p1 = h.position(|p| p.position_id == f1.position_id).await;
    h.market("h2", "DEMO-H1", Side::Buy, 100_000).await;
    let f2 = h.filled("h2").await;
    assert_ne!(
        f1.position_id, f2.position_id,
        "hedging: one position per fill"
    );
    let snap = demo.hub.account_snapshot("DEMO-H1").await.unwrap();
    assert_eq!(snap.positions.len(), 2);
    assert!(snap.positions.iter().all(|p| p.symbol == "EURUSD"));

    // partial close of the first position: realized P&L on half of it
    h.acked(
        "pc1",
        Body::ClosePosition(ClosePosition {
            request_id: "pc1".into(),
            account_id: "DEMO-H1".into(),
            position_id: p1.position_id.clone(),
            qty: Some(Fixed::from_int(50_000).into()),
        }),
    )
    .await;
    let close = h.filled("pc1").await;
    assert_eq!(close.side, Side::Sell as i32);
    assert_eq!(close.close_position_id, p1.position_id);
    let out = h
        .until(|b| match b {
            Body::DealUpdate(d) => d.deal.filter(|d| d.entry == DealEntry::Out as i32),
            _ => None,
        })
        .await;
    assert_eq!(out.client_request_id, "pc1");
    assert_eq!(out.reason, DealReason::Client as i32);
    let realized = Fixed::from_raw((fx(close.avg_price).raw() - fx(p1.avg_price).raw()) * 50_000);
    assert_eq!(fx(out.realized_pnl), realized);
    assert_eq!(fx(out.commission), "-1.75".parse().unwrap());
    let rest = h
        .position(|p| p.position_id == p1.position_id && fx(p.qty) == Fixed::from_int(50_000))
        .await;
    assert_eq!(fx(rest.net_qty), Fixed::from_int(50_000));
    let snap = demo.hub.account_snapshot("DEMO-H1").await.unwrap();
    assert_eq!(snap.positions.len(), 2);
    let mut bad = ClosePosition {
        request_id: "pc-bad".into(),
        account_id: "DEMO-H1".into(),
        position_id: "999999".into(),
        qty: None,
    };
    h.send(Body::ClosePosition(bad.clone())).await;
    let e = h
        .until(|b| match b {
            Body::Error(e) if e.request_id == "pc-bad" => Some(e),
            _ => None,
        })
        .await;
    assert_eq!(e.code, ErrorCode::UnknownOrder as i32);
    // closing more than the position holds is rejected
    bad.request_id = "pc-big".into();
    bad.position_id = p1.position_id.clone();
    bad.qty = Some(Fixed::from_int(500_000).into());
    h.send(Body::ClosePosition(bad)).await;
    let e = h
        .until(|b| match b {
            Body::Error(e) if e.request_id == "pc-big" => Some(e),
            _ => None,
        })
        .await;
    assert_eq!(e.code, ErrorCode::OrderRejected as i32);

    // --- deal history, paged ---
    let mut all = Vec::new();
    let mut cursor = String::new();
    let mut pages = 0;
    loop {
        let id = format!("hist{pages}");
        h.send(Body::DealHistoryRequest(DealHistoryRequest {
            request_id: id.clone(),
            account_id: "DEMO-H1".into(),
            limit: 2,
            cursor: cursor.clone(),
            ..Default::default()
        }))
        .await;
        let page = h
            .until(|b| match b {
                Body::DealHistory(d) if d.request_id == id => Some(d),
                Body::Error(e) if e.request_id == id => panic!("{e:?}"),
                _ => None,
            })
            .await;
        assert!(page.deals.len() <= 2);
        pages += 1;
        all.extend(page.deals);
        if page.next_cursor.is_empty() {
            break;
        }
        cursor = page.next_cursor;
    }
    assert_eq!(pages, 2);
    assert_eq!(all.len(), 3, "{all:?}");
    let entries: Vec<i32> = all.iter().map(|d| d.entry).collect();
    assert_eq!(
        entries,
        vec![
            DealEntry::In as i32,
            DealEntry::In as i32,
            DealEntry::Out as i32
        ]
    );
    assert!(all.windows(2).all(|w| w[0].ts_ns <= w[1].ts_ns));
    // someone else's history is forbidden
    h.send(Body::DealHistoryRequest(DealHistoryRequest {
        request_id: "hist-x".into(),
        account_id: "DEMO-1".into(),
        ..Default::default()
    }))
    .await;
    let e = h
        .until(|b| match b {
            Body::Error(e) if e.request_id == "hist-x" => Some(e),
            _ => None,
        })
        .await;
    assert_eq!(e.code, ErrorCode::Forbidden as i32);

    // --- pending order: native modify, order list, OCO ---
    let (mut c, snap) = Client::login(&addr, "DEMO-2").await;
    assert_eq!(snap.margin_mode, MarginMode::Netting as i32);
    let m = mid();
    let mut lim = pending("lim", "DEMO-2", Side::Buy, OrderType::Limit, 100_000);
    lim.limit_price = Some(px_off(m, "-0.0200").into());
    lim.oco_group = 7;
    c.acked("lim", Body::PlaceOrder(lim)).await;
    let new = c.order_update("lim", OrderStatus::New).await;
    assert_eq!(new.order_type, OrderType::Limit as i32);
    let mut stp = pending("stp", "DEMO-2", Side::Buy, OrderType::Stop, 100_000);
    stp.stop_price = Some(px_off(m, "0.0015").into());
    stp.oco_group = 7;
    stp.tp = Some(px_off(m, "0.0300").into());
    c.acked("stp", Body::PlaceOrder(stp)).await;
    c.order_update("stp", OrderStatus::New).await;

    // modify the limit in place: price, qty, SL
    c.acked(
        "mod1",
        Body::ModifyOrder(ModifyOrder {
            request_id: "mod1".into(),
            account_id: "DEMO-2".into(),
            target_request_id: "lim".into(),
            qty: Some(Fixed::from_int(200_000).into()),
            limit_price: Some(px_off(m, "-0.0150").into()),
            sl: Some(px_off(m, "-0.0250").into()),
            ..Default::default()
        }),
    )
    .await;
    let modified = c.order_update("lim", OrderStatus::New).await;
    assert_eq!(modified.order_id, new.order_id, "modified in place");
    assert_eq!(fx(modified.qty), Fixed::from_int(200_000));
    assert_eq!(fx(modified.limit_price), px_off(m, "-0.0150"));
    assert_eq!(fx(modified.sl), px_off(m, "-0.0250"));
    // an invalid change is rejected and leaves the order alone
    c.send(Body::ModifyOrder(ModifyOrder {
        request_id: "mod-bad".into(),
        account_id: "DEMO-2".into(),
        target_request_id: "lim".into(),
        sl: Some(px_off(m, "0.0500").into()),
        ..Default::default()
    }))
    .await;
    let e = c
        .until(|b| match b {
            Body::Error(e) if e.request_id == "mod-bad" => Some(e),
            Body::Ack(a) if a.request_id == "mod-bad" => panic!("accepted"),
            _ => None,
        })
        .await;
    assert_eq!(e.code, ErrorCode::OrderRejected as i32);

    c.send(Body::OrderListRequest(OrderListRequest {
        request_id: "ol".into(),
        account_id: "DEMO-2".into(),
        include_history: false,
    }))
    .await;
    let list = c
        .until(|b| match b {
            Body::OrderList(l) if l.request_id == "ol" => Some(l),
            _ => None,
        })
        .await;
    let ids: Vec<&str> = list
        .orders
        .iter()
        .map(|o| o.client_request_id.as_str())
        .collect();
    assert_eq!(ids, vec!["lim", "stp"]);
    assert_eq!(list.orders[1].order_type, OrderType::Stop as i32);
    assert_eq!(fx(list.orders[1].stop_price), px_off(m, "0.0015"));
    assert_eq!(list.orders[0].oco_group, 7);

    // price rises through the stop: it fills, the OCO sibling is cancelled
    assert!(demo.sim.set_mid("EUR/USD", px_off(m, "0.0040")));
    // (the sibling is cancelled when the stop starts executing, before its fill)
    let mut cancelled = false;
    let filled = c
        .until(|b| match b {
            Body::OrderUpdate(u) if u.client_request_id == "lim" => {
                cancelled |= u.status == OrderStatus::Canceled as i32;
                None
            }
            Body::OrderUpdate(u)
                if u.client_request_id == "stp" && u.status == OrderStatus::Filled as i32 =>
            {
                Some(u)
            }
            _ => None,
        })
        .await;
    assert!(cancelled, "OCO sibling cancelled");
    assert!(fx(filled.avg_price) >= px_off(m, "0.0015"));
    let pos = c.position(|p| fx(p.net_qty).is_positive()).await;
    assert_eq!(
        fx(pos.tp),
        px_off(m, "0.0300"),
        "TP carried to the position"
    );
    assert!(!pos.position_id.is_empty());

    // --- SL/TP set + modify, trailing stop moves, SL hit closes ---
    let m = mid();
    let set = |id: &str, sl: Option<Decimal>, tp: Option<Decimal>, tr: Option<Decimal>| {
        Body::ModifyPosition(ModifyPosition {
            request_id: id.into(),
            account_id: "DEMO-2".into(),
            position_id: pos.position_id.clone(),
            sl,
            tp,
            trailing_distance: tr,
        })
    };
    c.acked(
        "mp1",
        set(
            "mp1",
            Some(px_off(m, "-0.0100").into()),
            Some(px_off(m, "0.0200").into()),
            None,
        ),
    )
    .await;
    let p = c.position(|p| p.sl.is_some() && p.tp.is_some()).await;
    assert_eq!(fx(p.sl), px_off(m, "-0.0100"));
    assert_eq!(fx(p.tp), px_off(m, "0.0200"));
    // invalid SL (above the bid of a long) is rejected
    c.send(set("mp-bad", Some(px_off(m, "0.0100").into()), None, None))
        .await;
    let e = c
        .until(|b| match b {
            Body::Error(e) if e.request_id == "mp-bad" => Some(e),
            Body::Ack(a) if a.request_id == "mp-bad" => panic!("accepted"),
            _ => None,
        })
        .await;
    assert_eq!(e.code, ErrorCode::OrderRejected as i32);
    // trailing 20 pips
    c.acked(
        "mp2",
        set(
            "mp2",
            Some(px_off(m, "-0.0100").into()),
            Some(px_off(m, "0.0200").into()),
            dec("0.0020"),
        ),
    )
    .await;
    assert!(demo.sim.set_mid("EUR/USD", px_off(m, "0.0080")));
    let trailed = c
        .position(|p| {
            p.sl.is_some_and(|s| s.to_fixed().unwrap() > px_off(m, "0.0040"))
        })
        .await;
    assert_eq!(fx(trailed.trailing_distance), "0.002".parse().unwrap());
    assert!(
        fx(trailed.sl) > fx(trailed.avg_price),
        "trailing locked in profit"
    );
    // fall back: the (trailed) SL closes the position
    assert!(demo.sim.set_mid("EUR/USD", px_off(m, "0.0030")));
    let sl = c
        .until(|b| match b {
            Body::OrderUpdate(u)
                if u.client_request_id.starts_with("sl-")
                    && u.status == OrderStatus::Filled as i32 =>
            {
                Some(u)
            }
            _ => None,
        })
        .await;
    assert_eq!(sl.close_position_id, pos.position_id);
    let deal = c
        .until(|b| match b {
            Body::DealUpdate(d) => d.deal.filter(|d| d.reason == DealReason::StopLoss as i32),
            _ => None,
        })
        .await;
    assert!(fx(deal.realized_pnl).is_positive(), "{deal:?}");
    let flat = c
        .position(|p| p.position_id == pos.position_id && fx(p.net_qty) == Fixed::ZERO)
        .await;
    assert_eq!(flat.symbol, "EURUSD");
    let snap = demo.hub.account_snapshot("DEMO-2").await.unwrap();
    assert!(snap.positions.is_empty());

    // --- take profit and GTD expiry on another account ---
    let (mut t, _) = Client::login(&addr, "DEMO-3").await;
    let m = mid();
    let mut o = pending("tp1", "DEMO-3", Side::Sell, OrderType::Market, 100_000);
    o.tp = Some(px_off(m, "-0.0030").into());
    o.sl = Some(px_off(m, "0.0100").into());
    t.send(Body::PlaceOrder(o)).await;
    t.filled("tp1").await;
    assert!(demo.sim.set_mid("EUR/USD", px_off(m, "-0.0060")));
    t.until(|b| match b {
        Body::DealUpdate(d) => d.deal.filter(|d| d.reason == DealReason::TakeProfit as i32),
        _ => None,
    })
    .await;
    let mut gtd = pending("gtd", "DEMO-3", Side::Buy, OrderType::StopLimit, 100_000);
    gtd.stop_price = Some(px_off(m, "0.0500").into());
    gtd.limit_price = Some(px_off(m, "0.0510").into());
    gtd.tif = TimeInForce::Gtd as i32;
    gtd.expire_at_ns = domain::now_ns() + 300_000_000;
    t.acked("gtd", Body::PlaceOrder(gtd)).await;
    let exp = t.order_update("gtd", OrderStatus::Expired).await;
    assert_eq!(exp.order_type, OrderType::StopLimit as i32);

    demo.shutdown().await;
}
