//! Binary value updates (MessagePack frames).
//!
//! A binary WebSocket frame holds one or more MessagePack arrays back to
//! back, each `[id, timestamp_us, type_id, value]`. The id is a topic id
//! (server to client) or a publisher id (client to server); `-1` is a time
//! sync ping or reply ([`RTT_ID`](crate::RTT_ID)).
//!
//! Timestamps are integer microseconds, as on the wire. (ntcore 2027 keeps
//! nanoseconds internally and divides by 1000 when encoding; this crate leaves
//! unit conversion to its callers.)
//!
//! ```
//! use nt4_proto::{encode_binary, BinaryMessage, BinaryReader, Value};
//! let msg = BinaryMessage { id: 3, timestamp_us: 1000, value: Value::Double(1.5) };
//! let mut frame = Vec::new();
//! encode_binary(&mut frame, &msg);
//! let decoded: Vec<_> = BinaryReader::new(&frame).collect::<Result<_, _>>().unwrap();
//! assert_eq!(decoded, vec![msg]);
//! ```

use crate::NtType;

/// A topic value.
///
/// Raw-family types (`raw`, `rpc`, `msgpack`, `protobuf`, structs and any
/// unknown type string) all travel as [`Value::Raw`]; `json` travels as
/// [`Value::String`]. The topic's [`NtType`] says how to read the bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Type id 0.
    Boolean(bool),
    /// Type id 1.
    Double(f64),
    /// Type id 2.
    Int(i64),
    /// Type id 3.
    Float(f32),
    /// Type id 4 (`string` and `json`).
    String(String),
    /// Type id 5 (raw bytes).
    Raw(Vec<u8>),
    /// Type id 16.
    BooleanArray(Vec<bool>),
    /// Type id 17.
    DoubleArray(Vec<f64>),
    /// Type id 18.
    IntArray(Vec<i64>),
    /// Type id 19.
    FloatArray(Vec<f32>),
    /// Type id 20.
    StringArray(Vec<String>),
}

impl Value {
    /// Binary type id written before this value.
    ///
    /// ```
    /// assert_eq!(nt4_proto::Value::IntArray(vec![]).type_id(), 18);
    /// ```
    pub fn type_id(&self) -> u8 {
        match self {
            Value::Boolean(_) => 0,
            Value::Double(_) => 1,
            Value::Int(_) => 2,
            Value::Float(_) => 3,
            Value::String(_) => 4,
            Value::Raw(_) => 5,
            Value::BooleanArray(_) => 16,
            Value::DoubleArray(_) => 17,
            Value::IntArray(_) => 18,
            Value::FloatArray(_) => 19,
            Value::StringArray(_) => 20,
        }
    }

    /// The topic type to announce for this value when nothing more specific
    /// is known (`string` rather than `json`, `raw` for bytes).
    pub fn default_type(&self) -> NtType {
        match self {
            Value::Boolean(_) => NtType::Boolean,
            Value::Double(_) => NtType::Double,
            Value::Int(_) => NtType::Int,
            Value::Float(_) => NtType::Float,
            Value::String(_) => NtType::String,
            Value::Raw(_) => NtType::Raw,
            Value::BooleanArray(_) => NtType::BooleanArray,
            Value::DoubleArray(_) => NtType::DoubleArray,
            Value::IntArray(_) => NtType::IntArray,
            Value::FloatArray(_) => NtType::FloatArray,
            Value::StringArray(_) => NtType::StringArray,
        }
    }
}

/// One value update in a binary frame.
#[derive(Debug, Clone, PartialEq)]
pub struct BinaryMessage {
    /// Topic id (from `announce`), publisher id (from `publish`), or `-1`
    /// for time sync.
    pub id: i32,
    /// Timestamp in integer microseconds. 0 means "set default" (a weak value).
    pub timestamp_us: i64,
    /// The value.
    pub value: Value,
}

