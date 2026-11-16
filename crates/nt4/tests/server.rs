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
