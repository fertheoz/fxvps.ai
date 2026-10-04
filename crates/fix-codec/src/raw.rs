//! Framing and borrowed (zero-copy) field access.

use domain::Fixed;

use crate::DecodeError;

pub const SOH: u8 = 0x01;
/// Hard cap on a single message; protects against hostile BodyLength values.
pub const MAX_MESSAGE_LEN: usize = 1 << 20;

/// One `tag=value` pair borrowing from the input buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field<'a> {
    pub tag: u32,
    pub value: &'a [u8],
}

/// Computes the FIX checksum (byte sum modulo 256).
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b))
}

fn parse_uint(v: &[u8]) -> Option<u64> {
    if v.is_empty() || v.len() > 19 {
        return None;
    }
    let mut n: u64 = 0;
    for &c in v {
        if !c.is_ascii_digit() {
            return None;
        }
        n = n * 10 + u64::from(c - b'0');
    }
    Some(n)
}

fn find_soh(buf: &[u8], from: usize) -> Option<usize> {
    buf.get(from..)?
        .iter()
        .position(|&b| b == SOH)
        .map(|p| p + from)
}

/// Determines whether `buf` starts with a complete FIX message.
///
/// Returns `Ok(Some(len))` with the full frame length, `Ok(None)` if more bytes are
/// needed, or an error if the prefix can never become a valid message (the caller
/// should then drop the connection: FIX framing cannot be resynchronized reliably).
/// Checksum and trailer are validated here.
pub fn frame_len(buf: &[u8]) -> Result<Option<usize>, DecodeError> {
    // "8=" prefix
    for (i, want) in b"8=".iter().enumerate() {
        match buf.get(i) {
            None => return Ok(None),
            Some(c) if c == want => {}
            Some(_) => return Err(DecodeError::BadBeginString),
        }
    }
    let Some(begin_end) = find_soh(buf, 2) else {
        if buf.len() > 32 {
            return Err(DecodeError::BadBeginString);
        }
        return Ok(None);
    };
    if begin_end == 2 {
        return Err(DecodeError::BadBeginString);
    }
    let bl_start = begin_end + 1;
    for (i, want) in b"9=".iter().enumerate() {
        match buf.get(bl_start + i) {
            None => return Ok(None),
            Some(c) if c == want => {}
            Some(_) => return Err(DecodeError::BadBodyLength),
        }
    }
    let Some(bl_end) = find_soh(buf, bl_start + 2) else {
        if buf.len() > bl_start + 2 + 8 {
            return Err(DecodeError::BadBodyLength);
        }
        return Ok(None);
    };
    let body_len = parse_uint(&buf[bl_start + 2..bl_end]).ok_or(DecodeError::BadBodyLength)?;
    let body_len = usize::try_from(body_len).map_err(|_| DecodeError::TooLarge)?;
    let body_start = bl_end + 1;
    let trailer_start = body_start
        .checked_add(body_len)
        .ok_or(DecodeError::TooLarge)?;
    let total = trailer_start + 7;
    if total > MAX_MESSAGE_LEN {
        return Err(DecodeError::TooLarge);
    }
    if buf.len() < total {
        return Ok(None);
    }
    let trailer = &buf[trailer_start..total];
    if &trailer[..3] != b"10=" || trailer[6] != SOH || buf[trailer_start - 1] != SOH {
        return Err(DecodeError::BadBodyLength);
    }
    let declared = parse_uint(&trailer[3..6]).ok_or(DecodeError::Malformed("checksum"))?;
    let actual = checksum(&buf[..trailer_start]);
    if declared != u64::from(actual) {
        return Err(DecodeError::BadChecksum {
            declared: u8::try_from(declared).unwrap_or(u8::MAX),
            actual,
        });
    }
    Ok(Some(total))
}

/// A validated message whose fields borrow from the input buffer.
#[derive(Clone, Debug)]
pub struct RawMessage<'a> {
    fields: Vec<Field<'a>>,
}

impl<'a> RawMessage<'a> {
    /// Parses exactly one complete frame. Trailing bytes are an error.
    pub fn parse(buf: &'a [u8]) -> Result<Self, DecodeError> {
        let len = frame_len(buf)?.ok_or(DecodeError::Incomplete)?;
        if len != buf.len() {
            return Err(DecodeError::Malformed("trailing bytes after message"));
        }
        let mut fields = Vec::with_capacity(32);
        let mut pos = 0;
        while pos < len {
            let end = find_soh(buf, pos).ok_or(DecodeError::Malformed("unterminated field"))?;
            let item = &buf[pos..end];
            let eq = item
                .iter()
                .position(|&b| b == b'=')
                .ok_or(DecodeError::Malformed("field without '='"))?;
            let tag = parse_uint(&item[..eq])
                .and_then(|t| u32::try_from(t).ok())
                .filter(|t| *t > 0)
                .ok_or(DecodeError::Malformed("invalid tag"))?;
            let value = &item[eq + 1..];
            if value.is_empty() {
                return Err(DecodeError::InvalidValue { tag });
            }
            fields.push(Field { tag, value });
            pos = end + 1;
        }
        // Standard header ordering: 8, 9, 35 must be first three.
        if fields.len() < 4 || fields[0].tag != 8 || fields[1].tag != 9 || fields[2].tag != 35 {
            return Err(DecodeError::Malformed("header must start with 8, 9, 35"));
        }
        Ok(RawMessage { fields })
    }

