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
}

impl Client {
    async fn login(addr: &str, account: &str) -> (Client, AccountSnapshot) {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .unwrap();
        let mut c = Client { ws, seq: 0 };
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
        loop {
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
