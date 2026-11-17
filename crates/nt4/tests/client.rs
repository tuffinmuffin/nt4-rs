//! The tokio client against raw WebSocket servers and our own server: server
//! fallback, how a connection ends, pings, and the handle types.
//!
//! Written by Claude. These need no Python; `interop.rs` checks the client
//! against a real ntcore server.

#![cfg(all(feature = "server", feature = "client"))]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nt4::{
    Client, ClientConfig, Event, Properties, PublishOptions, Server, ServerConfig, ServerEvent,
    SubscriptionOptions,
};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{Request, Response},
    http::{HeaderValue, header::SEC_WEBSOCKET_PROTOCOL},
};

type RawServerWs = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

/// A listener that accepts one NT 4.1 WebSocket connection and hands it to
/// `serve`. Returns the port.
async fn raw_server<F, Fut>(serve: F) -> u16
where
    F: FnOnce(RawServerWs) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        #[allow(clippy::result_large_err)]
        let callback = |_: &Request, mut resp: Response| {
            resp.headers_mut().insert(
                SEC_WEBSOCKET_PROTOCOL,
                HeaderValue::from_static("v4.1.networktables.first.wpi.edu"),
            );
            Ok(resp)
        };
        let ws = tokio_tungstenite::accept_hdr_async(tcp, callback)
            .await
            .unwrap();
        serve(ws).await;
    });
    port
}

/// The reason of the client's next `Disconnected` event, within 5 s.
async fn disconnect_reason(events: &mut nt4::Events) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Event::Disconnected { reason }) = events.recv().await {
                return reason;
            }
        }
    })
    .await
    .expect("no disconnect within 5 s")
}

fn config(port: u16) -> ClientConfig {
    let mut config = ClientConfig::server("raw", "127.0.0.1", port);
    // Don't reconnect during the test.
    config.reconnect_delay = Duration::from_secs(60);
    config
}

