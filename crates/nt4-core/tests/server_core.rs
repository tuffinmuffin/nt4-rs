//! Behavior tests for `ServerCore` beyond ntcore's ServerImplTest.
//!
//! Written by Claude (not ported from ntcore). Expected behavior follows
//! ntcore's ServerImpl, ServerStorage, ServerClient4 and NetworkPing, and the
//! NT 4.1 spec's meta topic section.

use nt4_core::{ClientId, ProtocolVersion, ServerCore, ServerEvent, Transmit};
use nt4_proto::{
    BinaryMessage, BinaryReader, ClientMessage, NtType, Properties, ServerMessage,
    SubscriptionOptions, Value, decode_server_text, encode_binary, encode_text_frame,
};
use serde_json::json;

fn add(server: &mut ServerCore, name: &str) -> ClientId {
    server
        .add_client(
            name,
            &format!("10.0.0.2:{}", 1000 + name.len()),
            false,
            ProtocolVersion::V4_1,
        )
        .0
}

fn send(server: &mut ServerCore, id: ClientId, msgs: &[ClientMessage]) {
    server.handle_text(id, 0, &encode_text_frame(msgs));
}

fn subscribe(topics: &[&str], options: SubscriptionOptions) -> ClientMessage {
    ClientMessage::Subscribe {
        subuid: 1,
        topics: topics.iter().map(|t| t.to_string()).collect(),
        options,
    }
}

fn publish(name: &str, pubuid: i32, properties: Properties) -> ClientMessage {
    ClientMessage::Publish {
        name: name.into(),
        pubuid,
        ty: NtType::Double,
        properties,
    }
}

fn props(v: serde_json::Value) -> Properties {
    v.as_object().unwrap().clone()
}

#[derive(Debug, PartialEq)]
enum Out {
    Text(Vec<ServerMessage>),
    Binary(Vec<BinaryMessage>),
    Ping,
    Close(String),
}

/// Flush a client's queue and decode everything it was sent.
fn flush(server: &mut ServerCore, id: ClientId, now_ms: i64) -> Vec<Out> {
    server.send_outgoing(id, now_ms * 1000, true);
    std::iter::from_fn(|| server.poll_transmit(id))
        .map(|t| match t {
            Transmit::Text(s) => {
                let d = decode_server_text(&s);
                assert!(d.warnings.is_empty(), "{:?}", d.warnings);
                Out::Text(d.messages)
            }
            Transmit::Binary(b) => {
                Out::Binary(BinaryReader::new(&b).collect::<Result<_, _>>().unwrap())
            }
            Transmit::Ping => Out::Ping,
            Transmit::Close { reason } => Out::Close(reason),
        })
        .collect()
}

