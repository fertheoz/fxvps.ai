//! FIX character codes for domain enums and FIX-only enums.

use domain::{ExecType, OrderStatus, OrderType, Side, TimeInForce};

/// Maps a type to/from its single-character FIX representation.
pub trait FixChar: Sized + Copy {
    fn to_fix(self) -> u8;
    fn from_fix(c: u8) -> Option<Self>;
}

macro_rules! fix_char {
    ($ty:ty { $($var:path => $c:literal),+ $(,)? }) => {
        impl FixChar for $ty {
            fn to_fix(self) -> u8 {
                match self { $($var => $c),+ }
            }
            fn from_fix(c: u8) -> Option<Self> {
                match c { $($c => Some($var),)+ _ => None }
            }
        }
    };
}

fix_char!(Side { Side::Buy => b'1', Side::Sell => b'2' });
fix_char!(OrderType { OrderType::Market => b'1', OrderType::Limit => b'2', OrderType::Stop => b'3' });
fix_char!(TimeInForce {
    TimeInForce::Day => b'0',
    TimeInForce::GoodTillCancel => b'1',
    TimeInForce::ImmediateOrCancel => b'3',
    TimeInForce::FillOrKill => b'4',
});
fix_char!(ExecType {
    ExecType::New => b'0',
    ExecType::Canceled => b'4',
    ExecType::Replaced => b'5',
    ExecType::Rejected => b'8',
    ExecType::Expired => b'C',
    ExecType::Trade => b'F',
    ExecType::OrderStatus => b'I',
});
fix_char!(OrderStatus {
    OrderStatus::New => b'0',
    OrderStatus::PartiallyFilled => b'1',
    OrderStatus::Filled => b'2',
    OrderStatus::Canceled => b'4',
    OrderStatus::Replaced => b'5',
    OrderStatus::Rejected => b'8',
    OrderStatus::Expired => b'C',
});

/// MDEntryType(269).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MdEntryType {
    Bid,
    Offer,
    Trade,
}
fix_char!(MdEntryType { MdEntryType::Bid => b'0', MdEntryType::Offer => b'1', MdEntryType::Trade => b'2' });

/// MDUpdateAction(279).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MdUpdateAction {
    New,
    Change,
    Delete,
}
fix_char!(MdUpdateAction { MdUpdateAction::New => b'0', MdUpdateAction::Change => b'1', MdUpdateAction::Delete => b'2' });

/// SubscriptionRequestType(263).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubscriptionRequestType {
    Snapshot,
    Subscribe,
    Unsubscribe,
}
fix_char!(SubscriptionRequestType {
    SubscriptionRequestType::Snapshot => b'0',
    SubscriptionRequestType::Subscribe => b'1',
    SubscriptionRequestType::Unsubscribe => b'2',
});

/// CxlRejResponseTo(434).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CxlRejResponseTo {
    Cancel,
    Replace,
}
fix_char!(CxlRejResponseTo { CxlRejResponseTo::Cancel => b'1', CxlRejResponseTo::Replace => b'2' });
