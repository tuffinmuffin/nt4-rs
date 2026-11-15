//! Struct and protobuf topics: ntcore's StructTest cases, and the schemas
//! that publishing them registers.

// Ported from allwpilib ntcore/src/test/native/cpp/StructTest.cpp
// at commit 4ccfe967ba1903b1f409b04b99db0922dd3b7318.
// Copyright (c) FIRST and other WPILib contributors.
// The test cases come from that file and stay under the WPILib BSD license;
// see tests/LICENSE-WPILIB.md. The Rust test code was written by Claude.
//
// Upstream publishes and subscribes on one ntcore instance. nt4's server
// doesn't echo its own values to its own subscribers, so here the server
// publishes and one of our clients subscribes. Upstream also reads values
// back through GetAtomic, GetInto and ReadQueue; nt4's Subscriber has
// get / get_timestamped only, so those checks collapse into one.

#![cfg(all(feature = "server", feature = "client"))]

use std::time::Duration;

use nt4::proto::structs::{DynamicStruct, StructDatabase};
use nt4::structs::{NtProtobuf, NtStruct, Proto, StructField, StructReader};
use nt4::wpimath::{Pose2d, Rotation2d, Translation2d};
use nt4::{
    Client, ClientConfig, Event, NtType, NtValue, Properties, PublishOptions, Server, ServerConfig,
    SubscriptionOptions, Value,
};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Inner {
    a: i32,
    b: i32,
}

