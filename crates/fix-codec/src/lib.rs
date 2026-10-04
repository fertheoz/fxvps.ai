//! Minimal FIX 4.4 tag=value codec.
//!
//! * [`frame_len`] finds message boundaries in a byte stream and validates
//!   BodyLength(9) and CheckSum(10).
//! * [`RawMessage`] gives borrowed (zero-copy) field access.
//! * [`Body`] / [`Header`] are the typed messages; [`encode`] / [`decode`] convert.
//!
//! Prices and quantities are [`domain::Fixed`]; floats are never used.

mod encode;
pub mod enums;
pub mod messages;
pub mod raw;
pub mod time;

pub use encode::Encoder;
pub use enums::{CxlRejResponseTo, FixChar, MdEntryType, MdUpdateAction, SubscriptionRequestType};
pub use messages::*;
pub use raw::{checksum, frame_len, Field, FieldView, RawMessage, SOH};
pub use time::{now_timestamp, utc_timestamp};

pub const FIX44: &str = "FIX.4.4";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("incomplete message")]
    Incomplete,
    #[error("message does not start with 8=BeginString")]
    BadBeginString,
    #[error("invalid or inconsistent BodyLength(9)")]
    BadBodyLength,
    #[error("checksum mismatch: declared {declared}, actual {actual}")]
    BadChecksum { declared: u8, actual: u8 },
    #[error("message exceeds maximum size")]
    TooLarge,
    #[error("malformed message: {0}")]
    Malformed(&'static str),
    #[error("required tag {tag} missing")]
    MissingField { tag: u32 },
    #[error("invalid value for tag {tag}")]
    InvalidValue { tag: u32 },
    #[error("repeating group {tag} malformed")]
    BadGroup { tag: u32 },
}

impl DecodeError {
    /// Tag the error refers to, if any (for Reject RefTagID).
    pub fn tag(&self) -> Option<u32> {
        match self {
            DecodeError::MissingField { tag }
            | DecodeError::InvalidValue { tag }
            | DecodeError::BadGroup { tag } => Some(*tag),
            _ => None,
        }
    }

    /// SessionRejectReason(373) code for this error.
    pub fn reject_reason(&self) -> u64 {
        match self {
            DecodeError::MissingField { .. } => 1,
            DecodeError::InvalidValue { .. } => 5,
            DecodeError::BadGroup { .. } => 16,
            _ => 99,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("value for tag {tag} is empty or contains SOH")]
    InvalidValue { tag: u32 },
}
