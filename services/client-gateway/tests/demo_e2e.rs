//! End-to-end: lp-simulator + fix-gateway + client-gateway in demo mode, driven by a
//! real WebSocket client.

use std::time::Duration;

use client_gateway::auth::{issue_hs256, Authenticator};
use client_gateway::demo::Demo;
use client_gateway::ClientGatewayConfig;
use client_proto::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

const KEY: &[u8] = b"e2e-test-key";

#[tokio::test]
async fn demo_quotes_and_market_order_fill() {
    let cfg = ClientGatewayConfig {
        max_quote_hz: 20,
        ..Default::default()
    };
    let demo = Demo::start(cfg, Authenticator::hs256(KEY), Some(20))
        .await
        .unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(client_gateway::serve(demo.hub.clone(), l));

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    let mut seq = 0;
    let mut send = |body: Body| {
        seq += 1;
        Message::Binary(Envelope::new(seq, body).to_protobuf().into())
    };
    async fn next_body(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Body {
        loop {
            match tokio::time::timeout(Duration::from_secs(15), ws.next())
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

    ws.send(send(Body::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        client_name: "e2e".into(),
        max_quote_hz: 0,
    })))
    .await
    .unwrap();
    assert!(matches!(next_body(&mut ws).await, Body::Hello(_)));
    ws.send(send(Body::Auth(Auth {
        token: issue_hs256(KEY, "trader", &["DEMO-1"], 300),
    })))
    .await
    .unwrap();
    assert!(matches!(next_body(&mut ws).await, Body::AuthOk(_)));
    match next_body(&mut ws).await {
        Body::AccountSnapshot(s) => assert_eq!(s.account_id, "DEMO-1"),
        b => panic!("expected snapshot, got {b:?}"),
    }

    ws.send(send(Body::Subscribe(Subscribe {
        request_id: "s1".into(),
        symbols: vec!["EURUSD".into()],
        depth_symbols: vec![],
    })))
    .await
    .unwrap();

    // Live simulated quotes arrive.
    let mut quotes = 0;
    while quotes < 3 {
        if let Body::QuoteBatch(b) = next_body(&mut ws).await {
            for q in b.quotes {
                assert_eq!(q.symbol, "EURUSD");
                let bid = q.bid.unwrap().to_fixed().unwrap();
                let ask = q.ask.unwrap().to_fixed().unwrap();
                assert!(bid < ask, "{bid} < {ask}");
                quotes += 1;
            }
        }
    }

    ws.send(send(Body::PlaceOrder(PlaceOrder {
        request_id: "ord-1".into(),
        account_id: "DEMO-1".into(),
        symbol: "EURUSD".into(),
        side: Side::Buy as i32,
        order_type: OrderType::Market as i32,
        qty: Some(Decimal {
            value: 100_000,
            scale: 0,
        }),
        limit_price: None,
        tif: TimeInForce::Ioc as i32,
        ..Default::default()
    })))
    .await
    .unwrap();

    let mut acked = false;
    let filled = loop {
        match next_body(&mut ws).await {
            Body::Ack(a) if a.request_id == "ord-1" => acked = true,
            Body::Error(e) => panic!("order error: {e:?}"),
            Body::OrderUpdate(u) if u.status == OrderStatus::Filled as i32 => break u,
            _ => {}
        }
    };
    assert!(acked);
    assert_eq!(filled.client_request_id, "ord-1");
    assert_eq!(filled.account_id, "DEMO-1");
    assert_eq!(filled.symbol, "EURUSD");
    assert_eq!(
        filled.filled_qty.unwrap().to_fixed(),
        Some(domain::Fixed::from_int(100_000))
    );
    assert!(filled.avg_price.is_some());

    // Position follows the fill.
    let pos = loop {
        if let Body::PositionUpdate(p) = next_body(&mut ws).await {
            break p;
        }
    };
    assert_eq!(
        pos.position.unwrap().net_qty.unwrap().to_fixed(),
        Some(domain::Fixed::from_int(100_000))
    );

    demo.shutdown().await;
}
