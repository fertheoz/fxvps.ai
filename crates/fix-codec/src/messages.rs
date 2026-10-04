//! Typed FIX 4.4 messages used by the gateway and simulator.

use domain::{ExecType, OrderStatus, OrderType, Price, Qty, Side, TimeInForce};

use crate::enums::{
    CxlRejResponseTo, FixChar, MdEntryType, MdUpdateAction, SubscriptionRequestType,
};
use crate::raw::{FieldView, RawMessage};
use crate::{DecodeError, EncodeError, Encoder};

pub mod tags {
    pub const BEGIN_STRING: u32 = 8;
    pub const BODY_LENGTH: u32 = 9;
    pub const CHECKSUM: u32 = 10;
    pub const MSG_TYPE: u32 = 35;
    pub const SENDER_COMP_ID: u32 = 49;
    pub const TARGET_COMP_ID: u32 = 56;
    pub const MSG_SEQ_NUM: u32 = 34;
    pub const SENDING_TIME: u32 = 52;
    pub const POSS_DUP_FLAG: u32 = 43;
    pub const ORIG_SENDING_TIME: u32 = 122;
}

/// Standard header fields managed by the session layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub begin_string: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub msg_seq_num: u64,
    pub sending_time: String,
    pub poss_dup: bool,
    pub orig_sending_time: Option<String>,
}

impl Header {
    pub fn from_raw(raw: &RawMessage<'_>) -> Result<Header, DecodeError> {
        let v = raw.view();
        Ok(Header {
            begin_string: v.string(tags::BEGIN_STRING)?,
            sender_comp_id: v.string(tags::SENDER_COMP_ID)?,
            target_comp_id: v.string(tags::TARGET_COMP_ID)?,
            msg_seq_num: v.uint(tags::MSG_SEQ_NUM)?,
            sending_time: v.string(tags::SENDING_TIME)?,
            poss_dup: v.bool(tags::POSS_DUP_FLAG)?.unwrap_or(false),
            orig_sending_time: v.opt_string(tags::ORIG_SENDING_TIME)?,
        })
    }
}

/// A decoded message: header plus typed body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub header: Header,
    pub body: Body,
}

fn enum_field<T: FixChar>(v: &FieldView<'_, '_>, tag: u32) -> Result<T, DecodeError> {
    T::from_fix(v.char(tag)?).ok_or(DecodeError::InvalidValue { tag })
}

fn opt_enum_field<T: FixChar>(v: &FieldView<'_, '_>, tag: u32) -> Result<Option<T>, DecodeError> {
    v.opt_char(tag)?
        .map(|c| T::from_fix(c).ok_or(DecodeError::InvalidValue { tag }))
        .transpose()
}

fn opt_enum<T: FixChar>(v: Option<T>) -> Option<u8> {
    v.map(FixChar::to_fix)
}

/// Logon (A).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Logon {
    pub heart_bt_int: u64,
    pub reset_seq_num: bool,
    /// Username(553) — LMAX assumption: credentials carried in Logon.
    pub username: Option<String>,
    /// Password(554). Never logged.
    pub password: Option<String>,
}

/// Heartbeat (0).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Heartbeat {
    pub test_req_id: Option<String>,
}

/// TestRequest (1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRequest {
    pub test_req_id: String,
}

/// ResendRequest (2). `end_seq_no == 0` means "to infinity".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResendRequest {
    pub begin_seq_no: u64,
    pub end_seq_no: u64,
}

/// Session-level Reject (3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reject {
    pub ref_seq_num: u64,
    pub ref_tag_id: Option<u64>,
    pub ref_msg_type: Option<String>,
    pub reason: Option<u64>,
    pub text: Option<String>,
}

/// SequenceReset (4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceReset {
    pub gap_fill: bool,
    pub new_seq_no: u64,
}

/// Logout (5).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Logout {
    pub text: Option<String>,
}

