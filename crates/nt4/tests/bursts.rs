//! Many frames arriving at once: the client and the server handle the frames
//! tungstenite has already read in batches, and none may be lost at a batch
//! boundary.
//!
//! Written by Claude. A raw WebSocket peer writes every frame before flushing,
//! so they reach the socket together, as ntcore's unbatched localhost path
//! sends them.

#![cfg(all(feature = "server", feature = "client"))]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nt4::proto::{
    BinaryMessage, ClientMessage, NtType, Properties, ServerMessage, encode_binary,
    encode_text_frame,
};
use nt4::{
    Client, ClientConfig, Event, Server, ServerConfig, ServerEvent, SubscriptionOptions, Value,
};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::{
    Message,
    client::IntoClientRequest,
    handshake::server::{Request, Response},
    http::{HeaderValue, header::SEC_WEBSOCKET_PROTOCOL},
};

/// More than one batch (256 frames), with a remainder.
const BURST: i64 = 1000;

fn value_frame(id: i32, value: i64) -> Message {
    let mut out = Vec::new();
    encode_binary(
        &mut out,
        &BinaryMessage {
            id,
            timestamp_us: 1,
            value: Value::Int(value),
        },
    );
    Message::Binary(out.into())
}

fn all() -> SubscriptionOptions {
    SubscriptionOptions {
        all: true,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn server_keeps_every_frame_of_a_burst() {
    let server = Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut events = server.events();
    let _sub = server.subscribe::<i64>("/burst", all());

    let url = format!("ws://{}/nt/raw", server.local_addr());
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("v4.1.networktables.first.wpi.edu"),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let publish = ClientMessage::Publish {
        name: "/burst".into(),
        pubuid: 1,
        ty: NtType::Int,
        properties: Properties::new(),
    };
    ws.send(Message::Text(encode_text_frame([&publish]).into()))
        .await
        .unwrap();
    for i in 0..BURST {
        ws.feed(value_frame(1, i)).await.unwrap();
    }
    ws.flush().await.unwrap();

    let mut got = Vec::new();
    while got.len() < BURST as usize {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out after {} values", got.len()))
            .unwrap();
        if let ServerEvent::Value { value, .. } = event {
            got.push(value.value);
        }
    }
    assert_eq!(got, (0..BURST).map(Value::Int).collect::<Vec<_>>());
}

#[tokio::test(flavor = "multi_thread")]
async fn client_keeps_every_frame_of_a_burst() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let raw_server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        // tungstenite's callback type, as in the server.
        #[allow(clippy::result_large_err)]
        let callback = |_: &Request, mut resp: Response| {
            resp.headers_mut().insert(
                SEC_WEBSOCKET_PROTOCOL,
                HeaderValue::from_static("v4.1.networktables.first.wpi.edu"),
            );
            Ok(resp)
        };
        let mut ws = tokio_tungstenite::accept_hdr_async(tcp, callback)
            .await
            .unwrap();
        let announce = ServerMessage::Announce {
            name: "/burst".into(),
            id: 7,
            ty: NtType::Int,
            pubuid: None,
            properties: Properties::new(),
        };
        ws.feed(Message::Text(encode_text_frame([&announce]).into()))
            .await
            .unwrap();
        for i in 0..BURST {
            ws.feed(value_frame(7, i)).await.unwrap();
        }
        ws.flush().await.unwrap();
        // Keep the connection open (and read what the client sends) until
        // the test ends.
        while ws.next().await.is_some() {}
    });

    let client = Client::start(ClientConfig::server("burst", "127.0.0.1", port));
    let mut events = client.events();
    let _sub = client.subscribe::<i64>("/burst", all());

    let mut got = Vec::new();
    while got.len() < BURST as usize {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out after {} values", got.len()))
            .unwrap();
        if let Event::Value { value, .. } = event {
            got.push(value.value);
        }
    }
    assert_eq!(got, (0..BURST).map(Value::Int).collect::<Vec<_>>());
    raw_server.abort();
}