fn messages(out: &[Out]) -> Vec<&ServerMessage> {
    out.iter()
        .flat_map(|o| match o {
            Out::Text(m) => m.iter().collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn values(out: &[Out]) -> Vec<&BinaryMessage> {
    out.iter()
        .flat_map(|o| match o {
            Out::Binary(m) => m.iter().collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn events(server: &mut ServerCore) -> Vec<ServerEvent> {
    std::iter::from_fn(|| server.poll_event()).collect()
}

/// mpack's encoding of a short string.
fn s(v: &str) -> Vec<u8> {
    let mut out = vec![0xa0 | v.len() as u8];
    out.extend_from_slice(v.as_bytes());
    out
}

#[test]
fn rtt_ping_is_answered_with_server_time() {
    let mut server = ServerCore::new();
    let id = add(&mut server, "c");
    let mut ping = Vec::new();
    encode_binary(
        &mut ping,
        &BinaryMessage {
            id: -1,
            timestamp_us: 0,
            value: Value::Int(1234),
        },
    );
    server.handle_binary(id, 99_000, &ping);
    let Some(Transmit::Binary(reply)) = server.poll_transmit(id) else {
        panic!("no reply")
    };
    let reply = BinaryReader::new(&reply).next().unwrap().unwrap();
    assert_eq!(
        reply,
        BinaryMessage {
            id: -1,
            timestamp_us: 99_000,
            value: Value::Int(1234)
        }
    );
}

#[test]
fn bad_binary_frame_closes_the_connection() {
    let mut server = ServerCore::new();
    let id = add(&mut server, "c");
    server.handle_binary(id, 0, &[0x94, 0x01]);
    let Some(Transmit::Close { reason }) = server.poll_transmit(id) else {
        panic!("no close")
    };
    assert!(reason.starts_with("binary decode error: "), "{reason}");
    // Nothing more is sent once closing.
    server.send_outgoing(id, 1_000_000, true);
    assert_eq!(server.poll_transmit(id), None);
}

#[test]
fn ping_timeout_closes_and_pongs_keep_it_open() {
    let mut server = ServerCore::new();
    let id = add(&mut server, "c");
    assert_eq!(flush(&mut server, id, 100), [Out::Ping]);
    // Pongs keep it alive.
    server.handle_pong(id, 400_000);
    assert_eq!(flush(&mut server, id, 400), [Out::Ping]);
    assert_eq!(flush(&mut server, id, 1_400), [Out::Ping]);
    // Nothing since 400 ms: more than a second later it times out.
    assert_eq!(flush(&mut server, id, 1_401), []);
    assert_eq!(
        flush(&mut server, id, 1_600),
        [Out::Close("connection timed out".into())]
    );
}

#[test]
fn nt40_clients_get_no_websocket_pings() {
    let mut server = ServerCore::new();
    let (id, _) = server.add_client("old", "10.0.0.3:1", false, ProtocolVersion::V4_0);
    assert_eq!(flush(&mut server, id, 100), []);
    assert_eq!(flush(&mut server, id, 5_000), []);
}

#[test]
fn values_reach_subscribers_but_not_the_publisher() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    let b = add(&mut server, "b");
    send(
        &mut server,
        a,
        &[
            publish("/x", 7, Properties::new()),
            subscribe(&["/x"], Default::default()),
        ],
    );
    send(&mut server, b, &[subscribe(&["/x"], Default::default())]);
    flush(&mut server, a, 100);
    flush(&mut server, b, 100);

    let mut frame = Vec::new();
    encode_binary(
        &mut frame,
        &BinaryMessage {
            id: 7,
            timestamp_us: 5_000,
            value: Value::Double(3.0),
        },
    );
    server.handle_binary(a, 0, &frame);

    assert!(values(&flush(&mut server, a, 200)).is_empty());
    let got = flush(&mut server, b, 200);
    let v = values(&got);
    assert_eq!(v.len(), 1);
    assert_eq!(
        (v[0].timestamp_us, &v[0].value),
        (5_000, &Value::Double(3.0))
    );
}

#[test]
fn older_value_from_another_client_is_forwarded_but_not_retained() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    let b = add(&mut server, "b");
    send(&mut server, a, &[publish("/x", 1, Properties::new())]);
    send(&mut server, b, &[publish("/x", 1, Properties::new())]);
    for (id, t, v) in [(a, 2_000, 1.0), (b, 1_000, 2.0)] {
        let mut frame = Vec::new();
        encode_binary(
            &mut frame,
            &BinaryMessage {
                id: 1,
                timestamp_us: t,
                value: Value::Double(v),
            },
        );
        server.handle_binary(id, 0, &frame);
    }
    // A late subscriber gets the newest one, not the last one to arrive.
    let c = add(&mut server, "c");
    send(&mut server, c, &[subscribe(&["/x"], Default::default())]);
    let got = flush(&mut server, c, 100);
    assert_eq!(values(&got)[0].value, Value::Double(1.0));
}

#[test]
fn unpublish_deletes_the_topic_and_unannounces_it() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    let b = add(&mut server, "b");
    send(&mut server, b, &[subscribe(&["/x"], Default::default())]);
    send(&mut server, a, &[publish("/x", 1, Properties::new())]);
    let first = flush(&mut server, b, 100);
    let ServerMessage::Announce { id, .. } = messages(&first)[0] else {
        panic!()
    };
    let id = *id;
    send(&mut server, a, &[ClientMessage::Unpublish { pubuid: 1 }]);
    assert_eq!(
        messages(&flush(&mut server, b, 200)),
        [&ServerMessage::Unannounce {
            name: "/x".into(),
            id
        }]
    );
}

#[test]
fn retained_topic_survives_its_publisher() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    send(
        &mut server,
        a,
        &[publish("/kept", 1, props(json!({"retained": true})))],
    );
    let mut frame = Vec::new();
    encode_binary(
        &mut frame,
        &BinaryMessage {
            id: 1,
            timestamp_us: 10,
            value: Value::Double(4.0),
        },
    );
    server.handle_binary(a, 0, &frame);
    server.remove_client(a);

    let b = add(&mut server, "b");
    send(&mut server, b, &[subscribe(&["/kept"], Default::default())]);
    let got = flush(&mut server, b, 100);
    assert!(matches!(messages(&got)[0], ServerMessage::Announce { name, .. } if name == "/kept"));
    assert_eq!(values(&got)[0].value, Value::Double(4.0));

    // Clearing `retained` with nobody publishing deletes it.
    send(
        &mut server,
        b,
        &[ClientMessage::SetProperties {
            name: "/kept".into(),
            update: props(json!({"retained": null})),
        }],
    );
    assert!(matches!(
        messages(&flush(&mut server, b, 200))[..],
        [ServerMessage::Unannounce { .. }]
    ));
}

#[test]
fn set_properties_acks_the_sender_only() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    let b = add(&mut server, "b");
    send(&mut server, a, &[publish("/x", 1, Properties::new())]);
    send(&mut server, b, &[subscribe(&["/x"], Default::default())]);
    flush(&mut server, a, 100);
    flush(&mut server, b, 100);

    let update = props(json!({"units": "m"}));
    send(
        &mut server,
        a,
        &[ClientMessage::SetProperties {
            name: "/x".into(),
            update: update.clone(),
        }],
    );
    let expect = |ack| ServerMessage::Properties {
        name: "/x".into(),
        update: update.clone(),
        ack,
    };
    assert_eq!(messages(&flush(&mut server, a, 200)), [&expect(true)]);
    assert_eq!(messages(&flush(&mut server, b, 200)), [&expect(false)]);
}

#[test]
fn set_properties_on_unknown_or_meta_topic_warns() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    send(
        &mut server,
        a,
        &[
            ClientMessage::SetProperties {
                name: "/nope".into(),
                update: props(json!({"x": 1})),
            },
            ClientMessage::SetProperties {
                name: "$clients".into(),
                update: props(json!({"x": 1})),
            },
        ],
    );
    assert_eq!(
        events(&mut server),
        [
            ServerEvent::Warning(
                r#"server ignoring SetProperties({"x":1}) from client 1 on unpublished topic '/nope'; publish or set a value first"#.into()
            ),
            ServerEvent::Warning(r#"server ignoring SetProperties({"x":1}) from client 1 on meta topic '$clients'"#.into()),
        ]
    );
}

#[test]
fn conflicting_type_keeps_the_first() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    let b = add(&mut server, "b");
    send(&mut server, a, &[publish("/x", 1, Properties::new())]);
    send(
        &mut server,
        b,
        &[ClientMessage::Publish {
            name: "/x".into(),
            pubuid: 1,
            ty: NtType::Int,
            properties: Properties::new(),
        }],
    );
    assert_eq!(
        events(&mut server),
        [ServerEvent::Warning(
            "client b@2 publish '/x' conflicting type 'int' (currently 'double')".into()
        )]
    );
    assert!(matches!(
        messages(&flush(&mut server, b, 100))[..],
        [ServerMessage::Announce {
            ty: NtType::Double,
            pubuid: Some(1),
            ..
        }]
    ));
}

