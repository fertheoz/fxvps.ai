//! TLS towards the LP: lp-simulator behind a TLS-terminating proxy (test PKI in
//! tests/data), gateway with `[md.tls]`/`[trade.tls]` logs on to both sessions.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use domain::{Fixed, Instrument};
use fix_gateway::{GatewayConfig, GatewayEvent, SessionEndpoint, SessionKind, TlsEndpoint};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::{
    self,
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
};
use tokio_rustls::TlsAcceptor;

fn data(f: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(f)
}

/// Accepts TLS on a random port and pipes each connection to `upstream`.
async fn tls_proxy(upstream: std::net::SocketAddr) -> std::net::SocketAddr {
    let certs = CertificateDer::pem_file_iter(data("lp.crt"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_file(data("lp.key")).unwrap();
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = l.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(sock).await else {
                    return;
                };
                let Ok(mut up) = TcpStream::connect(upstream).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut tls, &mut up).await;
            });
        }
    });
    addr
}

fn config(md: String, trade: String, server_name: &str) -> GatewayConfig {
    let ep = |addr: String, sender: &str, target: &str| SessionEndpoint {
        addr,
        sender_comp_id: sender.into(),
        target_comp_id: target.into(),
        username: Some("demo".into()),
        password: Some("demo".into()),
        reset_on_logon: true,
        tls: Some(TlsEndpoint {
            server_name: Some(server_name.into()),
            ca_file: Some(data("ca.crt")),
            ..Default::default()
        }),
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
        md: ep(md, "FXVPS-MD", "LMXBDM"),
        trade: ep(trade, "FXVPS-TRD", "LMXBD"),
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

#[tokio::test]
async fn logs_on_over_tls() {
    let sim = lp_simulator::start(lp_simulator::SimConfig::for_tests())
        .await
        .unwrap();
    let md = tls_proxy(sim.md_addr).await;
    let trade = tls_proxy(sim.trade_addr).await;
    let gw = fix_gateway::start(config(md.to_string(), trade.to_string(), "lp.test")).unwrap();
    let mut rx = gw.subscribe();
    let (mut m, mut t) = (false, false);
    tokio::time::timeout(Duration::from_secs(10), async {
        while !(m && t) {
            if let Ok(GatewayEvent::SessionUp { session }) = rx.recv().await {
                match session {
                    SessionKind::MarketData => m = true,
                    SessionKind::Trading => t = true,
                }
            }
        }
    })
    .await
    .expect("both sessions up over TLS");
    gw.shutdown().await;
}

#[tokio::test]
async fn rejects_wrong_server_name() {
    let sim = lp_simulator::start(lp_simulator::SimConfig::for_tests())
        .await
        .unwrap();
    let md = tls_proxy(sim.md_addr).await;
    let trade = tls_proxy(sim.trade_addr).await;
    let gw = fix_gateway::start(config(md.to_string(), trade.to_string(), "evil.test")).unwrap();
    let mut rx = gw.subscribe();
    let reason = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(GatewayEvent::SessionDown { reason, .. }) = rx.recv().await {
                return reason;
            }
        }
    })
    .await
    .unwrap();
    assert!(reason.contains("connect"), "{reason}");
    assert!(gw.status().iter().all(|s| !s.logged_on));
    gw.shutdown().await;
}
