//! WebSocket-level tests against an in-process server.

use std::sync::Arc;
use std::time::Duration;

use client_gateway::auth::{issue_hs256, Authenticator};
use client_gateway::hub::ClientQuote;
use client_gateway::{ClientGatewayConfig, Hub};
use client_proto::*;
use domain::Fixed;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const KEY: &[u8] = b"test-only-key";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn server(hub: Arc<Hub>) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(client_gateway::serve(hub, l));
    format!("{addr}")
}

fn hub(cfg: ClientGatewayConfig, symbols: &[&str]) -> Arc<Hub> {
    Hub::new(
        cfg,
        Authenticator::hs256(KEY),
        symbols.iter().map(|s| s.to_string()),
        None,
    )
}

struct Client {
    ws: Ws,
    seq: u64,
}

enum Recv {
    Env(Box<Envelope>),
    Close(Option<u16>),
}

impl Client {
    async fn connect(addr: &str) -> Client {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .unwrap();
        Client { ws, seq: 0 }
    }

    async fn send(&mut self, body: Body) {
        self.seq += 1;
        let env = Envelope::new(self.seq, body);
        self.ws
            .send(Message::Binary(env.to_protobuf().into()))
            .await
            .unwrap();
    }

    async fn recv(&mut self) -> Recv {
        loop {
            let m = tokio::time::timeout(Duration::from_secs(10), self.ws.next())
                .await
                .expect("timed out waiting for frame");
            match m {
                Some(Ok(Message::Binary(b))) => {
                    return Recv::Env(Box::new(Envelope::from_protobuf(&b).unwrap()))
                }
                Some(Ok(Message::Text(t))) => {
                    return Recv::Env(Box::new(Envelope::from_json(&t).unwrap()))
                }
                Some(Ok(Message::Close(f))) => return Recv::Close(f.map(|f| u16::from(f.code))),
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => return Recv::Close(None),
            }
        }
    }

    async fn body(&mut self) -> Body {
        match self.recv().await {
            Recv::Env(e) => e.body.unwrap(),
            Recv::Close(c) => panic!("unexpected close {c:?}"),
        }
    }

    /// Next body matching `f`, skipping others (heartbeats, quotes...).
    async fn until<T>(&mut self, mut f: impl FnMut(Body) -> Option<T>) -> T {
        loop {
            if let Some(t) = f(self.body().await) {
                return t;
            }
        }
    }

    async fn login(addr: &str, accounts: &[&str], hz: u32) -> Client {
        let mut c = Client::connect(addr).await;
        c.send(Body::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "test".into(),
            max_quote_hz: hz,
        }))
        .await;
        assert!(matches!(c.body().await, Body::Hello(_)));
        c.send(Body::Auth(Auth {
            token: issue_hs256(KEY, "u1", accounts, 60),
        }))
        .await;
        let ok = c
            .until(|b| match b {
                Body::AuthOk(a) => Some(a),
                _ => None,
            })
            .await;
        assert_eq!(ok.subject, "u1");
        c
    }

    async fn subscribe(&mut self, syms: &[&str]) {
        self.send(Body::Subscribe(Subscribe {
            request_id: "sub".into(),
            symbols: syms.iter().map(|s| s.to_string()).collect(),
        }))
        .await;
        self.until(|b| matches!(b, Body::Ack(a) if a.request_id == "sub").then_some(()))
            .await;
    }
}

async fn err(c: &mut Client) -> client_proto::Error {
    c.until(|b| match b {
        Body::Error(e) => Some(e),
        _ => None,
    })
    .await
}

fn quote(sym: &str, raw_bid: i64) -> ClientQuote {
    ClientQuote {
        symbol: sym.into(),
        bid: Some(Fixed::from_raw(raw_bid)),
        ask: Some(Fixed::from_raw(raw_bid + 2)),
        bid_size: None,
        ask_size: None,
        ts_ns: domain::now_ns(),
    }
}

