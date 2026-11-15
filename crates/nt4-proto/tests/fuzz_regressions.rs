//! Inputs where the differential fuzzer (crates/nt4-fuzz) found nt4-proto
//! differing from ntcore, kept as regression tests.
//!
//! Tests written by Claude.

use nt4_proto::{BinaryReader, ClientMessage, Value, decode_client_text, encode_text_frame};

/// ntcore parses JSON numbers exactly; serde_json only does with its
/// `float_roundtrip` feature. Without it this value came out one step low.
#[test]
fn json_numbers_parse_exactly() {
    let frame =
        r#"[{"method":"setproperties","params":{"name":"","update":{"k":123.23456123456789}}}]"#;
    let d = decode_client_text(frame);
    let ClientMessage::SetProperties { update, .. } = &d.messages[0] else {
        panic!("{:?}", d.messages);
    };
    assert_eq!(
        update["k"].as_f64().unwrap().to_bits(),
        123.234_561_234_567_89_f64.to_bits()
    );
    // The same number with a trailing zero is the same value, and survives
    // encoding and decoding again.
    let zero = frame.replace("789}", "7890}");
    assert_eq!(decode_client_text(&zero).messages, d.messages);
    assert_eq!(
        decode_client_text(&encode_text_frame(&d.messages)).messages,
        d.messages
    );
}

/// ntcore reads `-0` as the integer 0, so it is a valid id.
#[test]
fn negative_zero_is_an_integer() {
    let d = decode_client_text(r#"[{"method":"unpublish","params":{"pubuid":-0}}]"#);
    assert_eq!(d.messages, [ClientMessage::Unpublish { pubuid: 0 }]);
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
}

/// An id above `i64::MAX` is rejected, not wrapped to a negative id.
#[test]
fn id_above_i64_is_rejected() {
    let d =
        decode_client_text(r#"[{"method":"unpublish","params":{"pubuid":18446744073709551615}}]"#);
    assert!(d.messages.is_empty(), "{:?}", d.messages);
    assert_eq!(d.warnings, ["0: pubuid out of range"]);
}

/// ntcore accepts a string value that isn't UTF-8; here the bad bytes become
/// U+FFFD.
#[test]
fn string_value_with_invalid_utf8_decodes() {
    // [id 1, time 0, type 4 (string), a one-byte string 0xff]
    let frame = [0x94, 0x01, 0x00, 0x04, 0xa1, 0xff];
    let msgs: Vec<_> = BinaryReader::new(&frame).collect::<Result<_, _>>().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].value, Value::String("\u{fffd}".into()));
}
