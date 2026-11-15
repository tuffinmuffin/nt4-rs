//! Server cases ported from ntcore's ServerImplTest.

// Ported from allwpilib ntcore/src/test/native/cpp/server/ServerImplTest.cpp
// at commit 4ccfe967ba1903b1f409b04b99db0922dd3b7318.
// Copyright (c) FIRST and other WPILib contributors.
// The test cases come from that file and stay under the WPILib BSD license;
// see tests/LICENSE-WPILIB.md. The Rust test code was written by Claude.
//
// Upstream drives `ServerImpl` with mocks. Here:
// - The local `MockServerMessageHandler` and `MockClientMessageQueue` are
//   the server's own API (`publish`, `set_value`, `subscribe`, ...) and
//   `poll_event`. Upstream queues local messages and processes them with
//   `ProcessLocalMessages`; `ServerCore` applies them at once, which is the
//   same thing.
// - A `MockWireConnection` is `poll_transmit` for that client. Its calls map
//   to frames: `SendPing` is `Transmit::Ping`, and the `WriteText` (or
//   `WriteBinary`) calls up to a `Flush` are one text (or binary) frame.
// - The `setPeriodic` callback is `tick_period_ms`, which returns the
//   latest period (capped at 100 ms) rather than recording each call.
// - Times passed to `SendOutgoing` are milliseconds upstream and
//   microseconds here.

use nt4_core::{ClientId, ProtocolVersion, ServerCore, ServerEvent, Transmit};
use nt4_proto::{
    BinaryMessage, BinaryReader, ClientMessage, NtType, Properties, ServerMessage,
    SubscriptionOptions, Value, encode_binary, encode_text_frame,
};

fn ms(t: i64) -> i64 {
    t * 1000
}

fn add(server: &mut ServerCore, name: &str, version: ProtocolVersion) -> ClientId {
    server.add_client(name, name, false, version).0
}

/// Every frame queued for a client.
fn drain(server: &mut ServerCore, id: ClientId) -> Vec<Transmit> {
    std::iter::from_fn(|| server.poll_transmit(id)).collect()
}

/// The value messages in a client's queued binary frames.
fn drain_values(server: &mut ServerCore, id: ClientId) -> Vec<BinaryMessage> {
    drain(server, id)
        .into_iter()
        .filter_map(|t| match t {
            Transmit::Binary(b) => Some(b),
            _ => None,
        })
        .flat_map(|b| {
            BinaryReader::new(&b)
                .collect::<Result<Vec<_>, _>>()
                .expect("valid binary frame")
        })
        .collect()
}

/// Events for the server's own client, without warnings.
fn local_events(server: &mut ServerCore) -> Vec<ServerEvent> {
    std::iter::from_fn(|| server.poll_event())
        .filter(|e| !matches!(e, ServerEvent::Warning(_)))
        .collect()
}

fn warnings(server: &mut ServerCore) -> Vec<String> {
    std::iter::from_fn(|| server.poll_event())
        .filter_map(|e| match e {
            ServerEvent::Warning(w) => Some(w),
            _ => None,
        })
        .collect()
}

fn announce(name: &str, id: i32, pubuid: Option<i32>) -> ServerMessage {
    ServerMessage::Announce {
        name: name.into(),
        id,
        ty: NtType::Double,
        pubuid,
        properties: Properties::new(),
    }
}

fn text(msgs: &[ServerMessage]) -> Transmit {
    Transmit::Text(encode_text_frame(msgs))
}

fn value_frame(id: i32, timestamp_us: i64, value: Value) -> Vec<u8> {
    let mut out = Vec::new();
    encode_binary(
        &mut out,
        &BinaryMessage {
            id,
            timestamp_us,
            value,
        },
    );
    out
}

fn process_publish(server: &mut ServerCore, id: ClientId, pubuid: i32, name: &str) {
    let msg = ClientMessage::Publish {
        name: name.into(),
        pubuid,
        ty: NtType::Double,
        properties: Properties::new(),
    };
    server.handle_text(id, 0, &encode_text_frame(&[msg]));
}

fn process_subscribe(
    server: &mut ServerCore,
    id: ClientId,
    subuid: i32,
    topics: &[&str],
    options: SubscriptionOptions,
) {
    let msg = ClientMessage::Subscribe {
        subuid,
        topics: topics.iter().map(|t| t.to_string()).collect(),
        options,
    };
    server.handle_text(id, 0, &encode_text_frame(&[msg]));
}