/// SecurityID(48) + SecurityIDSource(22).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentRef {
    pub security_id: String,
    /// LMAX assumption: `8` (exchange symbol). Unverified, see docs/02.
    pub security_id_source: String,
}

/// MarketDataRequest (V).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketDataRequest {
    pub md_req_id: String,
    pub subscription_type: SubscriptionRequestType,
    pub market_depth: u64,
    /// MDUpdateType(265): 0 = full refresh, 1 = incremental.
    pub md_update_type: Option<u64>,
    pub entry_types: Vec<MdEntryType>,
    pub instruments: Vec<InstrumentRef>,
}

/// One entry of a full refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdEntry {
    pub entry_type: MdEntryType,
    pub price: Price,
    pub size: Qty,
}

/// MarketDataSnapshotFullRefresh (W).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketDataSnapshot {
    pub md_req_id: Option<String>,
    pub instrument: InstrumentRef,
    pub entries: Vec<MdEntry>,
}

/// One entry of an incremental refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdIncEntry {
    pub action: MdUpdateAction,
    pub entry_type: MdEntryType,
    pub instrument: Option<InstrumentRef>,
    pub price: Option<Price>,
    pub size: Option<Qty>,
}

/// MarketDataIncrementalRefresh (X).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketDataIncremental {
    pub md_req_id: Option<String>,
    pub entries: Vec<MdIncEntry>,
}

/// NewOrderSingle (D).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewOrderSingle {
    pub cl_ord_id: String,
    pub instrument: InstrumentRef,
    pub side: Side,
    pub transact_time: String,
    pub order_qty: Qty,
    pub ord_type: OrderType,
    pub price: Option<Price>,
    pub time_in_force: Option<TimeInForce>,
}

/// ExecutionReport (8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionReport {
    pub order_id: String,
    pub cl_ord_id: Option<String>,
    pub orig_cl_ord_id: Option<String>,
    pub exec_id: String,
    pub exec_type: ExecType,
    pub ord_status: OrderStatus,
    pub instrument: InstrumentRef,
    pub side: Side,
    pub order_qty: Option<Qty>,
    pub ord_type: Option<OrderType>,
    pub price: Option<Price>,
    pub time_in_force: Option<TimeInForce>,
    pub last_qty: Option<Qty>,
    pub last_px: Option<Price>,
    pub leaves_qty: Qty,
    pub cum_qty: Qty,
    pub avg_px: Option<Price>,
    pub ord_rej_reason: Option<u64>,
    pub text: Option<String>,
    pub transact_time: Option<String>,
}

/// OrderCancelRequest (F).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderCancelRequest {
    pub orig_cl_ord_id: String,
    pub cl_ord_id: String,
    pub instrument: InstrumentRef,
    pub side: Side,
    pub transact_time: String,
    pub order_qty: Option<Qty>,
}

/// OrderCancelReplaceRequest (G).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderCancelReplaceRequest {
    pub orig_cl_ord_id: String,
    pub cl_ord_id: String,
    pub instrument: InstrumentRef,
    pub side: Side,
    pub transact_time: String,
    pub order_qty: Qty,
    pub ord_type: OrderType,
    pub price: Option<Price>,
    pub time_in_force: Option<TimeInForce>,
}

/// OrderCancelReject (9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderCancelReject {
    pub order_id: String,
    pub cl_ord_id: String,
    pub orig_cl_ord_id: String,
    pub ord_status: OrderStatus,
    pub response_to: CxlRejResponseTo,
    pub reason: Option<u64>,
    pub text: Option<String>,
}

