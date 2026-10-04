//! Fixed-point decimal used for every price and quantity. Floating point is never
//! used for money.

use std::fmt;
use std::ops::{Add, Neg, Sub};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Number of decimal places carried by [`Fixed`].
pub const SCALE_DIGITS: u32 = 8;
/// `10^SCALE_DIGITS`.
pub const SCALE: i64 = 100_000_000;

/// Signed fixed-point decimal with 8 fractional digits stored in an `i64`.
///
/// Range is roughly +/- 92 billion, which is ample for FX prices and lot sizes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fixed(i64);

/// Price alias (same representation as [`Fixed`]).
pub type Price = Fixed;
/// Quantity alias (same representation as [`Fixed`]).
pub type Qty = Fixed;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FixedParseError {
    #[error("empty decimal")]
    Empty,
    #[error("invalid character in decimal")]
    InvalidChar,
    #[error("more than {SCALE_DIGITS} fractional digits")]
    TooPrecise,
    #[error("decimal out of range")]
    Overflow,
}

impl Fixed {
    pub const ZERO: Fixed = Fixed(0);

    /// Builds from the raw scaled integer (`units * 10^-8`).
    pub const fn from_raw(raw: i64) -> Self {
        Fixed(raw)
    }

    /// Raw scaled integer.
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Builds from a whole number.
    pub const fn from_int(v: i64) -> Self {
        Fixed(v * SCALE)
    }

    /// Builds `mantissa * 10^-decimals`; panics in const context if `decimals > 8`.
    pub const fn from_parts(mantissa: i64, decimals: u32) -> Self {
        assert!(decimals <= SCALE_DIGITS);
        Fixed(mantissa * 10i64.pow(SCALE_DIGITS - decimals))
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    pub fn checked_add(self, o: Fixed) -> Option<Fixed> {
        self.0.checked_add(o.0).map(Fixed)
    }

    pub fn checked_sub(self, o: Fixed) -> Option<Fixed> {
        self.0.checked_sub(o.0).map(Fixed)
    }

    /// `self * other`, truncated toward zero, computed in i128.
    pub fn checked_mul(self, o: Fixed) -> Option<Fixed> {
        let v = (self.0 as i128 * o.0 as i128) / SCALE as i128;
        i64::try_from(v).ok().map(Fixed)
    }

    /// `self / other`, truncated toward zero, computed in i128.
    pub fn checked_div(self, o: Fixed) -> Option<Fixed> {
        if o.0 == 0 {
            return None;
        }
        let v = (self.0 as i128 * SCALE as i128) / o.0 as i128;
        i64::try_from(v).ok().map(Fixed)
    }

    /// Rounds down to a multiple of `step` (e.g. a tick size). `step` must be positive.
    pub fn floor_to(self, step: Fixed) -> Fixed {
        if step.0 <= 0 {
            return self;
        }
        Fixed(self.0.div_euclid(step.0) * step.0)
    }

    /// Parses an ASCII decimal (`-12.345`) without allocating.
    pub fn parse_bytes(s: &[u8]) -> Result<Fixed, FixedParseError> {
        let (neg, digits) = match s.first() {
            None => return Err(FixedParseError::Empty),
            Some(b'-') => (true, &s[1..]),
            Some(b'+') => (false, &s[1..]),
            Some(_) => (false, s),
        };
        if digits.is_empty() || digits == b"." {
            return Err(FixedParseError::Empty);
        }
        let mut int: i64 = 0;
        let mut frac: i64 = 0;
        let mut frac_digits = 0u32;
        let mut seen_dot = false;
        for &c in digits {
            match c {
                b'0'..=b'9' => {
                    let d = i64::from(c - b'0');
                    if seen_dot {
                        if frac_digits == SCALE_DIGITS {
                            // Allow trailing zeros beyond the scale, reject real precision loss.
                            if d != 0 {
                                return Err(FixedParseError::TooPrecise);
                            }
                            continue;
                        }
                        frac = frac * 10 + d;
                        frac_digits += 1;
                    } else {
                        int = int
                            .checked_mul(10)
                            .and_then(|v| v.checked_add(d))
                            .ok_or(FixedParseError::Overflow)?;
                    }
                }
                b'.' if !seen_dot => seen_dot = true,
                _ => return Err(FixedParseError::InvalidChar),
            }
        }
        let frac = frac * 10i64.pow(SCALE_DIGITS - frac_digits);
        let raw = int
            .checked_mul(SCALE)
            .and_then(|v| v.checked_add(frac))
            .ok_or(FixedParseError::Overflow)?;
        Ok(Fixed(if neg { -raw } else { raw }))
    }

    /// Writes the canonical representation (no trailing fractional zeros) into `out`.
    pub fn write_to(self, out: &mut Vec<u8>) {
        let neg = self.0 < 0;
        let abs = self.0.unsigned_abs();
        let int = abs / SCALE as u64;
        let mut frac = abs % SCALE as u64;
        if neg {
            out.push(b'-');
        }
        let mut buf = itoa_u64(int);
        out.extend_from_slice(buf.as_bytes());
        if frac != 0 {
            let mut width = SCALE_DIGITS as usize;
            while frac.is_multiple_of(10) {
                frac /= 10;
                width -= 1;
            }
            out.push(b'.');
            buf = itoa_u64(frac);
            out.extend(std::iter::repeat_n(b'0', width - buf.len()));
            out.extend_from_slice(buf.as_bytes());
        }
    }
}

fn itoa_u64(v: u64) -> String {
    v.to_string()
}

impl fmt::Display for Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut v = Vec::with_capacity(24);
        self.write_to(&mut v);
        // write_to only emits ASCII.
        f.write_str(std::str::from_utf8(&v).map_err(|_| fmt::Error)?)
    }
}

