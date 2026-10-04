use client_proto::*;
use domain::Fixed;
use proptest::prelude::*;

fn samples() -> Vec<Envelope> {
    let d = |s: &str| Some(Decimal::from_fixed(s.parse::<Fixed>().unwrap()));
    let bodies = vec![
        Body::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "test".into(),
            max_quote_hz: 5,
        }),
        Body::Auth(Auth {
            token: "eyJ.x.y".into(),
        }),
        Body::AuthOk(AuthOk {
            subject: "u1".into(),
            account_ids: vec!["A1".into(), "A2".into()],
            expires_at_s: 42,
        }),
        Body::Subscribe(Subscribe {
            request_id: "s1".into(),
            symbols: vec!["EURUSD".into(), "GBPUSD".into()],
        }),
        Body::Unsubscribe(Unsubscribe {
            request_id: "s2".into(),
            symbols: vec!["EURUSD".into()],
        }),
        Body::QuoteBatch(QuoteBatch {
            quotes: vec![Quote {
                symbol: "EURUSD".into(),
                bid: d("1.08501"),
                ask: d("1.08503"),
                bid_size: d("1000000"),
                ask_size: d("2000000"),
                ts_ns: 1_700_000_000_000_000_000,
            }],
        }),
        Body::SymbolListRequest(SymbolListRequest {
            request_id: "l1".into(),
        }),
        Body::SymbolList(SymbolList {
            request_id: "l1".into(),
            instruments: vec![Instrument {
                symbol: "EURUSD".into(),
                base: "EUR".into(),
                quote: "USD".into(),
                tick_size: d("0.00001"),
                qty_step: d("1000"),
                digits: 5,
                contract_size: d("100000"),
            }],
        }),
        Body::CandleRequest(CandleRequest {
            request_id: "c1".into(),
            symbol: "EURUSD".into(),
            timeframe: Timeframe::M5 as i32,
            from_ns: 1,
            to_ns: 2,
            limit: 100,
        }),
        Body::CandleResponse(CandleResponse {
            request_id: "c1".into(),
            symbol: "EURUSD".into(),
            timeframe: Timeframe::D1 as i32,
            candles: vec![Candle {
                open_time_ns: 60,
                open: d("1.1"),
                high: d("1.2"),
                low: d("1.0"),
                close: d("1.15"),
                ticks: 9,
            }],
        }),
        Body::AccountSnapshot(AccountSnapshot {
            account_id: "A1".into(),
            currency: "USD".into(),
            balance: d("100000"),
            equity: d("100010.5"),
            margin_used: d("0"),
            positions: vec![Position {
                symbol: "EURUSD".into(),
                net_qty: d("-1000"),
                avg_price: d("1.085"),
                unrealized_pnl: d("-3.2"),
            }],
            free_margin: d("99000"),
            margin_level: d("2500"),
        }),
        Body::PositionUpdate(PositionUpdate {
            account_id: "A1".into(),
            position: Some(Position::default()),
        }),
        Body::OrderUpdate(OrderUpdate {
            account_id: "A1".into(),
            client_request_id: "o1".into(),
            order_id: "X".into(),
            symbol: "EURUSD".into(),
            side: Side::Buy as i32,
            status: OrderStatus::Filled as i32,
            filled_qty: d("1000"),
            leaves_qty: d("0"),
            avg_price: d("1.08503"),
            last_qty: d("1000"),
            last_price: d("1.08503"),
            text: String::new(),
            ts_ns: 7,
        }),
        Body::PlaceOrder(PlaceOrder {
            request_id: "o1".into(),
            account_id: "A1".into(),
            symbol: "EURUSD".into(),
            side: Side::Sell as i32,
            order_type: OrderType::Limit as i32,
            qty: d("1000"),
            limit_price: d("1.1"),
            tif: TimeInForce::Gtc as i32,
        }),
        Body::CancelOrder(CancelOrder {
            request_id: "o2".into(),
            account_id: "A1".into(),
            target_request_id: "o1".into(),
        }),
        Body::ModifyOrder(ModifyOrder {
            request_id: "o3".into(),
            account_id: "A1".into(),
            target_request_id: "o1".into(),
            qty: d("2000"),
            limit_price: d("1.2"),
        }),
        Body::Ack(Ack {
            request_id: "o1".into(),
        }),
        Body::Error(Error {
            request_id: "o1".into(),
            code: ErrorCode::RateLimited as i32,
            message: "slow down".into(),
        }),
        Body::Heartbeat(Heartbeat { ts_ns: 1 }),
        Body::Ping(Ping { nonce: 9, ts_ns: 2 }),
        Body::Pong(Pong { nonce: 9, ts_ns: 3 }),
    ];
    bodies
        .into_iter()
        .enumerate()
        .map(|(i, b)| Envelope::new(i as u64, b))
        .collect()
}

#[test]
fn protobuf_round_trip_all_messages() {
    for env in samples() {
        let bytes = env.to_protobuf();
        assert_eq!(Envelope::from_protobuf(&bytes).unwrap(), env);
    }
}

#[test]
fn json_round_trip_all_messages() {
    for env in samples() {
        let s = env.to_json();
        assert_eq!(Envelope::from_json(&s).unwrap(), env, "{s}");
    }
}

#[test]
fn json_is_readable() {
    let env = Envelope::new(
        1,
        Body::Subscribe(Subscribe {
            request_id: "r".into(),
            symbols: vec!["EURUSD".into()],
        }),
    );
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).unwrap();
    assert_eq!(v["body"]["subscribe"]["symbols"][0], "EURUSD");
    // Missing fields default (handy for hand-written debug frames).
    let e = Envelope::from_json(r#"{"version":1,"body":{"ping":{"nonce":5}}}"#).unwrap();
    assert_eq!(e.body, Some(Body::Ping(Ping { nonce: 5, ts_ns: 0 })));
}

#[test]
fn garbage_is_rejected() {
    assert!(Envelope::from_protobuf(&[0xff, 0xff, 0xff]).is_err());
    assert!(Envelope::from_json("{nope").is_err());
}

#[test]
fn decimal_rescaling() {
    let d = Decimal {
        value: 108_501,
        scale: 5,
    };
    assert_eq!(d.to_fixed(), Some("1.08501".parse().unwrap()));
    let d = Decimal {
        value: 1_085_010_000,
        scale: 9,
    };
    assert_eq!(d.to_fixed(), Some("1.08501".parse().unwrap()));
    assert_eq!(Decimal { value: 1, scale: 9 }.to_fixed(), None);
    assert_eq!(
        Decimal {
            value: i64::MAX,
            scale: 0
        }
        .to_fixed(),
        None
    );
    assert_eq!(client_symbol("EUR/USD"), "EURUSD");
}

proptest! {
    #[test]
    fn fixed_decimal_round_trip(raw in any::<i64>()) {
        let f = Fixed::from_raw(raw);
        prop_assert_eq!(Decimal::from_fixed(f).to_fixed(), Some(f));
    }

    #[test]
    fn quote_round_trip(sym in "[A-Z]{6}", bid in any::<i64>(), ts in any::<u64>()) {
        let env = Envelope::new(ts, Body::QuoteBatch(QuoteBatch { quotes: vec![Quote {
            symbol: sym, bid: Some(Decimal { value: bid, scale: 8 }), ts_ns: ts, ..Default::default()
        }]}));
        prop_assert_eq!(Envelope::from_protobuf(&env.to_protobuf()).unwrap(), env.clone());
        prop_assert_eq!(Envelope::from_json(&env.to_json()).unwrap(), env);
    }
}
