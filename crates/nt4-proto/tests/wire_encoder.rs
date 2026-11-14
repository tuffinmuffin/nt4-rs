//! Encoder golden vectors ported from ntcore's WireEncoderTest.

// Ported from allwpilib ntcore/src/test/native/cpp/net/WireEncoderTest.cpp
// at commit 4ccfe967ba1903b1f409b04b99db0922dd3b7318.
// Copyright (c) FIRST and other WPILib contributors.
// The expected bytes and text come from that file and stay under the WPILib
// BSD license; see tests/LICENSE-WPILIB.md. The Rust test code was written by
// Claude.
//
// Each test carries an `upstream:` marker with the Catch2 test name it ports.
// `scripts/upstream_audit.py` checks that every upstream case is ported or
// listed as not applicable.

use nt4_proto::{
    BinaryMessage, ClientMessage, NtType, Properties, ServerMessage, SubscriptionOptions,
    TextMessage, Value, encode_binary,
};

fn text<M: TextMessage>(m: &M) -> String {
    let mut out = String::new();
    m.encode(&mut out);
    out
}

fn k6() -> Properties {
    let mut p = Properties::new();
    p.insert("k".into(), 6.into());
    p
}

fn publish(properties: Properties) -> ClientMessage {
    ClientMessage::Publish {
        name: "test".into(),
        pubuid: 5,
        ty: NtType::Double,
        properties,
    }
}

fn subscribe(options: SubscriptionOptions) -> ClientMessage {
    ClientMessage::Subscribe {
        subuid: 5,
        topics: vec!["a".into(), "b".into()],
        options,
    }
}

fn announce(properties: Properties, pubuid: Option<i32>) -> ServerMessage {
    ServerMessage::Announce {
        name: "test".into(),
        id: 5,
        ty: NtType::Double,
        pubuid,
        properties,
    }
}

// upstream: "WireEncoderTextTest PublishPropsEmpty"
// upstream: "WireEncoderTextTest MessagePublish" (same message, other C++ entry point)
#[test]
fn publish_props_empty() {
    assert_eq!(
        text(&publish(Properties::new())),
        r#"{"method":"publish","params":{"name":"test","properties":{},"pubuid":5,"type":"double"}}"#
    );
}

// upstream: "WireEncoderTextTest PublishProps"
#[test]
fn publish_props() {
    assert_eq!(
        text(&publish(k6())),
        r#"{"method":"publish","params":{"name":"test","properties":{"k":6},"pubuid":5,"type":"double"}}"#
    );
}

// upstream: "WireEncoderTextTest Unpublish"
// upstream: "WireEncoderTextTest MessageUnpublish"
#[test]
fn unpublish() {
    assert_eq!(
        text(&ClientMessage::Unpublish { pubuid: 5 }),
        r#"{"method":"unpublish","params":{"pubuid":5}}"#
    );
}

// upstream: "WireEncoderTextTest SetProperties"
// upstream: "WireEncoderTextTest MessageSetProperties"
#[test]
fn set_properties() {
    let msg = ClientMessage::SetProperties {
        name: "test".into(),
        update: k6(),
    };
    assert_eq!(
        text(&msg),
        r#"{"method":"setproperties","params":{"name":"test","update":{"k":6}}}"#
    );
}

// upstream: "WireEncoderTextTest Subscribe"
// upstream: "WireEncoderTextTest MessageSubscribe"
#[test]
fn subscribe_default_options() {
    assert_eq!(
        text(&subscribe(SubscriptionOptions::default())),
        r#"{"method":"subscribe","params":{"options":{},"topics":["a","b"],"subuid":5}}"#
    );
}

// upstream: "WireEncoderTextTest SubscribeSendAll"
#[test]
fn subscribe_send_all() {
    let options = SubscriptionOptions {
        all: true,
        ..Default::default()
    };
    assert_eq!(
        text(&subscribe(options)),
        r#"{"method":"subscribe","params":{"options":{"all":true},"topics":["a","b"],"subuid":5}}"#
    );
}

