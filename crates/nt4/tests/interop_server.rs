//! Interop tests: real ntcore clients (pyntcore) against our server.
//!
//! Written by Claude. Each test starts one or more
//! `tests/interop/ntcore_peer.py` processes as ntcore clients of an
//! `nt4::Server`. Without `NT4_INTEROP_PYTHON` the tests print a note and
//! pass; see `tests/interop.rs` for setting it up.
//!
//! ntcore 2027 sends to a server on `127.x` without batching, and so do we
//! to a client there, so most tests run on that path; `batched_path_*` uses
//! the machine's network address when it has one.

#![cfg(feature = "server")]

use std::collections::HashMap;
use std::net::{IpAddr, UdpSocket};
use std::time::Duration;

use nt4::{Server, ServerConfig, ServerEvent, SubscriptionOptions, Value};
use serde_json::{Value as Json, json};

#[path = "interop/peer.rs"]
mod peer;
#[path = "support/wpi_types.rs"]
mod wpi_types;
use peer::{Peer, eventually};
use wpi_types::{Rotation2d, SwerveModuleVelocity};

async fn server() -> Server {
    Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap()
}

/// A peer running an ntcore client of `server`, once connected; `None`
/// without `NT4_INTEROP_PYTHON`.
async fn ntcore_client(server: &Server, name: &str) -> Option<Peer> {
    ntcore_client_at(server.local_addr().port(), name).await
}

async fn ntcore_client_at(port: u16, name: &str) -> Option<Peer> {
    let mut peer = Peer::spawn().await?;
    peer.start_client(name, port).await;
    wait_connected(&mut peer).await;
    Some(peer)
}

