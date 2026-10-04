//! End-to-end: loadgen against an in-process `client-gateway --demo`.

use std::time::Duration;

use client_gateway::auth::Authenticator;
use client_gateway::demo::Demo;
use client_gateway::ClientGatewayConfig;
use loadgen::{dev_tokens, run, LoadConfig};

const KEY: &str = "loadgen-test-key";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn small_run_against_demo() {
    let gw = ClientGatewayConfig {
        max_quote_hz: 20,
        ..Default::default()
    };
    let demo = Demo::start(gw, Authenticator::hs256(KEY.as_bytes()), Some(20))
        .await
        .unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/ws", l.local_addr().unwrap());
    tokio::spawn(client_gateway::serve(demo.hub.clone(), l));

    let accounts = vec!["DEMO-1".to_string(), "DEMO-2".to_string()];
    let cfg = LoadConfig {
        url,
        clients: 4,
        duration: Duration::from_secs(4),
        orders_per_sec: 1.0,
        tokens: dev_tokens(KEY, &accounts, 600),
        accounts,
        ..Default::default()
    };
    let r = run(cfg).await;
    eprintln!("{}", r.table());
    assert_eq!(r.connected, 4, "{r:?}");
    assert!(r.quotes > 0 && r.quote_latency.count > 0, "{r:?}");
    assert!(r.orders_sent > 0 && r.acks > 0, "{r:?}");
    assert!(r.fills > 0 && r.fill_latency.count > 0, "{r:?}");
    assert!(!r.errors.contains_key("auth"), "{r:?}");

    demo.shutdown().await;
}
