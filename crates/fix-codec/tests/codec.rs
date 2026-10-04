use domain::{ExecType, Fixed, OrderStatus, OrderType, Side, TimeInForce};
use fix_codec::*;
use proptest::prelude::*;

fn hdr(seq: u64) -> Header {
    Header {
        begin_string: FIX44.into(),
        sender_comp_id: "CLIENT".into(),
        target_comp_id: "LMAX".into(),
        msg_seq_num: seq,
        sending_time: "20260101-00:00:00.000".into(),
        poss_dup: false,
        orig_sending_time: None,
    }
}

fn pipe(s: &str) -> Vec<u8> {
    s.replace('|', "\x01").into_bytes()
}

fn eurusd() -> InstrumentRef {
    InstrumentRef {
        security_id: "4001".into(),
        security_id_source: "8".into(),
    }
}

#[test]
fn checksum_matches_reference_example() {
    // Classic heartbeat example; checksum computed independently.
    let body = "35=0|49=A|56=B|34=1|52=20260101-00:00:00.000|";
    let mut msg = format!("8=FIX.4.4|9={}|{}", body.len(), body);
    let sum: u32 = msg.replace('|', "\x01").bytes().map(u32::from).sum::<u32>() % 256;
    msg.push_str(&format!("10={sum:03}|"));
    let bytes = pipe(&msg);
    assert_eq!(frame_len(&bytes).unwrap(), Some(bytes.len()));
    let m = decode(&bytes).unwrap();
    assert_eq!(m.body, Body::Heartbeat(Heartbeat::default()));
    // Our encoder produces byte-identical output.
    assert_eq!(encode(&m.header, &m.body).unwrap(), bytes);
}

#[test]
fn encode_sets_body_length_and_checksum() {
    let b = encode(
        &hdr(7),
        &Body::TestRequest(TestRequest {
            test_req_id: "T1".into(),
        }),
    )
    .unwrap();
    let s = String::from_utf8(b.clone()).unwrap().replace('\x01', "|");
    assert!(s.starts_with("8=FIX.4.4|9="));
    let blen: usize = s.split('|').nth(1).unwrap()[2..].parse().unwrap();
    let body_start = s.find("35=").unwrap();
    let trailer = s.rfind("10=").unwrap();
    assert_eq!(trailer - body_start, blen);
    let sum: u32 = b[..trailer].iter().map(|&x| u32::from(x)).sum::<u32>() % 256;
    assert_eq!(&s[trailer..], format!("10={sum:03}|"));
}

#[test]
fn frame_len_incremental_and_concatenated() {
    let a = encode(&hdr(1), &Body::Heartbeat(Heartbeat::default())).unwrap();
    let b = encode(
        &hdr(2),
        &Body::Logout(Logout {
            text: Some("bye".into()),
        }),
    )
    .unwrap();
    for i in 0..a.len() {
        assert_eq!(frame_len(&a[..i]).unwrap(), None, "prefix {i}");
    }
    let mut both = a.clone();
    both.extend_from_slice(&b);
    assert_eq!(frame_len(&both).unwrap(), Some(a.len()));
    assert_eq!(frame_len(&both[a.len()..]).unwrap(), Some(b.len()));
}

#[test]
fn malformed_inputs_rejected() {
    let good = encode(&hdr(1), &Body::Heartbeat(Heartbeat::default())).unwrap();
    // bad checksum
    let mut bad = good.clone();
    let n = bad.len();
    bad[n - 2] = if bad[n - 2] == b'0' { b'1' } else { b'0' };
    assert!(matches!(
        frame_len(&bad),
        Err(DecodeError::BadChecksum { .. })
    ));
    // garbage prefix
    assert_eq!(frame_len(b"XX=FIX"), Err(DecodeError::BadBeginString));
    // body length lies (too short -> trailer misaligned)
    let s = String::from_utf8(good.clone()).unwrap();
    let wrong = s.replacen("9=", "9=1", 1);
    assert!(frame_len(wrong.as_bytes()).is_err() || frame_len(wrong.as_bytes()).unwrap().is_none());
    // non-numeric body length
    assert_eq!(
        frame_len(&pipe("8=FIX.4.4|9=abc|")),
        Err(DecodeError::BadBodyLength)
    );
    // huge body length
    assert_eq!(
        frame_len(&pipe("8=FIX.4.4|9=99999999|")),
        Err(DecodeError::TooLarge)
    );
    // missing required field (TestRequest without 112)
    let body = "35=1|49=A|56=B|34=1|52=20260101-00:00:00.000|";
    let mut msg = format!("8=FIX.4.4|9={}|{}", body.len(), body);
    let sum: u32 = msg.replace('|', "\x01").bytes().map(u32::from).sum::<u32>() % 256;
    msg.push_str(&format!("10={sum:03}|"));
    assert_eq!(
        decode(&pipe(&msg)),
        Err(DecodeError::MissingField { tag: 112 })
    );
    // SOH inside value cannot be encoded
    assert!(encode(
        &hdr(1),
        &Body::TestRequest(TestRequest {
            test_req_id: "a\x01b".into()
        })
    )
    .is_err());
    // trailing bytes
    let mut extra = good.clone();
    extra.push(b'x');
    assert!(RawMessage::parse(&extra).is_err());
}