// upstream: "WireEncoderTextTest SubscribePeriodic"
#[test]
fn subscribe_periodic() {
    let options = SubscriptionOptions {
        periodic: Some(0.5),
        ..Default::default()
    };
    assert_eq!(
        text(&subscribe(options)),
        r#"{"method":"subscribe","params":{"options":{"periodic":0.5},"topics":["a","b"],"subuid":5}}"#
    );
}

// upstream: "WireEncoderTextTest SubscribeAllOptions"
#[test]
fn subscribe_all_options() {
    let options = SubscriptionOptions {
        all: true,
        periodic: Some(0.5),
        ..Default::default()
    };
    assert_eq!(
        text(&subscribe(options)),
        r#"{"method":"subscribe","params":{"options":{"all":true,"periodic":0.5},"topics":["a","b"],"subuid":5}}"#
    );
}

/// Not in upstream: every option at once, in ntcore's key order.
#[test]
fn subscribe_every_option() {
    let options = SubscriptionOptions {
        all: true,
        topics_only: true,
        prefix: true,
        periodic: Some(0.25),
    };
    assert_eq!(
        text(&subscribe(options)),
        r#"{"method":"subscribe","params":{"options":{"all":true,"topicsonly":true,"prefix":true,"periodic":0.25},"topics":["a","b"],"subuid":5}}"#
    );
}

// upstream: "WireEncoderTextTest Unsubscribe"
// upstream: "WireEncoderTextTest MessageUnsubscribe"
#[test]
fn unsubscribe() {
    assert_eq!(
        text(&ClientMessage::Unsubscribe { subuid: 5 }),
        r#"{"method":"unsubscribe","params":{"subuid":5}}"#
    );
}

// upstream: "WireEncoderTextTest Announce"
// upstream: "WireEncoderTextTest MessageAnnounce"
#[test]
fn announce_plain() {
    assert_eq!(
        text(&announce(Properties::new(), None)),
        r#"{"method":"announce","params":{"id":5,"name":"test","properties":{},"type":"double"}}"#
    );
}

// upstream: "WireEncoderTextTest AnnounceProperties"
// upstream: "WireEncoderTextTest MessageAnnounceProperties"
#[test]
fn announce_properties() {
    assert_eq!(
        text(&announce(k6(), None)),
        r#"{"method":"announce","params":{"id":5,"name":"test","properties":{"k":6},"type":"double"}}"#
    );
}

// upstream: "WireEncoderTextTest AnnouncePubuid"
// upstream: "WireEncoderTextTest MessageAnnouncePubuid"
#[test]
fn announce_pubuid() {
    assert_eq!(
        text(&announce(Properties::new(), Some(6))),
        r#"{"method":"announce","params":{"id":5,"name":"test","properties":{},"pubuid":6,"type":"double"}}"#
    );
}

// upstream: "WireEncoderTextTest Unannounce"
// upstream: "WireEncoderTextTest MessageUnannounce"
#[test]
fn unannounce() {
    let msg = ServerMessage::Unannounce {
        name: "test".into(),
        id: 5,
    };
    assert_eq!(
        text(&msg),
        r#"{"method":"unannounce","params":{"id":5,"name":"test"}}"#
    );
}

/// Not in upstream as a test; key order from ntcore's WireEncodePropertiesUpdate.
#[test]
fn properties_update_with_ack() {
    let msg = ServerMessage::Properties {
        name: "test".into(),
        update: k6(),
        ack: true,
    };
    assert_eq!(
        text(&msg),
        r#"{"method":"properties","params":{"name":"test","update":{"k":6},"ack":true}}"#
    );
}

// Binary cases. ntcore's test passes 6000 ns, which it writes as 6 us; this
// crate takes microseconds directly, so the timestamp here is 6.

fn binary(value: Value) -> Vec<u8> {
    let mut out = Vec::new();
    encode_binary(
        &mut out,
        &BinaryMessage {
            id: 5,
            timestamp_us: 6,
            value,
        },
    );
    out
}

// upstream: "WireEncoderBinaryTest Boolean"
#[test]
fn binary_boolean() {
    assert_eq!(binary(Value::Boolean(true)), b"\x94\x05\x06\x00\xc3");
}