#[tokio::test]
async fn auth_rejects_bad_token() {
    let addr = server(hub(ClientGatewayConfig::default(), &[])).await;
    let mut c = Client::connect(&addr).await;
    c.send(Body::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        ..Default::default()
    }))
    .await;
    assert!(matches!(c.body().await, Body::Hello(_)));
    c.send(Body::Auth(Auth {
        token: issue_hs256(b"wrong-key", "u1", &["A1"], 60),
    }))
    .await;
    match c.body().await {
        Body::Error(e) => assert_eq!(e.code, ErrorCode::Unauthenticated as i32),
        b => panic!("expected error, got {b:?}"),
    }
    assert!(matches!(c.recv().await, Recv::Close(Some(4001))));
}

#[tokio::test]
async fn auth_required_before_commands_and_version_checked() {
    let addr = server(hub(ClientGatewayConfig::default(), &[])).await;
    let mut c = Client::connect(&addr).await;
    c.send(Body::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        ..Default::default()
    }))
    .await;
    c.body().await;
    c.send(Body::Subscribe(Subscribe::default())).await;
    assert!(matches!(c.body().await, Body::Error(_)));
    assert!(matches!(c.recv().await, Recv::Close(Some(4001))));

    let mut c = Client::connect(&addr).await;
    c.send(Body::Hello(Hello {
        protocol_version: 99,
        ..Default::default()
    }))
    .await;
    match c.body().await {
        Body::Error(e) => assert_eq!(e.code, ErrorCode::UnsupportedVersion as i32),
        b => panic!("{b:?}"),
    }
    assert!(matches!(c.recv().await, Recv::Close(Some(4002))));
}

#[tokio::test]
async fn json_debug_encoding_works() {
    let addr = server(hub(ClientGatewayConfig::default(), &[])).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    ws.send(Message::Text(
        r#"{"version":1,"body":{"hello":{"protocol_version":1}}}"#.into(),
    ))
    .await
    .unwrap();
    let Some(Ok(Message::Text(t))) = ws.next().await else {
        panic!("expected text frame")
    };
    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
    assert_eq!(v["body"]["hello"]["protocol_version"], 1);
}