/// Every message type this codec understands; anything else is kept as `Unknown`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Logon(Logon),
    Heartbeat(Heartbeat),
    TestRequest(TestRequest),
    ResendRequest(ResendRequest),
    Reject(Reject),
    SequenceReset(SequenceReset),
    Logout(Logout),
    MarketDataRequest(MarketDataRequest),
    MarketDataSnapshot(MarketDataSnapshot),
    MarketDataIncremental(MarketDataIncremental),
    NewOrderSingle(NewOrderSingle),
    ExecutionReport(ExecutionReport),
    OrderCancelRequest(OrderCancelRequest),
    OrderCancelReplaceRequest(OrderCancelReplaceRequest),
    OrderCancelReject(OrderCancelReject),
    /// Unsupported MsgType with its body fields (header/trailer stripped).
    Unknown {
        msg_type: String,
        fields: Vec<(u32, Vec<u8>)>,
    },
}

const HEADER_TAGS: [u32; 10] = [8, 9, 35, 49, 56, 34, 52, 43, 122, 10];

impl Body {
    pub fn msg_type(&self) -> &str {
        match self {
            Body::Logon(_) => "A",
            Body::Heartbeat(_) => "0",
            Body::TestRequest(_) => "1",
            Body::ResendRequest(_) => "2",
            Body::Reject(_) => "3",
            Body::SequenceReset(_) => "4",
            Body::Logout(_) => "5",
            Body::MarketDataRequest(_) => "V",
            Body::MarketDataSnapshot(_) => "W",
            Body::MarketDataIncremental(_) => "X",
            Body::NewOrderSingle(_) => "D",
            Body::ExecutionReport(_) => "8",
            Body::OrderCancelRequest(_) => "F",
            Body::OrderCancelReplaceRequest(_) => "G",
            Body::OrderCancelReject(_) => "9",
            Body::Unknown { msg_type, .. } => msg_type,
        }
    }

    /// Session-level (administrative) message types.
    pub fn is_admin(&self) -> bool {
        is_admin_msg_type(self.msg_type())
    }

