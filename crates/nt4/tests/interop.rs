//! Interop tests: our client against a real ntcore server (pyntcore).
//!
//! Written by Claude. Each test starts `tests/interop/ntcore_peer.py` with
//! the Python in `NT4_INTEROP_PYTHON` (which must have `pyntcore`
//! installed). Without that variable the tests print a note and pass, so
//! `cargo test` works without Python; CI sets it.
//!
//! ```text
//! python -m venv .venv && .venv/bin/pip install --pre pyntcore==2027.0.0a8
//! NT4_INTEROP_PYTHON=.venv/bin/python cargo test -p nt4 --test interop
//! ```

use std::time::Duration;

use nt4::{Event, PublishOptions, SubscriptionOptions};
use serde_json::json;

#[path = "interop/peer.rs"]
mod peer;
use peer::{Peer, eventually};

/// A peer running an ntcore server, or `None` if `NT4_INTEROP_PYTHON` isn't
/// set.
async fn server_peer() -> Option<Peer> {
    let mut peer = Peer::spawn().await?;
    peer.start_server().await;
    Some(peer)
}

#[tokio::test(flavor = "multi_thread")]
async fn connects_and_syncs_time() {
    let Some(mut peer) = server_peer().await else {
        return;
    };
    let client = peer.client("interop-sync");
    assert!(client.wait_for_sync(Duration::from_secs(5)).await);
    assert!(client.is_connected());

    // ntcore lists us by our name plus its own `@<n>` suffix.
    let names = peer.cmd(json!({"cmd": "connections"})).await;
    let names = names["names"].as_array().unwrap();
    assert_eq!(names.len(), 1);
    assert!(
        names[0].as_str().unwrap().starts_with("interop-sync@"),
        "{names:?}"
    );

    // A value the server sets now is stamped close to our idea of server time.
    let sub = client.subscribe::<f64>("/sync/x", SubscriptionOptions::default());
    peer.cmd(json!({"cmd": "publish", "name": "/sync/x", "type": "double", "value": 1.0}))
        .await;
    let (_, stamped) = eventually(|| sub.get_timestamped()).await;
    let ours = client.server_time().unwrap();
    let diff = ours - stamped.server_time_us;
    assert!((0..500_000).contains(&diff), "clock difference {diff} us");
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_server_values() {
    let Some(mut peer) = server_peer().await else {
        return;
    };
    peer.cmd(json!({"cmd": "publish", "name": "/srv/d", "type": "double", "value": 1.5}))
        .await;
    peer.cmd(json!({"cmd": "publish", "name": "/srv/s", "type": "string", "value": "hi"}))
        .await;
    peer.cmd(json!({"cmd": "publish", "name": "/srv/da", "type": "double[]", "value": [1.0, 2.0]}))
        .await;

    let client = peer.client("interop-read");
    let mut d = client.subscribe::<f64>("/srv/d", SubscriptionOptions::default());
    let s = client.subscribe::<String>("/srv/s", SubscriptionOptions::default());
    let da = client.subscribe::<Vec<f64>>("/srv/da", SubscriptionOptions::default());
    assert_eq!(eventually(|| d.get()).await, 1.5);
    assert_eq!(eventually(|| s.get()).await, "hi");
    assert_eq!(eventually(|| da.get()).await, vec![1.0, 2.0]);

    // Later changes arrive too, and wake `changed`.
    peer.cmd(json!({"cmd": "publish", "name": "/srv/d", "type": "double", "value": 2.5}))
        .await;
    let next = tokio::time::timeout(Duration::from_secs(5), d.changed()).await;
    assert_eq!(next, Ok(Ok(Some(2.5))));
}

#[tokio::test(flavor = "multi_thread")]
async fn prefix_subscription_sees_announcements() {
    let Some(mut peer) = server_peer().await else {
        return;
    };
    let client = peer.client("interop-prefix");
    let mut events = client.events();
    let _sub = client.subscribe_multiple(
        vec!["/tree/".into()],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );
    assert!(client.wait_for_sync(Duration::from_secs(5)).await);
    peer.cmd(json!({"cmd": "publish", "name": "/tree/a", "type": "int", "value": 7}))
        .await;
    peer.cmd(json!({"cmd": "publish", "name": "/other", "type": "int", "value": 8}))
        .await;

    let got = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Event::Value { name, value, .. }) = events.recv().await {
                return (name, value.value);
            }
        }
    })
    .await
    .expect("value event");
    assert_eq!(&*got.0, "/tree/a");
    assert_eq!(got.1, nt4::Value::Int(7));
}