fn process_unsubscribe(server: &mut ServerCore, id: ClientId, subuid: i32) {
    let msg = ClientMessage::Unsubscribe { subuid };
    server.handle_text(id, 0, &encode_text_frame(&[msg]));
}

fn process_value(server: &mut ServerCore, id: ClientId, pubuid: i32, time_us: i64, v: f64) {
    server.handle_binary(id, 0, &value_frame(pubuid, time_us, Value::Double(v)));
}

fn drain_and_clear(server: &mut ServerCore, id: ClientId, cur_time_ms: i64) {
    server.send_all_outgoing(ms(cur_time_ms), true);
    drain(server, id);
}

fn prefix() -> SubscriptionOptions {
    SubscriptionOptions {
        prefix: true,
        ..Default::default()
    }
}

fn send_all() -> SubscriptionOptions {
    SubscriptionOptions {
        all: true,
        ..Default::default()
    }
}

fn double() -> NtType {
    NtType::Double
}

// upstream: "ServerImplTest AddClient"
#[test]
fn add_client() {
    let mut server = ServerCore::new();
    let (_, name) = server.add_client("test", "connInfo", false, ProtocolVersion::V4_1);
    assert_eq!(name, "test@1");
}

// upstream: "ServerImplTest AddDuplicateClient"
#[test]
fn add_duplicate_client() {
    let mut server = ServerCore::new();
    let (id1, name1) = server.add_client("test", "connInfo", false, ProtocolVersion::V4_1);
    let (id2, name2) = server.add_client("test", "connInfo2", false, ProtocolVersion::V4_1);
    assert_eq!(name1, "test@1");
    assert_eq!(name2, "test@2");
    assert_ne!(id1, id2);
}

// upstream: "ServerImplTest PublishLocal"
#[test]
fn publish_local() {
    let mut server = ServerCore::new();
    // publish before client connect
    let p1 = server.publish("test", double(), Properties::new());

    // client connect; it should get already-published topic as soon as it
    // subscribes
    let id = add(&mut server, "test", ProtocolVersion::V4_1);
    process_subscribe(&mut server, id, 1, &[""], prefix());

    // publish before send control
    let p2 = server.publish("test2", double(), Properties::new());

    server.send_all_outgoing(ms(100), false);

    // publish after send control
    let p3 = server.publish("test3", double(), Properties::new());

    server.send_all_outgoing(ms(200), false);

    let events = local_events(&mut server);
    let announced: Vec<_> = events
        .iter()
        .map(|e| match e {
            ServerEvent::Announced {
                name, publisher, ..
            } => (name.to_string(), *publisher),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        announced,
        [
            ("test".into(), Some(p1)),
            ("test2".into(), Some(p2)),
            ("test3".into(), Some(p3)),
        ]
    );

    assert_eq!(server.tick_period_ms(id), 100);
    assert_eq!(
        drain(&mut server, id),
        [
            Transmit::Ping,
            text(&[announce("test", 3, None), announce("test2", 8, None)]),
            text(&[announce("test3", 11, None)]),
        ]
    );
}

// upstream: "ServerImplTest ClientSubTopicOnlyThenValue"
#[test]
fn client_sub_topic_only_then_value() {
    let mut server = ServerCore::new();
    // publish before client connect
    let p = server.publish("test", double(), Properties::new());
    server.set_value(p, Value::Double(1.0), 10).unwrap();

    // connect client
    let id = add(&mut server, "test", ProtocolVersion::V4_1);

    // subscribe topics only; will not send value
    process_subscribe(
        &mut server,
        id,
        1,
        &[""],
        SubscriptionOptions {
            topics_only: true,
            prefix: true,
            ..Default::default()
        },
    );

    server.send_outgoing(id, ms(100), false);
    let first = drain(&mut server, id);

    // subscribe normal; will not resend announcement, but will send value
    process_subscribe(
        &mut server,
        id,
        2,
        &["test"],
        SubscriptionOptions::default(),
    );

    server.send_outgoing(id, ms(200), false);
    let second = drain(&mut server, id);

    let events = local_events(&mut server);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        ServerEvent::Announced { name, publisher: Some(x), .. } if &**name == "test" && *x == p
    ));
    assert_eq!(server.tick_period_ms(id), 100);
    assert_eq!(first, [Transmit::Ping, text(&[announce("test", 3, None)])]);
    assert_eq!(
        second,
        [Transmit::Binary(value_frame(3, 10, Value::Double(1.0)))]
    );
}