/// Why a binary message could not be decoded.
///
/// The spec lets a peer either skip a bad message or drop the connection.
/// After an error the rest of the frame can't be trusted, so
/// [`BinaryReader`] stops there.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BinaryError {
    /// The bytes are not valid MessagePack, or end mid-message.
    #[error("invalid msgpack: {0}")]
    MsgPack(String),
    /// The message is not an array of exactly 4 elements.
    #[error("expected a 4 element array")]
    Shape,
    /// The id does not fit in a 32-bit signed integer, or is not an integer.
    #[error("id out of range")]
    Id,
    /// The timestamp is not an integer that fits in an `i64`.
    #[error("timestamp out of range")]
    Timestamp,
    /// The type id is not an integer.
    #[error("type must be an integer")]
    TypeNotInt,
    /// The type id is not one the spec defines.
    #[error("unrecognized type {0}")]
    UnknownType(i64),
    /// The value does not match its type id.
    #[error("value does not match type {0}")]
    ValueMismatch(i64),
}

/// Append one value update to `out`.
///
/// Integers use the smallest MessagePack form, floats are always `float 32`
/// and doubles always `float 64`, the same as ntcore. Several messages may be
/// appended to the same buffer to send them in one frame.
pub fn encode_binary(out: &mut Vec<u8>, msg: &BinaryMessage) {
    use rmp::encode as e;
    // Writing to a Vec cannot fail, so the results are ignored.
    let _ = e::write_array_len(out, 4);
    let _ = e::write_sint(out, i64::from(msg.id));
    let _ = e::write_sint(out, msg.timestamp_us);
    let _ = e::write_uint(out, u64::from(msg.value.type_id()));
    match &msg.value {
        Value::Boolean(b) => e::write_bool(out, *b).unwrap_or(()),
        Value::Double(d) => e::write_f64(out, *d).unwrap_or(()),
        Value::Int(i) => {
            let _ = e::write_sint(out, *i);
        }
        Value::Float(f) => e::write_f32(out, *f).unwrap_or(()),
        Value::String(s) => e::write_str(out, s).unwrap_or(()),
        Value::Raw(b) => e::write_bin(out, b).unwrap_or(()),
        Value::BooleanArray(a) => {
            let _ = e::write_array_len(out, len_u32(a.len()));
            for v in a {
                let _ = e::write_bool(out, *v);
            }
        }
        Value::DoubleArray(a) => {
            let _ = e::write_array_len(out, len_u32(a.len()));
            for v in a {
                let _ = e::write_f64(out, *v);
            }
        }
        Value::IntArray(a) => {
            let _ = e::write_array_len(out, len_u32(a.len()));
            for v in a {
                let _ = e::write_sint(out, *v);
            }
        }
        Value::FloatArray(a) => {
            let _ = e::write_array_len(out, len_u32(a.len()));
            for v in a {
                let _ = e::write_f32(out, *v);
            }
        }
        Value::StringArray(a) => {
            let _ = e::write_array_len(out, len_u32(a.len()));
            for v in a {
                let _ = e::write_str(out, v);
            }
        }
    }
}

fn len_u32(len: usize) -> u32 {
    u32::try_from(len).expect("NT4 arrays are limited to u32::MAX elements")
}

/// Reads the value updates in one binary frame, in order.
///
/// Yields `Err` once for the first message that can't be decoded and then
/// stops, since the position of the next message is unknown.
pub struct BinaryReader<'a> {
    bytes: Bytes<'a>,
    done: bool,
}

impl<'a> BinaryReader<'a> {
    /// Start reading a frame.
    pub fn new(frame: &'a [u8]) -> Self {
        Self {
            bytes: Bytes(frame),
            done: false,
        }
    }
}

impl Iterator for BinaryReader<'_> {
    type Item = Result<BinaryMessage, BinaryError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.bytes.0.is_empty() {
            return None;
        }
        let result = decode_message(&mut self.bytes);
        if result.is_err() {
            self.done = true;
        }
        Some(result)
    }
}

