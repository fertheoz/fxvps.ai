use super::*;
use money::{px, qty};
use proptest::prelude::*;

const USD: Currency = Currency::USD;
const EUR: Currency = Currency::EUR;
const JPY: Currency = Currency::JPY;

fn book() -> QuoteBook {
    let mut b = QuoteBook::new();
    b.set(
        "EURUSD",
        Quote {
            bid: px("1.09990"),
            ask: px("1.10010"),
        },
    );
    b.set(
        "USDJPY",
        Quote {
            bid: px("149.99"),
            ask: px("150.01"),
        },
    );
    b
}

fn specs() -> BTreeMap<String, SymbolSpec> {
    let mut m = BTreeMap::new();
    m.insert("EURUSD".into(), SymbolSpec::fx("EURUSD", EUR, USD, 5));
    m.insert("USDJPY".into(), SymbolSpec::fx("USDJPY", USD, JPY, 3));
    m
}

fn client_quotes(b: &QuoteBook) -> BTreeMap<String, Quote> {
    ["EURUSD", "USDJPY"]
        .iter()
        .map(|s| (s.to_string(), b.get(s).unwrap()))
        .collect()
}

fn usd(s: &str) -> Money {
    Money::parse(s, USD).unwrap()
}

#[test]
fn esma_caps() {
    let mut g = GroupConfig::retail("r", USD, Routing::BBook);
    g.leverage = 500;
    let mut s = SymbolSpec::fx("EURUSD", EUR, USD, 5);
    assert_eq!(g.effective_leverage(&s), 30);
    s.asset_class = AssetClass::Crypto;
    assert_eq!(g.effective_leverage(&s), 2);
    s.asset_class = AssetClass::Gold;
    assert_eq!(g.effective_leverage(&s), 20);
    g.esma = Some(EsmaPreset::Professional);
    assert_eq!(g.effective_leverage(&s), 500);
    g.esma = None;
    g.leverage = 10;
    assert_eq!(g.effective_leverage(&s), 10);
}

#[test]
fn conversion_paths() {
    let b = book();
    // direct
    let r = b
        .convert(Money::parse("100", EUR).unwrap(), USD, Rounding::HalfEven)
        .unwrap();
    assert_eq!(r, usd("110"));
    // inverse
    let r = b.convert(usd("110"), EUR, Rounding::HalfEven).unwrap();
    assert_eq!(r, Money::parse("100", EUR).unwrap());
    // cross via USD: EUR -> JPY = 100 * 1.1 * 150
    let r = b
        .convert(Money::parse("100", EUR).unwrap(), JPY, Rounding::HalfEven)
        .unwrap();
    assert_eq!(r, Money::parse("16500", JPY).unwrap());
    assert!(matches!(
        b.convert(
            Money::parse("1", Currency::GBP).unwrap(),
            USD,
            Rounding::Down
        ),
        Err(RiskError::NoConversion(..))
    ));
}

#[test]
fn margin_fx_and_hedged() {
    let b = book();
    let g = GroupConfig::retail("r", USD, Routing::BBook);
    let s = &specs()["EURUSD"];
    // 1 lot EURUSD @1:30 = 100000 EUR / 30 = 3333.34 EUR -> * 1.1 USD
    let m = symbol_margin(&g, s, qty("1"), Qty::ZERO, &b).unwrap();
    assert_eq!(m, usd("3666.68"));
    // fully hedged 1/1 at 50% per leg == one full position
    let h = symbol_margin(&g, s, qty("1"), qty("1"), &b).unwrap();
    assert_eq!(h, m);
    let mut n = g.clone();
    n.margin_mode = MarginMode::Netting;
    assert!(symbol_margin(&n, s, qty("1"), qty("1"), &b)
        .unwrap()
        .is_zero());
    // margin currency = quote (e.g. index) uses price
    let mut idx = s.clone();
    idx.margin_currency = USD;
    let mi = symbol_margin(&g, &idx, qty("1"), Qty::ZERO, &b).unwrap();
    assert_eq!(mi, usd("3666.67"));
    // JPY quote, USD base: margin 100000/30 USD
    let mj = symbol_margin(&g, &specs()["USDJPY"], qty("1"), Qty::ZERO, &b).unwrap();
    assert_eq!(mj, usd("3333.34"));
}

#[test]
fn pnl_cross_currency() {
    let b = book();
    let s = &specs()["USDJPY"];
    // buy 1 lot at 150.00, close 151.00 -> 100000 JPY -> /150 USD
    let p = pnl_money(s, Side::Buy, px("150"), px("151"), qty("1"), USD, &b).unwrap();
    assert_eq!(p, usd("666.67"));
    let p = pnl_money(
        &specs()["EURUSD"],
        Side::Sell,
        px("1.1"),
        px("1.1010"),
        qty("0.5"),
        USD,
        &b,
    )
    .unwrap();
    assert_eq!(p, usd("-50"));
}