    /// Decodes the typed body from a validated raw message.
    pub fn from_raw(raw: &RawMessage<'_>) -> Result<Body, DecodeError> {
        let v = raw.view();
        let mt = raw.msg_type();
        Ok(match mt {
            b"A" => Body::Logon(Logon {
                heart_bt_int: v.uint(108)?,
                reset_seq_num: v.bool(141)?.unwrap_or(false),
                username: v.opt_string(553)?,
                password: v.opt_string(554)?,
            }),
            b"0" => Body::Heartbeat(Heartbeat {
                test_req_id: v.opt_string(112)?,
            }),
            b"1" => Body::TestRequest(TestRequest {
                test_req_id: v.string(112)?,
            }),
            b"2" => Body::ResendRequest(ResendRequest {
                begin_seq_no: v.uint(7)?,
                end_seq_no: v.uint(16)?,
            }),
            b"3" => Body::Reject(Reject {
                ref_seq_num: v.uint(45)?,
                ref_tag_id: v.opt_uint(371)?,
                ref_msg_type: v.opt_string(372)?,
                reason: v.opt_uint(373)?,
                text: v.opt_string(58)?,
            }),
            b"4" => Body::SequenceReset(SequenceReset {
                gap_fill: v.bool(123)?.unwrap_or(false),
                new_seq_no: v.uint(36)?,
            }),
            b"5" => Body::Logout(Logout {
                text: v.opt_string(58)?,
            }),
            b"V" => {
                let entry_types = v
                    .group(267, 269, &[269])?
                    .iter()
                    .map(|e| enum_field(e, 269))
                    .collect::<Result<_, _>>()?;
                let instruments = v
                    .group(146, 48, &[48, 22, 55])?
                    .iter()
                    .map(instrument)
                    .collect::<Result<_, _>>()?;
                Body::MarketDataRequest(MarketDataRequest {
                    md_req_id: v.string(262)?,
                    subscription_type: enum_field(&v, 263)?,
                    market_depth: v.uint(264)?,
                    md_update_type: v.opt_uint(265)?,
                    entry_types,
                    instruments,
                })
            }
            b"W" => {
                let entries = v
                    .group(268, 269, &[269, 270, 271, 272, 273])?
                    .iter()
                    .map(|e| {
                        Ok(MdEntry {
                            entry_type: enum_field(e, 269)?,
                            price: e.fixed(270)?,
                            size: e.fixed(271)?,
                        })
                    })
                    .collect::<Result<_, DecodeError>>()?;
                Body::MarketDataSnapshot(MarketDataSnapshot {
                    md_req_id: v.opt_string(262)?,
                    instrument: instrument(&v)?,
                    entries,
                })
            }
            b"X" => {
                let entries = v
                    .group(268, 279, &[279, 269, 48, 22, 55, 270, 271, 272, 273])?
                    .iter()
                    .map(|e| {
                        Ok(MdIncEntry {
                            action: enum_field(e, 279)?,
                            entry_type: enum_field(e, 269)?,
                            instrument: e.get(48).map(|_| instrument(e)).transpose()?,
                            price: e.opt_fixed(270)?,
                            size: e.opt_fixed(271)?,
                        })
                    })
                    .collect::<Result<_, DecodeError>>()?;
                Body::MarketDataIncremental(MarketDataIncremental {
                    md_req_id: v.opt_string(262)?,
                    entries,
                })
            }
            b"D" => Body::NewOrderSingle(NewOrderSingle {
                cl_ord_id: v.string(11)?,
                instrument: instrument(&v)?,
                side: enum_field(&v, 54)?,
                transact_time: v.string(60)?,
                order_qty: v.fixed(38)?,
                ord_type: enum_field(&v, 40)?,
                price: v.opt_fixed(44)?,
                time_in_force: opt_enum_field(&v, 59)?,
            }),
            b"8" => Body::ExecutionReport(ExecutionReport {
                order_id: v.string(37)?,
                cl_ord_id: v.opt_string(11)?,
                orig_cl_ord_id: v.opt_string(41)?,
                exec_id: v.string(17)?,
                exec_type: enum_field(&v, 150)?,
                ord_status: enum_field(&v, 39)?,
                instrument: instrument(&v)?,
                side: enum_field(&v, 54)?,
                order_qty: v.opt_fixed(38)?,
                ord_type: opt_enum_field(&v, 40)?,
                price: v.opt_fixed(44)?,
                time_in_force: opt_enum_field(&v, 59)?,
                last_qty: v.opt_fixed(32)?,
                last_px: v.opt_fixed(31)?,
                leaves_qty: v.fixed(151)?,
                cum_qty: v.fixed(14)?,
                avg_px: v.opt_fixed(6)?,
                ord_rej_reason: v.opt_uint(103)?,
                text: v.opt_string(58)?,
                transact_time: v.opt_string(60)?,
            }),
            b"F" => Body::OrderCancelRequest(OrderCancelRequest {
                orig_cl_ord_id: v.string(41)?,
                cl_ord_id: v.string(11)?,
                instrument: instrument(&v)?,
                side: enum_field(&v, 54)?,
                transact_time: v.string(60)?,
                order_qty: v.opt_fixed(38)?,
            }),
            b"G" => Body::OrderCancelReplaceRequest(OrderCancelReplaceRequest {
                orig_cl_ord_id: v.string(41)?,
                cl_ord_id: v.string(11)?,
                instrument: instrument(&v)?,
                side: enum_field(&v, 54)?,
                transact_time: v.string(60)?,
                order_qty: v.fixed(38)?,
                ord_type: enum_field(&v, 40)?,
                price: v.opt_fixed(44)?,
                time_in_force: opt_enum_field(&v, 59)?,
            }),
            b"9" => Body::OrderCancelReject(OrderCancelReject {
                order_id: v.string(37)?,
                cl_ord_id: v.string(11)?,
                orig_cl_ord_id: v.string(41)?,
                ord_status: enum_field(&v, 39)?,
                response_to: enum_field(&v, 434)?,
                reason: v.opt_uint(102)?,
                text: v.opt_string(58)?,
            }),
            other => Body::Unknown {
                msg_type: String::from_utf8(other.to_vec()).map_err(|_| {
                    DecodeError::InvalidValue {
                        tag: tags::MSG_TYPE,
                    }
                })?,
                fields: raw
                    .fields()
                    .iter()
                    .filter(|f| !HEADER_TAGS.contains(&f.tag))
                    .map(|f| (f.tag, f.value.to_vec()))
                    .collect(),
            },
        })
    }