// upstream: "ServerImplTest ClientDisconnectUnpublish"
#[test]
fn client_disconnect_unpublish() {
    let mut server = ServerCore::new();
    let p = server.publish("test2", double(), Properties::new());
    server.set_value(p, Value::Double(1.0), 10).unwrap();
    server.subscribe(vec!["test".into()], SubscriptionOptions::default());

    // connect client
    let id = add(&mut server, "test", ProtocolVersion::V4_1);

    // publish topic
    process_publish(&mut server, id, 1, "test");

    server.send_outgoing(id, ms(100), false);
    let frames = drain(&mut server, id);

    // disconnect client
    server.remove_client(id);

    let events = local_events(&mut server);
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(matches!(
        &events[0],
        ServerEvent::Announced { name, publisher: Some(x), .. } if &**name == "test2" && *x == p
    ));
    assert!(matches!(
        &events[1],
        ServerEvent::Announced { name, publisher: None, .. } if &**name == "test"
    ));
    assert!(matches!(
        &events[2],
        ServerEvent::Unannounced { name, .. } if &**name == "test"
    ));
    assert_eq!(
        frames,
        [Transmit::Ping, text(&[announce("test", 8, Some(1))])]
    );
}

// upstream: "ServerImplTest ZeroTimestampNegativeTime"
#[test]
fn zero_timestamp_negative_time() {
    let mut server = ServerCore::new();
    // publish before client connect
    let p = server.publish("test", double(), Properties::new());
    server.set_value(p, Value::Double(1.0), 0).unwrap();
    server.subscribe(vec!["test".into()], SubscriptionOptions::default());

    // client connect
    let id = add(&mut server, "test", ProtocolVersion::V4_1);

    // publish and send non-default value with negative time offset
    process_publish(&mut server, id, 2, "test");
    process_value(&mut server, id, 2, -10_000, 5.0);

    let events = local_events(&mut server);
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(matches!(
        &events[0],
        ServerEvent::Announced { name, publisher: Some(x), .. } if &**name == "test" && *x == p
    ));
    let values: Vec<_> = events[1..]
        .iter()
        .map(|e| match e {
            ServerEvent::Value { name, value, .. } => {
                assert_eq!(&**name, "test");
                (value.server_time_us, value.value.clone())
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        values,
        [(0, Value::Double(1.0)), (-10_000, Value::Double(5.0))]
    );
}

// upstream: "ServerImplTest ResubscribeShorterPeriodUpdatesTopicOutgoing"
#[test]
fn resubscribe_shorter_period_updates_topic_outgoing() {
    // When a client re-subscribes with a shorter period for a topic it
    // already matched, the topic must move to the shorter period's queue.
    let mut server = ServerCore::new();
    let p = server.publish("test", double(), Properties::new());
    server.set_value(p, Value::Double(1.0), 50).unwrap();

    let id = add(&mut server, "test", ProtocolVersion::V4_1);

    // Subscribe with period=200ms; the initial value is queued in the 200 ms
    // queue.
    let every = |periodic| SubscriptionOptions {
        periodic: Some(periodic),
        prefix: true,
        ..Default::default()
    };
    process_subscribe(&mut server, id, 1, &[""], every(0.2));

    // First send at t=100 sends the announce and the value; the 200 ms
    // queue is next due at 200.
    server.send_outgoing(id, ms(100), false);
    let first = drain(&mut server, id);

    // Re-subscribe with period=100ms; the last value is queued again, in the
    // 100 ms queue, which is due now.
    process_subscribe(&mut server, id, 1, &[""], every(0.1));

    // At t=150 (between 100ms and 200ms) the value goes out.
    server.send_outgoing(id, ms(150), false);
    let second = drain(&mut server, id);

    let value = Transmit::Binary(value_frame(3, 50, Value::Double(1.0)));
    assert_eq!(
        first,
        [
            Transmit::Ping,
            text(&[announce("test", 3, None)]),
            value.clone()
        ]
    );
    assert_eq!(
        second,
        [value],
        "re-subscribing with a shorter period must update the topic's outgoing queue period"
    );
}

/// A publishing client and a subscribing client on NT 4.0 (no pings), with
/// `topic` published.
fn pub_sub(name: &str) -> (ServerCore, ClientId, ClientId) {
    let mut server = ServerCore::new();
    let pub_id = add(&mut server, "pub", ProtocolVersion::V4_0);
    let sub_id = add(&mut server, "sub", ProtocolVersion::V4_0);
    process_publish(&mut server, pub_id, 1, name);
    (server, pub_id, sub_id)
}

// upstream: "ServerImplTest ReplacingSendAllSubscriptionRestoresCoalescing"
#[test]
fn replacing_send_all_subscription_restores_coalescing() {
    let (mut server, pub_id, sub_id) = pub_sub("topic");
    process_subscribe(&mut server, sub_id, 1, &["topic"], send_all());
    drain_and_clear(&mut server, sub_id, 5);

    process_subscribe(
        &mut server,
        sub_id,
        1,
        &["topic"],
        SubscriptionOptions::default(),
    );
    drain_and_clear(&mut server, sub_id, 10);

    process_value(&mut server, pub_id, 1, 10_000, 1.0);
    process_value(&mut server, pub_id, 1, 20_000, 2.0);
    server.send_all_outgoing(ms(15), true);

    let values = drain_values(&mut server, sub_id);
    assert_eq!(values.len(), 1);
    assert!(values[0].id >= 0);
    assert_eq!(values[0].timestamp_us, 20_000);
    assert_eq!(values[0].value, Value::Double(2.0));
}

// upstream: "ServerImplTest RemovingSendAllSubscriptionLeavesNormalCoalescing"
#[test]
fn removing_send_all_subscription_leaves_normal_coalescing() {
    let (mut server, pub_id, sub_id) = pub_sub("topic");
    process_subscribe(
        &mut server,
        sub_id,
        1,
        &["topic"],
        SubscriptionOptions::default(),
    );
    process_subscribe(&mut server, sub_id, 2, &["topic"], send_all());
    drain_and_clear(&mut server, sub_id, 5);

    process_unsubscribe(&mut server, sub_id, 2);
    drain_and_clear(&mut server, sub_id, 10);

    process_value(&mut server, pub_id, 1, 10_000, 1.0);
    process_value(&mut server, pub_id, 1, 20_000, 2.0);
    server.send_all_outgoing(ms(15), true);

    let values = drain_values(&mut server, sub_id);
    assert_eq!(values.len(), 1);
    assert!(values[0].id >= 0);
    assert_eq!(values[0].timestamp_us, 20_000);
    assert_eq!(values[0].value, Value::Double(2.0));
}

// upstream: "ServerImplTest RemovingFinalValueSubscriptionStopsFutureDelivery"
#[test]
fn removing_final_value_subscription_stops_future_delivery() {
    let (mut server, pub_id, sub_id) = pub_sub("topic");
    process_subscribe(
        &mut server,
        sub_id,
        1,
        &["topic"],
        SubscriptionOptions::default(),
    );
    drain_and_clear(&mut server, sub_id, 5);

    process_value(&mut server, pub_id, 1, 10_000, 1.0);
    process_unsubscribe(&mut server, sub_id, 1);
    server.send_all_outgoing(ms(10), true);

    // The value queued before the unsubscribe still goes out.
    let values = drain_values(&mut server, sub_id);
    assert_eq!(values.len(), 1);
    assert!(values[0].id >= 0);
    assert_eq!(values[0].timestamp_us, 10_000);
    assert_eq!(values[0].value, Value::Double(1.0));

    process_value(&mut server, pub_id, 1, 20_000, 2.0);
    server.send_all_outgoing(ms(15), true);
    assert!(drain_values(&mut server, sub_id).is_empty());
}

// upstream: "ServerImplTest ReplacingValueSubscriptionWithTopicsOnlyStopsFutureDelivery"
#[test]
fn replacing_value_subscription_with_topics_only_stops_future_delivery() {
    let (mut server, pub_id, sub_id) = pub_sub("topic");
    process_subscribe(
        &mut server,
        sub_id,
        1,
        &["topic"],
        SubscriptionOptions::default(),
    );
    drain_and_clear(&mut server, sub_id, 5);

    process_subscribe(
        &mut server,
        sub_id,
        1,
        &["topic"],
        SubscriptionOptions {
            topics_only: true,
            ..Default::default()
        },
    );
    drain_and_clear(&mut server, sub_id, 10);

    process_value(&mut server, pub_id, 1, 10_000, 1.0);
    process_value(&mut server, pub_id, 1, 20_000, 2.0);
    server.send_all_outgoing(ms(15), true);

    assert!(drain_values(&mut server, sub_id).is_empty());
}

// upstream: "ServerImplTest ReplacingSubscriptionTopicSelectionStopsOldTopicDelivery"
#[test]
fn replacing_subscription_topic_selection_stops_old_topic_delivery() {
    let (mut server, pub_id, sub_id) = pub_sub("old");
    process_subscribe(
        &mut server,
        sub_id,
        1,
        &["old"],
        SubscriptionOptions::default(),
    );
    drain_and_clear(&mut server, sub_id, 5);

    process_subscribe(
        &mut server,
        sub_id,
        1,
        &["new"],
        SubscriptionOptions::default(),
    );
    drain_and_clear(&mut server, sub_id, 10);

    process_value(&mut server, pub_id, 1, 10_000, 1.0);
    process_value(&mut server, pub_id, 1, 20_000, 2.0);
    server.send_all_outgoing(ms(15), true);

    assert!(drain_values(&mut server, sub_id).is_empty());
}

// upstream: "ServerImplTest RemainingOverlappingSendAllSubscriptionKeepsAll"
#[test]
fn remaining_overlapping_send_all_subscription_keeps_all() {
    let (mut server, pub_id, sub_id) = pub_sub("topic");
    process_subscribe(&mut server, sub_id, 1, &["topic"], send_all());
    process_subscribe(
        &mut server,
        sub_id,
        2,
        &["top"],
        SubscriptionOptions {
            all: true,
            prefix: true,
            ..Default::default()
        },
    );
    drain_and_clear(&mut server, sub_id, 5);

    process_unsubscribe(&mut server, sub_id, 1);
    drain_and_clear(&mut server, sub_id, 10);

    process_value(&mut server, pub_id, 1, 10_000, 1.0);
    process_value(&mut server, pub_id, 1, 20_000, 2.0);
    server.send_all_outgoing(ms(15), true);

    let values = drain_values(&mut server, sub_id);
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].id, values[1].id);
    assert_eq!(values[0].timestamp_us, 10_000);
    assert_eq!(values[0].value, Value::Double(1.0));
    assert_eq!(values[1].timestamp_us, 20_000);
    assert_eq!(values[1].value, Value::Double(2.0));
}