// The decoder reads MessagePack straight into `Value`, in order, like
// ntcore's mpack reader. (Going through a generic MessagePack tree first cost
// about half of a client's CPU under load.)

/// One MessagePack item; arrays and maps give only their length.
enum Item<'a> {
    Nil,
    Bool(bool),
    /// Every integer that fits in an `i64`.
    Int(i64),
    /// Integers above `i64::MAX`.
    BigUint(u64),
    F32(f32),
    F64(f64),
    Str(&'a [u8]),
    Bin(&'a [u8]),
    Array(usize),
    /// A map or extension; never valid in a value update.
    Other,
}

impl Item<'_> {
    fn as_i64(&self) -> Option<i64> {
        match *self {
            Item::Int(i) => Some(i),
            _ => None,
        }
    }

    /// Any number as `f32`, like mpack's `expect_float`: a `float 32` keeps
    /// its exact bits (a signaling NaN stays one), anything else is
    /// converted.
    fn as_f32(&self) -> Option<f32> {
        match *self {
            Item::F32(f) => Some(f),
            _ => self.as_f64().map(|d| d as f32),
        }
    }

    /// Any number as `f64`, like mpack's `expect_double`. Some peers pack a
    /// whole-number double as an integer.
    fn as_f64(&self) -> Option<f64> {
        match *self {
            Item::F64(d) => Some(d),
            Item::F32(f) => Some(f64::from(f)),
            Item::Int(i) => Some(i as f64),
            Item::BigUint(u) => Some(u as f64),
            _ => None,
        }
    }
}

struct Bytes<'a>(&'a [u8]);

fn truncated() -> BinaryError {
    BinaryError::MsgPack("frame ends mid-message".into())
}

impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], BinaryError> {
        if self.0.len() < n {
            return Err(truncated());
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], BinaryError> {
        Ok(self.take(N)?.try_into().expect("took N bytes"))
    }

    fn len(&mut self, size: usize) -> Result<usize, BinaryError> {
        Ok(match size {
            1 => usize::from(self.array::<1>()?[0]),
            2 => usize::from(u16::from_be_bytes(self.array()?)),
            _ => u32::from_be_bytes(self.array()?) as usize,
        })
    }

    /// Read one item; for an array or map, only its header.
    fn item(&mut self) -> Result<Item<'a>, BinaryError> {
        let m = self.array::<1>()?[0];
        Ok(match m {
            0x00..=0x7f => Item::Int(i64::from(m)),
            0x80..=0x8f => Item::Other,
            0x90..=0x9f => Item::Array(usize::from(m & 0x0f)),
            0xa0..=0xbf => Item::Str(self.take(usize::from(m & 0x1f))?),
            0xc0 => Item::Nil,
            0xc1 => return Err(BinaryError::MsgPack("reserved marker 0xc1".into())),
            0xc2 => Item::Bool(false),
            0xc3 => Item::Bool(true),
            0xc4..=0xc6 => {
                let n = self.len(1 << (m - 0xc4))?;
                Item::Bin(self.take(n)?)
            }
            0xc7..=0xc9 => {
                let n = self.len(1 << (m - 0xc7))?;
                self.take(n + 1)?;
                Item::Other
            }
            0xca => Item::F32(f32::from_be_bytes(self.array()?)),
            0xcb => Item::F64(f64::from_be_bytes(self.array()?)),
            0xcc => Item::Int(i64::from(self.array::<1>()?[0])),
            0xcd => Item::Int(i64::from(u16::from_be_bytes(self.array()?))),
            0xce => Item::Int(i64::from(u32::from_be_bytes(self.array()?))),
            0xcf => {
                let u = u64::from_be_bytes(self.array()?);
                i64::try_from(u).map_or(Item::BigUint(u), Item::Int)
            }
            0xd0 => Item::Int(i64::from(i8::from_be_bytes(self.array()?))),
            0xd1 => Item::Int(i64::from(i16::from_be_bytes(self.array()?))),
            0xd2 => Item::Int(i64::from(i32::from_be_bytes(self.array()?))),
            0xd3 => Item::Int(i64::from_be_bytes(self.array()?)),
            0xd4..=0xd8 => {
                self.take((1 << (m - 0xd4)) + 1)?;
                Item::Other
            }
            0xd9..=0xdb => {
                let n = self.len(1 << (m - 0xd9))?;
                Item::Str(self.take(n)?)
            }
            0xdc | 0xdd => Item::Array(self.len(2 << (m - 0xdc))?),
            0xde | 0xdf => {
                self.len(2 << (m - 0xde))?;
                Item::Other
            }
            0xe0..=0xff => Item::Int(i64::from(m as i8)),
        })
    }
}

