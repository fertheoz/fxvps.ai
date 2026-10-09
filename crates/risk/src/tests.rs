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

#[test]
fn toxicity_score_needs_evidence_and_weighs_scalping() {
    let mut f = FlowStats::default();
    for _ in 0..4 {
        f.record_close(5, 100, -100);
    }
    assert_eq!(f.toxicity(), 0, "fewer than 5 trades");
    f.record_close(5, 100, -100);
    // 5/5 short holds, 100 % wins, no requested prices
    assert_eq!(f.toxicity(), 75);
    f.record_fill(10);
    assert_eq!(f.toxicity(), 100);
    let mut calm = FlowStats::default();
    for i in 0..10 {
        calm.record_close(3_600, if i % 2 == 0 { 50 } else { -50 }, 0);
    }
    assert_eq!(calm.toxicity(), 0);
}

#[test]
fn swap_modes_weekday_and_multiplier() {
    let mut eu = SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5);
    eu.swap_long = px("-7");
    eu.swap_short = px("2");
    // money mode: -7 USD per lot per day × 2 lots × 50 % = -7
    assert_eq!(
        swap_scaled(&eu, Side::Buy, qty("2"), 50),
        px("-7").raw() as i128
    );
    assert_eq!(
        swap_scaled(&eu, Side::Sell, qty("1"), 100),
        px("2").raw() as i128
    );
    // points mode: -7 points × 0.00001 × 100 000 × 1 lot = -7 USD
    eu.swap_mode = SwapMode::Points;
    assert_eq!(
        swap_scaled(&eu, Side::Buy, qty("1"), 100),
        px("-7").raw() as i128
    );
    // 1970-01-01 Thursday; +2 days Saturday; +3 Sunday
    assert_eq!(weekday_utc(0), 4);
    assert_eq!(weekday_utc(2 * 86_400_000_000_000), 6);
    assert_eq!(weekday_utc(3 * 86_400_000_000_000), 0);
    assert!(SwapConfig {
        rollover_hour_utc: 24,
        ..SwapConfig::default()
    }
    .validate()
    .is_err());
}

#[test]
fn sessions_and_holidays() {
    let mut eu = SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5);
    assert!(eu.is_open_at(0));
    // Thursday 1970-01-01: open 08:00-17:00 only
    eu.sessions = vec![TradingSession {
        day: 4,
        open_min: 480,
        close_min: 1020,
    }];
    assert!(!eu.is_open_at(7 * 3_600_000_000_000));
    assert!(eu.is_open_at(9 * 3_600_000_000_000));
    assert!(!eu.is_open_at(17 * 3_600_000_000_000));
    let cal = TradingCalendar {
        holidays: vec!["1970-01-02".into()],
    };
    assert!(cal.validate().is_ok());
    assert!(!cal.is_holiday(0));
    assert!(cal.is_holiday(86_400_000_000_000 + 5));
    assert_eq!(day_from_iso("2026-10-06"), Some(20732));
    assert!(TradingCalendar {
        holidays: vec!["2026-13-01".into()]
    }
    .validate()
    .is_err());
}

#[test]
fn weekend_window_boundaries() {
    use crate::in_weekend_window;
    let at = |y_m_d_h: u64| y_m_d_h * 1_000_000_000;
    // 2026-10-09 is a Friday
    let fri = 1_791_504_000u64; // 2026-10-09 00:00:00 UTC
    assert!(!in_weekend_window(at(fri + 19 * 3600 + 3599)));
    assert!(in_weekend_window(at(fri + 20 * 3600)));
    assert!(in_weekend_window(at(fri + 86_400 + 12 * 3600))); // Saturday noon
    assert!(in_weekend_window(at(fri + 2 * 86_400 + 21 * 3600 + 3599))); // Sunday 21:59
    assert!(!in_weekend_window(at(fri + 2 * 86_400 + 22 * 3600))); // Sunday 22:00 open
    assert!(!in_weekend_window(at(fri + 3 * 86_400 + 10 * 3600))); // Monday
}

#[test]
fn leverage_at_takes_the_lowest_active_cap() {
    let mut g = GroupConfig::retail("g", USD, Routing::BBook);
    g.leverage = 500;
    g.weekend_leverage = Some(100);
    g.leverage_windows.push(crate::LeverageWindow {
        from_ns: 10,
        to_ns: 20,
        leverage: 50,
    });
    let monday = (1_791_504_000u64 + 3 * 86_400) * 1_000_000_000;
    let saturday = (1_791_504_000u64 + 86_400) * 1_000_000_000;
    assert_eq!(g.leverage_at(monday), 500);
    assert_eq!(g.leverage_at(saturday), 100);
    assert_eq!(g.leverage_at(15), 50);
    assert_eq!(g.leverage_at(20), 500, "window end is exclusive");
    assert!(matches!(g.at(monday), std::borrow::Cow::Borrowed(_)));
    assert_eq!(g.at(saturday).leverage, 100);
}

