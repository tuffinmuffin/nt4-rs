//! A small MessagePack writer for the server's meta topic values
//! (`$clients`, `$pub$<topic>` and so on).
//!
//! It writes each value in its most compact form, as mpack (which ntcore
//! uses) does, so meta values match ntcore's byte for byte.

use nt4_proto::SubscriptionOptions;

use crate::outgoing::DEFAULT_PERIOD_MS;

#[derive(Debug, Default)]
pub(crate) struct Writer(pub(crate) Vec<u8>);

impl Writer {
    pub(crate) fn array(&mut self, len: usize) {
        self.len_header(len, 0x90, 0xdc, 0xdd);
    }

    pub(crate) fn map(&mut self, len: usize) {
        self.len_header(len, 0x80, 0xde, 0xdf);
    }

    fn len_header(&mut self, len: usize, fix: u8, m16: u8, m32: u8) {
        if len <= 15 {
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
        if len <= 31 {
            self.0.push(0xa0 | len as u8);
        } else if let Ok(n) = u8::try_from(len) {
            self.0.push(0xd9);
            self.0.push(n);
        } else if let Ok(n) = u16::try_from(len) {
            self.0.push(0xda);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else {
            self.0.push(0xdb);
            self.0
                .extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_be_bytes());
        }
        self.0.extend_from_slice(s.as_bytes());
    }

    pub(crate) fn uint(&mut self, v: u64) {
        if v <= 0x7f {
            self.0.push(v as u8);
        } else if let Ok(n) = u8::try_from(v) {
            self.0.extend_from_slice(&[0xcc, n]);
        } else if let Ok(n) = u16::try_from(v) {
            self.0.push(0xcd);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else if let Ok(n) = u32::try_from(v) {
            self.0.push(0xce);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else {
            self.0.push(0xcf);
            self.0.extend_from_slice(&v.to_be_bytes());
        }
    }

    pub(crate) fn int(&mut self, v: i64) {
        if let Ok(u) = u64::try_from(v) {
            self.uint(u);
        } else if v >= -32 {
            self.0.push(v as i8 as u8);
        } else if let Ok(n) = i8::try_from(v) {
            self.0.extend_from_slice(&[0xd0, n as u8]);
        } else if let Ok(n) = i16::try_from(v) {
            self.0.push(0xd1);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else if let Ok(n) = i32::try_from(v) {
            self.0.push(0xd2);
            self.0.extend_from_slice(&n.to_be_bytes());
        } else {
            self.0.push(0xd3);
            self.0.extend_from_slice(&v.to_be_bytes());
        }
    }

    pub(crate) fn bool(&mut self, v: bool) {
        self.0.push(if v { 0xc3 } else { 0xc2 });
    }

    pub(crate) fn f32(&mut self, v: f32) {
        self.0.push(0xca);
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
