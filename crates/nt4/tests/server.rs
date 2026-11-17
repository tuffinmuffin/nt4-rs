//! The tokio server against our own client and raw WebSockets.
//!
//! Written by Claude. These need no Python; `interop_server.rs` checks the
//! same server against real ntcore clients.

#![cfg(all(feature = "server", feature = "client"))]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nt4::proto::{BinaryMessage, BinaryReader, encode_binary};
use nt4::{
    Client, ClientConfig, PublishOptions, Server, ServerConfig, ServerEvent, SubscriptionOptions,
    Value,
};
use tokio_tungstenite::tungstenite::{
    Message,
    client::IntoClientRequest,
    http::{HeaderValue, StatusCode, header::SEC_WEBSOCKET_PROTOCOL},
};

async fn server() -> Server {
    Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap()
}

fn client(server: &Server, name: &str) -> Client {
    let mut config = ClientConfig::server(name, "127.0.0.1", server.local_addr().port());
    config.reconnect_delay = Duration::from_millis(50);
    Client::start(config)
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

#[tokio::test(flavor = "multi_thread")]
async fn client_and_server_exchange_values() {
    let server = server().await;
    let from_server = server.publish::<f64>("/s/x");
    from_server.set(1.5);
    let to_server = server.subscribe::<String>("/c/name", SubscriptionOptions::default());

    let c = client(&server, "ours");
    let x = c.subscribe::<f64>("/s/x", SubscriptionOptions::default());
    let name = c.publish::<String>("/c/name", PublishOptions::default());
    assert!(c.wait_for_sync(Duration::from_secs(5)).await);
    name.set("hello".into());

    assert_eq!(eventually(|| x.get()).await, 1.5);
    assert_eq!(eventually(|| to_server.get()).await, "hello");

    // The client's server time is ours, give or take the RTT.
    let diff = server.now_us() - c.server_time().unwrap();
    assert!(diff.abs() < 100_000, "{diff} us");

    let clients = server.clients();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].name, "ours@1");
}

