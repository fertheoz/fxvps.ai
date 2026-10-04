//! Fixed-point monetary primitives for the fxvps.ai core.
//!
//! * [`Price`] and [`Qty`] are `i64` values scaled by [`SCALE`] (1e8).
//! * [`Money`] is an `i128` amount in the *minor units* of its [`Currency`]
//!   (ISO 4217 exponent, e.g. cents for USD, yen for JPY).
//! * All arithmetic is checked; every lossy operation takes an explicit
//!   [`Rounding`] mode. Floats are never used.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Scale of [`Price`] and [`Qty`] raw values (8 decimal places).
pub const SCALE: i64 = 100_000_000;
/// Number of decimal places represented by [`SCALE`].
pub const SCALE_DIGITS: u32 = 8;

/// Errors produced by checked money arithmetic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoneyError {
    #[error("currency mismatch: {0} vs {1}")]
    CurrencyMismatch(Currency, Currency),
    #[error("arithmetic overflow")]
    Overflow,
    #[error("division by zero")]
    DivByZero,
    #[error("parse error: {0}")]
    Parse(String),
}

/// Rounding modes for lossy fixed-point operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rounding {
    /// Banker's rounding: ties to even.
    HalfEven,
    /// Ties away from zero.
    HalfUp,
    /// Toward zero (truncate).
    Down,
    /// Away from zero.
    Up,
    /// Toward negative infinity.
    Floor,
    /// Toward positive infinity.
    Ceiling,
}

/// Divides `n / d` rounding according to `mode`. Returns `None` on division
/// by zero or overflow.
pub fn div_round(n: i128, d: i128, mode: Rounding) -> Option<i128> {
    if d == 0 {
        return None;
    }
    let q = n.checked_div(d)?;
    let r = n.checked_rem(d)?;
    if r == 0 {
        return Some(q);
    }
    // sign of the exact quotient
    let positive = (n < 0) == (d < 0);
    let away = |q: i128| if positive { q + 1 } else { q - 1 };
    let res = match mode {
        Rounding::Down => q,
        Rounding::Up => away(q),
        Rounding::Floor => {
            if positive {
                q
            } else {
                q - 1
            }
        }
        Rounding::Ceiling => {
            if positive {
                q + 1
            } else {
                q
            }
        }
        Rounding::HalfUp | Rounding::HalfEven => {
            let twice = r.unsigned_abs().checked_mul(2)?;
            let da = d.unsigned_abs();
            match twice.cmp(&da) {
                std::cmp::Ordering::Less => q,
                std::cmp::Ordering::Greater => away(q),
                std::cmp::Ordering::Equal => {
                    if mode == Rounding::HalfUp || q % 2 != 0 {
                        away(q)
                    } else {
                        q
                    }
                }
            }
        }
    };
    Some(res)
}

fn pow10(e: u32) -> i128 {
    10i128.pow(e)
}

/// Parses a decimal string into an integer scaled by `10^digits`.
/// Rejects inputs with more fractional digits than `digits`.
pub fn parse_scaled(s: &str, digits: u32) -> Result<i128, MoneyError> {
    let err = || MoneyError::Parse(s.to_string());
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, f),
        None => (body, ""),
    };
    if (int.is_empty() && frac.is_empty())
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
        || frac.len() > digits as usize
    {
        return Err(err());
    }
    let mut v: i128 = 0;
    for b in int.bytes() {
        v = v
            .checked_mul(10)
            .and_then(|v| v.checked_add((b - b'0') as i128))
            .ok_or(MoneyError::Overflow)?;
    }
    let mut f: i128 = 0;
    for b in frac.bytes() {
        f = f * 10 + (b - b'0') as i128;
    }
    f *= pow10(digits - frac.len() as u32);
    let v = v
        .checked_mul(pow10(digits))
        .and_then(|v| v.checked_add(f))
        .ok_or(MoneyError::Overflow)?;
    Ok(if neg { -v } else { v })
}

/// Formats an integer scaled by `10^digits` as a decimal string.
pub fn format_scaled(v: i128, digits: u32) -> String {
    if digits == 0 {
        return v.to_string();
    }
    let p = pow10(digits);
    let sign = if v < 0 { "-" } else { "" };
    let a = v.unsigned_abs();
    format!(
        "{sign}{}.{:0width$}",
        a / p as u128,
        a % p as u128,
        width = digits as usize
    )
}

// ---------------------------------------------------------------------------
// Currency
// ---------------------------------------------------------------------------

/// ISO-4217-like three letter currency code (also used for metals/crypto).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Currency([u8; 3]);