// upstream: "ServerImplTest LocalUnsubscribeStopsFutureDelivery"
#[test]
fn local_unsubscribe_stops_future_delivery() {
    let mut server = ServerCore::new();
    let pub_id = add(&mut server, "pub", ProtocolVersion::V4_0);
    process_publish(&mut server, pub_id, 1, "topic");
    let sub = server.subscribe(vec!["topic".into()], SubscriptionOptions::default());
    local_events(&mut server);

    server.unsubscribe(sub);

    process_value(&mut server, pub_id, 1, 10_000, 1.0);

    assert!(
        local_events(&mut server)
            .iter()
            .all(|e| !matches!(e, ServerEvent::Value { .. }))
    );
}

// upstream: "ServerImplTest InvalidPubUid"
#[test]
fn invalid_pub_uid() {
    let mut server = ServerCore::new();

    // connect client
    let id = add(&mut server, "test", ProtocolVersion::V4_1);

    server.handle_text(
        id,
        0,
        r#"[{"method":"publish","params":{"type":"string","name":"myvalue","pubuid":2147483647,"properties":{}}}]"#,
    );
    let events: Vec<_> = std::iter::from_fn(|| server.poll_event()).collect();
    assert_eq!(
        events,
        [ServerEvent::Warning("0: pubuid out of range".into())]
    );
}

// upstream: "ServerImplTest RejectsNetworkPublishOfReservedTopic"
#[test]
fn rejects_network_publish_of_reserved_topic() {
    let mut server = ServerCore::new();
    let id = add(&mut server, "test", ProtocolVersion::V4_1);
    let msg = ClientMessage::Publish {
        name: "$clients".into(),
        pubuid: 1,
        ty: NtType::MsgPack,
        properties: Properties::new(),
    };

    server.handle_text(id, 0, &encode_text_frame(&[msg]));
    server.send_outgoing(id, ms(100), false);

    assert_eq!(
        warnings(&mut server),
        ["client 1 publish of reserved topic '$clients' ignored"]
    );
    assert!(
        drain(&mut server, id)
            .iter()
            .all(|t| !matches!(t, Transmit::Text(_)))
    );
}
