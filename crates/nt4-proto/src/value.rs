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

use std::io::Cursor;

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
    cursor: Cursor<&'a [u8]>,
    done: bool,
}

impl<'a> BinaryReader<'a> {
    /// Start reading a frame.
    pub fn new(frame: &'a [u8]) -> Self {
        Self {
            cursor: Cursor::new(frame),
            done: false,
        }
    }
}

impl Iterator for BinaryReader<'_> {
    type Item = Result<BinaryMessage, BinaryError>;

    fn next(&mut self) -> Option<Self::Item> {
        let len = self.cursor.get_ref().len() as u64;
        if self.done || self.cursor.position() >= len {
            return None;
        }
        let result = rmpv::decode::read_value(&mut self.cursor)
            .map_err(|e| BinaryError::MsgPack(e.to_string()))
            .and_then(decode_message);
        if result.is_err() {
            self.done = true;
        }
        Some(result)
    }
}

fn decode_message(item: rmpv::Value) -> Result<BinaryMessage, BinaryError> {
    let rmpv::Value::Array(parts) = item else {
        return Err(BinaryError::Shape);
    };
    let [id, time, ty, value]: [rmpv::Value; 4] =
        parts.try_into().map_err(|_| BinaryError::Shape)?;
    let id = id
        .as_i64()
        .and_then(|i| i32::try_from(i).ok())
        .ok_or(BinaryError::Id)?;
    let timestamp_us = time.as_i64().ok_or(BinaryError::Timestamp)?;
    let type_id = ty.as_i64().ok_or(BinaryError::TypeNotInt)?;
    let value = decode_value(type_id, value)?;
    Ok(BinaryMessage {
        id,
        timestamp_us,
        value,
    })
}

fn decode_value(type_id: i64, v: rmpv::Value) -> Result<Value, BinaryError> {
    use rmpv::Value as V;
    let bad = || BinaryError::ValueMismatch(type_id);
    let array = |v: V| match v {
        V::Array(a) => Ok(a),
        _ => Err(bad()),
    };
    Ok(match type_id {
        0 => Value::Boolean(v.as_bool().ok_or_else(bad)?),
        1 => Value::Double(number(&v).ok_or_else(bad)?),
        2 => Value::Int(v.as_i64().ok_or_else(bad)?),
        3 => Value::Float(number(&v).ok_or_else(bad)? as f32),
        4 => Value::String(match v {
            V::String(s) => s.into_str().ok_or_else(bad)?,
            _ => return Err(bad()),
        }),
        5 => Value::Raw(match v {
            V::Binary(b) => b,
            _ => return Err(bad()),
        }),
        16 => Value::BooleanArray(
            array(v)?
                .iter()
                .map(|x| x.as_bool().ok_or_else(bad))
                .collect::<Result<_, _>>()?,
        ),
        17 => Value::DoubleArray(
            array(v)?
                .iter()
                .map(|x| number(x).ok_or_else(bad))
                .collect::<Result<_, _>>()?,
        ),
        18 => Value::IntArray(
            array(v)?
                .iter()
                .map(|x| x.as_i64().ok_or_else(bad))
                .collect::<Result<_, _>>()?,
        ),
        19 => Value::FloatArray(
            array(v)?
                .iter()
                .map(|x| number(x).map(|f| f as f32).ok_or_else(bad))
                .collect::<Result<_, _>>()?,
        ),
        20 => Value::StringArray(
            array(v)?
                .iter()
                .map(|x| x.as_str().map(str::to_owned).ok_or_else(bad))
                .collect::<Result<_, _>>()?,
        ),
        other => return Err(BinaryError::UnknownType(other)),
    })
}

/// Read any MessagePack number as `f64`, like mpack's `expect_double`.
/// Some peers pack a whole-number double as an integer.
fn number(v: &rmpv::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|i| i as f64))
        .or_else(|| v.as_u64().map(|u| u as f64))
}

#[cfg(test)]
mod tests {
    // Tests written by Claude.
    use super::*;

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