#[test]
fn leverage_tiers_are_progressive() {
    use crate::{tiered_margin, LeverageTier};
    let sc = money::SCALE as i128;
    let tiers = [
        LeverageTier {
            from: 100_000,
            leverage: 50,
        },
        LeverageTier {
            from: 500_000,
            leverage: 10,
        },
    ];
    // 50k notional at 1:100 -> 500
    assert_eq!(tiered_margin(50_000 * sc, 100, &tiers), 500 * sc);
    // 300k: 100k/100 + 200k/50 = 1000 + 4000
    assert_eq!(tiered_margin(300_000 * sc, 100, &tiers), 5_000 * sc);
    // 1M: 1000 + 400k/50 (8000) + 500k/10 (50000)
    assert_eq!(tiered_margin(1_000_000 * sc, 100, &tiers), 59_000 * sc);
    // a tier above the group leverage never lowers margin
    assert_eq!(
        tiered_margin(
            300_000 * sc,
            30,
            &[LeverageTier {
                from: 100_000,
                leverage: 200
            }]
        ),
        10_000 * sc
    );
    assert_eq!(tiered_margin(300_000 * sc, 100, &[]), 3_000 * sc);
}

#[test]
fn markout_weighs_in_once_measured() {
    let mut f = FlowStats::default();
    for _ in 0..5 {
        f.record_close(600, -10, 10); // long holds, losing: not toxic by the old signals
    }
    assert_eq!(f.toxicity(), 0);
    for _ in 0..5 {
        f.record_markout(1, 4); // but the price keeps running their way after fills
    }
    assert_eq!(f.avg_markout(1), Some(4.0));
    assert_eq!(f.toxicity(), 25);
}

#[test]
fn markout_never_lowers_the_score() {
    // scalper: all short holds, all wins, ≥ 5 points of price improvement
    let mut f = FlowStats::default();
    for _ in 0..10 {
        f.record_close(10, 100, -100);
        f.record_fill(6);
    }
    assert_eq!(f.toxicity(), 100);
    // flat (or adverse) markout once measured: the score must not drop, or a
    // `minToxicity: 80` routing rule would silently stop matching this flow
    for _ in 0..5 {
        f.record_markout(1, -2);
    }
    assert_eq!(f.toxicity(), 100);
    // short holds and wins without improvement: 45 + 30, still 75 with markout
    let mut g = FlowStats::default();
    for _ in 0..5 {
        g.record_close(10, 100, -100);
        g.record_markout(1, 0);
    }
    assert_eq!(g.toxicity(), 75);
    // a positive markout raises it: 35 + 22 + 25 = 82
    for _ in 0..5 {
        g.record_markout(1, 6);
    }
    assert_eq!(g.toxicity(), 82);
}

#[test]
fn daily_window_wraps_midnight_and_honours_weekdays() {
    use crate::{daily_window_active, minute_of_day_utc, weekday_mon0};
    const H: u64 = 3_600_000_000_000;
    // 2026-10-09 (a Friday) 21:50 UTC
    let fri_2150 = 1_791_582_600 * 1_000_000_000u64;
    assert_eq!(weekday_mon0(fri_2150), 4);
    assert_eq!(minute_of_day_utc(fri_2150), 21 * 60 + 50);
    // rollover window 21:55 .. 22:10 every day
    assert!(!daily_window_active(
        &[],
        21 * 60 + 55,
        22 * 60 + 10,
        fri_2150
    ));
    assert!(daily_window_active(
        &[],
        21 * 60 + 55,
        22 * 60 + 10,
        fri_2150 + 10 * 60_000_000_000
    ));
    // wrapping window 23:00 .. 01:00: Friday 23:30 and Saturday 00:30 (counts as Friday's window)
    assert!(daily_window_active(
        &[4],
        23 * 60,
        60,
        fri_2150 + 100 * 60_000_000_000
    ));
    assert!(daily_window_active(
        &[4],
        23 * 60,
        60,
        fri_2150 + 160 * 60_000_000_000
    ));
    assert!(!daily_window_active(&[4], 23 * 60, 60, fri_2150 + 5 * H));
    // weekday filter: Monday only
    assert!(!daily_window_active(&[0], 0, 1440, fri_2150));
}
