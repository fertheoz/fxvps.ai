//! core-engine + FixLpRouter + fix-gateway + lp-simulator, driven through
//! the `CoreApi`; includes a restart from the journal mid-session.

use std::sync::Arc;
use std::time::Duration;

use core_engine::api::{CoreApi, CoreEvent, OrderStatus, PlaceOrderRequest};
use core_engine::stack::{CoreStack, Seed, StackConfig};
use core_engine::{recover, Settings};
use domain::Fixed;
use fix_gateway::GatewayConfig;
use lp_simulator::SimConfig;
use money::{Currency, Money};
use tokio::sync::broadcast;

const SIM_TOML: &str = include_str!("../../lp-simulator/config/default.toml");
const GW_TOML: &str = include_str!("../../fix-gateway/config/default.toml");

async fn sim() -> lp_simulator::SimHandle {
    let mut c = SimConfig::from_toml(SIM_TOML).unwrap();
    c.md.listen = "127.0.0.1:0".into();
    c.trade.listen = "127.0.0.1:0".into();
    c.tick_interval_ms = 20;
    for i in &mut c.instruments {
        i.volatility_ticks = 1;
    }
    lp_simulator::start(c).await.unwrap()
}

fn stack_cfg(sim: &lp_simulator::SimHandle, dir: &std::path::Path) -> StackConfig {
    let mut gw = GatewayConfig::from_toml(GW_TOML).unwrap();
    gw.lp = "SIM".into();
    gw.md.addr = sim.md_addr.to_string();
    gw.trade.addr = sim.trade_addr.to_string();
    gw.reconnect_delay_ms = 100;
    gw.store_dir = None;
    gw.nats = None;
    let seed = Seed::demo(&gw, 3, Money::parse("10000", Currency::USD).unwrap());
    let mut c = StackConfig::new(gw, dir);
    c.snapshot_every = 7; // snapshot + journal tail on restart
    c.seed = Some(seed);
    c
}

async fn wait_for<T>(
    rx: &mut broadcast::Receiver<Arc<CoreEvent>>,
    mut f: impl FnMut(&CoreEvent) -> Option<T>,
) -> T {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Some(t) = f(&ev) {
                        return t;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .expect("timeout waiting for core event")
}

fn market(account: &str, id: &str, side: domain::Side, units: i64) -> PlaceOrderRequest {
    PlaceOrderRequest::market(account, id, "EURUSD", side, Fixed::from_int(units))
}

async fn place_and_fill(core: &dyn CoreApi, req: PlaceOrderRequest) -> Fixed {
    let mut rx = core.subscribe();
    let id = req.client_order_id.clone();
    core.place_order(req).await.unwrap();
    wait_for(&mut rx, |e| match e {
        CoreEvent::Order(o) if o.client_order_id == id && o.status == OrderStatus::Filled => {
            o.avg_price
        }
        _ => None,
    })
    .await
}

async fn wait_quote(core: &dyn CoreApi) {
    let mut rx = core.subscribe();
    wait_for(&mut rx, |e| {
        matches!(e, CoreEvent::Quote(q) if q.symbol == "EURUSD").then_some(())
    })
    .await;
}

#[tokio::test]
async fn fix_routed_fill_restart_and_close() {
    let sim = sim().await;
    let dir = tempfile::tempdir().unwrap();
    let stack = CoreStack::start(stack_cfg(&sim, dir.path())).await.unwrap();
    let core = stack.core.clone();
    wait_quote(&*core).await;

    let snap0 = core.account_snapshot("DEMO-1").await.unwrap();
    assert_eq!(snap0.balance, Fixed::from_int(10_000));
    assert_eq!(snap0.group, "demo-retail");

    // Insufficient margin: 5 lots EURUSD at 1:30 needs ~18k USD.
    let err = core
        .place_order(market("DEMO-1", "big", domain::Side::Buy, 500_000))
        .await
        .unwrap_err();
    assert_eq!(
        err.code,
        core_engine::api::CoreErrorCode::InsufficientMargin
    );

    let open = place_and_fill(&*core, market("DEMO-1", "o1", domain::Side::Buy, 100_000)).await;
    let snap1 = core.account_snapshot("DEMO-1").await.unwrap();
    assert_eq!(snap1.positions.len(), 1);
    assert_eq!(snap1.positions[0].net_qty, Fixed::from_int(100_000));
    assert_eq!(snap1.positions[0].avg_price, open);
    assert!(snap1.margin.is_positive());
    // opening commission 3.50
    assert_eq!(snap1.balance, "9996.5".parse().unwrap());

    // Restart core-engine from snapshot + journal.
    let cfg = stack_cfg(&sim, dir.path());
    stack.shutdown().await;
    let mut s = Settings::new(dir.path());
    s.simulate_lp = false;
    let (from_snapshot, _) = recover(&s).unwrap();
    // full replay of the journal alone yields the same state
    let full = tempfile::tempdir().unwrap();
    std::fs::copy(
        dir.path().join("journal.jsonl"),
        full.path().join("journal.jsonl"),
    )
    .unwrap();
    let (from_journal, _) = recover(&Settings::new(full.path())).unwrap();
    assert_eq!(from_snapshot.state_digest(), from_journal.state_digest());
    assert_eq!(from_journal.positions_of(1).len(), 1);

    let stack = CoreStack::start(cfg).await.unwrap();
    let core = stack.core.clone();
    let snap2 = core.account_snapshot("DEMO-1").await.unwrap();
    assert_eq!(snap2.balance, snap1.balance);
    assert_eq!(snap2.positions[0].net_qty, Fixed::from_int(100_000));
    wait_quote(&*core).await;

    // Continue: close through the LP; balance moves by realized P&L - commission.
    let close = place_and_fill(&*core, market("DEMO-1", "c1", domain::Side::Sell, 100_000)).await;
    let snap3 = core.account_snapshot("DEMO-1").await.unwrap();
    assert!(snap3.positions.is_empty());
    let pnl = Fixed::from_raw((close.raw() - open.raw()) * 100_000);
    let commission: Fixed = "3.5".parse().unwrap();
    assert_eq!(snap3.balance - snap2.balance, pnl - commission);
    assert_eq!(snap3.margin, Fixed::ZERO);

    stack.shutdown().await;
    sim.shutdown().await;
}