#[tokio::test(flavor = "multi_thread")]
async fn a_close_from_the_server_ends_the_connection_with_its_reason() {
    let port = raw_server(|mut ws| async move {
        ws.send(Message::Close(None)).await.unwrap();
        while ws.next().await.is_some() {}
    })
    .await;
    let client = Client::start(config(port));
    let mut events = client.events();
    assert_eq!(disconnect_reason(&mut events).await, "closed by server");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_socket_ends_the_connection() {
    let port = raw_server(|ws| async move { drop(ws) }).await;
    let client = Client::start(config(port));
    let mut events = client.events();
    // Dropping the socket without a close frame: either the stream ends or
    // tungstenite reports the reset.
    let reason = disconnect_reason(&mut events).await;
    assert!(!reason.is_empty());
    assert!(!client.is_connected());
}

// The pong to a server's ping is queued by tungstenite and only goes out on
// a flush; the connection loop must flush even when it has nothing else to
// send.
#[tokio::test(flavor = "multi_thread")]
async fn the_client_answers_websocket_pings() {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let port = raw_server(|mut ws| async move {
        let mut pongs = Vec::new();
        for i in 0..3u8 {
            ws.send(Message::Ping(vec![i].into())).await.unwrap();
            loop {
                match ws.next().await {
                    Some(Ok(Message::Pong(p))) => {
                        pongs.push(p[0]);
                        break;
                    }
                    Some(Ok(_)) => {}
                    _ => return,
                }
            }
        }
        let _ = tx.send(pongs);
        while ws.next().await.is_some() {}
    })
    .await;
    let _client = Client::start(config(port));
    let pongs = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("no pongs within 5 s")
        .unwrap();
    assert_eq!(pongs, [0, 1, 2]);
}

async fn server() -> Server {
    Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_does_not_answer_is_skipped_for_the_next() {
    // A port with nothing listening, then a real server: the order a team
    // number's address list relies on.
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);
    let server = server().await;
    let mut config = ClientConfig::server("fallback", "127.0.0.1", dead_port);
    config
        .servers
        .push(("127.0.0.1".into(), server.local_addr().port()));
    let client = Client::start(config);
    assert!(client.wait_for_sync(Duration::from_secs(5)).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_sync_gives_up_at_the_timeout() {
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let client = Client::start(ClientConfig::server("nobody", "127.0.0.1", port));
    let start = tokio::time::Instant::now();
    assert!(!client.wait_for_sync(Duration::from_millis(200)).await);
    assert!(start.elapsed() >= Duration::from_millis(200));
    assert_eq!(client.server_time(), None);
}

#[test]
fn team_numbers_give_wpilibs_address_list() {
    let c = ClientConfig::team("dash", 254);
    let hosts: Vec<_> = c.servers.iter().map(|(h, p)| (h.as_str(), *p)).collect();
    assert_eq!(
        hosts,
        [
            ("10.2.54.2", 5810),
            ("roborio-254-frc.local", 5810),
            ("172.22.11.2", 5810),
            ("roborio-254-frc.lan", 5810),
            ("roborio-254-frc.frc-field.local", 5810),
        ]
    );
    // One- and four-digit teams.
    assert_eq!(ClientConfig::team("d", 1).servers[0].0, "10.0.1.2");
    assert_eq!(ClientConfig::team("d", 9999).servers[0].0, "10.99.99.2");
}

#[tokio::test(flavor = "multi_thread")]
async fn client_property_changes_and_flush_reach_the_server() {
    let server = server().await;
    let mut server_events = server.events();
    let _sub = server.subscribe::<i64>("/c/slow", SubscriptionOptions::default());
    let client = Client::start(ClientConfig::server(
        "props",
        "127.0.0.1",
        server.local_addr().port(),
    ));
    assert!(client.wait_for_sync(Duration::from_secs(5)).await);
    let p = client.publish::<i64>(
        "/c/slow",
        PublishOptions {
            // Far longer than the test: the value goes out only when flushed.
            periodic: Some(10.0),
            ..Default::default()
        },
    );
    let mut update = Properties::new();
    update.insert("persistent".into(), true.into());
    client.set_properties("/c/slow", update);
    p.set(3);
    client.flush();

    let (mut saw_props, mut saw_value) = (false, false);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !(saw_props && saw_value) {
            match server_events.recv().await.unwrap() {
                ServerEvent::Properties { name, update, .. } if name == "/c/slow" => {
                    assert_eq!(update.get("persistent"), Some(&true.into()));
                    saw_props = true;
                }
                ServerEvent::Value { name, value, .. } if &*name == "/c/slow" => {
                    assert_eq!(value.value, nt4::Value::Int(3));
                    saw_value = true;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("properties and flushed value within 5 s");
}

#[tokio::test(flavor = "multi_thread")]
async fn handles_report_what_they_hold() {
    let server = server().await;
    let client = Client::start(ClientConfig::server(
        "handles",
        "127.0.0.1",
        server.local_addr().port(),
    ));
    let mut events = client.events();
    assert!(client.wait_for_sync(Duration::from_secs(5)).await);
    // Connected and time-sync events are waiting, and can be counted.
    assert!(!events.is_empty());
    let waiting = events.len();
    assert!(std::iter::from_fn(|| events.try_recv()).count() >= waiting);
    assert!(events.is_empty());

    let sub = client.subscribe::<i64>("/x", SubscriptionOptions::default());
    let generic = client.publish_generic(
        "/g",
        nt4::NtType::Int,
        Properties::new(),
        PublishOptions::default(),
    );
    // Debug output names the ids, not the backend.
    assert!(format!("{:?}", sub.handle()).starts_with("SubscriptionHandle"));
    assert!(format!("{generic:?}").starts_with("GenericPublisher"));
    assert!(format!("{client:?}").starts_with("Client"));
    assert_eq!(
        nt4::ClientStopped.to_string(),
        "the NT client or server has stopped"
    );
}
