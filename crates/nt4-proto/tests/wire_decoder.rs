//! Decoder golden vectors ported from ntcore's WireDecoderTest.

// Ported from allwpilib ntcore/src/test/native/cpp/net/WireDecoderTest.cpp
// at commit 4ccfe967ba1903b1f409b04b99db0922dd3b7318.
// Copyright (c) FIRST and other WPILib contributors.
// The input frames and expected warnings come from that file and stay under
// the WPILib BSD license; see tests/LICENSE-WPILIB.md. The Rust test code was
// written by Claude.
//
// Upstream "WireDecodeTextClientTest" decodes messages a client sent, which
// is `decode_client_text` here. ntcore's logger accumulates across calls in
// one test; here each frame's warnings are checked on their own.

use nt4_proto::{
    BinaryError, BinaryMessage, BinaryReader, ClientMessage, NtType, Properties, ServerMessage,
    SubscriptionOptions, Value, decode_client_text, decode_server_text,
};

fn k6() -> Properties {
    let mut p = Properties::new();
    p.insert("k".into(), 6.into());
    p
}

/// Assert a frame decodes to no messages and exactly these warnings.
fn rejects(frame: &str, warnings: &[&str]) {
    let d = decode_client_text(frame);
    assert!(d.messages.is_empty(), "{frame}: {:?}", d.messages);
    assert_eq!(d.warnings, warnings, "{frame}");
}

/// ntcore's JSON library names the parse error ("absent_value",
/// "unexpected_eof"); serde_json words it differently, so only the prefix is
/// compared.
fn rejects_json(frame: &str) {
    let d = decode_client_text(frame);
    assert!(d.messages.is_empty());
    assert_eq!(d.warnings.len(), 1);
    assert!(
        d.warnings[0].starts_with("could not decode JSON message: "),
        "{:?}",
        d.warnings
    );
}

// upstream: "WireDecodeTextClientTest EmptyArray"
#[test]
fn empty_array() {
    rejects("[]", &[]);
}

// upstream: "WireDecodeTextClientTest ErrorEmpty"
#[test]
fn error_empty() {
    rejects_json("");
}

// upstream: "WireDecodeTextClientTest ErrorBadJson1"
#[test]
fn error_bad_json_1() {
    rejects_json("[");
}

// upstream: "WireDecodeTextClientTest ErrorBadJson2"
#[test]
fn error_bad_json_2() {
    rejects_json("[{");
}

// upstream: "WireDecodeTextClientTest ErrorNotArray"
#[test]
fn error_not_array() {
    rejects("{}", &["expected JSON array at top level"]);
}

// upstream: "WireDecodeTextClientTest ErrorMessageNotObject"
#[test]
fn error_message_not_object() {
    rejects("[5]", &["0: expected message to be an object"]);
}

// upstream: "WireDecodeTextClientTest ErrorNoMethodKey"
#[test]
fn error_no_method_key() {
    rejects("[{}]", &["0: no method key"]);
}

