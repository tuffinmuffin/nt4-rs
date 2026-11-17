//! Behavior tests for `ClientCore`, driven with a fake clock and no network.
//!
//! Written by Claude (not ported from ntcore). Expected behavior follows
//! ntcore's ClientImpl, NetworkPing and LocalStorage StartNetwork.

use nt4_core::{
    ClientCore, ClientError, Event, ProtocolVersion, PubId, PublishOptions, TopicValue, Transmit,
};
use nt4_proto::{
    BinaryMessage, BinaryReader, ClientMessage, NtType, Properties, ServerMessage,
    SubscriptionOptions, Value, decode_client_text, encode_binary, encode_text_frame,
};

/// Server time is this far ahead of local time once synced.
const OFFSET: i64 = 1_000_000;

fn rtt_reply(sent_us: i64, server_us: i64) -> Vec<u8> {
    let mut out = Vec::new();
    encode_binary(
        &mut out,
        &BinaryMessage {
            id: -1,
            timestamp_us: server_us,
            value: Value::Int(sent_us),
        },
    );
    out
}

/// Connect at local time `now_us` and answer the RTT ping with no delay, so
/// the offset is exactly `OFFSET`. Drops the ping frame and the events.
fn connect_synced(client: &mut ClientCore, now_us: i64, version: ProtocolVersion) {
    client.connect(now_us, version);
    assert!(matches!(client.poll_transmit(), Some(Transmit::Binary(_))));
    client.handle_binary(now_us, &rtt_reply(now_us, now_us + OFFSET));
    assert_eq!(client.server_time_offset(), Some(OFFSET));
    drain_events(client);
}

fn drain_events(client: &mut ClientCore) -> Vec<Event> {
    std::iter::from_fn(|| client.poll_event()).collect()
}

/// A decoded outgoing frame, for easy comparison.
#[derive(Debug, PartialEq)]
enum Out {
    Text(Vec<ClientMessage>),
    Binary(Vec<BinaryMessage>),
    Ping,
    Close(String),
}

fn drain_transmit(client: &mut ClientCore) -> Vec<Out> {
    std::iter::from_fn(|| client.poll_transmit())
        .map(|t| match t {
            Transmit::Text(s) => {
                let d = decode_client_text(&s);
                assert!(d.warnings.is_empty(), "{:?}", d.warnings);
                Out::Text(d.messages)
            }
            Transmit::Binary(b) => Out::Binary(
                BinaryReader::new(&b)
                    .collect::<Result<_, _>>()
                    .expect("valid binary frame"),
            ),
            Transmit::Ping => Out::Ping,
            Transmit::Close { reason } => Out::Close(reason),
        })
        .collect()
}

fn publish_msg(name: &str, pubuid: i32) -> ClientMessage {
    ClientMessage::Publish {
        name: name.into(),
        pubuid,
        ty: NtType::Double,
        properties: Properties::new(),
    }
}

fn value_msg(id: i32, timestamp_us: i64, d: f64) -> BinaryMessage {
    BinaryMessage {
        id,
        timestamp_us,
        value: Value::Double(d),
    }
}