#[tokio::test(flavor = "multi_thread")]
async fn values_flow_between_two_clients() {
    let server = server().await;
    let a = client(&server, "a");
    let b = client(&server, "b");
    let p = a.publish::<i64>("/relay", PublishOptions::default());
    let s = b.subscribe::<i64>("/relay", SubscriptionOptions::default());
    assert!(a.wait_for_sync(Duration::from_secs(5)).await);
    p.set(7);
    assert_eq!(eventually(|| s.get()).await, 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnect_removes_the_client_and_its_topics() {
    let server = server().await;
    let mut events = server.events();
    let _watch = server.subscribe_multiple(
        vec!["/".into()],
        SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );
    let c = client(&server, "leaving");
    let p = c.publish::<bool>("/leaving/flag", PublishOptions::default());
    assert!(c.wait_for_sync(Duration::from_secs(5)).await);
    p.set(true);
    eventually(|| (server.clients().len() == 1).then_some(())).await;
    drop(p);
    drop(c);
    eventually(|| server.clients().is_empty().then_some(())).await;

    let unannounced = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(ServerEvent::Unannounced { name, .. }) = events.recv().await {
                return name;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(&*unannounced, "/leaving/flag");
}

#[tokio::test(flavor = "multi_thread")]
async fn client_reconnects_when_the_server_comes_back() {
    let server = server().await;
    let port = server.local_addr().port();
    let c = client(&server, "re");
    let p = c.publish::<f64>("/re/x", PublishOptions::default());
    p.set(3.0);
    assert!(c.wait_for_sync(Duration::from_secs(5)).await);
    drop(server);
    eventually(|| (!c.is_connected()).then_some(())).await;

    let server = Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port,
        ..Default::default()
    })
    .await
    .unwrap();
    let s = server.subscribe::<f64>("/re/x", SubscriptionOptions::default());
    assert_eq!(eventually(|| s.get()).await, 3.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn rtt_subprotocol_answers_pings() {
    let server = server().await;
    let url = format!("ws://{}/nt/rtt-only", server.local_addr());
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("rtt.networktables.first.wpi.edu"),
    );
    let (mut ws, resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    assert_eq!(
        resp.headers()[SEC_WEBSOCKET_PROTOCOL],
        "rtt.networktables.first.wpi.edu"
    );
    let mut ping = Vec::new();
    encode_binary(
        &mut ping,
        &BinaryMessage {
            id: -1,
            timestamp_us: 0,
            value: Value::Int(77),
        },
    );
    ws.send(Message::Binary(ping.into())).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Binary(reply) = reply else {
        panic!("{reply:?}")
    };
    let reply = BinaryReader::new(&reply).next().unwrap().unwrap();
    assert_eq!((reply.id, reply.value), (-1, Value::Int(77)));
    assert!(reply.timestamp_us > 0);
    // An RTT connection is not a client.
    assert!(server.clients().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_path_is_refused() {
    let server = server().await;
    let url = format!("ws://{}/wrong", server.local_addr());
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("v4.1.networktables.first.wpi.edu"),
    );
    match tokio_tungstenite::connect_async(req).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_server_closes_connections() {
    let server = server().await;
    let c = client(&server, "x");
    assert!(c.wait_for_sync(Duration::from_secs(5)).await);
    drop(server);
    eventually(|| (!c.is_connected()).then_some(())).await;
}

/// Send `request` over plain TCP and return the whole response.
async fn http(server: &Server, request: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut tcp = tokio::net::TcpStream::connect(server.local_addr())
        .await
        .unwrap();
    // Two writes, so the request head arrives in pieces.
    let (a, b) = request.split_at(request.len() / 2);
    tcp.write_all(a.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    tcp.write_all(b.as_bytes()).await.unwrap();
    let mut out = String::new();
    tokio::time::timeout(Duration::from_secs(5), tcp.read_to_string(&mut out))
        .await
        .unwrap()
        .unwrap();
    out
}

// Plain HTTP is answered as ntcore's ServerConnection4::ProcessRequest does
// (AdvantageScope checks the server this way before opening its WebSocket).
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_is_answered_like_ntcore() {
    let server = server().await;
    let root = http(&server, "GET / HTTP/1.1\r\nHost: x\r\n\r\n").await;
    assert!(root.starts_with("HTTP/1.1 200 OK\r\n"), "{root}");
    assert!(root.contains("Content-Type: text/html\r\n"), "{root}");
    assert!(
        root.contains("Access-Control-Allow-Origin: *\r\n"),
        "{root}"
    );
    assert!(
        root.ends_with("<body><p>WebSockets must be used to access NetworkTables.</body></html>"),
        "{root}"
    );

    let query = http(&server, "GET /?x=1 HTTP/1.0\r\n\r\n").await;
    assert!(query.starts_with("HTTP/1.0 200 OK\r\n"), "{query}");

    let persistent = http(&server, "GET /nt/persistent.json HTTP/1.1\r\n\r\n").await;
    assert!(
        persistent.starts_with("HTTP/1.1 200 OK\r\n"),
        "{persistent}"
    );
    assert!(persistent.contains("Content-Type: application/json\r\n"));
    assert!(persistent.trim_end().ends_with(']'), "{persistent}");

    let missing = http(&server, "GET /nope HTTP/1.1\r\n\r\n").await;
    assert!(
        missing.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "{missing}"
    );
    assert!(missing.ends_with("404: Not Found!\r\nResource not found"));

    // HEAD gets ntcore's answer (a 404) without the body.
    let head = http(&server, "HEAD / HTTP/1.1\r\n\r\n").await;
    assert!(head.starts_with("HTTP/1.1 404 Not Found\r\n"), "{head}");
    assert!(head.ends_with("\r\n\r\n"), "{head}");

    // The server still takes WebSocket clients afterwards.
    let c = client(&server, "after-http");
    eventually(|| c.is_connected().then_some(())).await;
}

// Requests that aren't HTTP at all: wpinet closes the connection without an
// answer, and so do we.
#[tokio::test(flavor = "multi_thread")]
async fn requests_that_are_not_http_are_closed_unanswered() {
    let server = server().await;
    // Not a request line.
    assert_eq!(http(&server, "hello there\r\n\r\n").await, "");
    // A head larger than we read before giving up, with no end in sight.
    // The server stops reading partway, so the write itself may fail.
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut tcp = tokio::net::TcpStream::connect(server.local_addr())
            .await
            .unwrap();
        let huge = format!("GET / HTTP/1.1\r\nX: {}\r\n", "a".repeat(64 * 1024));
        let _ = tcp.write_all(huge.as_bytes()).await;
        let mut out = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(5), tcp.read_to_end(&mut out))
            .await
            .expect("not closed within 5 s");
        // Closed (possibly reset, since we were still writing), unanswered.
        assert!(
            read.is_err() || out.is_empty(),
            "{:?}",
            String::from_utf8_lossy(&out)
        );
    }
    // A target that isn't a path is a 400.
    let bad = http(&server, "GET nope HTTP/1.1\r\n\r\n").await;
    assert!(bad.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{bad}");
    // An absolute-form target is answered by its path.
    let absolute = http(&server, "GET http://robot:5810/ HTTP/1.1\r\n\r\n").await;
    assert!(absolute.starts_with("HTTP/1.1 200 OK\r\n"), "{absolute}");
    // The server is still fine.
    let c = client(&server, "after-garbage");
    eventually(|| c.is_connected().then_some(())).await;
}

fn rtt_ping(value: i64) -> Message {
    let mut out = Vec::new();
    encode_binary(
        &mut out,
        &BinaryMessage {
            id: -1,
            timestamp_us: 0,
            value: Value::Int(value),
        },
    );
    Message::Binary(out.into())
}

async fn raw_connect(server: &Server, path: &str, protocol: &'static str) -> RawWs {
    let url = format!("ws://{}/nt/{path}", server.local_addr());
    let mut req = url.into_client_request().unwrap();
    req.headers_mut()
        .insert(SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static(protocol));
    tokio_tungstenite::connect_async(req).await.unwrap().0
}

type RawWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next frame that isn't a message or the server's own keepalive ping
/// (a pong, a close, or the end), within 5 s.
async fn next_control(ws: &mut RawWs) -> Option<Message> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(_) | Message::Binary(_) | Message::Ping(_))) => {}
                other => return other.and_then(Result::ok),
            }
        }
    })
    .await
    .expect("nothing within 5 s")
}

