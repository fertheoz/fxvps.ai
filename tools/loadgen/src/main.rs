//! `loadgen` CLI. See `--help`.

use std::time::Duration;

use loadgen::{dev_tokens, run, LoadConfig};

const USAGE: &str = "usage: loadgen [--url ws://127.0.0.1:8080/ws | --spawn-demo] [--clients N] \
[--duration SECS] [--symbols EURUSD,GBPUSD] [--quote-hz HZ] [--orders-per-sec F] \
[--qty UNITS] [--accounts DEMO-1,DEMO-2] [--token JWT] [--json] [--ramp-ms MS]

Token: --token, else $FXVPS_TOKEN, else HS256 tokens minted per account with
$FXVPS_JWT_HS256_SECRET or the gateway's public DEV key (local --demo only).
--spawn-demo starts client-gateway --demo in-process on 127.0.0.1:0.";

fn val<T: std::str::FromStr>(a: Option<String>, name: &str) -> Result<T, String> {
    a.and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("{name}: missing or invalid value\n{USAGE}"))
}

fn list(s: String) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let mut cfg = LoadConfig::default();
    let mut json = false;
    let mut spawn_demo = false;
    let mut token = std::env::var("FXVPS_TOKEN").ok();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--url" => cfg.url = val(args.next(), "--url")?,
            "--spawn-demo" => spawn_demo = true,
            "--clients" => cfg.clients = val(args.next(), "--clients")?,
            "--duration" => cfg.duration = Duration::from_secs_f64(val(args.next(), "--duration")?),
            "--symbols" => cfg.symbols = list(val(args.next(), "--symbols")?),
            "--quote-hz" => cfg.quote_hz = val(args.next(), "--quote-hz")?,
            "--orders-per-sec" => cfg.orders_per_sec = val(args.next(), "--orders-per-sec")?,
            "--qty" => cfg.order_qty = val(args.next(), "--qty")?,
            "--accounts" => cfg.accounts = list(val(args.next(), "--accounts")?),
            "--token" => token = Some(val(args.next(), "--token")?),
            "--ramp-ms" => cfg.ramp = Duration::from_millis(val(args.next(), "--ramp-ms")?),
            "--json" => json = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}\n{USAGE}").into()),
        }
    }
    cfg.tokens = match token {
        Some(t) => vec![t],
        None => {
            let secret = std::env::var("FXVPS_JWT_HS256_SECRET")
                .unwrap_or_else(|_| client_gateway::auth::DEV_HS256_SECRET.to_string());
            dev_tokens(&secret, &cfg.accounts, 24 * 3600)
        }
    };
    let mut demo = None;
    if spawn_demo {
        let gw = client_gateway::ClientGatewayConfig {
            max_quote_hz: 20,
            ..Default::default()
        };
        let d = client_gateway::demo::Demo::start(
            gw,
            client_gateway::auth::Authenticator::hs256(
                client_gateway::auth::DEV_HS256_SECRET.as_bytes(),
            ),
            None,
        )
        .await?;
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        cfg.url = format!("ws://{}/ws", l.local_addr()?);
        tokio::spawn(client_gateway::serve(d.hub.clone(), l));
        demo = Some(d);
    }
    let report = run(cfg).await;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report.table());
    }
    if let Some(d) = demo {
        d.shutdown().await;
    }
    Ok(())
}
