//! Session hardening (finding G9): token expiry closes the connection,
//! per-subject / per-IP connection caps, Origin allow list, metrics listener.

use std::time::Duration;

use client_gateway::auth::{issue_hs256, Authenticator};
use client_gateway::{ClientGatewayConfig, Hub};
use client_proto::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const KEY: &[u8] = b"test-only-key";

async fn server(cfg: ClientGatewayConfig) -> String {
    let hub = Hub::new(cfg, Authenticator::hs256(KEY), Vec::<String>::new(), None);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(client_gateway::serve(hub, l));
    format!("{addr}")
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send(ws: &mut Ws, seq: u64, body: Body) {
    let env = Envelope::new(seq, body);
    ws.send(Message::Binary(env.to_protobuf().into()))
        .await
        .unwrap();
}

/// Next protocol body, or `Err(close code)`.
async fn next(ws: &mut Ws) -> Result<Body, Option<u16>> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("timed out");
        match m {
            Some(Ok(Message::Binary(b))) => {
                return Ok(Envelope::from_protobuf(&b).unwrap().body.unwrap())
            }
            Some(Ok(Message::Close(f))) => return Err(f.map(|f| u16::from(f.code))),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return Err(None),
        }
    }
}

async fn login(addr: &str, sub: &str, ttl: u64) -> (Ws, Result<Body, Option<u16>>) {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    send(
        &mut ws,
        1,
        Body::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            ..Default::default()
        }),
    )
    .await;
    assert!(matches!(next(&mut ws).await, Ok(Body::Hello(_))));
    send(
        &mut ws,
        2,
        Body::Auth(Auth {
            token: issue_hs256(KEY, sub, &["A1"], ttl),
        }),
    )
    .await;
    let r = next(&mut ws).await;
    (ws, r)
}

/// Waits for the close code, skipping frames.
async fn closed(ws: &mut Ws) -> Option<u16> {
    loop {
        if let Err(c) = next(ws).await {
            return c;
        }
    }
}

#[tokio::test]
async fn expired_token_closes_the_session() {
    let addr = server(ClientGatewayConfig::default()).await;
    let (mut ws, r) = login(&addr, "u1", 2).await;
    assert!(matches!(r, Ok(Body::AuthOk(_))), "{r:?}");
    assert_eq!(closed(&mut ws).await, Some(4001));
}

#[tokio::test]
async fn per_subject_and_per_ip_caps() {
    let addr = server(ClientGatewayConfig {
        max_connections_per_subject: 1,
        max_connections_per_ip: 3,
        ..ClientGatewayConfig::default()
    })
    .await;
    let (_a, r) = login(&addr, "u1", 60).await;
    assert!(matches!(r, Ok(Body::AuthOk(_))));
    let (mut b, r) = login(&addr, "u1", 60).await;
    assert!(
        matches!(r, Ok(Body::Error(ref e)) if e.code == ErrorCode::RateLimited as i32),
        "{r:?}"
    );
    assert_eq!(closed(&mut b).await, Some(4009));
    drop(b);
    // another subject is fine
    let (_c, r) = login(&addr, "u2", 60).await;
    assert!(matches!(r, Ok(Body::AuthOk(_))));
    let _d = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    // 4th connection from 127.0.0.1 (a, c, d open; b released) -> HTTP 429
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _e = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await;
    let r = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await;
    match r {
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
            assert_eq!(resp.status(), 429)
        }
        other => panic!("expected 429, got {:?}", other.map(|_| ())),
    }
}

#[tokio::test]
async fn origin_allow_list_and_separate_metrics() {
    let ml = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let maddr = ml.local_addr().unwrap();
    drop(ml);
    let addr = server(ClientGatewayConfig {
        allowed_origins: vec!["https://terminal.fxvps.test".into()],
        metrics_listen: Some(maddr.to_string()),
        ..ClientGatewayConfig::default()
    })
    .await;
    let req = |origin: &str| {
        let mut r = format!("ws://{addr}/ws").into_client_request().unwrap();
        r.headers_mut().insert("origin", origin.parse().unwrap());
        r
    };
    assert!(
        tokio_tungstenite::connect_async(req("https://terminal.fxvps.test"))
            .await
            .is_ok()
    );
    assert!(tokio_tungstenite::connect_async(req("https://evil.test"))
        .await
        .is_err());
    // no Origin (native client): allowed
    assert!(tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .is_ok());
    // /metrics is not on the public listener, only on the metrics one
    let http = reqwest::Client::new();
    let r = http
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let mut ok = false;
    for _ in 0..50 {
        if let Ok(r) = http.get(format!("http://{maddr}/metrics")).send().await {
            ok = r.status() == 200 && r.text().await.unwrap().contains("client_gw_connections");
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "metrics listener");
}