#[test]
fn invalid_enum_and_price_values() {
    let mut nos = new_order();
    if let Body::NewOrderSingle(ref mut n) = nos {
        n.cl_ord_id = "X".into();
    }
    let s = String::from_utf8(encode(&hdr(1), &nos).unwrap()).unwrap();
    // Corrupt Side to '9' and fix up checksum.
    let corrupted = refix(&s.replace("\x0154=1\x01", "\x0154=9\x01"));
    assert_eq!(
        decode(&corrupted),
        Err(DecodeError::InvalidValue { tag: 54 })
    );
    let corrupted = refix(&s.replace("\x0138=100000\x01", "\x0138=1e5\x01"));
    assert_eq!(
        decode(&corrupted),
        Err(DecodeError::InvalidValue { tag: 38 })
    );
}

fn refix(s: &str) -> Vec<u8> {
    let body_start = s.find("\x0135=").unwrap() + 1;
    let trailer = s.rfind("\x0110=").unwrap() + 1;
    let body = &s[body_start..trailer];
    let mut out = format!("8=FIX.4.4\x019={}\x01{}", body.len(), body);
    let sum: u32 = out.bytes().map(u32::from).sum::<u32>() % 256;
    out.push_str(&format!("10={sum:03}\x01"));
    out.into_bytes()
}

fn new_order() -> Body {
    Body::NewOrderSingle(NewOrderSingle {
        cl_ord_id: "ord-1".into(),
        instrument: eurusd(),
        side: Side::Buy,
        transact_time: "20260101-00:00:00.000".into(),
        order_qty: Fixed::from_int(100_000),
        ord_type: OrderType::Market,
        price: None,
        time_in_force: Some(TimeInForce::ImmediateOrCancel),
    })
}