#[tokio::test]
async fn quotes_are_conflated_to_max_hz_latest_wins() {
    let h = hub(
        ClientGatewayConfig {
            max_quote_hz: 5,
            ..Default::default()
        },
        &["EUR/USD", "GBP/USD"],
    );
    let addr = server(h.clone()).await;
    // Client asks for 50 Hz; server caps at 5.
    let mut c = Client::login(&addr, &["A1"], 50).await;
    c.subscribe(&["EURUSD"]).await;

    let pub_hub = h.clone();
    let producer = tokio::spawn(async move {
        for i in 1..=1000i64 {
            pub_hub.publish_client_quote(quote("EURUSD", 100_000_000 + i));
            pub_hub.publish_client_quote(quote("GBPUSD", i)); // not subscribed
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    let start = tokio::time::Instant::now();
    let mut batches = 0;
    let mut total = 0;
    let mut last_bid = 0;
    loop {
        let b = c.body().await;
        if let Body::QuoteBatch(qb) = b {
            batches += 1;
            for q in &qb.quotes {
                assert_eq!(q.symbol, "EURUSD");
                total += 1;
                last_bid = q.bid.unwrap().value;
            }
            assert!(qb.quotes.len() <= 1, "one quote per symbol per batch");
            if last_bid == 100_001_000 {
                break;
            }
        }
    }
    let secs = start.elapsed().as_secs_f64();
    producer.await.unwrap();
    // 1000 upstream ticks collapse into ~5 batches/s.
    assert!(total < 100, "received {total} quotes");
    assert!(
        f64::from(batches) <= secs * 5.0 + 2.0,
        "{batches} batches in {secs:.2}s"
    );
    assert!(h.metrics.quotes_conflated.get() > 500);
}

#[tokio::test]
async fn slow_consumer_is_dropped_with_close_code() {
    let syms: Vec<String> = (0..400).map(|i| format!("S{i:03}/X")).collect();
    let h = Hub::new(
        ClientGatewayConfig {
            max_quote_hz: 1000,
            queue_capacity: 4,
            heartbeat_secs: 3600,
            ..Default::default()
        },
        Authenticator::hs256(KEY),
        syms.clone(),
        None,
    );
    let addr = server(h.clone()).await;
    let mut c = Client::login(&addr, &["A1"], 0).await;
    let client_syms: Vec<String> = syms.iter().map(|s| client_symbol(s)).collect();
    let refs: Vec<&str> = client_syms.iter().map(|s| s.as_str()).collect();
    c.subscribe(&refs).await;

    // Flood while the client does not read.
    let pub_hub = h.clone();
    let producer = tokio::spawn(async move {
        let mut i = 0i64;
        while pub_hub.metrics.slow_consumer_drops.get() == 0 {
            i += 1;
            for s in &client_syms {
                pub_hub.publish_client_quote(quote(s, i));
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    tokio::time::timeout(Duration::from_secs(30), producer)
        .await
        .expect("slow consumer never detected")
        .unwrap();
    // Draining the socket ends in a 4008 close frame.
    let code = loop {
        match c.recv().await {
            Recv::Env(_) => continue,
            Recv::Close(code) => break code,
        }
    };
    assert_eq!(code, Some(4008));
    assert_eq!(h.metrics.slow_consumer_drops.get(), 1);
}

#[tokio::test]
async fn order_commands_are_authorized_and_rate_limited() {
    let h = hub(
        ClientGatewayConfig {
            orders_per_second: 1,
            order_burst: 2,
            ..Default::default()
        },
        &["EUR/USD"],
    );
    let addr = server(h.clone()).await;
    let mut c = Client::login(&addr, &["A1"], 0).await;
    let place = |id: &str, acc: &str| {
        Body::PlaceOrder(PlaceOrder {
            request_id: id.into(),
            account_id: acc.into(),
            symbol: "EURUSD".into(),
            side: Side::Buy as i32,
            order_type: OrderType::Market as i32,
            qty: Some(Decimal {
                value: 1000,
                scale: 0,
            }),
            ..Default::default()
        })
    };
    // Not my account.
    c.send(place("x", "OTHER")).await;
    assert_eq!(err(&mut c).await.code, ErrorCode::Forbidden as i32);
    // Burst of 2 passes the limiter (no trading path here -> UNAVAILABLE), then limited.
    for id in ["o1", "o2"] {
        c.send(place(id, "A1")).await;
        let e = err(&mut c).await;
        assert_eq!(
            (e.request_id.as_str(), e.code),
            (id, ErrorCode::Unavailable as i32)
        );
    }
    c.send(place("o3", "A1")).await;
    let e = err(&mut c).await;
    assert_eq!(e.request_id, "o3");
    assert_eq!(e.code, ErrorCode::RateLimited as i32);
    assert_eq!(h.metrics.orders_rate_limited.get(), 1);
}

#[tokio::test]
async fn candles_history_and_http_endpoints() {
    let h = hub(ClientGatewayConfig::default(), &["EUR/USD"]);
    let addr = server(h.clone()).await;
    for i in 0..10 {
        h.publish_client_quote(quote("EURUSD", 108_500_000 + i * 10));
    }
    let mut c = Client::login(&addr, &["A1"], 0).await;
    c.send(Body::CandleRequest(CandleRequest {
        request_id: "c".into(),
        symbol: "EURUSD".into(),
        timeframe: Timeframe::M1 as i32,
        ..Default::default()
    }))
    .await;
    let r = c
        .until(|b| match b {
            Body::CandleResponse(r) => Some(r),
            _ => None,
        })
        .await;
    let ticks: u64 = r.candles.iter().map(|c| c.ticks).sum();
    assert_eq!(ticks, 10);
    assert_eq!(
        r.candles[0].open.unwrap().to_fixed(),
        Some(Fixed::from_raw(108_500_001))
    );

    let get = |path: &'static str| {
        let addr = addr.clone();
        async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut s = TcpStream::connect(&addr).await.unwrap();
            s.write_all(
                format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            out
        }
    };
    assert!(get("/healthz").await.starts_with("HTTP/1.1 200"));
    let m = get("/metrics").await;
    assert!(m.contains("client_gw_connections 1"), "{m}");
    drop(c);
}