// upstream: "WireEncoderBinaryTest Integer"
#[test]
fn binary_integer() {
    assert_eq!(binary(Value::Int(7)), b"\x94\x05\x06\x02\x07");
}

// upstream: "WireEncoderBinaryTest Float"
#[test]
fn binary_float() {
    assert_eq!(
        binary(Value::Float(2.5)),
        b"\x94\x05\x06\x03\xca\x40\x20\x00\x00"
    );
}

// upstream: "WireEncoderBinaryTest Double"
#[test]
fn binary_double() {
    assert_eq!(
        binary(Value::Double(2.5)),
        b"\x94\x05\x06\x01\xcb\x40\x04\x00\x00\x00\x00\x00\x00"
    );
}

// upstream: "WireEncoderBinaryTest String"
#[test]
fn binary_string() {
    assert_eq!(
        binary(Value::String("hello".into())),
        b"\x94\x05\x06\x04\xa5hello"
    );
}

// upstream: "WireEncoderBinaryTest Raw"
#[test]
fn binary_raw() {
    assert_eq!(
        binary(Value::Raw(b"hello".to_vec())),
        b"\x94\x05\x06\x05\xc4\x05hello"
    );
}

// upstream: "WireEncoderBinaryTest BooleanArray"
#[test]
fn binary_boolean_array() {
    assert_eq!(
        binary(Value::BooleanArray(vec![true, false, true])),
        b"\x94\x05\x06\x10\x93\xc3\xc2\xc3"
    );
}

// upstream: "WireEncoderBinaryTest IntegerArray"
#[test]
fn binary_integer_array() {
    assert_eq!(
        binary(Value::IntArray(vec![1, 2, 4])),
        b"\x94\x05\x06\x12\x93\x01\x02\x04"
    );
}

// upstream: "WireEncoderBinaryTest FloatArray"
#[test]
fn binary_float_array() {
    assert_eq!(
        binary(Value::FloatArray(vec![1.0, 2.0, 3.0])),
        b"\x94\x05\x06\x13\x93\xca\x3f\x80\x00\x00\xca\x40\x00\x00\x00\xca\x40\x40\x00\x00"
    );
}

// upstream: "WireEncoderBinaryTest DoubleArray"
#[test]
fn binary_double_array() {
    assert_eq!(
        binary(Value::DoubleArray(vec![1.0, 2.0, 3.0])),
        b"\x94\x05\x06\x11\x93\
          \xcb\x3f\xf0\x00\x00\x00\x00\x00\x00\
          \xcb\x40\x00\x00\x00\x00\x00\x00\x00\
          \xcb\x40\x08\x00\x00\x00\x00\x00\x00"
    );
}

// upstream: "WireEncoderBinaryTest StringArray"
#[test]
fn binary_string_array() {
    assert_eq!(
        binary(Value::StringArray(vec!["hello".into(), "bye".into()])),
        b"\x94\x05\x06\x14\x92\xa5hello\xa3bye"
    );
}

/// The example from the NT4 spec's "Binary Data Frames" section: id 50,
/// timestamp of 2 minutes, double 0.1234. The spec writes the timestamp as
/// `int 32` (`d2`); ntcore's mpack and this crate write a positive integer as
/// `uint 32` (`ce`). Both are valid MessagePack, so the decoder must accept
/// the spec's bytes. (Decoding is checked in wire_decoder.rs.)
#[test]
fn spec_example_double() {
    let mut out = Vec::new();
    encode_binary(
        &mut out,
        &BinaryMessage {
            id: 50,
            timestamp_us: 120_000_000,
            value: Value::Double(0.1234),
        },
    );
    assert_eq!(hex::encode(&out), "9432ce07270e0001cb3fbf972474538ef3");
}

/// Not in upstream: negative ids (RTT) and large timestamps use signed or
/// unsigned forms like mpack_write_int.
#[test]
fn binary_rtt_and_wide_integers() {
    let mut out = Vec::new();
    encode_binary(
        &mut out,
        &BinaryMessage {
            id: -1,
            timestamp_us: 0,
            value: Value::Int(300),
        },
    );
    assert_eq!(hex::encode(&out), "94ff0002cd012c");
}
