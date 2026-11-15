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

/// An id above `i64::MAX` is not a number to ntcore (its ids are signed), so
/// it is rejected, not wrapped to a negative id.
#[test]
fn id_above_i64_is_rejected() {
    let d =
        decode_client_text(r#"[{"method":"unpublish","params":{"pubuid":18446744073709551615}}]"#);
    assert!(d.messages.is_empty(), "{:?}", d.messages);
    assert_eq!(d.warnings, ["0: pubuid must be a number"]);
}

/// ntcore's JSON parser stops at 20 nested arrays and objects and drops the
/// frame. A property value is already 4 deep, so 15 more arrays fit and 16
/// don't.
#[test]
fn json_nesting_limit_is_ntcores() {
    let frame = |depth: usize| {
        format!(
            r#"[{{"method":"setproperties","params":{{"name":"","update":{{"k":{}0{}}}}}}}]"#,
            "[".repeat(depth),
            "]".repeat(depth)
        )
    };
    let ok = decode_client_text(&frame(15));
    assert_eq!(ok.messages.len(), 1, "{:?}", ok.warnings);
    let deep = decode_client_text(&frame(16));
    assert!(deep.messages.is_empty());
    assert_eq!(
        deep.warnings,
        ["could not decode JSON message: depth_exceeded"]
    );
}

/// When an object repeats a key, ntcore keeps the first value.
#[test]
fn first_of_a_repeated_key_wins() {
    let d = decode_client_text(
        r#"[{"method":"unpublish","params":{"pubuid":1,"pubuid":2}},
            {"method":"setproperties","params":{"name":"","update":{"a":1,"a":2,"b":null}}}]"#,
    );
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.messages[0], ClientMessage::Unpublish { pubuid: 1 });
    let ClientMessage::SetProperties { update, .. } = &d.messages[1] else {
        panic!("{:?}", d.messages);
    };
    assert_eq!(update["a"], 1);
    // ntcore 2027's parser has a bug here and reads "b" as 2; we don't copy
    // it.
    assert!(update["b"].is_null());
}

/// ntcore 2027 holds times in nanoseconds, so it rejects a timestamp whose
/// microseconds times 1000 overflow an i64. We keep microseconds but reject
/// the same frames.
#[test]
fn timestamp_too_large_for_nanoseconds_is_rejected() {
    // [id 1, time, type 0 (boolean), true]
    let frame = |time: i64| {
        let mut f = vec![0x94, 0x01, 0xd3];
        f.extend_from_slice(&time.to_be_bytes());
        f.extend_from_slice(&[0x00, 0xc3]);
        f
    };
    let ok = frame(i64::MAX / 1000);
    assert!(
        BinaryReader::new(&ok)
            .collect::<Result<Vec<_>, _>>()
            .is_ok()
    );
    let bad = frame(i64::MAX / 1000 + 1);
    assert!(
        BinaryReader::new(&bad)
            .collect::<Result<Vec<_>, _>>()
            .is_err()
    );
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