    /// Writes the body fields (after the standard header).
    pub fn encode_fields(&self, e: &mut Encoder) {
        match self {
            Body::Logon(m) => {
                e.uint(98, 0).uint(108, m.heart_bt_int);
                if m.reset_seq_num {
                    e.bool(141, true);
                }
                e.opt_str(553, m.username.as_deref())
                    .opt_str(554, m.password.as_deref());
            }
            Body::Heartbeat(m) => {
                e.opt_str(112, m.test_req_id.as_deref());
            }
            Body::TestRequest(m) => {
                e.str(112, &m.test_req_id);
            }
            Body::ResendRequest(m) => {
                e.uint(7, m.begin_seq_no).uint(16, m.end_seq_no);
            }
            Body::Reject(m) => {
                e.uint(45, m.ref_seq_num)
                    .opt_uint(371, m.ref_tag_id)
                    .opt_str(372, m.ref_msg_type.as_deref())
                    .opt_uint(373, m.reason)
                    .opt_str(58, m.text.as_deref());
            }
            Body::SequenceReset(m) => {
                if m.gap_fill {
                    e.bool(123, true);
                }
                e.uint(36, m.new_seq_no);
            }
            Body::Logout(m) => {
                e.opt_str(58, m.text.as_deref());
            }
            Body::MarketDataRequest(m) => {
                e.str(262, &m.md_req_id)
                    .char(263, m.subscription_type.to_fix())
                    .uint(264, m.market_depth)
                    .opt_uint(265, m.md_update_type)
                    .uint(267, m.entry_types.len() as u64);
                for t in &m.entry_types {
                    e.char(269, t.to_fix());
                }
                e.uint(146, m.instruments.len() as u64);
                for i in &m.instruments {
                    put_instrument(e, i);
                }
            }
            Body::MarketDataSnapshot(m) => {
                e.opt_str(262, m.md_req_id.as_deref());
                put_instrument(e, &m.instrument);
                e.uint(268, m.entries.len() as u64);
                for en in &m.entries {
                    e.char(269, en.entry_type.to_fix())
                        .fixed(270, en.price)
                        .fixed(271, en.size);
                }
            }
            Body::MarketDataIncremental(m) => {
                e.opt_str(262, m.md_req_id.as_deref())
                    .uint(268, m.entries.len() as u64);
                for en in &m.entries {
                    e.char(279, en.action.to_fix())
                        .char(269, en.entry_type.to_fix());
                    if let Some(i) = &en.instrument {
                        put_instrument(e, i);
                    }
                    e.opt_fixed(270, en.price).opt_fixed(271, en.size);
                }
            }
            Body::NewOrderSingle(m) => {
                e.str(11, &m.cl_ord_id);
                put_instrument(e, &m.instrument);
                e.char(54, m.side.to_fix())
                    .str(60, &m.transact_time)
                    .fixed(38, m.order_qty)
                    .char(40, m.ord_type.to_fix())
                    .opt_fixed(44, m.price)
                    .opt_char(59, opt_enum(m.time_in_force));
            }
            Body::ExecutionReport(m) => {
                e.str(37, &m.order_id)
                    .opt_str(11, m.cl_ord_id.as_deref())
                    .opt_str(41, m.orig_cl_ord_id.as_deref())
                    .str(17, &m.exec_id)
                    .char(150, m.exec_type.to_fix())
                    .char(39, m.ord_status.to_fix());
                put_instrument(e, &m.instrument);
                e.char(54, m.side.to_fix())
                    .opt_fixed(38, m.order_qty)
                    .opt_char(40, opt_enum(m.ord_type))
                    .opt_fixed(44, m.price)
                    .opt_char(59, opt_enum(m.time_in_force))
                    .opt_fixed(32, m.last_qty)
                    .opt_fixed(31, m.last_px)
                    .fixed(151, m.leaves_qty)
                    .fixed(14, m.cum_qty)
                    .opt_fixed(6, m.avg_px)
                    .opt_uint(103, m.ord_rej_reason)
                    .opt_str(58, m.text.as_deref())
                    .opt_str(60, m.transact_time.as_deref());
            }
            Body::OrderCancelRequest(m) => {
                e.str(41, &m.orig_cl_ord_id).str(11, &m.cl_ord_id);
                put_instrument(e, &m.instrument);
                e.char(54, m.side.to_fix())
                    .str(60, &m.transact_time)
                    .opt_fixed(38, m.order_qty);
            }
            Body::OrderCancelReplaceRequest(m) => {
                e.str(41, &m.orig_cl_ord_id).str(11, &m.cl_ord_id);
                put_instrument(e, &m.instrument);
                e.char(54, m.side.to_fix())
                    .str(60, &m.transact_time)
                    .fixed(38, m.order_qty)
                    .char(40, m.ord_type.to_fix())
                    .opt_fixed(44, m.price)
                    .opt_char(59, opt_enum(m.time_in_force));
            }
            Body::OrderCancelReject(m) => {
                e.str(37, &m.order_id)
                    .str(11, &m.cl_ord_id)
                    .str(41, &m.orig_cl_ord_id)
                    .char(39, m.ord_status.to_fix())
                    .char(434, m.response_to.to_fix())
                    .opt_uint(102, m.reason)
                    .opt_str(58, m.text.as_deref());
            }
            Body::Unknown { fields, .. } => {
                for (tag, v) in fields {
                    e.bytes(*tag, v);
                }
            }
        }
    }
}

