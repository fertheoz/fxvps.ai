use std::time::{Duration, Instant};

use domain::{Fixed, OrderType, Side, TimeInForce};
use fix_codec::*;
use fix_session::*;

const HB: Duration = Duration::from_secs(30);

fn cfgs() -> (SessionConfig, SessionConfig) {
    let mut i = SessionConfig::new(Role::Initiator, "CLIENT", "LP");
    i.heartbeat_interval = HB;
    i.username = Some("user".into());
    i.password = Some("pass".into());
    let mut a = SessionConfig::new(Role::Acceptor, "LP", "CLIENT");
    a.username = Some("user".into());
    a.password = Some("pass".into());
    (i, a)
}

fn sends(actions: &[Action]) -> Vec<Vec<u8>> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Send(b) => Some(b.clone()),
            _ => None,
        })
        .collect()
}

fn delivered(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Deliver(m) => match &m.body {
                Body::NewOrderSingle(n) => Some(n.cl_ord_id.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn decoded(bytes: &[u8]) -> Message {
    decode(bytes).unwrap()
}

/// Delivers every frame in `frames` to `to`, returning all actions produced.
fn feed(to: &mut Session<MemoryStore>, frames: Vec<Vec<u8>>, now: Instant) -> Vec<Action> {
    frames
        .into_iter()
        .flat_map(|f| to.on_frame(&f, now).unwrap())
        .collect()
}

fn order(id: &str) -> Body {
    Body::NewOrderSingle(NewOrderSingle {
        cl_ord_id: id.into(),
        instrument: InstrumentRef {
            security_id: "4001".into(),
            security_id_source: "8".into(),
        },
        side: Side::Buy,
        transact_time: now_timestamp(),
        order_qty: Fixed::from_int(1),
        ord_type: OrderType::Market,
        price: None,
        stop_px: None,
        time_in_force: Some(TimeInForce::ImmediateOrCancel),
    })
}

/// Initiator + acceptor logged on to each other.
fn logged_on(now: Instant) -> (Session<MemoryStore>, Session<MemoryStore>) {
    let (ic, ac) = cfgs();
    let mut i = Session::new(ic, MemoryStore::new(), now);
    let mut a = Session::new(ac, MemoryStore::new(), now);
    let logon = sends(&i.on_connect(now).unwrap());
    assert!(a.on_connect(now).unwrap().is_empty());
    let resp = feed(&mut a, logon, now);
    assert!(resp.contains(&Action::LoggedOn));
    let back = feed(&mut i, sends(&resp), now);
    assert!(back.contains(&Action::LoggedOn));
    assert!(i.is_active() && a.is_active());
    assert_eq!((i.next_sender_seq(), i.next_target_seq()), (2, 2));
    assert_eq!((a.next_sender_seq(), a.next_target_seq()), (2, 2));
    (i, a)
}

#[test]
fn logon_handshake() {
    logged_on(Instant::now());
}

#[test]
fn wrong_credentials_are_rejected() {
    let now = Instant::now();
    let (mut ic, ac) = cfgs();
    ic.password = Some("nope".into());
    let mut i = Session::new(ic, MemoryStore::new(), now);
    let mut a = Session::new(ac, MemoryStore::new(), now);
    let out = feed(&mut a, sends(&i.on_connect(now).unwrap()), now);
    assert!(matches!(out.last(), Some(Action::Disconnect(r)) if r.contains("credentials")));
    assert!(matches!(decoded(&sends(&out)[0]).body, Body::Logout(_)));
    assert!(!a.is_active());
}

#[test]
fn first_message_must_be_logon() {
    let now = Instant::now();
    let (_, ac) = cfgs();
    let mut a = Session::new(ac, MemoryStore::new(), now);
    let hdr = Header {
        begin_string: FIX44.into(),
        sender_comp_id: "CLIENT".into(),
        target_comp_id: "LP".into(),
        msg_seq_num: 1,
        sending_time: now_timestamp(),
        poss_dup: false,
        orig_sending_time: None,
    };
    let out = a
        .on_frame(&encode(&hdr, &order("x")).unwrap(), now)
        .unwrap();
    assert!(matches!(out.last(), Some(Action::Disconnect(_))));
}

#[test]
fn heartbeat_sent_when_idle() {
    let t0 = Instant::now();
    let (mut i, _a) = logged_on(t0);
    assert!(i.on_timer(t0 + HB / 2).unwrap().is_empty());
    let out = i.on_timer(t0 + HB).unwrap();
    let msgs: Vec<_> = sends(&out).iter().map(|b| decoded(b).body).collect();
    assert_eq!(msgs, vec![Body::Heartbeat(Heartbeat::default())]);
}

#[test]
fn test_request_answered_with_heartbeat() {
    let t0 = Instant::now();
    let (mut i, mut a) = logged_on(t0);
    // Acceptor silent long enough on its side -> it sends TestRequest.
    let tr = a.on_timer(t0 + HB + HB / 5).unwrap();
    let tr_frames = sends(&tr);
    let tr_body = tr_frames
        .iter()
        .map(|b| decoded(b).body)
        .find(|b| matches!(b, Body::TestRequest(_)));
    let Some(Body::TestRequest(tr_body)) = tr_body else {
        panic!("no TestRequest")
    };
    let reply = feed(&mut i, tr_frames, t0 + HB + HB / 5);
    let hb = decoded(&sends(&reply)[0]);
    assert_eq!(
        hb.body,
        Body::Heartbeat(Heartbeat {
            test_req_id: Some(tr_body.test_req_id)
        })
    );
    // Reply clears the pending TestRequest: no disconnect later.
    feed(&mut a, sends(&reply), t0 + HB + HB / 4);
    let later = a.on_timer(t0 + 2 * HB + HB / 4).unwrap();
    assert!(!later.iter().any(|x| matches!(x, Action::Disconnect(_))));
}

#[test]
fn heartbeat_timeout_disconnects() {
    let t0 = Instant::now();
    let (mut i, _a) = logged_on(t0);
    let out = i.on_timer(t0 + HB + HB / 5).unwrap();
    assert!(sends(&out)
        .iter()
        .any(|b| matches!(decoded(b).body, Body::TestRequest(_))));
    let out = i.on_timer(t0 + 2 * HB).unwrap();
    assert!(!out.iter().any(|x| matches!(x, Action::Disconnect(_))));
    let out = i.on_timer(t0 + 2 * HB + HB / 5).unwrap();
    assert!(matches!(out.last(), Some(Action::Disconnect(r)) if r.contains("heartbeat timeout")));
    assert_eq!(i.state(), &State::Closed);
}

#[test]
fn gap_detected_and_filled_by_resend() {
    let now = Instant::now();
    let (mut i, mut a) = logged_on(now);
    let m1 = sends(&i.send_app(order("o1"), now).unwrap());
    let m2 = sends(&i.send_app(order("o2"), now).unwrap()); // lost
    let m3 = sends(&i.send_app(order("o3"), now).unwrap());
    assert_eq!(delivered(&feed(&mut a, m1, now)), vec!["o1"]);
    drop(m2);
    let out = feed(&mut a, m3, now);
    assert!(delivered(&out).is_empty(), "o3 must wait for o2");
    let rr = sends(&out);
    assert_eq!(
        decoded(&rr[0]).body,
        Body::ResendRequest(ResendRequest {
            begin_seq_no: 3,
            end_seq_no: 0
        })
    );
    // Initiator answers with PossDup resends of 3 and 4.
    let resent = sends(&feed(&mut i, rr, now));
    assert_eq!(resent.len(), 2);
    for r in &resent {
        let m = decoded(r);
        assert!(m.header.poss_dup);
        assert!(m.header.orig_sending_time.is_some());
    }
    let out = feed(&mut a, resent, now);
    assert_eq!(delivered(&out), vec!["o2", "o3"], "in-order, no duplicates");
    assert_eq!(a.next_target_seq(), 5);
    // Further traffic flows normally.
    let m4 = sends(&i.send_app(order("o4"), now).unwrap());
    assert_eq!(delivered(&feed(&mut a, m4, now)), vec!["o4"]);
}

#[test]
fn admin_messages_are_gap_filled_not_resent() {
    let t0 = Instant::now();
    let (mut i, mut a) = logged_on(t0);
    let hb = sends(&i.on_timer(t0 + HB).unwrap()); // seq 2, admin, lost
    assert_eq!(hb.len(), 1);
    let lost_app = sends(&i.send_app(order("o1"), t0 + HB).unwrap()); // seq 3, lost
    drop(lost_app);
    let hb2 = sends(&i.on_timer(t0 + 2 * HB).unwrap()); // seq 4, lost
    drop((hb, hb2));
    let m5 = sends(&i.send_app(order("o2"), t0 + 2 * HB).unwrap()); // seq 5
    let rr = sends(&feed(&mut a, m5, t0 + 2 * HB));
    let reply = sends(&feed(&mut i, rr, t0 + 2 * HB));
    let bodies: Vec<_> = reply.iter().map(|b| decoded(b)).collect();
    assert_eq!(bodies[0].header.msg_seq_num, 2);
    assert_eq!(
        bodies[0].body,
        Body::SequenceReset(SequenceReset {
            gap_fill: true,
            new_seq_no: 3
        })
    );
    assert_eq!(bodies[1].header.msg_seq_num, 3);
    assert!(matches!(bodies[1].body, Body::NewOrderSingle(_)));
    assert_eq!(bodies[2].header.msg_seq_num, 4);
    assert_eq!(
        bodies[2].body,
        Body::SequenceReset(SequenceReset {
            gap_fill: true,
            new_seq_no: 5
        })
    );
    assert_eq!(bodies[3].header.msg_seq_num, 5);
    let out = feed(&mut a, reply, t0 + 2 * HB);
    assert_eq!(delivered(&out), vec!["o1", "o2"]);
    assert_eq!(a.next_target_seq(), 6);
}

#[test]
fn resend_beyond_journal_gap_fills_to_next_seq() {
    let now = Instant::now();
    let (_i, mut a) = logged_on(now);
    // Peer asks for messages we never stored (only the logon at seq 1, admin).
    let hdr = Header {
        begin_string: FIX44.into(),
        sender_comp_id: "CLIENT".into(),
        target_comp_id: "LP".into(),
        msg_seq_num: 2,
        sending_time: now_timestamp(),
        poss_dup: false,
        orig_sending_time: None,
    };
    let rr = encode(
        &hdr,
        &Body::ResendRequest(ResendRequest {
            begin_seq_no: 1,
            end_seq_no: 0,
        }),
    )
    .unwrap();
    let out = sends(&a.on_frame(&rr, now).unwrap());
    assert_eq!(out.len(), 1);
    let m = decoded(&out[0]);
    assert_eq!(m.header.msg_seq_num, 1);
    assert_eq!(
        m.body,
        Body::SequenceReset(SequenceReset {
            gap_fill: true,
            new_seq_no: 2
        })
    );
}

#[test]
fn seq_too_low_without_possdup_is_fatal_and_possdup_is_ignored() {
    let now = Instant::now();
    let (mut i, mut a) = logged_on(now);
    let m = sends(&i.send_app(order("o1"), now).unwrap());
    assert_eq!(delivered(&feed(&mut a, m.clone(), now)), vec!["o1"]);
    // Same frame again but flagged PossDup -> silently ignored.
    let mut dup = decoded(&m[0]);
    dup.header.poss_dup = true;
    dup.header.orig_sending_time = Some(dup.header.sending_time.clone());
    let out = a
        .on_frame(&encode(&dup.header, &dup.body).unwrap(), now)
        .unwrap();
    assert!(out.is_empty());
    // Replayed without PossDup -> Logout + disconnect.
    let out = feed(&mut a, m, now);
    assert!(matches!(out.last(), Some(Action::Disconnect(r)) if r.contains("too low")));
}

#[test]
fn invalid_application_message_is_rejected_and_seq_consumed() {
    let now = Instant::now();
    let (_i, mut a) = logged_on(now);
    // NewOrderSingle missing OrderQty(38), hand-built.
    let mut e = Encoder::new();
    e.str(35, "D")
        .str(49, "CLIENT")
        .str(56, "LP")
        .uint(34, 2)
        .str(52, &now_timestamp());
    e.str(11, "bad")
        .str(48, "4001")
        .str(22, "8")
        .char(54, b'1')
        .str(60, &now_timestamp())
        .char(40, b'1');
    let frame = e.finish(FIX44).unwrap();
    let out = a.on_frame(&frame, now).unwrap();
    let rej = decoded(&sends(&out)[0]);
    match rej.body {
        Body::Reject(r) => {
            assert_eq!(r.ref_seq_num, 2);
            assert_eq!(r.ref_tag_id, Some(38));
            assert_eq!(r.reason, Some(1));
        }
        other => panic!("expected Reject, got {other:?}"),
    }
    assert_eq!(a.next_target_seq(), 3);
}

#[test]
fn sequence_reset_reset_mode_jumps_forward() {
    let now = Instant::now();
    let (_i, mut a) = logged_on(now);
    let hdr = Header {
        begin_string: FIX44.into(),
        sender_comp_id: "CLIENT".into(),
        target_comp_id: "LP".into(),
        msg_seq_num: 50,
        sending_time: now_timestamp(),
        poss_dup: false,
        orig_sending_time: None,
    };
    let sr = encode(
        &hdr,
        &Body::SequenceReset(SequenceReset {
            gap_fill: false,
            new_seq_no: 100,
        }),
    )
    .unwrap();
    assert!(a.on_frame(&sr, now).unwrap().is_empty());
    assert_eq!(a.next_target_seq(), 100);
}

#[test]
fn graceful_logout() {
    let now = Instant::now();
    let (mut i, mut a) = logged_on(now);
    let lo = sends(&i.logout(Some("eod".into()), now).unwrap());
    let reply = feed(&mut a, lo, now);
    assert!(matches!(reply.last(), Some(Action::Disconnect(r)) if r.contains("eod")));
    let fin = feed(&mut i, sends(&reply), now);
    assert_eq!(fin, vec![Action::Disconnect("logout complete".into())]);
}

#[test]
fn logout_timeout() {
    let now = Instant::now();
    let (mut i, _a) = logged_on(now);
    i.logout(None, now).unwrap();
    let out = i.on_timer(now + Duration::from_secs(6)).unwrap();
    assert!(matches!(out.last(), Some(Action::Disconnect(r)) if r.contains("logout timeout")));
}

#[test]
fn file_store_keeps_sequences_across_reconnect() {
    let dir = tempfile::tempdir().unwrap();
    let now = Instant::now();
    let (mut ic, _) = cfgs();
    ic.reset_on_logon = false;
    {
        let mut s = Session::new(ic.clone(), FileStore::open(dir.path()).unwrap(), now);
        s.on_connect(now).unwrap();
        assert_eq!(s.next_sender_seq(), 2);
    }
    let mut s = Session::new(ic, FileStore::open(dir.path()).unwrap(), now);
    let out = s.on_connect(now).unwrap();
    assert_eq!(decode(&sends(&out)[0]).unwrap().header.msg_seq_num, 2);
}

#[tokio::test]
async fn transport_over_duplex() {
    use tokio::sync::mpsc;
    let (ic, ac) = cfgs();
    let (x, y) = tokio::io::duplex(64 * 1024);
    let now = Instant::now();
    let (itx, irx) = mpsc::channel(16);
    let (iev_tx, mut iev) = mpsc::channel(16);
    let (_atx, arx) = mpsc::channel(16);
    let (aev_tx, mut aev) = mpsc::channel(16);
    let ih = tokio::spawn(run_session(
        x,
        Session::new(ic, MemoryStore::new(), now),
        irx,
        iev_tx,
    ));
    let ah = tokio::spawn(run_session(
        y,
        Session::new(ac, MemoryStore::new(), now),
        arx,
        aev_tx,
    ));
    assert_eq!(iev.recv().await, Some(SessionEvent::LoggedOn));
    assert_eq!(aev.recv().await, Some(SessionEvent::LoggedOn));
    itx.send(SessionCommand::Send(order("wire-1")))
        .await
        .unwrap();
    match aev.recv().await {
        Some(SessionEvent::App(m)) => {
            assert!(matches!(m.body, Body::NewOrderSingle(ref n) if n.cl_ord_id == "wire-1"))
        }
        other => panic!("unexpected {other:?}"),
    }
    itx.send(SessionCommand::Logout(None)).await.unwrap();
    assert!(matches!(
        iev.recv().await,
        Some(SessionEvent::Disconnected(_))
    ));
    assert!(matches!(
        aev.recv().await,
        Some(SessionEvent::Disconnected(_))
    ));
    ih.await.unwrap();
    ah.await.unwrap();
}

/// Solid FX restarts the Order Entry sequence every day at 17:05 ET: on
/// our reconnect their Logon comes with MsgSeqNum 1 and no ResetSeqNumFlag
/// while our store still expects a higher number. The session takes the
/// venue's restart (counter continues at 2) instead of "too low".
#[test]
fn peer_sequence_restart_at_logon_is_taken() {
    let now = Instant::now();
    let peer_logon = encode(
        &Header {
            begin_string: FIX44.into(),
            sender_comp_id: "LP".into(),
            target_comp_id: "CLIENT".into(),
            msg_seq_num: 1,
            sending_time: now_timestamp(),
            poss_dup: false,
            orig_sending_time: None,
        },
        &Body::Logon(Logon {
            heart_bt_int: 30,
            reset_seq_num: false,
            username: None,
            password: None,
        }),
    )
    .unwrap();
    let (mut ic, _) = cfgs();
    ic.reset_on_logon = false;
    let mut store = MemoryStore::new();
    store.set_next_target_seq(42).unwrap(); // yesterday's session
    let mut i = Session::new(ic, store, now);
    assert!(!sends(&i.on_connect(now).unwrap()).is_empty());
    let acts = feed(&mut i, vec![peer_logon], now);
    assert!(acts.contains(&Action::LoggedOn), "{acts:?}");
    assert!(i.is_active());
    assert_eq!(i.next_target_seq(), 2);
}
