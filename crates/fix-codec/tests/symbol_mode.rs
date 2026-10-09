//! Symbol(55)-only venues (Solid FX / MAS Markets, FIX 4.2): instruments
//! round-trip through 55 without 48/22, HandlInst(21) is written on orders.
use domain::{ExecType, Fixed, OrderStatus, OrderType, Side, TimeInForce};
use fix_codec::*;

fn header(_msg_type: &str) -> Header {
    Header {
        begin_string: "FIX.4.2".into(),
        sender_comp_id: "FXVPS".into(),
        target_comp_id: "SFX".into(),
        msg_seq_num: 7,
        sending_time: "20261009-12:00:00.000".into(),
        poss_dup: false,
        orig_sending_time: None,
    }
}

#[test]
fn new_order_single_uses_symbol_and_handl_inst_in_symbol_mode() {
    let body = Body::NewOrderSingle(NewOrderSingle {
        cl_ord_id: "LP-1".into(),
        instrument: InstrumentRef {
            security_id: "EUR/USD".into(),
            security_id_source: SYMBOL_SOURCE.into(),
        },
        side: Side::Buy,
        transact_time: "20261009-12:00:00.000".into(),
        order_qty: Fixed::from_parts(100_000, 0),
        ord_type: OrderType::Limit,
        price: Some(Fixed::from_parts(110_000, 5)),
        stop_px: None,
        time_in_force: Some(TimeInForce::ImmediateOrCancel),
    });
    let bytes = encode(&header("D"), &body).unwrap();
    let s = String::from_utf8_lossy(&bytes).replace('\u{1}', "|");
    assert!(s.starts_with("8=FIX.4.2|"), "{s}");
    assert!(s.contains("|55=EUR/USD|"), "{s}");
    assert!(s.contains("|21=1|"), "{s}");
    assert!(!s.contains("|48="), "{s}");
    assert!(!s.contains("|22="), "{s}");
    let m = decode(&bytes).unwrap();
    let Body::NewOrderSingle(o) = m.body else {
        panic!("not a NewOrderSingle");
    };
    assert_eq!(o.instrument.security_id, "EUR/USD");
    assert_eq!(o.instrument.security_id_source, SYMBOL_SOURCE);
}

#[test]
fn security_id_venues_are_unchanged() {
    let body = Body::NewOrderSingle(NewOrderSingle {
        cl_ord_id: "LP-2".into(),
        instrument: InstrumentRef {
            security_id: "4001".into(),
            security_id_source: "8".into(),
        },
        side: Side::Sell,
        transact_time: "20261009-12:00:00.000".into(),
        order_qty: Fixed::from_parts(1000, 0),
        ord_type: OrderType::Market,
        price: None,
        stop_px: None,
        time_in_force: Some(TimeInForce::ImmediateOrCancel),
    });
    let bytes = encode(&header("D"), &body).unwrap();
    let s = String::from_utf8_lossy(&bytes).replace('\u{1}', "|");
    assert!(s.contains("|48=4001|22=8|"), "{s}");
    assert!(!s.contains("|21="), "{s}");
    assert!(!s.contains("|55="), "{s}");
    let m = decode(&bytes).unwrap();
    let Body::NewOrderSingle(o) = m.body else {
        panic!("not a NewOrderSingle");
    };
    assert_eq!(o.instrument.security_id, "4001");
    assert_eq!(o.instrument.security_id_source, "8");
}

#[test]
fn execution_report_with_symbol_only_decodes() {
    let body = Body::ExecutionReport(ExecutionReport {
        order_id: "SFX-9".into(),
        cl_ord_id: Some("LP-1".into()),
        orig_cl_ord_id: None,
        exec_id: "E1".into(),
        exec_type: ExecType::Trade,
        ord_status: OrderStatus::Filled,
        instrument: InstrumentRef {
            security_id: "XAU/USD".into(),
            security_id_source: SYMBOL_SOURCE.into(),
        },
        side: Side::Buy,
        order_qty: Some(Fixed::from_parts(100, 0)),
        ord_type: Some(OrderType::Market),
        price: None,
        time_in_force: Some(TimeInForce::ImmediateOrCancel),
        last_qty: Some(Fixed::from_parts(100, 0)),
        last_px: Some(Fixed::from_parts(413_000, 2)),
        leaves_qty: Fixed::from_parts(0, 0),
        cum_qty: Fixed::from_parts(100, 0),
        avg_px: Some(Fixed::from_parts(413_000, 2)),
        ord_rej_reason: None,
        text: None,
        transact_time: Some("20261009-12:00:00.000".into()),
    });
    let bytes = encode(&header("8"), &body).unwrap();
    let s = String::from_utf8_lossy(&bytes).replace('\u{1}', "|");
    assert!(s.contains("|55=XAU/USD|") && !s.contains("|48="), "{s}");
    let Body::ExecutionReport(er) = decode(&bytes).unwrap().body else {
        panic!("not an ExecutionReport");
    };
    assert_eq!(er.instrument.security_id, "XAU/USD");
    assert_eq!(er.instrument.security_id_source, SYMBOL_SOURCE);
}
