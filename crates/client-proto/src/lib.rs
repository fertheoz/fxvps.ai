//! fxvps.ai client wire protocol (v1).
//!
//! Every WebSocket frame carries one [`Envelope`]. Two encodings share the same schema:
//! - **binary** frames: protobuf (production),
//! - **text** frames: JSON (debugging; field names are the proto names).
//!
//! Prices and quantities travel as [`Decimal`] (`value * 10^-scale`, int64). See
//! `PROTOCOL.md` for the session flow.

#[allow(clippy::all, missing_docs)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/fxvps.client.v1.rs"));
}

pub use v1::envelope::Body;
pub use v1::*;

use domain::fixed::{SCALE, SCALE_DIGITS};
use domain::Fixed;
use prost::Message;

/// Current protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("protobuf decode: {0}")]
    Proto(#[from] prost::DecodeError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Wire encoding of a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Protobuf,
    Json,
}

impl Envelope {
    /// Envelope with the current version and the given body.
    pub fn new(seq: u64, body: Body) -> Self {
        Envelope {
            version: PROTOCOL_VERSION,
            seq,
            body: Some(body),
        }
    }

    pub fn to_protobuf(&self) -> Vec<u8> {
        self.encode_to_vec()
    }

    pub fn from_protobuf(buf: &[u8]) -> Result<Self, CodecError> {
        Ok(Envelope::decode(buf)?)
    }

    pub fn to_json(&self) -> String {
        // Serializing generated plain-data types cannot fail.
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Result<Self, CodecError> {
        Ok(serde_json::from_str(s)?)
    }
}

impl Decimal {
    /// Exact conversion from domain [`Fixed`] (scale 8).
    pub fn from_fixed(f: Fixed) -> Self {
        Decimal {
            value: f.raw(),
            scale: SCALE_DIGITS,
        }
    }

    /// Converts to [`Fixed`]. Returns `None` if the scale exceeds 8 digits with a
    /// non-representable remainder or the value overflows.
    pub fn to_fixed(self) -> Option<Fixed> {
        if self.scale <= SCALE_DIGITS {
            let mul = 10i64.checked_pow(SCALE_DIGITS - self.scale)?;
            self.value.checked_mul(mul).map(Fixed::from_raw)
        } else {
            let div = 10i64.checked_pow(self.scale - SCALE_DIGITS)?;
            (self.value % div == 0).then(|| Fixed::from_raw(self.value / div))
        }
    }
}

impl From<Fixed> for Decimal {
    fn from(f: Fixed) -> Self {
        Decimal::from_fixed(f)
    }
}

const _: () = assert!(SCALE == 100_000_000);

/// Client-facing symbol (`EURUSD`) for an internal one (`EUR/USD`).
pub fn client_symbol(internal: &str) -> String {
    internal.chars().filter(|c| *c != '/').collect()
}

impl Side {
    pub fn to_domain(self) -> Option<domain::Side> {
        match self {
            Side::Buy => Some(domain::Side::Buy),
            Side::Sell => Some(domain::Side::Sell),
            Side::Unspecified => None,
        }
    }
    pub fn from_domain(s: domain::Side) -> Self {
        match s {
            domain::Side::Buy => Side::Buy,
            domain::Side::Sell => Side::Sell,
        }
    }
}

impl OrderStatus {
    pub fn from_domain(s: domain::OrderStatus) -> Self {
        match s {
            domain::OrderStatus::New => OrderStatus::New,
            domain::OrderStatus::PartiallyFilled => OrderStatus::PartiallyFilled,
            domain::OrderStatus::Filled => OrderStatus::Filled,
            domain::OrderStatus::Canceled => OrderStatus::Canceled,
            domain::OrderStatus::Replaced => OrderStatus::Replaced,
            domain::OrderStatus::Rejected => OrderStatus::Rejected,
            domain::OrderStatus::Expired => OrderStatus::Expired,
        }
    }
}

impl Timeframe {
    /// Bucket length in seconds.
    pub fn seconds(self) -> Option<u64> {
        Some(match self {
            Timeframe::Unspecified => return None,
            Timeframe::M1 => 60,
            Timeframe::M5 => 300,
            Timeframe::M15 => 900,
            Timeframe::M30 => 1800,
            Timeframe::H1 => 3600,
            Timeframe::H4 => 14_400,
            Timeframe::D1 => 86_400,
        })
    }

    pub const ALL: [Timeframe; 7] = [
        Timeframe::M1,
        Timeframe::M5,
        Timeframe::M15,
        Timeframe::M30,
        Timeframe::H1,
        Timeframe::H4,
        Timeframe::D1,
    ];
}