#[tokio::test(flavor = "multi_thread")]
async fn server_reads_every_type() {
    let Some(mut peer) = server_peer().await else {
        return;
    };
    let client = peer.client("interop-types");
    let o = PublishOptions::default;
    let b = client.publish::<bool>("/t/boolean", o());
    let d = client.publish::<f64>("/t/double", o());
    let i = client.publish::<i64>("/t/int", o());
    let f = client.publish::<f32>("/t/float", o());
    let s = client.publish::<String>("/t/string", o());
    let r = client.publish::<Vec<u8>>("/t/raw", o());
    let ba = client.publish::<Vec<bool>>("/t/boolean[]", o());
    let da = client.publish::<Vec<f64>>("/t/double[]", o());
    let ia = client.publish::<Vec<i64>>("/t/int[]", o());
    let fa = client.publish::<Vec<f32>>("/t/float[]", o());
    let sa = client.publish::<Vec<String>>("/t/string[]", o());
    b.set(true);
    d.set(1.25);
    i.set(-42);
    f.set(0.5);
    s.set("text".into());
    r.set(vec![1, 2, 255]);
    ba.set(vec![true, false]);
    da.set(vec![0.5, -1.0]);
    ia.set(vec![1, i64::MAX]);
    fa.set(vec![0.25]);
    sa.set(vec!["a".into(), "b".into()]);

    let expected = [
        ("boolean", json!(true)),
        ("double", json!(1.25)),
        ("int", json!(-42)),
        ("float", json!(0.5)),
        ("string", json!("text")),
        ("raw", json!([1, 2, 255])),
        ("boolean[]", json!([true, false])),
        ("double[]", json!([0.5, -1.0])),
        ("int[]", json!([1, i64::MAX])),
        ("float[]", json!([0.25])),
        ("string[]", json!(["a", "b"])),
    ];
    for (ty, value) in expected {
        let got = peer
            .cmd(json!({"cmd": "get", "name": format!("/t/{ty}"), "timeout": 5}))
            .await;
        assert_eq!(got["type"], ty);
        assert_eq!(got["value"], value, "{ty}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn republishes_after_server_restart() {
    let Some(mut peer) = server_peer().await else {
        return;
    };
    let client = peer.client("interop-reconnect");
    let x = client.publish::<f64>("/re/x", PublishOptions::default());
    let echo = client.subscribe::<f64>("/re/echo", SubscriptionOptions::default());
    x.set(1.0);
    let got = peer
        .cmd(json!({"cmd": "get", "name": "/re/x", "timeout": 5}))
        .await;
    assert_eq!(got["value"], 1.0);

    peer.stop_server().await;
    eventually(|| (!client.is_connected()).then_some(())).await;
    // Set while disconnected: kept and sent after the reconnect.
    x.set(2.0);
    peer.start_server().await;

    let got = peer
        .cmd(json!({"cmd": "get", "name": "/re/x", "timeout": 5, "not": 1.0}))
        .await;
    assert_eq!(got["value"], 2.0);

    // The subscription came back too.
    peer.cmd(json!({"cmd": "publish", "name": "/re/echo", "type": "double", "value": 3.0}))
        .await;
    assert_eq!(eventually(|| echo.get()).await, 3.0);
}

/// The bytes of a struct value, as JSON numbers (how the peer reports raw).
fn bytes_json<T: nt4::NtValue>(value: T) -> serde_json::Value {
    match value.into_value() {
        nt4::Value::Raw(bytes) => json!(bytes),
        other => panic!("not raw: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn struct_topics_and_schemas_match_ntcore() {
    use nt4::wpimath::{Pose2d, Rotation2d, Translation2d};
    let Some(mut peer) = server_peer().await else {
        return;
    };
    // ntcore registers one schema itself, to compare properties with.
    peer.cmd(json!({"cmd": "add_schema", "name": "struct:Ref", "type": "structschema", "schema": "double x"}))
        .await;
    let reference = peer
        .cmd(json!({"cmd": "properties", "name": "/.schema/struct:Ref"}))
        .await["properties"]
        .clone();
    assert_eq!(reference, json!({"retained": true}));

    let client = peer.client("interop-struct");
    let pose = Pose2d {
        translation: Translation2d { x: 1.0, y: -2.0 },
        rotation: Rotation2d { value: 0.25 },
    };
    let publisher = client.publish::<Pose2d>("/vision/pose", PublishOptions::default());
    publisher.set(pose);

    let got = peer
        .cmd(json!({"cmd": "get", "name": "/vision/pose"}))
        .await;
    assert_eq!(got["type"], "struct:Pose2d");
    assert_eq!(got["value"], bytes_json(pose));
    for (name, schema) in [
        ("struct:Translation2d", "double x;double y"),
        ("struct:Rotation2d", "double value"),
        (
            "struct:Pose2d",
            "Translation2d translation;Rotation2d rotation",
        ),
    ] {
        let topic = format!("/.schema/{name}");
        let got = peer.cmd(json!({"cmd": "get", "name": topic})).await;
        assert_eq!(got["type"], "structschema", "{name}");
        assert_eq!(got["value"], json!(schema.as_bytes()), "{name}");
        let props = peer.cmd(json!({"cmd": "properties", "name": topic})).await;
        assert_eq!(props["properties"], reference, "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_ntcore_structs_typed_and_by_schema() {
    use nt4::proto::structs::{DynamicStruct, StructDatabase};
    use nt4::wpimath::Translation2d;
    let Some(mut peer) = server_peer().await else {
        return;
    };
    let value = Translation2d { x: 3.0, y: 4.0 };
    peer.cmd(json!({"cmd": "add_schema", "name": "struct:Translation2d", "type": "structschema", "schema": "double x;double y"}))
        .await;
    peer.cmd(json!({"cmd": "publish", "name": "/t", "type": "struct:Translation2d", "value": bytes_json(value)}))
        .await;

    let client = peer.client("interop-struct-read");
    let typed = client.subscribe::<Translation2d>("/t", SubscriptionOptions::default());
    let schema = client.subscribe::<Vec<u8>>(
        "/.schema/struct:Translation2d",
        SubscriptionOptions::default(),
    );
    assert_eq!(eventually(|| typed.get()).await, value);

    // A dashboard that doesn't know the type reads it through the schema.
    let schema = String::from_utf8(eventually(|| schema.get()).await).unwrap();
    let mut db = StructDatabase::new();
    db.add("Translation2d", &schema).unwrap();
    let nt4::Value::Raw(bytes) = client.topic("/t").unwrap().value.unwrap().value else {
        panic!("not raw");
    };
    let t = DynamicStruct::new(&db, "Translation2d", &bytes).unwrap();
    assert_eq!(t.get_double(t.field("y").unwrap(), 0), Some(4.0));
    // ntcore's schema topic has the properties our add_schema gives.
    let props = client
        .topic("/.schema/struct:Translation2d")
        .unwrap()
        .properties;
    assert_eq!(
        serde_json::to_value(props).unwrap(),
        json!({"retained": true})
    );
}
