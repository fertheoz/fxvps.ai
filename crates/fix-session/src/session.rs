//! Sans-IO FIX session state machine. Feed it frames and clock ticks, execute the
//! returned [`Action`]s. Deterministic: time is always passed in.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use fix_codec::{
    decode, encode, now_timestamp, Body, DecodeError, Header, Heartbeat, Logon, Logout, Message,
    RawMessage, Reject, ResendRequest, SequenceReset, TestRequest, FIX44,
};
use tracing::{debug, info, warn};

use crate::store::MessageStore;
use crate::SessionError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Initiator,
    Acceptor,
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub role: Role,
    pub begin_string: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub heartbeat_interval: Duration,
    /// Initiator: send ResetSeqNumFlag(141)=Y on logon.
    pub reset_on_logon: bool,
    /// Initiator: credentials sent in Logon. Acceptor: credentials required from peer.
    pub username: Option<String>,
    pub password: Option<String>,
    pub logon_timeout: Duration,
    pub logout_timeout: Duration,
}

impl SessionConfig {
    pub fn new(role: Role, sender: impl Into<String>, target: impl Into<String>) -> Self {
        SessionConfig {
            role,
            begin_string: FIX44.to_owned(),
            sender_comp_id: sender.into(),
            target_comp_id: target.into(),
            heartbeat_interval: Duration::from_secs(30),
            reset_on_logon: true,
            username: None,
            password: None,
            logon_timeout: Duration::from_secs(10),
            logout_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Connected, Logon not yet exchanged (initiator: Logon sent).
    AwaitingLogon {
        since: Instant,
    },
    Active,
    LogoutSent {
        at: Instant,
    },
    Closed,
}

/// Something the transport must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Write these bytes to the wire.
    Send(Vec<u8>),
    /// Application message delivered in sequence.
    Deliver(Box<Message>),
    /// Session-level Reject received from the peer (informational).
    PeerReject(Reject),
    LoggedOn,
    /// Close the connection.
    Disconnect(String),
}

pub struct Session<S: MessageStore> {
    cfg: SessionConfig,
    store: S,
    state: State,
    last_sent: Instant,
    last_recv: Instant,
    test_request: Option<(String, Instant)>,
    test_req_counter: u64,
    /// Out-of-order inbound messages waiting for a gap to be filled.
    queue: BTreeMap<u64, Message>,
    /// Highest seq we already asked to be resent (avoid duplicate ResendRequests).
    resend_requested_to: Option<u64>,
}

impl<S: MessageStore> Session<S> {
    pub fn new(cfg: SessionConfig, store: S, now: Instant) -> Self {
        Session {
            cfg,
            store,
            state: State::AwaitingLogon { since: now },
            last_sent: now,
            last_recv: now,
            test_request: None,
            test_req_counter: 0,
            queue: BTreeMap::new(),
            resend_requested_to: None,
        }
    }