fn all_bodies() -> Vec<Body> {
    vec![
        Body::Logon(Logon {
            heart_bt_int: 30,
            reset_seq_num: true,
            username: Some("u".into()),
            password: Some("p".into()),
        }),
        Body::Heartbeat(Heartbeat {
            test_req_id: Some("T".into()),
        }),
        Body::TestRequest(TestRequest {
            test_req_id: "T".into(),
        }),
        Body::ResendRequest(ResendRequest {
            begin_seq_no: 3,
            end_seq_no: 0,
        }),
        Body::Reject(Reject {
            ref_seq_num: 4,
            ref_tag_id: Some(54),
            ref_msg_type: Some("D".into()),
            reason: Some(5),
            text: Some("bad".into()),
        }),
        Body::SequenceReset(SequenceReset {
            gap_fill: true,
            new_seq_no: 9,
        }),
        Body::Logout(Logout { text: None }),
        Body::MarketDataRequest(MarketDataRequest {
            md_req_id: "md1".into(),
            subscription_type: SubscriptionRequestType::Subscribe,
            market_depth: 5,
            md_update_type: Some(0),
            entry_types: vec![MdEntryType::Bid, MdEntryType::Offer],
            instruments: vec![
                eurusd(),
                InstrumentRef {
                    security_id: "4002".into(),
                    security_id_source: "8".into(),
                },
            ],
        }),
        Body::MarketDataSnapshot(MarketDataSnapshot {
            md_req_id: Some("md1".into()),
            instrument: eurusd(),
            entries: vec![
                MdEntry {
                    entry_type: MdEntryType::Bid,
                    price: Fixed::from_parts(108_512, 5),
                    size: Fixed::from_int(1_000_000),
                },
                MdEntry {
                    entry_type: MdEntryType::Offer,
                    price: Fixed::from_parts(108_514, 5),
                    size: Fixed::from_int(500_000),
                },
            ],
        }),
        Body::MarketDataIncremental(MarketDataIncremental {
            md_req_id: None,
            entries: vec![
                MdIncEntry {
                    action: MdUpdateAction::Change,
                    entry_type: MdEntryType::Bid,
                    instrument: Some(eurusd()),
                    price: Some(Fixed::from_parts(1, 0)),
                    size: Some(Fixed::from_int(2)),
                },
                MdIncEntry {
                    action: MdUpdateAction::Delete,
                    entry_type: MdEntryType::Offer,
                    instrument: None,
                    price: None,
                    size: None,
                },
            ],
        }),
        new_order(),
        Body::ExecutionReport(ExecutionReport {
            order_id: "O1".into(),
            cl_ord_id: Some("ord-1".into()),
            orig_cl_ord_id: None,
            exec_id: "E1".into(),
            exec_type: ExecType::Trade,
            ord_status: OrderStatus::PartiallyFilled,
            instrument: eurusd(),
            side: Side::Sell,
            order_qty: Some(Fixed::from_int(10)),
            ord_type: Some(OrderType::Limit),
            price: Some(Fixed::from_parts(108_500, 5)),
            time_in_force: Some(TimeInForce::Day),
            last_qty: Some(Fixed::from_int(4)),
            last_px: Some(Fixed::from_parts(108_501, 5)),
            leaves_qty: Fixed::from_int(6),
            cum_qty: Fixed::from_int(4),
            avg_px: Some(Fixed::from_parts(108_501, 5)),
            ord_rej_reason: None,
            text: None,
            transact_time: Some("20260101-00:00:00.000".into()),
        }),
        Body::OrderCancelRequest(OrderCancelRequest {
            orig_cl_ord_id: "a".into(),
            cl_ord_id: "b".into(),
            instrument: eurusd(),
            side: Side::Buy,
            transact_time: "t".into(),
            order_qty: None,
        }),
        Body::OrderCancelReplaceRequest(OrderCancelReplaceRequest {
            orig_cl_ord_id: "a".into(),
            cl_ord_id: "b".into(),
            instrument: eurusd(),
            side: Side::Buy,
            transact_time: "t".into(),
            order_qty: Fixed::from_int(3),
            ord_type: OrderType::Limit,
            price: Some(Fixed::from_parts(11, 1)),
            time_in_force: Some(TimeInForce::GoodTillCancel),
        }),
        Body::OrderCancelReject(OrderCancelReject {
            order_id: "NONE".into(),
            cl_ord_id: "b".into(),
            orig_cl_ord_id: "a".into(),
            ord_status: OrderStatus::Rejected,
            response_to: CxlRejResponseTo::Cancel,
            reason: Some(1),
            text: Some("unknown order".into()),
        }),
        Body::Unknown {
            msg_type: "BE".into(),
            fields: vec![(923, b"x".to_vec())],
        },
    ]
}

#[test]
fn every_message_type_roundtrips() {
    for (i, body) in all_bodies().into_iter().enumerate() {
        let mut h = hdr(i as u64 + 1);
        if i % 2 == 0 {
            h.poss_dup = true;
            h.orig_sending_time = Some("20251231-23:59:59.999".into());
        }
        let bytes = encode(&h, &body).unwrap();
        let m = decode(&bytes).unwrap_or_else(|e| panic!("{}: {e}", body.msg_type()));
        assert_eq!(m.header, h);
        assert_eq!(m.body, body);
        assert_eq!(encode(&m.header, &m.body).unwrap(), bytes);
    }
}

#[test]
fn group_count_mismatch_is_rejected() {
    let s = String::from_utf8(encode(&hdr(1), &all_bodies()[8]).unwrap()).unwrap();
    let bad = refix(&s.replace("\x01268=2\x01", "\x01268=3\x01"));
    assert_eq!(decode(&bad), Err(DecodeError::BadGroup { tag: 268 }));
}

fn ident() -> impl Strategy<Value = String> {
    "[A-Za-z0-9_.-]{1,24}"
}

fn fixed() -> impl Strategy<Value = Fixed> {
    any::<i64>().prop_map(Fixed::from_raw)
}

fn side() -> impl Strategy<Value = Side> {
    prop_oneof![Just(Side::Buy), Just(Side::Sell)]
}

fn tif() -> impl Strategy<Value = Option<TimeInForce>> {
    proptest::option::of(prop_oneof![
        Just(TimeInForce::Day),
        Just(TimeInForce::GoodTillCancel),
        Just(TimeInForce::ImmediateOrCancel),
        Just(TimeInForce::FillOrKill)
    ])
}