impl NtStruct for Inner {
    const TYPE_NAME: &'static str = "Inner";
    const SIZE: usize = 8;
    const SCHEMA: &'static str = "int32 a; int32 b";
    fn pack(&self, out: &mut Vec<u8>) {
        self.a.pack(out);
        self.b.pack(out);
    }
    fn unpack(data: &[u8]) -> Self {
        let mut r = StructReader::new(data);
        Inner {
            a: r.read(),
            b: r.read(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Outer {
    inner: Inner,
    c: i32,
}

impl NtStruct for Outer {
    const TYPE_NAME: &'static str = "Outer";
    const SIZE: usize = <Inner as NtStruct>::SIZE + 4;
    const SCHEMA: &'static str = "Inner inner; int32 c";
    fn pack(&self, out: &mut Vec<u8>) {
        StructField::pack(&self.inner, out);
        self.c.pack(out);
    }
    fn unpack(data: &[u8]) -> Self {
        let mut r = StructReader::new(data);
        Outer {
            inner: r.read(),
            c: r.read(),
        }
    }
    fn for_each_nested(f: &mut dyn FnMut(&str, &str)) {
        Inner::for_each_struct_schema(f);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct ThingA {
    x: u8,
}

impl NtStruct for ThingA {
    const TYPE_NAME: &'static str = "ThingA";
    const SIZE: usize = 1;
    const SCHEMA: &'static str = "uint8 value";
    fn pack(&self, out: &mut Vec<u8>) {
        out.push(self.x);
    }
    fn unpack(data: &[u8]) -> Self {
        ThingA { x: data[0] }
    }
}

async fn pair() -> (Server, Client) {
    let server = Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut config = ClientConfig::server("structs", "127.0.0.1", server.local_addr().port());
    config.reconnect_delay = Duration::from_millis(50);
    let client = Client::start(config);
    assert!(client.wait_for_sync(Duration::from_secs(5)).await);
    (server, client)
}

/// Poll `f` every 10 ms until it returns `Some`, for up to 5 s.
async fn eventually<T>(mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..500 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out");
}

async fn wait_for<T: NtValue + PartialEq>(sub: &nt4::Subscriber<T>, want: T) {
    eventually(|| (sub.get().as_ref() == Some(&want)).then_some(())).await;
}

// upstream: "StructTest InnerConstexpr"
#[tokio::test(flavor = "multi_thread")]
async fn inner() {
    let (server, client) = pair().await;
    let publisher = server.publish::<Inner>("inner");
    let sub = client.subscribe::<Inner>("inner", SubscriptionOptions::default());
    assert_eq!(Inner::nt_type().to_string(), "struct:Inner");

    publisher.set_default(Inner { a: 0, b: 1 });
    wait_for(&sub, Inner { a: 0, b: 1 }).await;
    publisher.set(Inner { a: 1, b: 2 });
    wait_for(&sub, Inner { a: 1, b: 2 }).await;
    let topic = client.topic("inner").unwrap();
    assert_eq!(topic.ty.to_string(), "struct:Inner");
}

// upstream: "StructTest OuterConstexpr"
#[tokio::test(flavor = "multi_thread")]
async fn outer() {
    let (server, client) = pair().await;
    let publisher = server.publish::<Outer>("outer");
    let sub = client.subscribe::<Outer>("outer", SubscriptionOptions::default());
    assert_eq!(Outer::nt_type().to_string(), "struct:Outer");

    let first = Outer {
        inner: Inner { a: 0, b: 1 },
        c: 2,
    };
    publisher.set_default(first);
    wait_for(&sub, first).await;
    let second = Outer {
        inner: Inner { a: 1, b: 2 },
        c: 3,
    };
    publisher.set(second);
    wait_for(&sub, second).await;
}

// upstream: "StructTest InnerArrayConstexpr"
#[tokio::test(flavor = "multi_thread")]
async fn inner_array() {
    let (server, client) = pair().await;
    let publisher = server.publish::<Vec<Inner>>("innerA");
    let sub = client.subscribe::<Vec<Inner>>("innerA", SubscriptionOptions::default());
    assert_eq!(<Vec<Inner>>::nt_type().to_string(), "struct:Inner[]");

    publisher.set_default(vec![Inner { a: 0, b: 1 }]);
    wait_for(&sub, vec![Inner { a: 0, b: 1 }]).await;
    publisher.set(vec![Inner { a: 1, b: 2 }]);
    wait_for(&sub, vec![Inner { a: 1, b: 2 }]).await;
}

// upstream: "StructTest StructA"
#[tokio::test(flavor = "multi_thread")]
async fn struct_a() {
    let (server, client) = pair().await;
    let publisher = server.publish::<ThingA>("a");
    let _publisher2 = server.publish_generic("a", ThingA::nt_type(), Properties::new());
    let sub = client.subscribe::<ThingA>("a", SubscriptionOptions::default());
    publisher.set_default(ThingA::default());
    publisher.set_at(ThingA { x: 7 }, 5);
    wait_for(&sub, ThingA { x: 7 }).await;
    assert_eq!(sub.get_or(ThingA::default()), ThingA { x: 7 });
    let (value, stamped) = sub.get_timestamped().unwrap();
    assert_eq!((value, stamped.server_time_us), (ThingA { x: 7 }, 5));
}

// upstream: "StructTest StructArrayA"
#[tokio::test(flavor = "multi_thread")]
async fn struct_array_a() {
    let (server, client) = pair().await;
    let publisher = server.publish::<Vec<ThingA>>("a");
    let _publisher2 = server.publish_generic("a", <Vec<ThingA>>::nt_type(), Properties::new());
    let sub = client.subscribe::<Vec<ThingA>>("a", SubscriptionOptions::default());
    publisher.set_default(vec![ThingA::default(); 2]);
    publisher.set_at(vec![ThingA { x: 1 }, ThingA { x: 2 }], 5);
    wait_for(&sub, vec![ThingA { x: 1 }, ThingA { x: 2 }]).await;
}

/// The schema topic `/.schema/<name>` as the client sees it, once it has
/// a value.
async fn schema_topic(client: &Client, name: &str) -> nt4::Topic {
    let full = format!("/.schema/{name}");
    eventually(|| client.topic(&full).filter(|t| t.value.is_some())).await
}

// Written by Claude.
#[tokio::test(flavor = "multi_thread")]
async fn publishing_a_struct_publishes_its_schemas_like_ntcore() {
    let (server, client) = pair().await;
    let _schemas = client.subscribe_multiple(
        vec!["/.schema/".into()],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );
    let _outer = server.publish::<Outer>("/outer");
    for (name, schema) in [
        ("struct:Inner", "int32 a; int32 b"),
        ("struct:Outer", "Inner inner; int32 c"),
    ] {
        let topic = schema_topic(&client, name).await;
        assert_eq!(topic.ty, NtType::StructSchema);
        assert_eq!(topic.ty.to_string(), "structschema");
        assert_eq!(
            topic.value.unwrap().value,
            Value::Raw(schema.as_bytes().to_vec())
        );
        // As ntcore's AddSchema: retained, nothing else.
        let mut retained = Properties::new();
        retained.insert("retained".into(), true.into());
        assert_eq!(topic.properties, retained, "{name}");
    }
    // Once per server: a second topic of the same type adds nothing, and
    // the schema outlives the publishers.
    let _again = server.publish::<Vec<Outer>>("/outers");
    drop(_outer);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(server.has_schema("struct:Outer"));
    assert!(client.topic("/.schema/struct:Outer").is_some());
}

// Written by Claude.
#[tokio::test(flavor = "multi_thread")]
async fn client_struct_schemas_reach_the_server() {
    let (server, client) = pair().await;
    let schema =
        server.subscribe::<Vec<u8>>("/.schema/struct:Pose2d", SubscriptionOptions::default());
    let pose = server.subscribe::<Pose2d>("/vision/pose", SubscriptionOptions::default());
    let publisher = client.publish::<Pose2d>("/vision/pose", PublishOptions::default());
    let value = Pose2d {
        translation: Translation2d { x: 1.0, y: 2.0 },
        rotation: Rotation2d { value: 0.5 },
    };
    publisher.set(value);
    wait_for(&pose, value).await;
    wait_for(
        &schema,
        b"Translation2d translation;Rotation2d rotation".to_vec(),
    )
    .await;
    assert!(client.has_schema("struct:Translation2d"));
    assert!(client.has_schema("struct:Rotation2d"));
}

// Written by Claude: what a dashboard does with a struct it doesn't know.
#[tokio::test(flavor = "multi_thread")]
async fn dashboard_decodes_unknown_structs_from_their_schemas() {
    let (server, client) = pair().await;
    let mut events = client.events();
    let _all = client.subscribe_multiple(
        vec!["/".into()],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );
    let publisher = server.publish::<Outer>("/robot/outer");
    publisher.set(Outer {
        inner: Inner { a: -1, b: 2 },
        c: 3,
    });

    let mut db = StructDatabase::new();
    let mut outer_bytes = None;
    tokio::time::timeout(Duration::from_secs(5), async {
        while db.find("Outer").is_none_or(|d| !d.is_valid()) || outer_bytes.is_none() {
            let Some(Event::Value { name, value, .. }) = events.recv().await else {
                continue;
            };
            let Value::Raw(bytes) = value.value else {
                continue;
            };
            if let Some(ty) = name.strip_prefix("/.schema/struct:") {
                db.add(ty, std::str::from_utf8(&bytes).unwrap()).unwrap();
            } else if &*name == "/robot/outer" {
                outer_bytes = Some(bytes);
            }
        }
    })
    .await
    .unwrap();

    let bytes = outer_bytes.unwrap();
    let outer = DynamicStruct::new(&db, "Outer", &bytes).unwrap();
    let inner = outer.get_struct(outer.field("inner").unwrap(), 0).unwrap();
    assert_eq!(inner.get_int(inner.field("a").unwrap(), 0), Some(-1));
    assert_eq!(outer.get_int(outer.field("c").unwrap(), 0), Some(3));
}

#[derive(Debug, Clone, PartialEq)]
struct Speed(f64);

impl NtProtobuf for Speed {
    const TYPE_NAME: &'static str = "demo.Speed";
    fn for_each_descriptor(f: &mut dyn FnMut(&str, &[u8])) {
        f("units.proto", b"units descriptor");
        f("demo.proto", b"demo descriptor");
    }
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(0x09);
        out.extend_from_slice(&self.0.to_le_bytes());
    }
    fn decode(data: &[u8]) -> Option<Self> {
        Some(Speed(f64::from_le_bytes(
            data.strip_prefix(&[0x09])?.try_into().ok()?,
        )))
    }
}

// Written by Claude.
#[tokio::test(flavor = "multi_thread")]
async fn protobuf_topics_publish_file_descriptors() {
    let (server, client) = pair().await;
    let _schemas = client.subscribe_multiple(
        vec!["/.schema/".into()],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );
    let sub = client.subscribe::<Proto<Speed>>("/speed", SubscriptionOptions::default());
    let publisher = server.publish::<Proto<Speed>>("/speed");
    publisher.set(Proto(Speed(2.5)));
    wait_for(&sub, Proto(Speed(2.5))).await;
    assert_eq!(
        client.topic("/speed").unwrap().ty.to_string(),
        "proto:demo.Speed"
    );
    for (file, data) in [
        ("units.proto", "units descriptor"),
        ("demo.proto", "demo descriptor"),
    ] {
        let topic = schema_topic(&client, &format!("proto:{file}")).await;
        assert_eq!(topic.ty.to_string(), "proto:FileDescriptorProto");
        assert_eq!(
            topic.value.unwrap().value,
            Value::Raw(data.as_bytes().to_vec())
        );
    }
}

// Written by Claude.
#[test]
fn values_of_the_wrong_size_are_not_structs() {
    assert_eq!(Inner::from_value(&Value::Raw(vec![0; 7])), None);
    assert_eq!(<Vec<Inner>>::from_value(&Value::Raw(vec![0; 12])), None);
    assert_eq!(<Vec<Inner>>::from_value(&Value::Raw(vec![])), Some(vec![]));
    assert_eq!(Inner::from_value(&Value::Double(1.0)), None);
    assert_eq!(Proto::<Speed>::from_value(&Value::Raw(vec![1, 2])), None);
}
