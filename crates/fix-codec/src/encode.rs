//! Message builder computing BodyLength(9) and CheckSum(10).

use domain::Fixed;

use crate::raw::{checksum, SOH};
use crate::EncodeError;

/// Accumulates body fields (everything after `9=`), then frames them.
#[derive(Debug, Default)]
pub struct Encoder {
    body: Vec<u8>,
    error: Option<EncodeError>,
}

impl Encoder {
    pub fn new() -> Self {
        Encoder {
            body: Vec::with_capacity(256),
            error: None,
        }
    }

    fn tag(&mut self, tag: u32) {
        self.body.extend_from_slice(tag.to_string().as_bytes());
        self.body.push(b'=');
    }

    pub fn bytes(&mut self, tag: u32, v: &[u8]) -> &mut Self {
        if v.is_empty() || v.contains(&SOH) {
            self.error.get_or_insert(EncodeError::InvalidValue { tag });
            return self;
        }
        self.tag(tag);
        self.body.extend_from_slice(v);
        self.body.push(SOH);
        self
    }

    pub fn str(&mut self, tag: u32, v: &str) -> &mut Self {
        self.bytes(tag, v.as_bytes())
    }

    pub fn opt_str(&mut self, tag: u32, v: Option<&str>) -> &mut Self {
        if let Some(v) = v {
            self.str(tag, v);
        }
        self
    }

    pub fn uint(&mut self, tag: u32, v: u64) -> &mut Self {
        self.tag(tag);
        self.body.extend_from_slice(v.to_string().as_bytes());
        self.body.push(SOH);
        self
    }

    pub fn opt_uint(&mut self, tag: u32, v: Option<u64>) -> &mut Self {
        if let Some(v) = v {
            self.uint(tag, v);
        }
        self
    }

    pub fn char(&mut self, tag: u32, c: u8) -> &mut Self {
        self.bytes(tag, &[c])
    }

    pub fn opt_char(&mut self, tag: u32, c: Option<u8>) -> &mut Self {
        if let Some(c) = c {
            self.char(tag, c);
        }
        self
    }

    pub fn bool(&mut self, tag: u32, v: bool) -> &mut Self {
        self.char(tag, if v { b'Y' } else { b'N' })
    }

    pub fn fixed(&mut self, tag: u32, v: Fixed) -> &mut Self {
        self.tag(tag);
        v.write_to(&mut self.body);
        self.body.push(SOH);
        self
    }

    pub fn opt_fixed(&mut self, tag: u32, v: Option<Fixed>) -> &mut Self {
        if let Some(v) = v {
            self.fixed(tag, v);
        }
        self
    }

    /// Produces `8=<begin>|9=<len>|<body>10=<sum>|`.
    pub fn finish(self, begin_string: &str) -> Result<Vec<u8>, EncodeError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        if begin_string.is_empty() || begin_string.as_bytes().contains(&SOH) {
            return Err(EncodeError::InvalidValue { tag: 8 });
        }
        let mut out = Vec::with_capacity(self.body.len() + 32);
        out.extend_from_slice(b"8=");
        out.extend_from_slice(begin_string.as_bytes());
        out.push(SOH);
        out.extend_from_slice(b"9=");
        out.extend_from_slice(self.body.len().to_string().as_bytes());
        out.push(SOH);
        out.extend_from_slice(&self.body);
        let sum = checksum(&out);
        out.extend_from_slice(format!("10={sum:03}").as_bytes());
        out.push(SOH);
        Ok(out)
    }
}