#[test]
fn pre_trade_rejections() {
    let b = book();
    let cq = client_quotes(&b);
    let sp = specs();
    let mut g = GroupConfig::retail("r", USD, Routing::BBook);
    g.allowed_symbols = Some(["EURUSD".to_string()].into_iter().collect());
    let ord = |sym, v: &str| OrderIntent {
        symbol: sym,
        side: Side::Buy,
        volume: qty(v),
        requested_price: None,
        price: None,
    };
    let chk = |g: &GroupConfig, bal: &str, pos: &[PositionView], o: &OrderIntent| {
        pre_trade_check(g, &sp, usd(bal), pos, o, &b, &cq, false)
    };
    assert_eq!(
        chk(&g, "10000", &[], &ord("USDJPY", "1")),
        Err(RiskReject::SymbolNotAllowed)
    );
    assert_eq!(
        chk(&g, "10000", &[], &ord("EURUSD", "0.001")),
        Err(RiskReject::InvalidVolume)
    );
    assert_eq!(
        chk(&g, "10000", &[], &ord("EURUSD", "0.015")),
        Err(RiskReject::InvalidVolume)
    );
    assert_eq!(
        chk(&g, "10000", &[], &ord("EURUSD", "60")),
        Err(RiskReject::VolumeLimit)
    );
    assert_eq!(
        chk(&g, "1000", &[], &ord("EURUSD", "1")),
        Err(RiskReject::InsufficientMargin)
    );
    let ok = chk(&g, "10000", &[], &ord("EURUSD", "1")).unwrap();
    assert_eq!(ok.margin, usd("3666.68"));
    let mut o = ord("EURUSD", "1");
    o.requested_price = Some(px("1.1020"));
    assert_eq!(chk(&g, "10000", &[], &o), Err(RiskReject::PriceOffQuote));
    o.requested_price = Some(px("1.1005"));
    assert!(chk(&g, "10000", &[], &o).is_ok());
    g.max_net_volume = qty("1.5");
    let pos = vec![PositionView {
        symbol: "EURUSD".into(),
        side: Side::Buy,
        volume: qty("1"),
        open_price: px("1.1"),
    }];
    assert_eq!(
        chk(&g, "100000", &pos, &ord("EURUSD", "1")),
        Err(RiskReject::ExposureLimit)
    );
    // hedge does not increase margin: allowed even with tiny free margin
    let mut sell = ord("EURUSD", "1");
    sell.side = Side::Sell;
    assert!(chk(&g, "3700", &pos, &sell).is_ok());
}

#[test]
fn margin_call_stop_out_levels() {
    let g = GroupConfig::retail("r", USD, Routing::BBook);
    let r = AccountRisk::new(usd("1000"), usd("-500"), usd("1000")).unwrap();
    assert_eq!(r.margin_level_x100(), Some(5000));
    assert!(r.is_margin_call(&g));
    assert!(r.is_stop_out(&g));
    let r = AccountRisk::new(usd("1000"), usd("-100"), usd("1000")).unwrap();
    assert!(r.is_margin_call(&g) && !r.is_stop_out(&g));
    let r = AccountRisk::new(usd("1000"), usd("0"), usd("0")).unwrap();
    assert!(!r.is_margin_call(&g) && r.margin_level_x100().is_none());
}

#[test]
fn liquidation_and_nbp() {
    let items = [
        (1u64, usd("-10")),
        (2, usd("-50")),
        (3, usd("20")),
        (4, usd("-50")),
    ];
    assert_eq!(liquidation_order(&items), vec![2, 4, 1, 3]);
    let g = GroupConfig::retail("r", USD, Routing::BBook);
    assert_eq!(
        negative_balance_compensation(&g, usd("-12.5")),
        Some(usd("12.5"))
    );
    assert_eq!(negative_balance_compensation(&g, usd("1")), None);
}

proptest! {
    #[test]
    fn margin_monotonic_and_subadditive(l in 0i64..10_000, s in 0i64..10_000, d in 1i64..10_000, hb in 0u32..=10_000) {
        let b = book();
        let mut g = GroupConfig::retail("r", USD, Routing::BBook);
        g.hedged_margin_bps = hb;
        let sp = &specs()["EURUSD"];
        let lot = |n: i64| Qty::from_raw(n * 1_000_000);
        let (big, small) = (l.max(s), l.min(s));
        let m0 = symbol_margin(&g, sp, lot(big), lot(small), &b).unwrap();
        let m1 = symbol_margin(&g, sp, lot(big + d), lot(small), &b).unwrap();
        prop_assert!(m0.minor >= 0);
        prop_assert!(m1.minor >= m0.minor, "growing unhedged side must not reduce margin");
        let sep = symbol_margin(&g, sp, lot(l), Qty::ZERO, &b).unwrap().minor
            + symbol_margin(&g, sp, Qty::ZERO, lot(s), &b).unwrap().minor;
        prop_assert!(symbol_margin(&g, sp, lot(l), lot(s), &b).unwrap().minor <= sep + 1);
        let mut n = g.clone();
        n.margin_mode = MarginMode::Netting;
        prop_assert!(symbol_margin(&n, sp, lot(l), lot(s), &b).unwrap().minor
            <= symbol_margin(&g, sp, lot(l), lot(s), &b).unwrap().minor);
    }

    #[test]
    fn leverage_monotonic(lev in 1u32..500) {
        let b = book();
        let mut g = GroupConfig::retail("r", USD, Routing::BBook);
        g.esma = None;
        g.leverage = lev;
        let sp = &specs()["EURUSD"];
        let a = symbol_margin(&g, sp, qty("1"), Qty::ZERO, &b).unwrap();
        g.leverage = lev + 1;
        let c = symbol_margin(&g, sp, qty("1"), Qty::ZERO, &b).unwrap();
        prop_assert!(c.minor <= a.minor);
    }
}