#[test]
fn unknown_pubuid_warns() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    let mut frame = Vec::new();
    encode_binary(
        &mut frame,
        &BinaryMessage {
            id: 9,
            timestamp_us: 1,
            value: Value::Double(0.0),
        },
    );
    server.handle_binary(a, 0, &frame);
    assert_eq!(
        events(&mut server),
        [ServerEvent::Warning(
            "unrecognized client 1 pubuid 9, ignoring set".into()
        )]
    );
}

#[test]
fn meta_topics_need_a_dollar_prefix() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    send(
        &mut server,
        a,
        &[subscribe(
            &[""],
            SubscriptionOptions {
                prefix: true,
                ..Default::default()
            },
        )],
    );
    assert!(messages(&flush(&mut server, a, 100)).is_empty());

    let b = add(&mut server, "b");
    send(
        &mut server,
        b,
        &[subscribe(
            &["$"],
            SubscriptionOptions {
                prefix: true,
                ..Default::default()
            },
        )],
    );
    let got = flush(&mut server, b, 100);
    let names: Vec<&str> = messages(&got)
        .iter()
        .map(|m| match m {
            ServerMessage::Announce { name, ty, .. } => {
                assert_eq!(*ty, NtType::MsgPack);
                name.as_str()
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        names,
        [
            "$serverpub",
            "$serversub",
            "$clients",
            "$clientpub$a@1",
            "$clientsub$a@1",
            "$clientpub$b@2",
            "$clientsub$b@2"
        ]
    );
}

#[test]
fn clients_meta_topic_lists_connections() {
    let mut server = ServerCore::new();
    let (a, _) = server.add_client("dash", "10.0.0.5:5800", false, ProtocolVersion::V4_1);
    server.add_client("old", "10.0.0.6:5801", false, ProtocolVersion::V4_0);
    send(
        &mut server,
        a,
        &[subscribe(&["$clients"], Default::default())],
    );
    let got = flush(&mut server, a, 100);

    let mut expect = vec![0x92];
    for (id, conn, ver) in [
        ("dash@1", "10.0.0.5:5800", 0x01u8),
        ("old@2", "10.0.0.6:5801", 0x00),
    ] {
        expect.push(0x83);
        expect.extend(s("id"));
        expect.extend(s(id));
        expect.extend(s("conn"));
        expect.extend(s(conn));
        expect.extend(s("ver"));
        expect.extend([0xcd, 0x04, ver]);
    }
    let v = values(&got);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].value, Value::Raw(expect));
    // Meta values carry no timestamp.
    assert_eq!(v[0].timestamp_us, 0);
}