fn server_text(msgs: &[ServerMessage]) -> String {
    encode_text_frame(msgs)
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

fn double_pub(client: &mut ClientCore, name: &str) -> PubId {
    client.publish(
        name,
        NtType::Double,
        Properties::new(),
        PublishOptions::default(),
    )
}

#[test]
fn nothing_but_pings_before_time_sync() {
    let mut client = ClientCore::new();
    let p = double_pub(&mut client, "/a");
    client.set_value(p, Value::Double(1.0), 5).unwrap();
    client.connect(0, ProtocolVersion::V4_1);
    assert!(matches!(client.poll_transmit(), Some(Transmit::Binary(_))));

    client.send_outgoing(100_000, true);
    assert_eq!(drain_transmit(&mut client), vec![Out::Ping]);
}

#[test]
fn publish_value_and_subscribe_go_out_after_sync() {
    let mut client = ClientCore::new();
    let p = double_pub(&mut client, "/robot/x");
    client.set_value(p, Value::Double(1.5), 2_000).unwrap();
    let sub_opts = SubscriptionOptions {
        prefix: true,
        ..Default::default()
    };
    let s = client.subscribe(vec!["/robot/".into()], sub_opts.clone());

    // Everything goes out as soon as the time sync reply arrives, without
    // waiting for the next send_outgoing (ntcore waits for its timer).
    connect_synced(&mut client, 10_000, ProtocolVersion::V4_1);

    assert_eq!(
        drain_transmit(&mut client),
        vec![
            Out::Text(vec![publish_msg("/robot/x", p.uid())]),
            // Local time 2000 us plus the offset.
            Out::Binary(vec![value_msg(p.uid(), 2_000 + OFFSET, 1.5)]),
            Out::Text(vec![ClientMessage::Subscribe {
                subuid: s.uid(),
                topics: vec!["/robot/".into()],
                options: sub_opts,
            }]),
        ]
    );
}

#[test]
fn values_coalesce_unless_send_all() {
    let mut client = ClientCore::new();
    let normal = double_pub(&mut client, "/normal");
    let all = client.publish(
        "/all",
        NtType::Double,
        Properties::new(),
        PublishOptions {
            send_all: true,
            ..Default::default()
        },
    );
    connect_synced(&mut client, 0, ProtocolVersion::V4_1);
    client.send_outgoing(10_000, true);
    drain_transmit(&mut client);

    for i in 1..=3 {
        client
            .set_value(normal, Value::Double(f64::from(i)), 20_000 + i64::from(i))
            .unwrap();
        client
            .set_value(all, Value::Double(f64::from(i)), 20_000 + i64::from(i))
            .unwrap();
    }
    client.send_outgoing(200_000, true);
    let out = drain_transmit(&mut client);
    let values: Vec<_> = out
        .into_iter()
        .filter_map(|o| match o {
            Out::Binary(v) => Some(v),
            _ => None,
        })
        .flatten()
        .collect();
    let normal_vals: Vec<_> = values.iter().filter(|m| m.id == normal.uid()).collect();
    let all_vals: Vec<_> = values.iter().filter(|m| m.id == all.uid()).collect();
    assert_eq!(normal_vals.len(), 1);
    assert_eq!(normal_vals[0].value, Value::Double(3.0));
    assert_eq!(all_vals.len(), 3);
}

#[test]
fn announce_value_properties_unannounce() {
    let mut client = ClientCore::new();
    let p = double_pub(&mut client, "/mine");
    connect_synced(&mut client, 0, ProtocolVersion::V4_1);

    client.handle_text(
        1_000,
        &server_text(&[
            announce("/mine", 7, Some(p.uid())),
            announce("/other", 8, None),
        ]),
    );
    let events = drain_events(&mut client);
    assert!(matches!(
        &events[0],
        Event::Announced { name, id: 7, publisher: Some(pp), .. } if &**name == "/mine" && *pp == p
    ));
    assert!(matches!(
        &events[1],
        Event::Announced { name, id: 8, publisher: None, .. } if &**name == "/other"
    ));
    assert_eq!(client.topic_by_name("/other").map(|t| t.id), Some(8));

    // A value in server time arrives; the event gives local time too.
    let mut frame = Vec::new();
    encode_binary(&mut frame, &value_msg(8, OFFSET + 3_000, 2.5));
    encode_binary(&mut frame, &value_msg(99, OFFSET + 3_000, 0.0));
    client.handle_binary(4_000, &frame);
    let expected = TopicValue {
        value: Value::Double(2.5),
        server_time_us: OFFSET + 3_000,
        local_time_us: 3_000,
    };
    assert_eq!(
        drain_events(&mut client),
        vec![
            Event::Value {
                name: "/other".into(),
                id: 8,
                value: expected.clone(),
            },
            Event::Warning("received unknown id 99".into()),
        ]
    );
    assert_eq!(client.topic(8).unwrap().value, Some(expected));

    let mut update = Properties::new();
    update.insert("persistent".into(), true.into());
    client.handle_text(
        5_000,
        &server_text(&[ServerMessage::Properties {
            name: "/other".into(),
            update: update.clone(),
            ack: false,
        }]),
    );
    assert_eq!(client.topic(8).unwrap().properties, update);
    drain_events(&mut client);

    client.handle_text(
        6_000,
        &server_text(&[ServerMessage::Unannounce {
            name: "/other".into(),
            id: 8,
        }]),
    );
    assert_eq!(
        drain_events(&mut client),
        vec![Event::Unannounced {
            name: "/other".into(),
            id: 8
        }]
    );
    assert!(client.topic(8).is_none());
}

#[test]
fn bad_text_frame_warns() {
    let mut client = ClientCore::new();
    connect_synced(&mut client, 0, ProtocolVersion::V4_1);
    client.handle_text(0, r#"[{"method":"announce","params":{}}]"#);
    assert_eq!(
        drain_events(&mut client),
        vec![Event::Warning("0: no name key".into())]
    );
}

#[test]
fn disconnect_unannounces_everything() {
    let mut client = ClientCore::new();
    connect_synced(&mut client, 0, ProtocolVersion::V4_1);
    client.handle_text(
        0,
        &server_text(&[announce("/b", 2, None), announce("/a", 1, None)]),
    );
    drain_events(&mut client);

    client.disconnect("socket closed");
    assert_eq!(
        drain_events(&mut client),
        vec![
            Event::Unannounced {
                name: "/a".into(),
                id: 1
            },
            Event::Unannounced {
                name: "/b".into(),
                id: 2
            },
            Event::TimeSync(nt4_core::TimeSyncUpdate::INVALID),
            Event::Disconnected {
                reason: "socket closed".into()
            },
        ]
    );
    assert!(!client.is_connected());
    assert_eq!(client.topics().count(), 0);
}

#[test]
fn reconnect_replays_publishers_last_values_and_subscriptions() {
    let mut client = ClientCore::new();
    let a1 = double_pub(&mut client, "/a");
    let gone = double_pub(&mut client, "/gone");
    let b = double_pub(&mut client, "/b");
    let a2 = double_pub(&mut client, "/a");
    let s = client.subscribe(vec!["/x".into()], SubscriptionOptions::default());

    connect_synced(&mut client, 0, ProtocolVersion::V4_1);
    client.set_value(a1, Value::Double(1.0), 100).unwrap();
    client.set_value(gone, Value::Double(9.0), 100).unwrap();
    client.send_outgoing(10_000, true);
    drain_transmit(&mut client);

    client.disconnect("server restarted");
    // Changes while disconnected are kept for the next connection.
    client.unpublish(gone);
    client.set_value(a2, Value::Double(2.0), 200).unwrap();

    connect_synced(&mut client, 1_000_000, ProtocolVersion::V4_1);
    assert_eq!(
        drain_transmit(&mut client),
        vec![
            Out::Text(vec![
                publish_msg("/a", a1.uid()),
                publish_msg("/a", a2.uid()),
            ]),
            // One value per topic, the newest, sent by the newest publisher.
            Out::Binary(vec![value_msg(a2.uid(), 200 + OFFSET, 2.0)]),
            Out::Text(vec![
                publish_msg("/b", b.uid()),
                ClientMessage::Subscribe {
                    subuid: s.uid(),
                    topics: vec!["/x".into()],
                    options: SubscriptionOptions::default(),
                },
            ]),
        ]
    );
}

#[test]
fn incoming_value_on_our_topic_is_replayed() {
    let mut client = ClientCore::new();
    let p = double_pub(&mut client, "/shared");
    connect_synced(&mut client, 0, ProtocolVersion::V4_1);
    client.handle_text(0, &server_text(&[announce("/shared", 3, Some(p.uid()))]));
    let mut frame = Vec::new();
    encode_binary(&mut frame, &value_msg(3, OFFSET + 500, 7.0));
    client.handle_binary(600, &frame);

    client.disconnect("bye");
    connect_synced(&mut client, 10_000, ProtocolVersion::V4_1);
    client.send_outgoing(20_000, true);
    let out = drain_transmit(&mut client);
    assert!(out.contains(&Out::Binary(vec![value_msg(p.uid(), OFFSET + 500, 7.0)])));
}

#[test]
fn unpublish_and_unsubscribe_while_connected() {
    let mut client = ClientCore::new();
    let p = double_pub(&mut client, "/a");
    let s = client.subscribe(vec!["/a".into()], SubscriptionOptions::default());
    connect_synced(&mut client, 0, ProtocolVersion::V4_1);
    client.send_outgoing(10_000, true);
    drain_transmit(&mut client);

    client.unpublish(p);
    client.unsubscribe(s);
    assert_eq!(
        client.set_value(p, Value::Double(1.0), 0),
        Err(ClientError::UnknownPublisher(p.uid()))
    );
    client.send_outgoing(20_000, true);
    assert_eq!(
        drain_transmit(&mut client),
        vec![Out::Text(vec![
            ClientMessage::Unpublish { pubuid: p.uid() },
            ClientMessage::Unsubscribe { subuid: s.uid() },
        ])]
    );
}

#[test]
fn set_value_checks_type() {
    let mut client = ClientCore::new();
    let p = double_pub(&mut client, "/a");
    assert_eq!(
        client.set_value(p, Value::String("no".into()), 0),
        Err(ClientError::TypeMismatch {
            expected: NtType::Double,
            got: 4
        })
    );
}

#[test]
fn set_properties_is_sent_and_kept_for_replay() {
    let mut client = ClientCore::new();
    let p = double_pub(&mut client, "/a");
    connect_synced(&mut client, 0, ProtocolVersion::V4_1);
    client.send_outgoing(10_000, true);
    drain_transmit(&mut client);

    let mut update = Properties::new();
    update.insert("retained".into(), true.into());
    client.set_properties("/a", update.clone());
    client.send_outgoing(20_000, true);
    assert_eq!(
        drain_transmit(&mut client),
        vec![Out::Text(vec![ClientMessage::SetProperties {
            name: "/a".into(),
            update: update.clone(),
        }])]
    );

    client.disconnect("bye");
    connect_synced(&mut client, 100_000, ProtocolVersion::V4_1);
    client.send_outgoing(110_000, true);
    assert!(
        drain_transmit(&mut client).contains(&Out::Text(vec![ClientMessage::Publish {
            name: "/a".into(),
            pubuid: p.uid(),
            ty: NtType::Double,
            properties: update,
        }]))
    );
}

#[test]
fn v4_1_pings_and_times_out() {
    let mut client = ClientCore::new();
    connect_synced(&mut client, 0, ProtocolVersion::V4_1);

    // First tick pings right away, then every 200 ms.
    client.send_outgoing(0, false);
    assert_eq!(drain_transmit(&mut client), vec![Out::Ping]);
    client.send_outgoing(100_000, false);
    assert_eq!(drain_transmit(&mut client), vec![]);
    client.send_outgoing(200_000, false);
    assert_eq!(drain_transmit(&mut client), vec![Out::Ping]);

    // A pong at 300 ms keeps it alive until 1300 ms.
    client.handle_pong(300_000);
    client.send_outgoing(1_200_000, false);
    assert_eq!(drain_transmit(&mut client), vec![Out::Ping]);
    client.send_outgoing(1_400_000, false);
    assert_eq!(
        drain_transmit(&mut client),
        vec![Out::Close("connection timed out".into())]
    );
    assert!(!client.is_connected());
    assert!(drain_events(&mut client).contains(&Event::Disconnected {
        reason: "connection timed out".into()
    }));
}

#[test]
fn v4_0_uses_rtt_pings_for_keepalive() {
    let mut client = ClientCore::new();
    connect_synced(&mut client, 0, ProtocolVersion::V4_0);

    // No WebSocket pings on 4.0; an RTT ping every 3 s.
    client.send_outgoing(1_000_000, false);
    assert_eq!(drain_transmit(&mut client), vec![]);
    client.send_outgoing(3_000_000, false);
    assert_eq!(
        drain_transmit(&mut client),
        vec![Out::Binary(vec![BinaryMessage {
            id: -1,
            timestamp_us: 0,
            value: Value::Int(3_000_000),
        }])]
    );

    // Answered: stays connected at the next interval.
    client.handle_binary(3_010_000, &rtt_reply(3_000_000, 4_000_000));
    client.send_outgoing(6_000_000, false);
    assert!(client.is_connected());
    drain_transmit(&mut client);

    // Not answered: closed at the one after.
    client.send_outgoing(9_000_000, false);
    assert_eq!(
        drain_transmit(&mut client),
        vec![Out::Close("connection timed out".into())]
    );
}

/// What was queued before the first RTT reply goes out with that reply, even
/// when the reply comes within a few milliseconds of the clock's zero (a
/// local server, a client that just started). Written by Claude.
#[test]
fn queued_messages_go_out_at_first_sync() {
    let mut client = ClientCore::new();
    double_pub(&mut client, "/a");
    client.connect(0, ProtocolVersion::V4_1);
    assert!(matches!(client.poll_transmit(), Some(Transmit::Binary(_))));
    assert_eq!(drain_transmit(&mut client), vec![]);
    client.handle_binary(1_000, &rtt_reply(0, OFFSET));
    assert_eq!(
        drain_transmit(&mut client),
        vec![Out::Text(vec![publish_msg("/a", 1)])]
    );
}

#[test]
fn blocked_wire_holds_messages() {
    let mut client = ClientCore::new();
    connect_synced(&mut client, 0, ProtocolVersion::V4_1);
    client.set_writable(false);
    double_pub(&mut client, "/a");
    client.send_outgoing(10_000, true);
    assert_eq!(drain_transmit(&mut client), vec![Out::Ping]);
    client.set_writable(true);
    client.send_outgoing(20_000, true);
    assert_eq!(
        drain_transmit(&mut client),
        vec![Out::Text(vec![publish_msg("/a", 1)])]
    );
}

#[test]
fn tick_period_follows_publish_periods() {
    let mut client = ClientCore::new();
    // Capped at 100 ms, the period of publish/subscribe messages.
    assert_eq!(client.tick_period_ms(), 100);
    let opts = |s: f64| PublishOptions {
        periodic: Some(s),
        ..Default::default()
    };
    let a = client.publish("/a", NtType::Double, Properties::new(), opts(0.1));
    assert_eq!(client.tick_period_ms(), 100);
    let b = client.publish("/b", NtType::Double, Properties::new(), opts(0.04));
    assert_eq!(client.tick_period_ms(), 20);
    client.unpublish(b);
    assert_eq!(client.tick_period_ms(), 100);
    client.unpublish(a);
    assert_eq!(client.tick_period_ms(), 100);
    client.publish("/c", NtType::Double, Properties::new(), opts(0.001));
    assert_eq!(client.tick_period_ms(), 5);
}

// ntcore queues `setproperties` under id 0 while the `publish` it follows
// waits in the publisher's own queue; with a non-default period both come due
// in one send and the `setproperties` can go first, so a server ignores it.
// nt4 keeps it behind the publish (see `ClientCore::set_properties`).
#[test]
fn set_properties_right_after_publish_is_sent_after_it() {
    let mut client = ClientCore::new();
    connect_synced(&mut client, 1_000, ProtocolVersion::V4_1);
    drain_transmit(&mut client);
    let options = PublishOptions {
        periodic: Some(10.0),
        ..Default::default()
    };
    client.publish("/slow", NtType::Int, Properties::new(), options);
    let mut update = Properties::new();
    update.insert("persistent".into(), true.into());
    client.set_properties("/slow", update);
    client.send_outgoing(2_000, false);

    let methods: Vec<&str> = drain_transmit(&mut client)
        .iter()
        .flat_map(|o| match o {
            Out::Text(msgs) => msgs.iter().collect(),
            _ => Vec::new(),
        })
        .map(|m| match m {
            ClientMessage::Publish { .. } => "publish",
            ClientMessage::SetProperties { .. } => "setproperties",
            _ => "other",
        })
        .collect();
    assert_eq!(methods, ["publish", "setproperties"]);
}