async fn wait_connected(peer: &mut Peer) {
    for _ in 0..500 {
        if peer.connected().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("ntcore client did not connect");
}

/// The value an ntcore client reads, waiting up to 5 s.
async fn ntcore_get(peer: &mut Peer, name: &str) -> Json {
    peer.cmd(json!({"cmd": "get", "name": name, "timeout": 5}))
        .await
}

fn warnings(events: &mut nt4::Events<ServerEvent>) -> Vec<String> {
    std::iter::from_fn(|| events.try_recv())
        .filter_map(|e| match e {
            ServerEvent::Warning(w) => Some(w),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn ntcore_client_connects_and_syncs_time() {
    let server = server().await;
    let Some(mut peer) = ntcore_client(&server, "ntdash").await else {
        return;
    };

    // We list it under its name plus our `@<n>` suffix, as ntcore does.
    let clients = eventually(|| {
        let c = server.clients();
        (!c.is_empty()).then_some(c)
    })
    .await;
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].name, "ntdash@1");
    assert_eq!(clients[0].version, nt4::ProtocolVersion::V4_1);

    // Its clock is synced to ours: a value it sets now is stamped close to
    // our server time.
    let offset = peer.cmd(json!({"cmd": "time_offset"})).await;
    assert!(offset["offset"].is_i64(), "{offset}");
    let sub = server.subscribe::<f64>("/sync/x", SubscriptionOptions::default());
    peer.cmd(json!({"cmd": "publish", "name": "/sync/x", "type": "double", "value": 1.0}))
        .await;
    let (_, stamped) = eventually(|| sub.get_timestamped()).await;
    let diff = server.now_us() - stamped.server_time_us;
    assert!((0..500_000).contains(&diff), "clock difference {diff} us");
}

#[tokio::test(flavor = "multi_thread")]
async fn ntcore_client_reads_every_type_from_the_server() {
    let server = server().await;
    let b = server.publish::<bool>("/t/boolean");
    let d = server.publish::<f64>("/t/double");
    let i = server.publish::<i64>("/t/int");
    let f = server.publish::<f32>("/t/float");
    let s = server.publish::<String>("/t/string");
    let r = server.publish::<Vec<u8>>("/t/raw");
    let ba = server.publish::<Vec<bool>>("/t/boolean[]");
    let da = server.publish::<Vec<f64>>("/t/double[]");
    let ia = server.publish::<Vec<i64>>("/t/int[]");
    let fa = server.publish::<Vec<f32>>("/t/float[]");
    let sa = server.publish::<Vec<String>>("/t/string[]");
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

    let Some(mut peer) = ntcore_client(&server, "reader").await else {
        return;
    };
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
        let got = ntcore_get(&mut peer, &format!("/t/{ty}")).await;
        assert_eq!(got["type"], ty);
        assert_eq!(got["value"], value, "{ty}");
    }

    // Later changes arrive too.
    d.set(2.5);
    let got = peer
        .cmd(json!({"cmd": "get", "name": "/t/double", "timeout": 5, "not": 1.25}))
        .await;
    assert_eq!(got["value"], 2.5);
}

#[tokio::test(flavor = "multi_thread")]
async fn server_reads_every_type_from_an_ntcore_client() {
    let server = server().await;
    let mut events = server.events();
    let _sub = server.subscribe_multiple(
        vec!["/t/".into()],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );
    let Some(mut peer) = ntcore_client(&server, "writer").await else {
        return;
    };
    let sent = [
        ("boolean", json!(true), Value::Boolean(true)),
        ("double", json!(1.25), Value::Double(1.25)),
        ("int", json!(-42), Value::Int(-42)),
        ("float", json!(0.5), Value::Float(0.5)),
        ("string", json!("text"), Value::String("text".into())),
        ("raw", json!([1, 2, 255]), Value::Raw(vec![1, 2, 255])),
        (
            "boolean[]",
            json!([true, false]),
            Value::BooleanArray(vec![true, false]),
        ),
        (
            "double[]",
            json!([0.5, -1.0]),
            Value::DoubleArray(vec![0.5, -1.0]),
        ),
        (
            "int[]",
            json!([1, i64::MAX]),
            Value::IntArray(vec![1, i64::MAX]),
        ),
        ("float[]", json!([0.25]), Value::FloatArray(vec![0.25])),
        (
            "string[]",
            json!(["a", "b"]),
            Value::StringArray(vec!["a".into(), "b".into()]),
        ),
    ];
    for (ty, json, _) in &sent {
        peer.cmd(json!({"cmd": "publish", "name": format!("/t/{ty}"), "type": ty, "value": json}))
            .await;
    }

    let mut got = HashMap::new();
    let mut types = HashMap::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while got.len() < sent.len() {
            match events.recv().await.unwrap() {
                ServerEvent::Value { name, value, .. } => {
                    got.insert(name.to_string(), value.value);
                }
                ServerEvent::Announced { name, ty, .. } => {
                    types.insert(name.to_string(), ty.to_string());
                }
                _ => {}
            }
        }
    })
    .await
    .expect("every value");
    for (ty, _, value) in sent {
        let name = format!("/t/{ty}");
        assert_eq!(got[&name], value, "{ty}");
        assert_eq!(types[&name], ty);
    }
    assert_eq!(warnings(&mut events), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn relays_between_ntcore_clients() {
    let server = server().await;
    let Some(mut a) = ntcore_client(&server, "a").await else {
        return;
    };
    let mut b = ntcore_client(&server, "b").await.unwrap();
    a.cmd(json!({"cmd": "publish", "name": "/relay/x", "type": "double", "value": 4.5}))
        .await;
    assert_eq!(ntcore_get(&mut b, "/relay/x").await["value"], 4.5);
    a.cmd(json!({"cmd": "publish", "name": "/relay/x", "type": "double", "value": 5.5}))
        .await;
    let got = b
        .cmd(json!({"cmd": "get", "name": "/relay/x", "timeout": 5, "not": 4.5}))
        .await;
    assert_eq!(got["value"], 5.5);
}

#[tokio::test(flavor = "multi_thread")]
async fn unpublished_topics_go_away() {
    let server = server().await;
    let Some(mut a) = ntcore_client(&server, "a").await else {
        return;
    };
    let mut b = ntcore_client(&server, "b").await.unwrap();
    a.cmd(json!({"cmd": "publish", "name": "/gone", "type": "int", "value": 1}))
        .await;
    ntcore_get(&mut b, "/gone").await;
    a.cmd(json!({"cmd": "unpublish", "name": "/gone"})).await;
    let mut names = Json::Null;
    for _ in 0..500 {
        names = b.cmd(json!({"cmd": "topics", "prefix": "/"})).await["names"].clone();
        if names == json!([]) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(names, json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn retained_topic_outlives_its_ntcore_publisher() {
    let server = server().await;
    let Some(mut a) = ntcore_client(&server, "a").await else {
        return;
    };
    a.cmd(json!({"cmd": "publish", "name": "/keep", "type": "string", "value": "v"}))
        .await;
    a.cmd(json!({"cmd": "set_properties", "name": "/keep", "properties": {"retained": true}}))
        .await;
    // Wait until the property reached the server before A leaves.
    let mut events = server.events();
    let _watch = server.subscribe::<String>("/keep", SubscriptionOptions::default());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.unwrap() {
                ServerEvent::Announced { properties, .. }
                | ServerEvent::Properties {
                    update: properties, ..
                } if properties.get("retained") == Some(&json!(true)) => {
                    return;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("retained property");
    a.cmd(json!({"cmd": "stop_client"})).await;
    eventually(|| server.clients().is_empty().then_some(())).await;

    let mut b = ntcore_client(&server, "b").await.unwrap();
    assert_eq!(ntcore_get(&mut b, "/keep").await["value"], "v");
    let props = b.cmd(json!({"cmd": "properties", "name": "/keep"})).await;
    assert_eq!(props["properties"]["retained"], true, "{props}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ntcore_client_sees_meta_topics() {
    let server = server().await;
    let Some(mut peer) = ntcore_client(&server, "meta").await else {
        return;
    };
    // $clients: one map with our name for it.
    let got = ntcore_get(&mut peer, "$clients").await;
    assert_eq!(got["type"], "msgpack");
    let bytes: Vec<u8> = got["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(bytes[0], 0x91, "one client: {bytes:?}");
    assert!(bytes.windows(6).any(|w| w == b"meta@1"), "{bytes:?}");
    // $clientpub$<name> lists its own publisher.
    peer.cmd(json!({"cmd": "publish", "name": "/m/x", "type": "int", "value": 1}))
        .await;
    let got = peer
        .cmd(json!({"cmd": "get", "name": "$clientpub$meta@1", "timeout": 5, "not": [144]}))
        .await;
    let bytes: Vec<u8> = got["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert!(bytes.windows(4).any(|w| w == b"/m/x"), "{bytes:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ntcore_client_reconnects_after_server_restart() {
    let server = server().await;
    let port = server.local_addr().port();
    let Some(mut peer) = ntcore_client(&server, "re").await else {
        return;
    };
    peer.cmd(json!({"cmd": "publish", "name": "/re/x", "type": "double", "value": 1.0}))
        .await;
    let sub = server.subscribe::<f64>("/re/x", SubscriptionOptions::default());
    assert_eq!(eventually(|| sub.get()).await, 1.0);
    drop(sub);
    drop(server);

    // The client notices and reconnects to a new server on the same port,
    // publishing its topic and last value again.
    for _ in 0..500 {
        if !peer.connected().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let server = Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port,
        ..Default::default()
    })
    .await
    .unwrap();
    let sub = server.subscribe::<f64>("/re/x", SubscriptionOptions::default());
    assert_eq!(eventually(|| sub.get()).await, 1.0);
    wait_connected(&mut peer).await;
}

/// This machine's address on its default route, if it has one (no packets
/// are sent).
fn network_address() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[tokio::test(flavor = "multi_thread")]
async fn batched_path_relays_values() {
    // Over a non-loopback address both sides batch per period, as on a
    // robot network.
    let Some(ip) = network_address() else {
        eprintln!("no network address; skipping");
        return;
    };
    let server = Server::start(ServerConfig {
        listen_address: ip,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let port = server.local_addr().port();
    let Some(mut a) = Peer::spawn().await else {
        return;
    };
    a.cmd(json!({"cmd": "start_client", "name": "a", "port": port, "host": ip.to_string()}))
        .await;
    wait_connected(&mut a).await;
    let mut b = Peer::spawn().await.unwrap();
    b.cmd(json!({"cmd": "start_client", "name": "b", "port": port, "host": ip.to_string()}))
        .await;
    wait_connected(&mut b).await;
    let mut events = server.events();

    for v in 1..=5 {
        a.cmd(json!({"cmd": "publish", "name": "/batch/x", "type": "int", "value": v}))
            .await;
    }
    // B ends up with the last value (earlier ones may coalesce away).
    let mut got = Json::Null;
    for _ in 0..250 {
        got = ntcore_get(&mut b, "/batch/x").await;
        if got["value"] == 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(got["value"], 5, "{got}");
    assert!(
        server
            .clients()
            .iter()
            .all(|c| !c.address.ip().is_loopback())
    );
    assert_eq!(warnings(&mut events), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn persistent_file_matches_ntcore_byte_for_byte() {
    // ntcore saves a file of awkward values; our core loads it and dumps it
    // again, and must write exactly the same bytes.
    let Some(mut peer) = Peer::spawn().await else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("nt4-persist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("networktables.json");
    let _ = std::fs::remove_file(&file);
    let port = peer.port4;
    peer.cmd(json!({"cmd": "start_server", "port": port, "persist": file.to_str().unwrap()}))
        .await;
    let values = [
        ("/p/double", "double", json!(0.1)),
        ("/p/big", "double", json!(1e21)),
        ("/p/tiny", "double", json!(1.5e-7)),
        ("/p/negzero", "double", json!(-0.0)),
        ("/p/int", "int", json!(-42)),
        ("/p/bool", "boolean", json!(true)),
        ("/p/float", "float", json!(0.1)),
        // ntcore on Windows writes characters above U+FFFF wrongly (😀 as
        // a lone : wpiutil's JSON writer decodes UTF-8 into a 16-bit
        // `wint_t` there), so leave them out on Windows. nt4 writes the
        // surrogate pair everywhere.
        (
            "/p/string",
            "string",
            json!(if cfg!(windows) {
                "a/b<é\"\n&'=>"
            } else {
                "a/b<é😀\"\n&'=>"
            }),
        ),
        ("/p/raw", "raw", json!([0, 255, 7, 128])),
        ("/p/da", "double[]", json!([0.5, 1e-7, 3.0])),
        ("/p/fa", "float[]", json!([0.25, 0.1])),
        ("/p/ia", "int[]", json!([1, -2, i64::MAX])),
        ("/p/ba", "boolean[]", json!([true, false])),
        ("/p/sa", "string[]", json!(["x/y", "é", ""])),
    ];
    for (name, ty, value) in &values {
        peer.cmd(json!({"cmd": "publish", "name": name, "type": ty, "value": value}))
            .await;
        peer.cmd(
            json!({"cmd": "set_properties", "name": name, "properties": {"persistent": true}}),
        )
        .await;
    }
    // More than one property puts the object on several lines.
    peer.cmd(json!({"cmd": "set_properties", "name": "/p/double", "properties": {"units": "m", "max": 2.5}}))
        .await;

    // ntcore saves once a second when something changed.
    let mut text = String::new();
    for _ in 0..100 {
        text = std::fs::read_to_string(&file).unwrap_or_default();
        if values
            .iter()
            .all(|(n, _, _)| text.contains(&n.replace('/', r"\/")))
            && text.contains("units")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(text.contains("units"), "ntcore never saved: {text}");

    // Real UTF-8 reached ntcore: é (and off Windows the emoji's surrogate
    // pair), escaped.
    let esc = |hex: &str| format!("{}u{hex}", '\\');
    assert!(text.contains(&esc("00e9")), "{text}");
    if !cfg!(windows) {
        assert!(
            text.contains(&format!("{}{}", esc("d83d"), esc("de00"))),
            "{text}"
        );
    }

    let mut core = nt4::core::ServerCore::new();
    let errors = core.load_persistent(&text, 1);
    assert_eq!(errors, Vec::<String>::new());
    // ntcore writes the file in text mode, so with CRLF on Windows; the
    // core's dump uses LF and the server runtime converts on Windows.
    assert_eq!(core.dump_persistent(), text.replace("\r\n", "\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn ntcore_client_reads_server_structs_and_schemas() {
    let server = server().await;
    let modules: Vec<_> = (0..4)
        .map(|i| SwerveModuleVelocity {
            velocity: i as f64,
            angle: Rotation2d { value: -(i as f64) },
        })
        .collect();
    let publisher = server.publish::<Vec<SwerveModuleVelocity>>("/drive/modules");
    publisher.set(modules.clone());
    let Some(mut peer) = ntcore_client(&server, "dash-structs").await else {
        return;
    };

    let got = ntcore_get(&mut peer, "/drive/modules").await;
    assert_eq!(got["type"], "struct:SwerveModuleVelocity[]");
    let nt4::Value::Raw(bytes) = nt4::NtValue::into_value(modules) else {
        unreachable!();
    };
    assert_eq!(got["value"], json!(bytes));
    for (name, schema) in [
        ("struct:Rotation2d", "double value"),
        (
            "struct:SwerveModuleVelocity",
            "double velocity;Rotation2d angle",
        ),
    ] {
        let topic = format!("/.schema/{name}");
        let got = ntcore_get(&mut peer, &topic).await;
        assert_eq!(got["type"], "structschema", "{name}");
        assert_eq!(got["value"], json!(schema.as_bytes()), "{name}");
        let props = peer.cmd(json!({"cmd": "properties", "name": topic})).await;
        assert_eq!(props["properties"], json!({"retained": true}), "{name}");
    }
}

/// Send `request` to `port` over plain TCP and return the whole response,
/// retrying the connect for up to 5 s while a server starts.
async fn http_get(port: u16, request: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut tcp = None;
    for _ in 0..500 {
        if let Ok(t) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
            tcp = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut tcp = tcp.expect("server did not start");
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), tcp.read_to_end(&mut out))
        .await
        .expect("server did not close")
        .unwrap();
    String::from_utf8(out).unwrap()
}

// Plain HTTP (AdvantageScope checks `GET /` before opening its WebSocket)
// gets the same bytes from our server as from an ntcore server. HTTP/1.0,
// so ntcore closes after one response as we always do.
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_matches_ntcore() {
    let Some(mut peer) = Peer::spawn().await else {
        return;
    };
    peer.start_server().await;
    let ours = server().await;
    for request in [
        "GET / HTTP/1.0\r\n\r\n",
        "GET /?probe=1 HTTP/1.0\r\nHost: x\r\n\r\n",
        "GET /nt/persistent.json HTTP/1.0\r\n\r\n",
        "GET /nope HTTP/1.0\r\n\r\n",
        "POST / HTTP/1.0\r\nContent-Length: 0\r\n\r\n",
    ] {
        let theirs = http_get(peer.port4, request).await;
        let mine = http_get(ours.local_addr().port(), request).await;
        assert_eq!(mine, theirs, "{request:?}");
    }
}