#[test]
fn pub_and_sub_meta_topics() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    let watcher = add(&mut server, "w");
    send(
        &mut server,
        a,
        &[
            publish("/x", 3, Properties::new()),
            subscribe(
                &["/x"],
                SubscriptionOptions {
                    all: true,
                    periodic: Some(0.02),
                    ..Default::default()
                },
            ),
        ],
    );
    let all_meta = SubscriptionOptions {
        prefix: true,
        ..Default::default()
    };
    send(
        &mut server,
        watcher,
        &[ClientMessage::Subscribe {
            subuid: 1,
            topics: vec![
                "$pub$/x".into(),
                "$sub$/x".into(),
                "$clientpub$a@1".into(),
                "$clientsub$a@1".into(),
            ],
            options: all_meta,
        }],
    );
    let got = flush(&mut server, watcher, 100);
    let ids: std::collections::HashMap<i32, String> = messages(&got)
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Announce { name, id, .. } => Some((*id, name.clone())),
            _ => None,
        })
        .collect();
    let by_name: std::collections::HashMap<&str, &Value> = values(&got)
        .iter()
        .map(|v| (ids[&v.id].as_str(), &v.value))
        .collect();

    // {"all": true, "periodic": 0.02 as float32}
    let mut options = vec![0x82];
    options.extend(s("all"));
    options.push(0xc3);
    options.extend(s("periodic"));
    options.push(0xca);
    options.extend(0.02f32.to_be_bytes());

    let mut pub_meta = vec![0x91, 0x82];
    pub_meta.extend(s("client"));
    pub_meta.extend(s("a@1"));
    pub_meta.extend(s("pubuid"));
    pub_meta.push(3);
    assert_eq!(by_name["$pub$/x"], &Value::Raw(pub_meta));

    let mut sub_meta = vec![0x91, 0x83];
    sub_meta.extend(s("client"));
    sub_meta.extend(s("a@1"));
    sub_meta.extend(s("subuid"));
    sub_meta.push(1);
    sub_meta.extend(s("options"));
    sub_meta.extend(&options);
    assert_eq!(by_name["$sub$/x"], &Value::Raw(sub_meta));

    let mut client_pub = vec![0x91, 0x82];
    client_pub.extend(s("uid"));
    client_pub.push(3);
    client_pub.extend(s("topic"));
    client_pub.extend(s("/x"));
    assert_eq!(by_name["$clientpub$a@1"], &Value::Raw(client_pub));

    let mut client_sub = vec![0x91, 0x83];
    client_sub.extend(s("uid"));
    client_sub.push(1);
    client_sub.extend(s("topics"));
    client_sub.push(0x91);
    client_sub.extend(s("/x"));
    client_sub.extend(s("options"));
    client_sub.extend(&options);
    assert_eq!(by_name["$clientsub$a@1"], &Value::Raw(client_sub));
}