impl Currency {
    pub const USD: Currency = Currency(*b"USD");
    pub const EUR: Currency = Currency(*b"EUR");
    pub const GBP: Currency = Currency(*b"GBP");
    pub const JPY: Currency = Currency(*b"JPY");
    pub const CHF: Currency = Currency(*b"CHF");
    pub const AUD: Currency = Currency(*b"AUD");
    pub const CAD: Currency = Currency(*b"CAD");
    pub const XAU: Currency = Currency(*b"XAU");

    /// Builds a currency from three upper-case ASCII letters.
    pub const fn new(code: [u8; 3]) -> Currency {
        Currency(code)
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("???")
    }

    /// Number of decimal places of the minor unit.
    pub fn minor_exponent(&self) -> u32 {
        match &self.0 {
            b"JPY" | b"KRW" | b"HUF" => 0,
            b"KWD" | b"BHD" | b"OMR" => 3,
            b"BTC" | b"ETH" => 8,
            _ => 2,
        }
    }
}

impl FromStr for Currency {
    type Err = MoneyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let b = s.as_bytes();
        if b.len() != 3 || !b.iter().all(|c| c.is_ascii_uppercase()) {
            return Err(MoneyError::Parse(s.to_string()));
        }
        Ok(Currency([b[0], b[1], b[2]]))
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl Serialize for Currency {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// Price / Qty
// ---------------------------------------------------------------------------

macro_rules! fixed_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl $name {
            pub const ZERO: $name = $name(0);
            /// Builds from a raw value scaled by [`SCALE`].
            pub const fn from_raw(raw: i64) -> Self { $name(raw) }
            /// Builds from a whole number of units.
            pub const fn from_units(u: i64) -> Self { $name(u * SCALE) }
            pub const fn raw(self) -> i64 { self.0 }
            pub fn is_zero(self) -> bool { self.0 == 0 }
            pub fn is_positive(self) -> bool { self.0 > 0 }
            pub fn checked_add(self, o: Self) -> Option<Self> { self.0.checked_add(o.0).map($name) }
            pub fn checked_sub(self, o: Self) -> Option<Self> { self.0.checked_sub(o.0).map($name) }
            pub fn abs(self) -> Self { $name(self.0.abs()) }
            /// Multiplies by an integer factor.
            pub fn checked_mul_int(self, k: i64) -> Option<Self> { self.0.checked_mul(k).map($name) }
            /// Rounds to a multiple of `step` using `mode`.
            pub fn round_to(self, step: Self, mode: Rounding) -> Option<Self> {
                if step.0 <= 0 { return None; }
                let q = div_round(self.0 as i128, step.0 as i128, mode)?;
                i64::try_from(q.checked_mul(step.0 as i128)?).ok().map($name)
            }
            /// `self * other` (both scaled), result scaled.
            pub fn mul_scaled(self, other: i64, mode: Rounding) -> Option<Self> {
                let v = div_round(self.0 as i128 * other as i128, SCALE as i128, mode)?;
                i64::try_from(v).ok().map($name)
            }
        }
        impl FromStr for $name {
            type Err = MoneyError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let v = parse_scaled(s, SCALE_DIGITS)?;
                i64::try_from(v).map($name).map_err(|_| MoneyError::Overflow)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let s = format_scaled(self.0 as i128, SCALE_DIGITS);
                let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
                f.write_str(s)
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self)
            }
        }
        impl std::ops::Neg for $name {
            type Output = $name;
            fn neg(self) -> $name { $name(-self.0) }
        }
    };
}

fixed_type!(
    /// Price scaled by 1e8 (e.g. `1.23456` = `123_456_000`).
    Price
);
fixed_type!(
    /// Quantity in lots scaled by 1e8 (e.g. `0.01` lot = `1_000_000`).
    Qty
);

/// Parses a decimal literal, panicking on invalid input. Intended for tests
/// and static configuration.
pub fn px(s: &str) -> Price {
    s.parse().expect("invalid price literal")
}
/// Parses a quantity literal, panicking on invalid input.
pub fn qty(s: &str) -> Qty {
    s.parse().expect("invalid qty literal")
}

// ---------------------------------------------------------------------------
// Money
// ---------------------------------------------------------------------------

/// Amount of a currency in minor units.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Money {
    /// Amount in minor units (cents for USD).
    pub minor: i128,
    pub currency: Currency,
}