// upstream: "WireDecodeTextClientTest ErrorMethodNotString"
#[test]
fn error_method_not_string() {
    rejects(r#"[{"method":5}]"#, &["0: method must be a string"]);
}

// upstream: "WireDecodeTextClientTest ErrorNoParamsKey"
#[test]
fn error_no_params_key() {
    rejects(r#"[{"method":"a"}]"#, &["0: no params key"]);
}

// upstream: "WireDecodeTextClientTest ErrorParamsNotObject"
#[test]
fn error_params_not_object() {
    rejects(
        r#"[{"method":"a","params":5}]"#,
        &["0: params must be an object"],
    );
}

// upstream: "WireDecodeTextClientTest ErrorUnknownMethod"
#[test]
fn error_unknown_method() {
    rejects(
        r#"[{"method":"a","params":{}}]"#,
        &["0: unrecognized method 'a'"],
    );
}

// upstream: "WireDecodeTextClientTest PublishPropsEmpty"
#[test]
fn publish_props_empty() {
    let expected = vec![ClientMessage::Publish {
        name: "test".into(),
        pubuid: 5,
        ty: NtType::Double,
        properties: Properties::new(),
    }];
    let d = decode_client_text(
        r#"[{"method":"publish","params":{"name":"test","properties":{},"pubuid":5,"type":"double"}}]"#,
    );
    assert!(d.warnings.is_empty());
    assert_eq!(d.messages, expected);

    // A missing properties key is treated as empty.
    let d = decode_client_text(
        r#"[{"method":"publish","params":{"name":"test","pubuid":5,"type":"double"}}]"#,
    );
    assert!(d.warnings.is_empty());
    assert_eq!(d.messages, expected);
}

// upstream: "WireDecodeTextClientTest PublishProps"
#[test]
fn publish_props() {
    let d = decode_client_text(
        r#"[{"method":"publish","params":{"name":"test","properties":{"k":6},"pubuid":5,"type":"double"}}]"#,
    );
    assert!(d.warnings.is_empty());
    assert_eq!(
        d.messages,
        vec![ClientMessage::Publish {
            name: "test".into(),
            pubuid: 5,
            ty: NtType::Double,
            properties: k6(),
        }]
    );
}

// upstream: "WireDecodeTextClientTest PublishPropsError"
#[test]
fn publish_props_error() {
    rejects(
        r#"[{"method":"publish","params":{"name":"test","properties":["k"],"pubuid":5,"type":"double"}}]"#,
        &["0: properties must be an object"],
    );
}

// upstream: "WireDecodeTextClientTest PublishError"
#[test]
fn publish_error() {
    rejects(
        r#"[{"method":"publish","params":{"pubuid":5,"type":"double"}}]"#,
        &["0: no name key"],
    );
    rejects(
        r#"[{"method":"publish","params":{"name":"test","pubuid":5}}]"#,
        &["0: no type key"],
    );
    rejects(
        r#"[{"method":"publish","params":{"name":"test","type":"double"}}]"#,
        &["0: no pubuid key"],
    );
}

// upstream: "WireDecodeTextClientTest Unpublish"
#[test]
fn unpublish() {
    let d = decode_client_text(r#"[{"method":"unpublish","params":{"pubuid":5}}]"#);
    assert!(d.warnings.is_empty());
    assert_eq!(d.messages, vec![ClientMessage::Unpublish { pubuid: 5 }]);
}

// upstream: "WireDecodeTextClientTest PeriodicOutOfRange"
#[test]
fn periodic_out_of_range() {
    rejects(
        r#"[{"method":"subscribe","params":{"subuid":1,"topics":["test"],"options":{"periodic":-1}}}]"#,
        &["0: periodic value out of range"],
    );
    rejects(
        r#"[{"method":"subscribe","params":{"subuid":2,"topics":["test"],"options":{"periodic":1e100}}}]"#,
        &["0: periodic value out of range"],
    );
}

// upstream: "WireDecodeTextClientTest UnpublishMultiple"
#[test]
fn unpublish_multiple() {
    let d = decode_client_text(
        r#"[{"method":"unpublish","params":{"pubuid":5}},{"method":"unpublish","params":{"pubuid":6}}]"#,
    );
    assert!(d.warnings.is_empty());
    assert_eq!(
        d.messages,
        vec![
            ClientMessage::Unpublish { pubuid: 5 },
            ClientMessage::Unpublish { pubuid: 6 }
        ]
    );
}

// upstream: "WireDecodeTextClientTest UnpublishError"
#[test]
fn unpublish_error() {
    rejects(
        r#"[{"method":"unpublish","params":{}}]"#,
        &["0: no pubuid key"],
    );
    rejects(
        r#"[{"method":"unpublish","params":{"pubuid":"5"}}]"#,
        &["0: pubuid must be a number"],
    );
}

// upstream: "WireDecodeBinary rejects overflowing timestamp adjustments"
// ntcore fails when converting wire microseconds to its internal nanoseconds
// overflows. This crate keeps microseconds, so the matching case is a wire
// timestamp that doesn't fit in an i64 at all.
#[test]
fn binary_rejects_out_of_range_timestamp() {
    let item = rmpv::Value::Array(vec![1.into(), u64::MAX.into(), 2.into(), 1.into()]);
    let mut bytes = Vec::new();
    rmpv::encode::write_value(&mut bytes, &item).unwrap();
    assert_eq!(
        BinaryReader::new(&bytes).next(),
        Some(Err(BinaryError::Timestamp))
    );
}

// The cases below are not upstream tests; they cover decoder rules from
// ntcore's WireDecoder.cpp and the spec.

/// A bad message is skipped and the rest of the frame still decodes.
#[test]
fn bad_message_does_not_drop_the_frame() {
    let d = decode_client_text(
        r#"[{"method":"unpublish","params":{}},{"method":"unsubscribe","params":{"subuid":7}}]"#,
    );
    assert_eq!(d.warnings, vec!["0: no pubuid key"]);
    assert_eq!(d.messages, vec![ClientMessage::Unsubscribe { subuid: 7 }]);
}

#[test]
fn ids_outside_32_bits_are_rejected() {
    rejects(
        r#"[{"method":"unpublish","params":{"pubuid":2147483647}}]"#,
        &["0: pubuid out of range"],
    );
    rejects(
        r#"[{"method":"unpublish","params":{"pubuid":-2147483648}}]"#,
        &["0: pubuid out of range"],
    );
    let d = decode_client_text(r#"[{"method":"unpublish","params":{"pubuid":2147483646}}]"#);
    assert_eq!(
        d.messages,
        vec![ClientMessage::Unpublish { pubuid: 2147483646 }]
    );
}

#[test]
fn non_integer_id_is_not_a_number() {
    rejects(
        r#"[{"method":"unpublish","params":{"pubuid":5.0}}]"#,
        &["0: pubuid must be a number"],
    );
}

#[test]
fn subscribe_with_every_option() {
    let d = decode_client_text(
        r#"[{"method":"subscribe","params":{"subuid":3,"topics":["/a",""],"options":{"periodic":0.02,"all":true,"topicsonly":false,"prefix":true}}}]"#,
    );
    assert!(d.warnings.is_empty());
    assert_eq!(
        d.messages,
        vec![ClientMessage::Subscribe {
            subuid: 3,
            topics: vec!["/a".into(), "".into()],
            options: SubscriptionOptions {
                periodic: Some(0.02),
                all: true,
                topics_only: false,
                prefix: true,
            },
        }]
    );
}

#[test]
fn subscribe_errors() {
    rejects(
        r#"[{"method":"subscribe","params":{"subuid":1}}]"#,
        &["0: no topics key"],
    );
    rejects(
        r#"[{"method":"subscribe","params":{"subuid":1,"topics":"a"}}]"#,
        &["0: topics must be an array"],
    );
    rejects(
        r#"[{"method":"subscribe","params":{"subuid":1,"topics":["a",5]}}]"#,
        &["0: topics/1 must be a string"],
    );
    rejects(
        r#"[{"method":"subscribe","params":{"subuid":1,"topics":[],"options":[]}}]"#,
        &["0: options must be an object"],
    );
    rejects(
        r#"[{"method":"subscribe","params":{"subuid":1,"topics":[],"options":{"all":1}}}]"#,
        &["0: all value must be a boolean"],
    );
    rejects(
        r#"[{"method":"subscribe","params":{"subuid":1,"topics":[],"options":{"periodic":"x"}}}]"#,
        &["0: periodic value must be a number"],
    );
}

#[test]
fn set_properties_needs_an_update_object() {
    let d = decode_client_text(
        r#"[{"method":"setproperties","params":{"name":"test","update":{"k":6}}}]"#,
    );
    assert_eq!(
        d.messages,
        vec![ClientMessage::SetProperties {
            name: "test".into(),
            update: k6()
        }]
    );
    rejects(
        r#"[{"method":"setproperties","params":{"name":"test"}}]"#,
        &["0: no update key"],
    );
    rejects(
        r#"[{"method":"setproperties","params":{"name":"test","update":5}}]"#,
        &["0: update must be an object"],
    );
}

#[test]
fn server_messages_decode() {
    let d = decode_server_text(
        r#"[{"method":"announce","params":{"id":5,"name":"test","properties":{"k":6},"pubuid":6,"type":"struct:Pose2d"}},
            {"method":"unannounce","params":{"id":5,"name":"test"}},
            {"method":"properties","params":{"name":"test","update":{"k":null},"ack":true}}]"#,
    );
    assert!(d.warnings.is_empty());
    let mut gone = Properties::new();
    gone.insert("k".into(), serde_json::Value::Null);
    assert_eq!(
        d.messages,
        vec![
            ServerMessage::Announce {
                name: "test".into(),
                id: 5,
                ty: NtType::Struct {
                    name: "Pose2d".into(),
                    array: false
                },
                pubuid: Some(6),
                properties: k6(),
            },
            ServerMessage::Unannounce {
                name: "test".into(),
                id: 5
            },
            ServerMessage::Properties {
                name: "test".into(),
                update: gone,
                ack: true
            },
        ]
    );
}

#[test]
fn announce_rules() {
    // Properties are required on announce, unlike publish.
    let d =
        decode_server_text(r#"[{"method":"announce","params":{"id":5,"name":"t","type":"int"}}]"#);
    assert!(d.messages.is_empty());
    assert_eq!(d.warnings, vec!["0: no properties key"]);

    // Non-object properties: ntcore warns and keeps the message with none.
    let d = decode_server_text(
        r#"[{"method":"announce","params":{"id":5,"name":"t","type":"int","properties":5}}]"#,
    );
    assert_eq!(d.warnings, vec!["t: properties is not an object"]);
    assert_eq!(d.messages.len(), 1);

    // A client must not accept client-to-server methods.
    let d = decode_server_text(r#"[{"method":"publish","params":{}}]"#);
    assert_eq!(d.warnings, vec!["0: unrecognized method 'publish'"]);
}

/// The spec's 17-byte example, with its `int 32` timestamp.
#[test]
fn binary_spec_example() {
    let bytes = hex::decode("9432d207270e0001cb3fbf972474538ef3").unwrap();
    let msgs: Vec<_> = BinaryReader::new(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(
        msgs,
        vec![BinaryMessage {
            id: 50,
            timestamp_us: 120_000_000,
            value: Value::Double(0.1234)
        }]
    );
}

/// Upstream's encoder vectors decode back to the same values.
#[test]
fn binary_upstream_vectors_round_trip() {
    let cases: [(&[u8], Value); 11] = [
        (b"\x94\x05\x06\x00\xc3", Value::Boolean(true)),
        (b"\x94\x05\x06\x02\x07", Value::Int(7)),
        (b"\x94\x05\x06\x03\xca\x40\x20\x00\x00", Value::Float(2.5)),
        (
            b"\x94\x05\x06\x01\xcb\x40\x04\x00\x00\x00\x00\x00\x00",
            Value::Double(2.5),
        ),
        (b"\x94\x05\x06\x04\xa5hello", Value::String("hello".into())),
        (b"\x94\x05\x06\x05\xc4\x05hello", Value::Raw(b"hello".to_vec())),
        (
            b"\x94\x05\x06\x10\x93\xc3\xc2\xc3",
            Value::BooleanArray(vec![true, false, true]),
        ),
        (
            b"\x94\x05\x06\x12\x93\x01\x02\x04",
            Value::IntArray(vec![1, 2, 4]),
        ),
        (
            b"\x94\x05\x06\x13\x93\xca\x3f\x80\x00\x00\xca\x40\x00\x00\x00\xca\x40\x40\x00\x00",
            Value::FloatArray(vec![1.0, 2.0, 3.0]),
        ),
        (
            b"\x94\x05\x06\x11\x93\xcb\x3f\xf0\x00\x00\x00\x00\x00\x00\xcb\x40\x00\x00\x00\x00\x00\x00\x00\xcb\x40\x08\x00\x00\x00\x00\x00\x00",
            Value::DoubleArray(vec![1.0, 2.0, 3.0]),
        ),
        (
            b"\x94\x05\x06\x14\x92\xa5hello\xa3bye",
            Value::StringArray(vec!["hello".into(), "bye".into()]),
        ),
    ];
    for (bytes, value) in cases {
        let msgs: Vec<_> = BinaryReader::new(bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(
            msgs,
            vec![BinaryMessage {
                id: 5,
                timestamp_us: 6,
                value
            }]
        );
    }
}

/// Like mpack's expect_double, any number type is accepted for doubles and
/// floats; ints must be integers.
#[test]
fn binary_number_coercion() {
    let enc = |ty: u8, v: rmpv::Value| {
        let mut b = Vec::new();
        rmpv::encode::write_value(
            &mut b,
            &rmpv::Value::Array(vec![1.into(), 0.into(), ty.into(), v]),
        )
        .unwrap();
        BinaryReader::new(&b).next().unwrap()
    };
    assert_eq!(enc(1, 2.into()).unwrap().value, Value::Double(2.0));
    assert_eq!(
        enc(3, rmpv::Value::F64(0.5)).unwrap().value,
        Value::Float(0.5)
    );
    assert_eq!(
        enc(2, rmpv::Value::F64(1.0)),
        Err(BinaryError::ValueMismatch(2))
    );
    assert_eq!(enc(0, 1.into()), Err(BinaryError::ValueMismatch(0)));
    assert_eq!(enc(6, 1.into()), Err(BinaryError::UnknownType(6)));
    assert_eq!(
        enc(4, rmpv::Value::Binary(b"x".to_vec())),
        Err(BinaryError::ValueMismatch(4))
    );
}