fn decode_message(bytes: &mut Bytes<'_>) -> Result<BinaryMessage, BinaryError> {
    let Item::Array(4) = bytes.item()? else {
        return Err(BinaryError::Shape);
    };
    let id = bytes
        .item()?
        .as_i64()
        .and_then(|i| i32::try_from(i).ok())
        .ok_or(BinaryError::Id)?;
    let timestamp_us = bytes.item()?.as_i64().ok_or(BinaryError::Timestamp)?;
    let type_id = bytes.item()?.as_i64().ok_or(BinaryError::TypeNotInt)?;
    let value = decode_value(type_id, bytes)?;
    Ok(BinaryMessage {
        id,
        timestamp_us,
        value,
    })
}

/// A MessagePack string as UTF-8. ntcore takes any bytes; a Rust `String`
/// can't hold that, so invalid sequences become U+FFFD instead of failing
/// the message (and with it the rest of the frame).
fn utf8_lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn decode_value(type_id: i64, bytes: &mut Bytes<'_>) -> Result<Value, BinaryError> {
    let bad = || BinaryError::ValueMismatch(type_id);

    // An array of `f`'s results; any other item is a mismatch.
    fn array<T>(
        bytes: &mut Bytes<'_>,
        bad: impl Fn() -> BinaryError,
        f: impl Fn(Item<'_>) -> Option<T>,
    ) -> Result<Vec<T>, BinaryError> {
        let Item::Array(n) = bytes.item()? else {
            return Err(bad());
        };
        // Each element takes at least one byte; don't trust `n` further.
        let mut out = Vec::with_capacity(n.min(bytes.0.len()));
        for _ in 0..n {
            out.push(f(bytes.item()?).ok_or_else(&bad)?);
        }
        Ok(out)
    }
    if !matches!(type_id, 0..=5 | 16..=20) {
        return Err(BinaryError::UnknownType(type_id));
    }
    Ok(match type_id {
        16 => Value::BooleanArray(array(bytes, bad, |i| match i {
            Item::Bool(b) => Some(b),
            _ => None,
        })?),
        17 => Value::DoubleArray(array(bytes, bad, |i| i.as_f64())?),
        18 => Value::IntArray(array(bytes, bad, |i| i.as_i64())?),
        19 => Value::FloatArray(array(bytes, bad, |i| i.as_f32())?),
        20 => Value::StringArray(array(bytes, bad, |i| match i {
            Item::Str(b) => Some(utf8_lossy(b)),
            _ => None,
        })?),
        _ => {
            let item = bytes.item()?;
            match (type_id, item) {
                (0, Item::Bool(b)) => Value::Boolean(b),
                (1, i) => Value::Double(i.as_f64().ok_or_else(bad)?),
                (2, i) => Value::Int(i.as_i64().ok_or_else(bad)?),
                (3, i) => Value::Float(i.as_f32().ok_or_else(bad)?),
                (4, Item::Str(b)) => Value::String(utf8_lossy(b)),
                (5, Item::Bin(b)) => Value::Raw(b.to_vec()),
                _ => return Err(bad()),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    // Tests written by Claude.
    use super::*;

    /// The decoder this crate used before reading MessagePack directly: rmpv
    /// builds a generic tree, then it is converted. Kept as the reference the
    /// direct decoder must agree with.
    mod reference {
        use super::super::{BinaryMessage, Value};
        use rmpv::Value as V;
        use std::io::Cursor;

        pub fn decode(frame: &[u8]) -> Vec<Result<BinaryMessage, ()>> {
            let mut cursor = Cursor::new(frame);
            let mut out = Vec::new();
            while (cursor.position() as usize) < frame.len() {
                let r = rmpv::decode::read_value(&mut cursor)
                    .ok()
                    .and_then(message)
                    .ok_or(());
                let stop = r.is_err();
                out.push(r);
                if stop {
                    break;
                }
            }
            out
        }

        fn number(v: &V) -> Option<f64> {
            v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
        }

        fn message(item: V) -> Option<BinaryMessage> {
            let V::Array(parts) = item else { return None };
            let [id, time, ty, v]: [V; 4] = parts.try_into().ok()?;
            let id = i32::try_from(id.as_i64()?).ok()?;
            let timestamp_us = time.as_i64()?;
            let arr = |v: V| match v {
                V::Array(a) => Some(a),
                _ => None,
            };
            let value = match ty.as_i64()? {
                0 => Value::Boolean(v.as_bool()?),
                1 => Value::Double(number(&v)?),
                2 => Value::Int(v.as_i64()?),
                3 => Value::Float(number(&v)? as f32),
                4 => match v {
                    V::String(s) => {
                        Value::String(String::from_utf8_lossy(s.as_bytes()).into_owned())
                    }
                    _ => return None,
                },
                5 => match v {
                    V::Binary(b) => Value::Raw(b),
                    _ => return None,
                },
                16 => Value::BooleanArray(arr(v)?.iter().map(V::as_bool).collect::<Option<_>>()?),
                17 => Value::DoubleArray(arr(v)?.iter().map(number).collect::<Option<_>>()?),
                18 => Value::IntArray(arr(v)?.iter().map(V::as_i64).collect::<Option<_>>()?),
                19 => Value::FloatArray(
                    arr(v)?
                        .iter()
                        .map(|x| number(x).map(|f| f as f32))
                        .collect::<Option<_>>()?,
                ),
                20 => Value::StringArray(
                    arr(v)?
                        .iter()
                        .map(|x| match x {
                            V::String(s) => {
                                Some(String::from_utf8_lossy(s.as_bytes()).into_owned())
                            }
                            _ => None,
                        })
                        .collect::<Option<_>>()?,
                ),
                _ => return None,
            };
            Some(BinaryMessage {
                id,
                timestamp_us,
                value,
            })
        }
    }

    /// Both decoders accept the same messages with the same values, and
    /// reject at the same message. `{:?}` so NaN compares equal.
    fn assert_same_as_reference(frame: &[u8]) {
        let ours: Vec<_> = BinaryReader::new(frame)
            .map(|r| r.map_err(|_| ()))
            .collect();
        assert_eq!(
            format!("{ours:?}"),
            format!("{:?}", reference::decode(frame)),
            "frame {frame:02x?}"
        );
    }

    /// xorshift64*, so the test needs no extra crates and is repeatable.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// A random item in the encodings a peer might use, valid for the type or
    /// not.
    fn random_item(rng: &mut Rng, out: &mut Vec<u8>, depth: u32) {
        use rmp::encode as e;
        match rng.below(if depth > 1 { 11 } else { 12 }) {
            0 => e::write_bool(out, rng.below(2) == 0).unwrap(),
            1 => e::write_f64(out, rng.next() as f64 / 7.0).unwrap(),
            2 => e::write_f32(out, rng.next() as f32 / 3.0).unwrap(),
            3 => {
                e::write_sint(out, rng.next() as i64 >> rng.below(64)).unwrap();
            }
            4 => {
                e::write_uint(out, rng.next() >> rng.below(64)).unwrap();
            }
            5 => e::write_str(out, &"x".repeat(rng.below(40) as usize)).unwrap(),
            6 => {
                // Not UTF-8.
                e::write_str_len(out, 2).unwrap();
                out.extend([0xff, 0xfe]);
            }
            7 => e::write_bin(out, &vec![7; rng.below(300) as usize]).unwrap(),
            8 => e::write_nil(out).unwrap(),
            9 => {
                e::write_map_len(out, 1).unwrap();
                e::write_nil(out).unwrap();
                e::write_nil(out).unwrap();
            }
            10 => e::write_ext_meta(out, 2, 1)
                .map(|_| out.extend([0, 0]))
                .unwrap(),
            _ => {
                let n = rng.below(20) as u32;
                e::write_array_len(out, n).unwrap();
                // Mostly one kind, so typed arrays are sometimes valid.
                let seed = rng.next();
                for _ in 0..n {
                    let mut r = Rng(if rng.below(8) == 0 { rng.next() } else { seed });
                    random_item(&mut r, out, depth + 1);
                }
            }
        }
    }

    /// A value that is valid for `type_id`, in one of the encodings a peer
    /// might use.
    fn valid_value(rng: &mut Rng, out: &mut Vec<u8>, type_id: i64) {
        use rmp::encode as e;
        let number = |rng: &mut Rng, out: &mut Vec<u8>| match rng.below(3) {
            0 => e::write_f64(out, rng.next() as f64 / 7.0).unwrap(),
            1 => e::write_f32(out, rng.next() as f32 / 3.0).unwrap(),
            _ => {
                e::write_sint(out, rng.next() as i64 >> rng.below(64)).unwrap();
            }
        };
        let n = rng.below(12) as u32;
        match type_id {
            0 => e::write_bool(out, rng.below(2) == 0).unwrap(),
            1 | 3 => number(rng, out),
            2 => {
                e::write_sint(out, rng.next() as i64 >> rng.below(64)).unwrap();
            }
            4 => e::write_str(out, &"é".repeat(rng.below(40) as usize)).unwrap(),
            5 => e::write_bin(out, &vec![9; rng.below(300) as usize]).unwrap(),
            16..=20 => {
                e::write_array_len(out, n).unwrap();
                for _ in 0..n {
                    valid_value(rng, out, type_id - 16);
                }
            }
            _ => random_item(rng, out, 0),
        }
    }

    fn random_message(rng: &mut Rng, out: &mut Vec<u8>) {
        use rmp::encode as e;
        let n = if rng.below(20) == 0 {
            rng.below(6) as u32
        } else {
            4
        };
        e::write_array_len(out, n).unwrap();
        let ids = [0, 1, 2, 3, 4, 5, 16, 17, 18, 19, 20, 6, 21, -1];
        let type_id = ids[rng.below(ids.len() as u64) as usize];
        for i in 0..n {
            match (i, rng.below(10)) {
                (_, 0) => random_item(rng, out, 0),
                (0, _) => {
                    e::write_sint(out, rng.next() as i64 >> (32 + rng.below(32))).unwrap();
                }
                (1, _) => {
                    e::write_sint(out, rng.next() as i64 >> rng.below(64)).unwrap();
                }
                (2, _) => {
                    e::write_sint(out, type_id).unwrap();
                }
                _ => valid_value(rng, out, type_id),
            }
        }
    }

    #[test]
    fn direct_decoder_matches_rmpv_reference() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut accepted = 0;
        for case in 0..20_000 {
            let mut frame = Vec::new();
            for _ in 0..1 + rng.below(4) {
                random_message(&mut rng, &mut frame);
            }
            match case % 4 {
                // Flip a byte.
                1 if !frame.is_empty() => {
                    let i = rng.below(frame.len() as u64) as usize;
                    frame[i] = rng.next() as u8;
                }
                // Cut it short.
                2 => frame.truncate(rng.below(frame.len() as u64 + 1) as usize),
                // Plain noise.
                3 => frame = (0..rng.below(40)).map(|_| rng.next() as u8).collect(),
                _ => {}
            }
            assert_same_as_reference(&frame);
            accepted += BinaryReader::new(&frame).filter(Result::is_ok).count();
        }
        // The generator must make plenty of valid messages too.
        assert!(accepted > 5_000, "only {accepted} valid messages");
    }

    #[test]
    fn every_integer_and_float_width_decodes() {
        use rmp::encode as e;
        let mut frame = Vec::new();
        for v in [0i64, 127, -32, -33, 255, 256, 65_535, 65_536, -129, -32_769] {
            for ty in [1u64, 2, 3] {
                e::write_array_len(&mut frame, 4).unwrap();
                e::write_sint(&mut frame, 1).unwrap();
                e::write_sint(&mut frame, v).unwrap();
                e::write_uint(&mut frame, ty).unwrap();
                e::write_sint(&mut frame, v).unwrap();
            }
        }
        for big in [u64::from(u32::MAX) + 1, u64::MAX] {
            e::write_array_len(&mut frame, 4).unwrap();
            e::write_sint(&mut frame, 1).unwrap();
            e::write_sint(&mut frame, 0).unwrap();
            e::write_uint(&mut frame, 1).unwrap();
            e::write_u64(&mut frame, big).unwrap();
        }
        assert_same_as_reference(&frame);
        assert!(BinaryReader::new(&frame).all(|r| r.is_ok()));
    }

    fn frame(msgs: &[BinaryMessage]) -> Vec<u8> {
        let mut out = Vec::new();
        for m in msgs {
            encode_binary(&mut out, m);
        }
        out
    }

    #[test]
    fn decodes_several_messages_in_one_frame() {
        let msgs = vec![
            BinaryMessage {
                id: 1,
                timestamp_us: 10,
                value: Value::Boolean(true),
            },
            BinaryMessage {
                id: 2,
                timestamp_us: 20,
                value: Value::StringArray(vec!["a".into()]),
            },
            BinaryMessage {
                id: 3,
                timestamp_us: 30,
                value: Value::Raw(vec![1, 2, 3]),
            },
            BinaryMessage {
                id: -1,
                timestamp_us: 0,
                value: Value::Int(123_456),
            },
        ];
        let decoded: Vec<_> = BinaryReader::new(&frame(&msgs))
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(decoded, msgs);
    }

    #[test]
    fn stops_after_the_first_bad_message() {
        let good = BinaryMessage {
            id: 1,
            timestamp_us: 1,
            value: Value::Int(1),
        };
        let mut bytes = frame(std::slice::from_ref(&good));
        rmpv::encode::write_value(&mut bytes, &rmpv::Value::from(5)).unwrap();
        bytes.extend(frame(std::slice::from_ref(&good)));
        let results: Vec<_> = BinaryReader::new(&bytes).collect();
        assert_eq!(results, vec![Ok(good), Err(BinaryError::Shape)]);
    }

    #[test]
    fn id_outside_i32_is_rejected() {
        let item = rmpv::Value::Array(vec![
            (i64::from(i32::MAX) + 1).into(),
            0.into(),
            2.into(),
            0.into(),
        ]);
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &item).unwrap();
        assert_eq!(BinaryReader::new(&bytes).next(), Some(Err(BinaryError::Id)));
    }

    #[test]
    fn truncated_frame_is_an_error() {
        let mut bytes = frame(&[BinaryMessage {
            id: 1,
            timestamp_us: 1,
            value: Value::String("hello".into()),
        }]);
        bytes.truncate(bytes.len() - 2);
        assert!(matches!(
            BinaryReader::new(&bytes).next(),
            Some(Err(BinaryError::MsgPack(_)))
        ));
    }
}