impl Money {
    pub const fn new(minor: i128, currency: Currency) -> Money {
        Money { minor, currency }
    }
    pub const fn zero(currency: Currency) -> Money {
        Money { minor: 0, currency }
    }
    /// Parses a major-unit decimal, e.g. `Money::parse("12.34", USD)`.
    pub fn parse(s: &str, currency: Currency) -> Result<Money, MoneyError> {
        Ok(Money {
            minor: parse_scaled(s, currency.minor_exponent())?,
            currency,
        })
    }
    /// Builds from a value scaled by [`SCALE`] (1e8), rounding to minor units.
    pub fn from_scaled(v: i128, currency: Currency, mode: Rounding) -> Result<Money, MoneyError> {
        let d = pow10(SCALE_DIGITS - currency.minor_exponent());
        let minor = div_round(v, d, mode).ok_or(MoneyError::Overflow)?;
        Ok(Money { minor, currency })
    }
    /// Value scaled by [`SCALE`] (1e8).
    pub fn to_scaled(self) -> Result<i128, MoneyError> {
        self.minor
            .checked_mul(pow10(SCALE_DIGITS - self.currency.minor_exponent()))
            .ok_or(MoneyError::Overflow)
    }
    pub fn is_zero(self) -> bool {
        self.minor == 0
    }
    pub fn is_negative(self) -> bool {
        self.minor < 0
    }
    pub fn is_positive(self) -> bool {
        self.minor > 0
    }
    pub fn abs(self) -> Money {
        Money::new(self.minor.abs(), self.currency)
    }
    fn same(self, o: Money) -> Result<(), MoneyError> {
        if self.currency != o.currency {
            Err(MoneyError::CurrencyMismatch(self.currency, o.currency))
        } else {
            Ok(())
        }
    }
    pub fn checked_add(self, o: Money) -> Result<Money, MoneyError> {
        self.same(o)?;
        let m = self
            .minor
            .checked_add(o.minor)
            .ok_or(MoneyError::Overflow)?;
        Ok(Money::new(m, self.currency))
    }
    pub fn checked_sub(self, o: Money) -> Result<Money, MoneyError> {
        self.same(o)?;
        let m = self
            .minor
            .checked_sub(o.minor)
            .ok_or(MoneyError::Overflow)?;
        Ok(Money::new(m, self.currency))
    }
    pub fn checked_neg(self) -> Result<Money, MoneyError> {
        Ok(Money::new(
            self.minor.checked_neg().ok_or(MoneyError::Overflow)?,
            self.currency,
        ))
    }
    /// Multiplies by a scaled ratio (`num / den`), rounding.
    pub fn mul_ratio(self, num: i128, den: i128, mode: Rounding) -> Result<Money, MoneyError> {
        if den == 0 {
            return Err(MoneyError::DivByZero);
        }
        let n = self.minor.checked_mul(num).ok_or(MoneyError::Overflow)?;
        Ok(Money::new(
            div_round(n, den, mode).ok_or(MoneyError::Overflow)?,
            self.currency,
        ))
    }
    /// Converts into `to` multiplying by `rate` (units of `to` per unit of
    /// `self.currency`). Single rounding step.
    pub fn convert_mul(
        self,
        rate: Price,
        to: Currency,
        mode: Rounding,
    ) -> Result<Money, MoneyError> {
        let num = self
            .minor
            .checked_mul(pow10(to.minor_exponent()))
            .and_then(|v| v.checked_mul(rate.0 as i128))
            .ok_or(MoneyError::Overflow)?;
        let den = pow10(self.currency.minor_exponent()) * SCALE as i128;
        Ok(Money::new(
            div_round(num, den, mode).ok_or(MoneyError::Overflow)?,
            to,
        ))
    }
    /// Converts into `to` dividing by `rate` (units of `self.currency` per
    /// unit of `to`). Single rounding step.
    pub fn convert_div(
        self,
        rate: Price,
        to: Currency,
        mode: Rounding,
    ) -> Result<Money, MoneyError> {
        if rate.0 == 0 {
            return Err(MoneyError::DivByZero);
        }
        let num = self
            .minor
            .checked_mul(pow10(to.minor_exponent()))
            .and_then(|v| v.checked_mul(SCALE as i128))
            .ok_or(MoneyError::Overflow)?;
        let den = pow10(self.currency.minor_exponent()) * rate.0 as i128;
        Ok(Money::new(
            div_round(num, den, mode).ok_or(MoneyError::Overflow)?,
            to,
        ))
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}",
            format_scaled(self.minor, self.currency.minor_exponent()),
            self.currency
        )
    }
}
impl fmt::Debug for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn rounding_modes() {
        use Rounding::*;
        let cases = [
            (5, 2, HalfEven, 2),
            (7, 2, HalfEven, 4),
            (5, 2, HalfUp, 3),
            (-5, 2, HalfUp, -3),
            (-5, 2, HalfEven, -2),
            (7, 3, Down, 2),
            (-7, 3, Down, -2),
            (7, 3, Up, 3),
            (-7, 3, Up, -3),
            (-7, 3, Floor, -3),
            (7, 3, Floor, 2),
            (-7, 3, Ceiling, -2),
            (7, 3, Ceiling, 3),
            (8, 3, HalfEven, 3),
        ];
        for (n, d, m, want) in cases {
            assert_eq!(div_round(n, d, m), Some(want), "{n}/{d} {m:?}");
        }
        assert_eq!(div_round(1, 0, Down), None);
    }

    #[test]
    fn parse_and_format() {
        assert_eq!(px("1.23456").raw(), 123_456_000);
        assert_eq!(px("-0.5").raw(), -50_000_000);
        assert_eq!(qty("0.01").to_string(), "0.01");
        assert_eq!(px("100").to_string(), "100");
        assert!("1.123456789".parse::<Price>().is_err());
        assert!("abc".parse::<Price>().is_err());
        assert!(".".parse::<Price>().is_err());
        let m = Money::parse("12.34", Currency::USD).unwrap();
        assert_eq!(m.minor, 1234);
        assert_eq!(m.to_string(), "12.34 USD");
        assert_eq!(
            Money::parse("-0.05", Currency::USD).unwrap().to_string(),
            "-0.05 USD"
        );
        assert_eq!(
            Money::parse("100", Currency::JPY).unwrap().to_string(),
            "100 JPY"
        );
        assert!(Money::parse("1.5", Currency::JPY).is_err());
    }

    #[test]
    fn checked_ops() {
        let a = Money::new(100, Currency::USD);
        let b = Money::new(50, Currency::EUR);
        assert!(matches!(
            a.checked_add(b),
            Err(MoneyError::CurrencyMismatch(..))
        ));
        assert_eq!(a.checked_sub(a).unwrap(), Money::zero(Currency::USD));
        assert_eq!(
            Money::new(i128::MAX, Currency::USD).checked_add(Money::new(1, Currency::USD)),
            Err(MoneyError::Overflow)
        );
        assert_eq!(Price(i64::MAX).checked_add(Price(1)), None);
    }

    #[test]
    fn conversion() {
        // 100 EUR * 1.10 = 110 USD
        let eur = Money::parse("100", Currency::EUR).unwrap();
        let usd = eur
            .convert_mul(px("1.1"), Currency::USD, Rounding::HalfEven)
            .unwrap();
        assert_eq!(usd, Money::parse("110", Currency::USD).unwrap());
        // 15000 JPY / 150 = 100 USD
        let jpy = Money::parse("15000", Currency::JPY).unwrap();
        let usd = jpy
            .convert_div(px("150"), Currency::USD, Rounding::HalfEven)
            .unwrap();
        assert_eq!(usd, Money::parse("100", Currency::USD).unwrap());
        assert_eq!(
            jpy.convert_div(Price::ZERO, Currency::USD, Rounding::Down),
            Err(MoneyError::DivByZero)
        );
    }

    #[test]
    fn scaled_roundtrip_and_lot_rounding() {
        let m = Money::from_scaled(123_456_789, Currency::USD, Rounding::HalfEven).unwrap();
        assert_eq!(m.minor, 123); // 1.23456789 -> 1.23
        assert_eq!(m.to_scaled().unwrap(), 123_000_000);
        assert_eq!(
            qty("0.137").round_to(qty("0.01"), Rounding::Down),
            Some(qty("0.13"))
        );
    }

    #[test]
    fn serde_roundtrip() {
        let m = Money::parse("1.5", Currency::GBP).unwrap();
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(s, r#"{"minor":150,"currency":"GBP"}"#);
        assert_eq!(serde_json::from_str::<Money>(&s).unwrap(), m);
        assert_eq!(serde_json::to_string(&px("1.2")).unwrap(), "120000000");
        assert!(serde_json::from_str::<Currency>("\"usd\"").is_err());
    }

    proptest! {
        #[test]
        fn div_round_bounds(n in -1_000_000_000i128..1_000_000_000, d in 1i128..100_000) {
            let fl = div_round(n, d, Rounding::Floor).unwrap();
            let ce = div_round(n, d, Rounding::Ceiling).unwrap();
            prop_assert!(fl * d <= n && n <= ce * d);
            prop_assert!(ce - fl <= 1);
            for m in [Rounding::HalfEven, Rounding::HalfUp, Rounding::Down, Rounding::Up] {
                let r = div_round(n, d, m).unwrap();
                prop_assert!(r == fl || r == ce);
            }
        }

        #[test]
        fn parse_format_roundtrip(v in -10_000_000_000_000i64..10_000_000_000_000) {
            let p = Price(v);
            prop_assert_eq!(p.to_string().parse::<Price>().unwrap(), p);
        }

        #[test]
        fn add_sub_inverse(a in -1_000_000_000_000i128..1_000_000_000_000, b in -1_000_000_000_000i128..1_000_000_000_000) {
            let x = Money::new(a, Currency::USD);
            let y = Money::new(b, Currency::USD);
            prop_assert_eq!(x.checked_add(y).unwrap().checked_sub(y).unwrap(), x);
        }
    }
}