    pub fn fields(&self) -> &[Field<'a>] {
        &self.fields
    }

    pub fn view(&self) -> FieldView<'a, '_> {
        FieldView(&self.fields)
    }

    pub fn msg_type(&self) -> &'a [u8] {
        self.fields[2].value
    }
}

/// Typed accessors over a slice of fields (a whole message or one group entry).
#[derive(Clone, Copy, Debug)]
pub struct FieldView<'a, 'b>(pub &'b [Field<'a>]);

impl<'a> FieldView<'a, '_> {
    pub fn get(&self, tag: u32) -> Option<&'a [u8]> {
        self.0.iter().find(|f| f.tag == tag).map(|f| f.value)
    }

    pub fn req(&self, tag: u32) -> Result<&'a [u8], DecodeError> {
        self.get(tag).ok_or(DecodeError::MissingField { tag })
    }

    pub fn str(&self, tag: u32) -> Result<&'a str, DecodeError> {
        std::str::from_utf8(self.req(tag)?).map_err(|_| DecodeError::InvalidValue { tag })
    }

    pub fn opt_str(&self, tag: u32) -> Result<Option<&'a str>, DecodeError> {
        match self.get(tag) {
            None => Ok(None),
            Some(v) => std::str::from_utf8(v)
                .map(Some)
                .map_err(|_| DecodeError::InvalidValue { tag }),
        }
    }

    pub fn string(&self, tag: u32) -> Result<String, DecodeError> {
        self.str(tag).map(str::to_owned)
    }

    pub fn opt_string(&self, tag: u32) -> Result<Option<String>, DecodeError> {
        self.opt_str(tag).map(|o| o.map(str::to_owned))
    }

    pub fn uint(&self, tag: u32) -> Result<u64, DecodeError> {
        parse_uint(self.req(tag)?).ok_or(DecodeError::InvalidValue { tag })
    }

    pub fn opt_uint(&self, tag: u32) -> Result<Option<u64>, DecodeError> {
        self.get(tag)
            .map(|v| parse_uint(v).ok_or(DecodeError::InvalidValue { tag }))
            .transpose()
    }

    pub fn fixed(&self, tag: u32) -> Result<Fixed, DecodeError> {
        Fixed::parse_bytes(self.req(tag)?).map_err(|_| DecodeError::InvalidValue { tag })
    }

    pub fn opt_fixed(&self, tag: u32) -> Result<Option<Fixed>, DecodeError> {
        self.get(tag)
            .map(|v| Fixed::parse_bytes(v).map_err(|_| DecodeError::InvalidValue { tag }))
            .transpose()
    }

    pub fn char(&self, tag: u32) -> Result<u8, DecodeError> {
        match self.req(tag)? {
            [c] => Ok(*c),
            _ => Err(DecodeError::InvalidValue { tag }),
        }
    }

    pub fn opt_char(&self, tag: u32) -> Result<Option<u8>, DecodeError> {
        match self.get(tag) {
            None => Ok(None),
            Some([c]) => Ok(Some(*c)),
            Some(_) => Err(DecodeError::InvalidValue { tag }),
        }
    }

    pub fn bool(&self, tag: u32) -> Result<Option<bool>, DecodeError> {
        match self.get(tag) {
            None => Ok(None),
            Some(b"Y") => Ok(Some(true)),
            Some(b"N") => Ok(Some(false)),
            Some(_) => Err(DecodeError::InvalidValue { tag }),
        }
    }

    /// Splits a repeating group. `first` is the delimiter tag that starts each entry,
    /// `members` lists every tag allowed inside an entry (including `first`).
    /// Returns an empty list when the count tag is absent.
    pub fn group(
        &self,
        count_tag: u32,
        first: u32,
        members: &[u32],
    ) -> Result<Vec<FieldView<'a, '_>>, DecodeError> {
        let Some(idx) = self.0.iter().position(|f| f.tag == count_tag) else {
            return Ok(Vec::new());
        };
        let n = parse_uint(self.0[idx].value)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(DecodeError::InvalidValue { tag: count_tag })?;
        let mut out = Vec::with_capacity(n);
        let mut i = idx + 1;
        while out.len() < n {
            if self.0.get(i).map(|f| f.tag) != Some(first) {
                return Err(DecodeError::BadGroup { tag: count_tag });
            }
            let start = i;
            i += 1;
            while let Some(f) = self.0.get(i) {
                if f.tag == first || !members.contains(&f.tag) {
                    break;
                }
                i += 1;
            }
            out.push(FieldView(&self.0[start..i]));
        }
        Ok(out)
    }
}