impl FromStr for Fixed {
    type Err = FixedParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Fixed::parse_bytes(s.as_bytes())
    }
}

impl Add for Fixed {
    type Output = Fixed;
    fn add(self, o: Fixed) -> Fixed {
        Fixed(self.0 + o.0)
    }
}

impl Sub for Fixed {
    type Output = Fixed;
    fn sub(self, o: Fixed) -> Fixed {
        Fixed(self.0 - o.0)
    }
}

impl Neg for Fixed {
    type Output = Fixed;
    fn neg(self) -> Fixed {
        Fixed(-self.0)
    }
}

impl Serialize for Fixed {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Fixed {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Str(String),
            Int(i64),
        }
        match Repr::deserialize(d)? {
            Repr::Str(s) => s.parse().map_err(serde::de::Error::custom),
            Repr::Int(i) => Ok(Fixed::from_int(i)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parse_and_format() {
        assert_eq!("1.23456".parse::<Fixed>().unwrap().raw(), 123_456_000);
        assert_eq!("-0.5".parse::<Fixed>().unwrap().raw(), -50_000_000);
        assert_eq!("100".parse::<Fixed>().unwrap().to_string(), "100");
        assert_eq!("1.10000".parse::<Fixed>().unwrap().to_string(), "1.1");
        assert_eq!(
            "0.00000001".parse::<Fixed>().unwrap().to_string(),
            "0.00000001"
        );
        assert_eq!("1.0000000000".parse::<Fixed>().unwrap(), Fixed::from_int(1));
        assert_eq!(
            "1.000000001".parse::<Fixed>(),
            Err(FixedParseError::TooPrecise)
        );
        assert!("1.2.3".parse::<Fixed>().is_err());
        assert!("abc".parse::<Fixed>().is_err());
        assert!("".parse::<Fixed>().is_err());
        assert!("99999999999999999999".parse::<Fixed>().is_err());
    }

    #[test]
    fn arithmetic() {
        let p = Fixed::from_parts(110_000, 5);
        let q = Fixed::from_int(2);
        assert_eq!(p.checked_mul(q).unwrap().to_string(), "2.2");
        assert_eq!(p.checked_div(q).unwrap().to_string(), "0.55");
        assert_eq!(
            Fixed::from_parts(110_007, 5).floor_to(Fixed::from_parts(5, 5)),
            Fixed::from_parts(110_005, 5)
        );
    }

    #[test]
    fn serde_as_string() {
        let v = Fixed::from_parts(108_512, 5);
        let j = serde_json::to_string(&v).unwrap();
        assert_eq!(j, "\"1.08512\"");
        assert_eq!(serde_json::from_str::<Fixed>(&j).unwrap(), v);
    }

    proptest! {
        #[test]
        fn roundtrip(raw in any::<i64>()) {
            let v = Fixed::from_raw(raw);
            prop_assert_eq!(v.to_string().parse::<Fixed>().unwrap(), v);
        }
    }
}