fn instr() -> impl Strategy<Value = InstrumentRef> {
    (ident(), "[0-9A-Z]{1,2}").prop_map(|(security_id, security_id_source)| InstrumentRef {
        security_id,
        security_id_source,
    })
}

fn body_strategy() -> impl Strategy<Value = Body> {
    let nos = (
        ident(),
        instr(),
        side(),
        fixed(),
        proptest::option::of(fixed()),
        tif(),
    )
        .prop_map(|(id, instrument, side, qty, price, tif)| {
            Body::NewOrderSingle(NewOrderSingle {
                cl_ord_id: id,
                instrument,
                side,
                transact_time: "20260101-00:00:00.000".into(),
                order_qty: qty,
                ord_type: if price.is_some() {
                    OrderType::Limit
                } else {
                    OrderType::Market
                },
                price,
                time_in_force: tif,
            })
        });
    let snap = (
        proptest::option::of(ident()),
        instr(),
        proptest::collection::vec(
            (
                prop_oneof![Just(MdEntryType::Bid), Just(MdEntryType::Offer)],
                fixed(),
                fixed(),
            ),
            0..8,
        ),
    )
        .prop_map(|(id, instrument, es)| {
            Body::MarketDataSnapshot(MarketDataSnapshot {
                md_req_id: id,
                instrument,
                entries: es
                    .into_iter()
                    .map(|(entry_type, price, size)| MdEntry {
                        entry_type,
                        price,
                        size,
                    })
                    .collect(),
            })
        });
    let er = (
        ident(),
        ident(),
        side(),
        fixed(),
        fixed(),
        proptest::option::of(fixed()),
        proptest::option::of("[ -~&&[^|]]{1,40}"),
    )
        .prop_map(|(oid, eid, side, leaves, cum, px, text)| {
            Body::ExecutionReport(ExecutionReport {
                order_id: oid,
                cl_ord_id: None,
                orig_cl_ord_id: None,
                exec_id: eid,
                exec_type: ExecType::Trade,
                ord_status: OrderStatus::Filled,
                instrument: InstrumentRef {
                    security_id: "4001".into(),
                    security_id_source: "8".into(),
                },
                side,
                order_qty: None,
                ord_type: None,
                price: None,
                time_in_force: None,
                last_qty: Some(cum),
                last_px: px,
                leaves_qty: leaves,
                cum_qty: cum,
                avg_px: px,
                ord_rej_reason: None,
                text,
                transact_time: None,
            })
        });
    let admin =
        (any::<u32>(), any::<u32>(), proptest::option::of(ident())).prop_map(|(a, b, t)| {
            match a % 3 {
                0 => Body::ResendRequest(ResendRequest {
                    begin_seq_no: a.into(),
                    end_seq_no: b.into(),
                }),
                1 => Body::SequenceReset(SequenceReset {
                    gap_fill: b % 2 == 0,
                    new_seq_no: b.into(),
                }),
                _ => Body::Heartbeat(Heartbeat { test_req_id: t }),
            }
        });
    prop_oneof![nos, snap, er, admin]
}

proptest! {
    #[test]
    fn prop_roundtrip(body in body_strategy(), seq in 1u64..u64::MAX / 2, sender in ident(), target in ident()) {
        let mut h = hdr(seq);
        h.sender_comp_id = sender;
        h.target_comp_id = target;
        let bytes = encode(&h, &body).unwrap();
        prop_assert_eq!(frame_len(&bytes).unwrap(), Some(bytes.len()));
        let m = decode(&bytes).unwrap();
        prop_assert_eq!(m.header, h);
        prop_assert_eq!(m.body, body);
    }

    #[test]
    fn prop_garbage_never_panics(data in proptest::collection::vec(any::<u8>(), 0..256)) {
        let _ = frame_len(&data);
        let _ = decode(&data);
    }

    #[test]
    fn prop_bitflip_detected(body in body_strategy(), idx in any::<prop::sample::Index>(), bit in 0u8..8) {
        let mut bytes = encode(&hdr(1), &body).unwrap();
        let i = idx.index(bytes.len());
        bytes[i] ^= 1 << bit;
        // A single bit flip must never decode to the same message silently.
        if let Ok(m) = decode(&bytes) {
            prop_assert!(m.body != body || m.header != hdr(1));
        }
    }
}
