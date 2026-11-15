//! A small MessagePack writer for the server's meta topic values
//! (`$clients`, `$pub$<topic>` and so on).
//!
//! It writes each value in its most compact form, as mpack (which ntcore
//! uses) does, so meta values match ntcore's byte for byte. The marker bytes
//! below are the format names from the MessagePack spec,
//! <https://github.com/msgpack/msgpack/blob/master/spec.md#formats>.

use nt4_proto::SubscriptionOptions;

use crate::outgoing::DEFAULT_PERIOD_MS;

// "fix" formats carry a small value or length in the marker's low bits.
/// positive fixint: 0x00-0x7f is the value itself.
const POSITIVE_FIXINT_MAX: u64 = 0x7f;
/// negative fixint: 0xe0-0xff is -32 to -1.
const NEGATIVE_FIXINT_MIN: i64 = -32;
/// fixmap: 0x80 | length (up to 15 entries).
const FIXMAP: u8 = 0x80;
/// fixarray: 0x90 | length (up to 15 elements).
const FIXARRAY: u8 = 0x90;
/// Longest map or array a fix format holds.
const FIX_CONTAINER_MAX: usize = 15;
/// fixstr: 0xa0 | length (up to 31 bytes).
const FIXSTR: u8 = 0xa0;
/// Longest string a fixstr holds.
const FIXSTR_MAX: usize = 31;

const FALSE: u8 = 0xc2;
const TRUE: u8 = 0xc3;
const FLOAT32: u8 = 0xca;
const UINT8: u8 = 0xcc;
const UINT16: u8 = 0xcd;
const UINT32: u8 = 0xce;
const UINT64: u8 = 0xcf;
const INT8: u8 = 0xd0;
const INT16: u8 = 0xd1;
const INT32: u8 = 0xd2;
const INT64: u8 = 0xd3;
const STR8: u8 = 0xd9;
const STR16: u8 = 0xda;
const STR32: u8 = 0xdb;
const ARRAY16: u8 = 0xdc;
const ARRAY32: u8 = 0xdd;
const MAP16: u8 = 0xde;
const MAP32: u8 = 0xdf;
#[derive(Debug, Default)]
pub(crate) struct Writer(pub(crate) Vec<u8>);

impl Writer {
    pub(crate) fn array(&mut self, len: usize) {
        self.len_header(len, FIXARRAY, ARRAY16, ARRAY32);
    }

    pub(crate) fn map(&mut self, len: usize) {
        self.len_header(len, FIXMAP, MAP16, MAP32);
    }

    fn len_header(&mut self, len: usize, fix: u8, m16: u8, m32: u8) {
        if len <= FIX_CONTAINER_MAX {
            self.0.push(fix | len as u8);
        } else if let Ok(n) = u16::try_from(len) {
            self.0.push(m16);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else {
            self.0.push(m32);
            self.0
                .extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_be_bytes());
        }
    }

    pub(crate) fn str(&mut self, s: &str) {
        let len = s.len();
        if len <= FIXSTR_MAX {
            self.0.push(FIXSTR | len as u8);
        } else if let Ok(n) = u8::try_from(len) {
            self.0.push(STR8);
            self.0.push(n);
        } else if let Ok(n) = u16::try_from(len) {
            self.0.push(STR16);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else {
            self.0.push(STR32);
            self.0
                .extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_be_bytes());
        }
        self.0.extend_from_slice(s.as_bytes());
    }

    pub(crate) fn uint(&mut self, v: u64) {
        if v <= POSITIVE_FIXINT_MAX {
            self.0.push(v as u8);
        } else if let Ok(n) = u8::try_from(v) {
            self.0.extend_from_slice(&[UINT8, n]);
        } else if let Ok(n) = u16::try_from(v) {
            self.0.push(UINT16);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else if let Ok(n) = u32::try_from(v) {
            self.0.push(UINT32);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else {
            self.0.push(UINT64);
            self.0.extend_from_slice(&v.to_be_bytes());
        }
    }

    pub(crate) fn int(&mut self, v: i64) {
        if let Ok(u) = u64::try_from(v) {
            self.uint(u);
        } else if v >= NEGATIVE_FIXINT_MIN {
            self.0.push(v as i8 as u8);
        } else if let Ok(n) = i8::try_from(v) {
            self.0.extend_from_slice(&[INT8, n as u8]);
        } else if let Ok(n) = i16::try_from(v) {
            self.0.push(INT16);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else if let Ok(n) = i32::try_from(v) {
            self.0.push(INT32);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else {
            self.0.push(INT64);
            self.0.extend_from_slice(&v.to_be_bytes());
        }
    }

    pub(crate) fn bool(&mut self, v: bool) {
        self.0.push(if v { TRUE } else { FALSE });
    }

    pub(crate) fn f32(&mut self, v: f32) {
        self.0.push(FLOAT32);
        self.0.extend_from_slice(&v.to_be_bytes());
    }

    /// Subscription options as ntcore's `WriteOptions` writes them: only the
    /// options that differ from the defaults.
    pub(crate) fn options(&mut self, options: &SubscriptionOptions, periodic_ms: u32) {
        let periodic = periodic_ms != DEFAULT_PERIOD_MS;
        let len = usize::from(options.all)
            + usize::from(options.topics_only)
            + usize::from(periodic)
            + usize::from(options.prefix);
        self.map(len);
        if options.all {
            self.str("all");
            self.bool(true);
        }
        if options.topics_only {
            self.str("topicsonly");
            self.bool(true);
        }
        if periodic {
            self.str("periodic");
            self.f32((f64::from(periodic_ms) / 1000.0) as f32);
        }
        if options.prefix {
            self.str("prefix");
            self.bool(true);
        }
    }
}

#[cfg(test)]
mod tests {
    // Written by Claude: the encodings checked against the MessagePack spec.
    use super::Writer;

    fn int(v: i64) -> Vec<u8> {
        let mut w = Writer::default();
        w.int(v);
        w.0
    }

    #[test]
    fn ints_are_compact() {
        assert_eq!(int(5), [0x05]);
        assert_eq!(int(200), [0xcc, 200]);
        assert_eq!(int(0x1234), [0xcd, 0x12, 0x34]);
        assert_eq!(int(-1), [0xff]);
        assert_eq!(int(-33), [0xd0, 0xdf]);
        assert_eq!(int(-200), [0xd1, 0xff, 0x38]);
    }

    #[test]
    fn strings_and_containers() {
        let mut w = Writer::default();
        w.array(1);
        w.map(1);
        w.str("id");
        w.str(&"x".repeat(40));
        assert_eq!(&w.0[..5], &[0x91, 0x81, 0xa2, b'i', b'd']);
        assert_eq!(&w.0[5..7], &[0xd9, 40]);
    }
}