#[test]
fn client_meta_topics_go_with_the_client() {
    let mut server = ServerCore::new();
    let w = add(&mut server, "w");
    send(
        &mut server,
        w,
        &[subscribe(
            &["$client"],
            SubscriptionOptions {
                prefix: true,
                ..Default::default()
            },
        )],
    );
    let a = add(&mut server, "a");
    flush(&mut server, w, 100);
    server.remove_client(a);
    let unannounced: Vec<String> = messages(&flush(&mut server, w, 200))
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Unannounce { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(unannounced, ["$clientpub$a@2", "$clientsub$a@2"]);
    // The slot is free for the next client.
    assert_eq!(
        server.add_client("b", "x", false, ProtocolVersion::V4_1).1,
        "b@2"
    );
}

#[test]
fn topic_ids_are_reused_after_sixteen_are_free() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    let w = add(&mut server, "w");
    send(
        &mut server,
        w,
        &[subscribe(
            &["/"],
            SubscriptionOptions {
                prefix: true,
                ..Default::default()
            },
        )],
    );
    let mut now_ms = 0;
    let mut id_of = |server: &mut ServerCore, name: &str| -> i32 {
        // Sends are at least 5 ms apart.
        now_ms += 100;
        let got = flush(server, w, now_ms);
        messages(&got)
            .iter()
            .find_map(|m| match m {
                ServerMessage::Announce { name: n, id, .. } if n == name => Some(*id),
                _ => None,
            })
            .unwrap()
    };
    // Each topic frees three ids (itself and its two meta topics).
    let mut freed = Vec::new();
    for i in 0..6 {
        let name = format!("/t{i}");
        send(&mut server, a, &[publish(&name, i, Properties::new())]);
        freed.push(id_of(&mut server, &name));
        send(&mut server, a, &[ClientMessage::Unpublish { pubuid: i }]);
    }
    // 18 free ids now. They were freed meta topics first ($pub$, $sub$),
    // then the topic, and the oldest comes back first.
    send(&mut server, a, &[publish("/again", 100, Properties::new())]);
    assert_eq!(id_of(&mut server, "/again"), freed[0] + 1, "{freed:?}");
    assert_eq!(server.tick_period_ms(w), 100);
}

#[test]
fn reused_topic_id_is_announced_again_to_a_former_publisher() {
    // A client that only published a topic, then unpublished it, must hear
    // about a later topic that gets the same id.
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    for i in 0..6 {
        send(
            &mut server,
            a,
            &[
                publish(&format!("/t{i}"), i, Properties::new()),
                ClientMessage::Unpublish { pubuid: i },
            ],
        );
    }
    send(
        &mut server,
        a,
        &[subscribe(
            &["/"],
            SubscriptionOptions {
                prefix: true,
                ..Default::default()
            },
        )],
    );
    flush(&mut server, a, 100);
    let b = add(&mut server, "b");
    send(&mut server, b, &[publish("/new", 1, Properties::new())]);
    let got = flush(&mut server, a, 200);
    assert!(
        messages(&got)
            .iter()
            .any(|m| matches!(m, ServerMessage::Announce { name, .. } if name == "/new")),
        "{got:?}"
    );
}

#[test]
fn local_client_gets_messages_without_waiting() {
    let mut server = ServerCore::new();
    let (id, _) = server.add_client("sim", "127.0.0.1:9", true, ProtocolVersion::V4_1);
    send(&mut server, id, &[subscribe(&["/x"], Default::default())]);
    let p = server.publish("/x", NtType::Double, Properties::new());
    server.set_value(p, Value::Double(1.0), 7).unwrap();
    // No send_outgoing needed: each message is its own frame, right away.
    let frames: Vec<Transmit> = std::iter::from_fn(|| server.poll_transmit(id)).collect();
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert!(matches!(&frames[0], Transmit::Text(t) if t.contains(r#""method":"announce""#)));
    assert!(matches!(&frames[1], Transmit::Binary(_)));
}

#[test]
fn server_subscription_sees_client_values_and_properties() {
    let mut server = ServerCore::new();
    server.subscribe(vec!["/x".into()], SubscriptionOptions::default());
    let a = add(&mut server, "a");
    send(&mut server, a, &[publish("/x", 1, Properties::new())]);
    let mut frame = Vec::new();
    encode_binary(
        &mut frame,
        &BinaryMessage {
            id: 1,
            timestamp_us: 42,
            value: Value::Double(9.0),
        },
    );
    server.handle_binary(a, 0, &frame);
    send(
        &mut server,
        a,
        &[ClientMessage::SetProperties {
            name: "/x".into(),
            update: props(json!({"k": 1})),
        }],
    );
    let ev = events(&mut server);
    assert!(matches!(&ev[0], ServerEvent::Announced { name, .. } if &**name == "/x"));
    assert!(
        matches!(&ev[1], ServerEvent::Value { value, .. } if value.server_time_us == 42 && value.value == Value::Double(9.0))
    );
    assert!(matches!(&ev[2], ServerEvent::Properties { name, ack: false, .. } if name == "/x"));
}

#[test]
fn tick_period_follows_subscriptions() {
    let mut server = ServerCore::new();
    let a = add(&mut server, "a");
    assert_eq!(server.tick_period_ms(a), 100);
    let every = |p| SubscriptionOptions {
        periodic: Some(p),
        ..Default::default()
    };
    send(&mut server, a, &[subscribe(&["/x"], every(0.02))]);
    assert_eq!(server.tick_period_ms(a), 20);
    send(&mut server, a, &[ClientMessage::Unsubscribe { subuid: 1 }]);
    assert_eq!(server.tick_period_ms(a), 100);
}
