//! Demo seed for development and the live e2e test (`CORE_SEED=1`): a few
//! symbols, groups, accounts, deposits and open positions. Applied as
//! ordinary journaled engine commands, only when the engine has no accounts.

use crate::EngineHandle;
use money::{px, qty, Currency, Money};
use oms::{Command, NewOrder};
use risk::{AssetClass, EsmaPreset, GroupConfig, Routing, Side, SymbolSpec};

pub fn commands() -> Vec<Command> {
    let mut gold = SymbolSpec::fx("XAUUSD", Currency::XAU, Currency::USD, 2);
    gold.contract_size = 100;
    gold.asset_class = AssetClass::Gold;
    let mut jpy = SymbolSpec::fx("USDJPY", Currency::USD, Currency::JPY, 3);
    jpy.asset_class = AssetClass::MajorFx;
    let mut pro = GroupConfig::retail("pro-b", Currency::USD, Routing::BBook);
    pro.esma = Some(EsmaPreset::Professional);
    pro.leverage = 200;
    let mut cmds = vec![
        Command::AddSymbol(SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5)),
        Command::AddSymbol(SymbolSpec::fx("GBPUSD", Currency::GBP, Currency::USD, 5)),
        Command::AddSymbol(jpy),
        Command::AddSymbol(gold),
        Command::SetGroup(GroupConfig::retail(
            "retail-a",
            Currency::USD,
            Routing::ABook,
        )),
        Command::SetGroup(GroupConfig::retail(
            "retail-b",
            Currency::USD,
            Routing::BBook,
        )),
        Command::SetGroup(pro),
        Command::Quote {
            symbol: "EURUSD".into(),
            bid: px("1.08520"),
            ask: px("1.08528"),
        },
        Command::Quote {
            symbol: "GBPUSD".into(),
            bid: px("1.27110"),
            ask: px("1.27122"),
        },
        Command::Quote {
            symbol: "USDJPY".into(),
            bid: px("149.210"),
            ask: px("149.222"),
        },
        Command::Quote {
            symbol: "XAUUSD".into(),
            bid: px("2350.10"),
            ask: px("2350.45"),
        },
    ];
    let accounts: [(u64, &str, &str); 6] = [
        (1001, "retail-a", "25000"),
        (1002, "retail-a", "5000"),
        (1003, "retail-b", "12000"),
        (1004, "retail-b", "800"),
        (1005, "pro-b", "150000"),
        (1006, "retail-a", "0"),
    ];
    for (id, g, dep) in accounts {
        cmds.push(Command::OpenAccount {
            account: id,
            group: g.into(),
        });
        if dep != "0" {
            cmds.push(Command::Deposit {
                account: id,
                amount: Money::parse(dep, Currency::USD).expect("seed amount"),
                key: "seed".into(),
            });
        }
    }
    let orders: [(u64, &str, Side, &str); 7] = [
        (1001, "EURUSD", Side::Buy, "1.5"),
        (1001, "XAUUSD", Side::Sell, "0.5"),
        (1002, "GBPUSD", Side::Buy, "0.8"),
        (1003, "EURUSD", Side::Sell, "2"),
        (1003, "USDJPY", Side::Buy, "1"),
        (1004, "EURUSD", Side::Buy, "0.1"),
        (1005, "EURUSD", Side::Buy, "10"),
    ];
    for (i, (a, s, side, v)) in orders.into_iter().enumerate() {
        cmds.push(Command::PlaceOrder(NewOrder::market(
            a,
            &format!("seed-{i}"),
            s,
            side,
            qty(v),
        )));
    }
    cmds
}

/// Seeds the engine when it has no accounts yet. Returns whether it did.
pub async fn seed_if_empty(h: &EngineHandle) -> Result<bool, String> {
    let empty = h
        .query(|e| serde_json::json!(e.accounts().next().is_none()))
        .await?;
    if empty != serde_json::Value::Bool(true) {
        return Ok(false);
    }
    for c in commands() {
        h.command(c).await?;
    }
    Ok(true)
}