/// Session-level MsgTypes (0,1,2,3,4,5,A).
pub fn is_admin_msg_type(mt: &str) -> bool {
    matches!(mt, "0" | "1" | "2" | "3" | "4" | "5" | "A")
}

fn instrument(v: &FieldView<'_, '_>) -> Result<InstrumentRef, DecodeError> {
    Ok(InstrumentRef {
        security_id: v.string(48)?,
        security_id_source: v.opt_string(22)?.unwrap_or_else(|| "8".to_owned()),
    })
}

fn put_instrument(e: &mut Encoder, i: &InstrumentRef) {
    e.str(48, &i.security_id).str(22, &i.security_id_source);
}

/// Encodes a full message.
pub fn encode(header: &Header, body: &Body) -> Result<Vec<u8>, EncodeError> {
    let mut e = Encoder::new();
    e.str(tags::MSG_TYPE, body.msg_type())
        .str(tags::SENDER_COMP_ID, &header.sender_comp_id)
        .str(tags::TARGET_COMP_ID, &header.target_comp_id)
        .uint(tags::MSG_SEQ_NUM, header.msg_seq_num)
        .str(tags::SENDING_TIME, &header.sending_time);
    if header.poss_dup {
        e.bool(tags::POSS_DUP_FLAG, true);
    }
    e.opt_str(tags::ORIG_SENDING_TIME, header.orig_sending_time.as_deref());
    body.encode_fields(&mut e);
    e.finish(&header.begin_string)
}

/// Decodes one complete frame into header + typed body.
pub fn decode(frame: &[u8]) -> Result<Message, DecodeError> {
    let raw = RawMessage::parse(frame)?;
    Ok(Message {
        header: Header::from_raw(&raw)?,
        body: Body::from_raw(&raw)?,
    })
}