const RTT: &str = "rtt.networktables.first.wpi.edu";
const NT41: &str = "v4.1.networktables.first.wpi.edu";

#[tokio::test(flavor = "multi_thread")]
async fn rtt_connection_answers_websocket_pings_and_closes_on_bad_frames() {
    let server = server().await;
    let mut ws = raw_connect(&server, "rtt", RTT).await;
    // A WebSocket ping gets a pong, and RTT pings keep working after it.
    ws.send(Message::Ping(b"p".to_vec().into())).await.unwrap();
    assert!(matches!(next_control(&mut ws).await, Some(Message::Pong(p)) if &p[..] == b"p"));
    ws.send(rtt_ping(5)).await.unwrap();
    let Some(Ok(Message::Binary(reply))) = ws.next().await else {
        panic!("no RTT reply")
    };
    assert_eq!(
        BinaryReader::new(&reply).next().unwrap().unwrap().value,
        Value::Int(5)
    );

    // As ntcore: a frame that doesn't decode ends the connection.
    ws.send(Message::Binary(vec![0xc1].into())).await.unwrap();
    assert!(matches!(
        next_control(&mut ws).await,
        Some(Message::Close(_)) | None
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_the_server_closes_rtt_connections() {
    let server = server().await;
    let mut ws = raw_connect(&server, "rtt", RTT).await;
    ws.send(rtt_ping(1)).await.unwrap();
    assert!(matches!(ws.next().await, Some(Ok(Message::Binary(_)))));
    drop(server);
    assert!(matches!(
        next_control(&mut ws).await,
        Some(Message::Close(_)) | None
    ));
}

// The pong to a client's ping is queued by tungstenite and only goes out on
// a flush; the connection loop must flush even when it has nothing else to
// send.
#[tokio::test(flavor = "multi_thread")]
async fn nt4_connection_answers_websocket_pings() {
    let server = server().await;
    let mut ws = raw_connect(&server, "pinger", NT41).await;
    for i in 0..3u8 {
        ws.send(Message::Ping(vec![i].into())).await.unwrap();
        let got = next_control(&mut ws).await;
        assert!(
            matches!(&got, Some(Message::Pong(p)) if p[..] == [i]),
            "ping {i}: {got:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn nt4_client_sending_an_undecodable_frame_is_closed() {
    let server = server().await;
    let mut ws = raw_connect(&server, "garbage", NT41).await;
    eventually(|| (server.clients().len() == 1).then_some(())).await;
    ws.send(Message::Binary(vec![0xc1].into())).await.unwrap();
    assert!(matches!(
        next_control(&mut ws).await,
        Some(Message::Close(_)) | None
    ));
    eventually(|| server.clients().is_empty().then_some(())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn server_property_changes_and_flush_reach_clients() {
    let server = server().await;
    let c = client(&server, "props");
    let mut events = c.events();
    let _sub = c.subscribe::<i64>(
        "/s/slow",
        SubscriptionOptions {
            // Far longer than the test: values arrive only when flushed.
            periodic: Some(10.0),
            ..Default::default()
        },
    );
    assert!(c.wait_for_sync(Duration::from_secs(5)).await);
    let p = server.publish::<i64>("/s/slow");
    eventually(|| {
        c.topics()
            .iter()
            .any(|t| &*t.name == "/s/slow")
            .then_some(())
    })
    .await;

    let mut update = nt4::Properties::new();
    update.insert("cached".into(), false.into());
    server.set_properties("/s/slow", update);
    p.set(9);
    server.flush();

    let (mut saw_props, mut saw_value) = (false, false);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !(saw_props && saw_value) {
            match events.recv().await.unwrap() {
                nt4::Event::Properties { name, update, .. } if &*name == "/s/slow" => {
                    assert_eq!(update.get("cached"), Some(&false.into()));
                    saw_props = true;
                }
                nt4::Event::Value { name, value, .. } if &*name == "/s/slow" => {
                    assert_eq!(value.value, Value::Int(9));
                    saw_value = true;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("properties and flushed value within 5 s");
    assert!(format!("{server:?}").contains("local_addr"));
}