    pub fn config(&self) -> &SessionConfig {
        &self.cfg
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn store(&self) -> &S {
        &self.store
    }
    pub fn is_active(&self) -> bool {
        self.state == State::Active
    }
    pub fn next_sender_seq(&self) -> u64 {
        self.store.next_sender_seq()
    }
    pub fn next_target_seq(&self) -> u64 {
        self.store.next_target_seq()
    }

    /// Call once the TCP connection is up. Initiators emit their Logon.
    pub fn on_connect(&mut self, now: Instant) -> Result<Vec<Action>, SessionError> {
        self.state = State::AwaitingLogon { since: now };
        self.last_recv = now;
        let mut out = Vec::new();
        if self.cfg.role == Role::Initiator {
            if self.cfg.reset_on_logon {
                self.store.reset()?;
            }
            let logon = Body::Logon(Logon {
                heart_bt_int: self.cfg.heartbeat_interval.as_secs().max(1),
                reset_seq_num: self.cfg.reset_on_logon,
                username: self.cfg.username.clone(),
                password: self.cfg.password.clone(),
            });
            out.push(self.send_body(logon, now)?);
        }
        Ok(out)
    }

    /// Sends an application message. Only allowed when the session is active.
    pub fn send_app(&mut self, body: Body, now: Instant) -> Result<Vec<Action>, SessionError> {
        if body.is_admin() {
            return Err(SessionError::AdminViaApp);
        }
        if !self.is_active() {
            return Err(SessionError::NotLoggedOn);
        }
        Ok(vec![self.send_body(body, now)?])
    }

    /// Starts a graceful logout.
    pub fn logout(
        &mut self,
        text: Option<String>,
        now: Instant,
    ) -> Result<Vec<Action>, SessionError> {
        match self.state {
            State::Active => {
                let a = self.send_body(Body::Logout(Logout { text }), now)?;
                self.state = State::LogoutSent { at: now };
                Ok(vec![a])
            }
            State::AwaitingLogon { .. } => {
                self.state = State::Closed;
                Ok(vec![Action::Disconnect("logout before logon".into())])
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Periodic timer: heartbeats, TestRequest, timeouts.
    pub fn on_timer(&mut self, now: Instant) -> Result<Vec<Action>, SessionError> {
        let hb = self.cfg.heartbeat_interval;
        let mut out = Vec::new();
        match self.state {
            State::AwaitingLogon { since } => {
                if now.duration_since(since) >= self.cfg.logon_timeout {
                    self.state = State::Closed;
                    out.push(Action::Disconnect("logon timeout".into()));
                }
            }
            State::LogoutSent { at } => {
                if now.duration_since(at) >= self.cfg.logout_timeout {
                    self.state = State::Closed;
                    out.push(Action::Disconnect("logout timeout".into()));
                }
            }
            State::Active => {
                if let Some((_, sent)) = &self.test_request {
                    if now.duration_since(*sent) >= hb {
                        self.state = State::Closed;
                        out.push(Action::Disconnect(
                            "heartbeat timeout (no TestRequest reply)".into(),
                        ));
                        return Ok(out);
                    }
                } else if now.duration_since(self.last_recv) >= hb + hb / 5 {
                    self.test_req_counter += 1;
                    let id = format!("TEST-{}", self.test_req_counter);
                    debug!(id, "inbound silent, sending TestRequest");
                    self.test_request = Some((id.clone(), now));
                    out.push(
                        self.send_body(Body::TestRequest(TestRequest { test_req_id: id }), now)?,
                    );
                }
                if now.duration_since(self.last_sent) >= hb {
                    out.push(self.send_body(Body::Heartbeat(Heartbeat::default()), now)?);
                }
            }
            State::Closed => {}
        }
        Ok(out)
    }

    /// Processes one complete inbound frame (as delimited by `fix_codec::frame_len`).
    pub fn on_frame(&mut self, frame: &[u8], now: Instant) -> Result<Vec<Action>, SessionError> {
        let mut out = Vec::new();
        if self.state == State::Closed {
            return Ok(out);
        }
        self.last_recv = now;
        self.test_request = None;

        let raw = match RawMessage::parse(frame) {
            Ok(r) => r,
            Err(e) => {
                // Garbled message: ignore per FIX spec (do not increment seq).
                warn!(error = %e, "garbled frame ignored");
                return Ok(out);
            }
        };
        let header = match Header::from_raw(&raw) {
            Ok(h) => h,
            Err(e) => return Ok(self.fatal(format!("bad header: {e}"), now, out)),
        };
        if header.begin_string != self.cfg.begin_string
            || header.sender_comp_id != self.cfg.target_comp_id
            || header.target_comp_id != self.cfg.sender_comp_id
        {
            return Ok(self.fatal("CompID or BeginString mismatch".into(), now, out));
        }
        let body = match Body::from_raw(&raw) {
            Ok(b) => b,
            Err(e) => {
                // Valid header, invalid body: Reject and consume the sequence number.
                if matches!(self.state, State::AwaitingLogon { .. }) {
                    return Ok(self.fatal(format!("invalid logon: {e}"), now, out));
                }
                let msg_type = String::from_utf8_lossy(raw.msg_type()).into_owned();
                return self.reject_invalid(&header, msg_type, &e, now, out);
            }
        };
        let msg = Message { header, body };

        if matches!(self.state, State::AwaitingLogon { .. }) {
            let Body::Logon(logon) = &msg.body else {
                return Ok(self.fatal("first message must be Logon".into(), now, out));
            };
            self.handle_logon(&msg.header, logon.clone(), now, &mut out)?;
            if self.state != State::Active {
                return Ok(out);
            }
            let seq = msg.header.msg_seq_num;
            let expected = self.store.next_target_seq();
            if seq > expected {
                self.request_resend(expected, seq, now, &mut out)?;
            } else {
                self.store.set_next_target_seq(seq + 1)?;
            }
            return Ok(out);
        }

        self.sequence_and_process(msg, now, &mut out)?;
        Ok(out)
    }

    fn sequence_and_process(
        &mut self,
        msg: Message,
        now: Instant,
        out: &mut Vec<Action>,
    ) -> Result<(), SessionError> {
        let seq = msg.header.msg_seq_num;
        let expected = self.store.next_target_seq();

        // SequenceReset-Reset ignores sequence numbers entirely.
        if let Body::SequenceReset(sr) = &msg.body {
            if !sr.gap_fill {
                return self.apply_reset(sr.new_seq_no, seq, now, out);
            }
        }

        if seq < expected {
            if msg.header.poss_dup {
                debug!(seq, expected, "duplicate PossDup message ignored");
                return Ok(());
            }
            let text = format!("MsgSeqNum too low, expecting {expected} but received {seq}");
            *out = self.fatal(text, now, std::mem::take(out));
            return Ok(());
        }
        if seq > expected {
            // Logout should still be honoured even with a gap.
            if let Body::Logout(_) = msg.body {
                self.process(msg, now, out)?;
                return Ok(());
            }
            self.queue.insert(seq, msg);
            if self.resend_requested_to.is_none() {
                self.request_resend(expected, seq, now, out)?;
            }
            return Ok(());
        }

        self.store.set_next_target_seq(expected + 1)?;
        self.process(msg, now, out)?;
        self.drain_queue(now, out)
    }

    fn drain_queue(&mut self, now: Instant, out: &mut Vec<Action>) -> Result<(), SessionError> {
        loop {
            let expected = self.store.next_target_seq();
            // Anything below `expected` was gap-filled over.
            while let Some((&k, _)) = self.queue.first_key_value() {
                if k >= expected {
                    break;
                }
                self.queue.remove(&k);
            }
            let Some(msg) = self.queue.remove(&expected) else {
                break;
            };
            if self.state != State::Active && !matches!(self.state, State::LogoutSent { .. }) {
                break;
            }
            self.store.set_next_target_seq(expected + 1)?;
            self.process(msg, now, out)?;
        }
        if self.queue.is_empty()
            && self
                .resend_requested_to
                .is_some_and(|to| self.store.next_target_seq() > to)
        {
            info!("gap filled, resend complete");
            self.resend_requested_to = None;
        }
        Ok(())
    }

    fn process(
        &mut self,
        msg: Message,
        now: Instant,
        out: &mut Vec<Action>,
    ) -> Result<(), SessionError> {
        match msg.body {
            Body::Logon(_) => {
                // Logon while active: protocol violation.
                *out = self.fatal(
                    "unexpected Logon while active".into(),
                    now,
                    std::mem::take(out),
                );
            }
            Body::Heartbeat(_) => {}
            Body::TestRequest(tr) => {
                let hb = Body::Heartbeat(Heartbeat {
                    test_req_id: Some(tr.test_req_id),
                });
                out.push(self.send_body(hb, now)?);
            }
            Body::ResendRequest(rr) => self.handle_resend_request(rr, now, out)?,
            Body::SequenceReset(sr) => {
                // Gap fill (reset mode handled earlier). Seq already consumed.
                let expected = self.store.next_target_seq();
                if sr.new_seq_no > expected {
                    self.store.set_next_target_seq(sr.new_seq_no)?;
                } else if sr.new_seq_no < expected {
                    let rej = Reject {
                        ref_seq_num: msg.header.msg_seq_num,
                        ref_tag_id: Some(36),
                        ref_msg_type: Some("4".into()),
                        reason: Some(5),
                        text: Some("NewSeqNo lower than expected".into()),
                    };
                    out.push(self.send_body(Body::Reject(rej), now)?);
                }
            }
            Body::Reject(r) => out.push(Action::PeerReject(r)),
            Body::Logout(l) => {
                if matches!(self.state, State::LogoutSent { .. }) {
                    self.state = State::Closed;
                    out.push(Action::Disconnect("logout complete".into()));
                } else {
                    info!(text = ?l.text, "peer initiated logout");
                    out.push(self.send_body(Body::Logout(Logout::default()), now)?);
                    self.state = State::Closed;
                    out.push(Action::Disconnect(format!(
                        "peer logout: {}",
                        l.text.unwrap_or_default()
                    )));
                }
            }
            Body::Unknown { ref msg_type, .. } => {
                let rej = Reject {
                    ref_seq_num: msg.header.msg_seq_num,
                    ref_tag_id: Some(35),
                    ref_msg_type: Some(msg_type.clone()),
                    reason: Some(11),
                    text: Some("Unsupported MsgType".into()),
                };
                out.push(self.send_body(Body::Reject(rej), now)?);
            }
            _ => out.push(Action::Deliver(Box::new(msg))),
        }
        Ok(())
    }

    fn handle_logon(
        &mut self,
        header: &Header,
        logon: Logon,
        now: Instant,
        out: &mut Vec<Action>,
    ) -> Result<(), SessionError> {
        match self.cfg.role {
            Role::Initiator => {
                if logon.reset_seq_num && header.msg_seq_num == 1 {
                    self.store.set_next_target_seq(1)?;
                }
            }
            Role::Acceptor => {
                if let Some(user) = &self.cfg.username {
                    if logon.username.as_deref() != Some(user.as_str())
                        || logon.password != self.cfg.password
                    {
                        *out = self.fatal("invalid credentials".into(), now, std::mem::take(out));
                        return Ok(());
                    }
                }
                if logon.heart_bt_int == 0 {
                    *out = self.fatal("HeartBtInt must be > 0".into(), now, std::mem::take(out));
                    return Ok(());
                }
                self.cfg.heartbeat_interval = Duration::from_secs(logon.heart_bt_int);
                if logon.reset_seq_num {
                    self.store.reset()?;
                }
                let reply = Body::Logon(Logon {
                    heart_bt_int: logon.heart_bt_int,
                    reset_seq_num: logon.reset_seq_num,
                    username: None,
                    password: None,
                });
                out.push(self.send_body(reply, now)?);
            }
        }
        info!(sender = %self.cfg.sender_comp_id, target = %self.cfg.target_comp_id, "logged on");
        self.state = State::Active;
        out.push(Action::LoggedOn);
        Ok(())
    }

    fn handle_resend_request(
        &mut self,
        rr: ResendRequest,
        now: Instant,
        out: &mut Vec<Action>,
    ) -> Result<(), SessionError> {
        let last_sent = self.store.next_sender_seq().saturating_sub(1);
        let begin = rr.begin_seq_no.max(1);
        let end = if rr.end_seq_no == 0 {
            last_sent
        } else {
            rr.end_seq_no.min(last_sent)
        };
        info!(begin, end, "peer requested resend");
        if begin > end {
            // Nothing we can resend: gap-fill straight to our next seq.
            if begin <= last_sent + 1 {
                out.push(self.gap_fill(begin, last_sent + 1)?);
            }
            return Ok(());
        }
        let stored = self.store.get(begin, end)?;
        let mut gap_start: Option<u64> = None;
        let mut cursor = begin;
        for (seq, bytes) in stored {
            if seq > cursor {
                gap_start.get_or_insert(cursor);
            }
            match decode(&bytes) {
                Ok(m) if !m.body.is_admin() => {
                    if let Some(g) = gap_start.take() {
                        out.push(self.gap_fill(g, seq)?);
                    }
                    let header = Header {
                        poss_dup: true,
                        orig_sending_time: Some(m.header.sending_time.clone()),
                        sending_time: now_timestamp(),
                        ..m.header
                    };
                    out.push(Action::Send(encode(&header, &m.body)?));
                }
                _ => {
                    gap_start.get_or_insert(seq);
                }
            }
            cursor = seq + 1;
        }
        if cursor <= end {
            gap_start.get_or_insert(cursor);
        }
        if let Some(g) = gap_start {
            out.push(self.gap_fill(g, end + 1)?);
        }
        self.last_sent = now;
        Ok(())
    }

    fn gap_fill(&self, seq: u64, new_seq_no: u64) -> Result<Action, SessionError> {
        let header = Header {
            poss_dup: true,
            orig_sending_time: Some(now_timestamp()),
            ..self.header(seq)
        };
        let body = Body::SequenceReset(SequenceReset {
            gap_fill: true,
            new_seq_no,
        });
        Ok(Action::Send(encode(&header, &body)?))
    }

    fn apply_reset(
        &mut self,
        new_seq_no: u64,
        seq: u64,
        now: Instant,
        out: &mut Vec<Action>,
    ) -> Result<(), SessionError> {
        let expected = self.store.next_target_seq();
        if new_seq_no < expected {
            let rej = Reject {
                ref_seq_num: seq,
                ref_tag_id: Some(36),
                ref_msg_type: Some("4".into()),
                reason: Some(5),
                text: Some("SequenceReset may not decrease sequence".into()),
            };
            out.push(self.send_body(Body::Reject(rej), now)?);
            return Ok(());
        }
        warn!(expected, new_seq_no, "SequenceReset-Reset received");
        self.store.set_next_target_seq(new_seq_no)?;
        self.drain_queue(now, out)
    }

    fn request_resend(
        &mut self,
        from: u64,
        seen: u64,
        now: Instant,
        out: &mut Vec<Action>,
    ) -> Result<(), SessionError> {
        info!(from, seen, "sequence gap detected, sending ResendRequest");
        self.resend_requested_to = Some(seen - 1);
        let rr = Body::ResendRequest(ResendRequest {
            begin_seq_no: from,
            end_seq_no: 0,
        });
        out.push(self.send_body(rr, now)?);
        Ok(())
    }

    fn reject_invalid(
        &mut self,
        header: &Header,
        msg_type: String,
        e: &DecodeError,
        now: Instant,
        mut out: Vec<Action>,
    ) -> Result<Vec<Action>, SessionError> {
        let seq = header.msg_seq_num;
        let expected = self.store.next_target_seq();
        if seq != expected {
            // Out-of-order invalid message: let the normal gap logic deal with it later.
            if seq > expected && self.resend_requested_to.is_none() {
                self.request_resend(expected, seq, now, &mut out)?;
            }
            return Ok(out);
        }
        self.store.set_next_target_seq(seq + 1)?;
        let rej = Reject {
            ref_seq_num: seq,
            ref_tag_id: e.tag().map(u64::from),
            ref_msg_type: Some(msg_type),
            reason: Some(e.reject_reason()),
            text: Some(e.to_string()),
        };
        out.push(self.send_body(Body::Reject(rej), now)?);
        self.drain_queue(now, &mut out)?;
        Ok(out)
    }

    /// Sends Logout (if possible) and disconnects.
    fn fatal(&mut self, text: String, now: Instant, mut out: Vec<Action>) -> Vec<Action> {
        warn!(%text, "session error");
        if let Ok(a) = self.send_body(
            Body::Logout(Logout {
                text: Some(text.clone()),
            }),
            now,
        ) {
            out.push(a);
        }
        self.state = State::Closed;
        out.push(Action::Disconnect(text));
        out
    }

    fn header(&self, seq: u64) -> Header {
        Header {
            begin_string: self.cfg.begin_string.clone(),
            sender_comp_id: self.cfg.sender_comp_id.clone(),
            target_comp_id: self.cfg.target_comp_id.clone(),
            msg_seq_num: seq,
            sending_time: now_timestamp(),
            poss_dup: false,
            orig_sending_time: None,
        }
    }

    fn send_body(&mut self, body: Body, now: Instant) -> Result<Action, SessionError> {
        let seq = self.store.next_sender_seq();
        let bytes = encode(&self.header(seq), &body)?;
        if !body.is_admin() {
            self.store.store(seq, &bytes)?;
        }
        self.store.set_next_sender_seq(seq + 1)?;
        self.last_sent = now;
        Ok(Action::Send(bytes))
    }
}
