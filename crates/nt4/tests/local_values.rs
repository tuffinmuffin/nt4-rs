//! A client (or the server's own code) sees the values it writes, as in
//! ntcore, where a local publisher's value is the topic's value for local
//! subscribers too.
//!
//! Written by Claude (not ported from ntcore).

use std::time::Duration;

use nt4::{Client, ClientConfig, Server, ServerConfig};

async fn server() -> Server {
    Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap()
}

/// Wait up to 5 s for `done`.
async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// The dashboard case: robot code (here the server) publishes a boolean, a
// dashboard client subscribes to it, publishes it too and flips it.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_sees_its_own_writes() {
    let server = server().await;
    let robot = server.publish::<bool>("/robot/enabled");
    robot.set(false);
    let seen_by_server = server.subscribe::<bool>("/robot/enabled", Default::default());

    let client = Client::start(ClientConfig::server(
        "dash",
        "127.0.0.1",
        server.local_addr().port(),
    ));
    let sub = client.subscribe::<bool>("/robot/enabled", Default::default());
    eventually("the robot's value", || sub.get() == Some(false)).await;

    let publisher = client.publish::<bool>("/robot/enabled", Default::default());
    publisher.set(true);
    // The write reaches the server...
    eventually("the server gets true", || {
        seen_by_server.get() == Some(true)
    })
    .await;
    // ...and the writing client's own subscriber and topic list show it.
    eventually("the client's subscriber shows true", || {
        sub.get() == Some(true)
    })
    .await;
    let topic = client.topic("/robot/enabled").expect("announced");
    assert_eq!(
        topic.value.map(|v| v.value),
        Some(nt4::Value::Boolean(true))
    );
}

// The same for the server's own code: its publisher's value is what its own
// subscriber sees.
#[tokio::test(flavor = "multi_thread")]
async fn the_server_sees_its_own_writes() {
    let server = server().await;
    let sub = server.subscribe::<f64>("/local/speed", Default::default());
    let publisher = server.publish::<f64>("/local/speed");
    publisher.set(1.5);
    eventually("the server's subscriber shows 1.5", || {
        sub.get() == Some(1.5)
    })
    .await;
}

// A subscriber made before connecting sees a value written before
// connecting (ntcore's topics exist locally, server or not).
#[tokio::test(flavor = "multi_thread")]
async fn writes_show_before_any_server() {
    // Port 1 has no server.
    let client = Client::start(ClientConfig::server("dash", "127.0.0.1", 1));
    let sub = client.subscribe::<bool>("/offline", Default::default());
    let publisher = client.publish::<bool>("/offline", Default::default());
    publisher.set(true);
    eventually("the subscriber shows true", || sub.get() == Some(true)).await;
}

// On disconnect ntcore resets topics that no longer exist (ClearNetwork,
// then ResetIfDoesNotExist): a subscriber to the robot's topic goes back to
// no value, while a topic this client publishes keeps its value.
#[tokio::test(flavor = "multi_thread")]
async fn a_disconnect_forgets_other_publishers_values() {
    let server = server().await;
    let robot = server.publish::<f64>("/robot/speed");
    robot.set(2.0);
    let client = Client::start(ClientConfig::server(
        "dash",
        "127.0.0.1",
        server.local_addr().port(),
    ));
    let theirs = client.subscribe::<f64>("/robot/speed", Default::default());
    let ours = client.subscribe::<f64>("/dash/target", Default::default());
    let publisher = client.publish::<f64>("/dash/target", Default::default());
    publisher.set(5.0);
    eventually("the robot's value", || theirs.get() == Some(2.0)).await;
    drop(robot);
    drop(server);
    eventually("disconnected", || !client.is_connected()).await;
    eventually("the robot's value is forgotten", || theirs.get().is_none()).await;
    assert_eq!(ours.get(), Some(5.0));
}

// ntcore's value listeners (NT_EVENT_VALUE_ALL) hear local writes too,
// flagged as local. nt4's events carry no such flag yet, so this checks
// only that a write is heard at all.
#[tokio::test(flavor = "multi_thread")]
async fn events_include_our_own_writes() {
    let server = server().await;
    let client = Client::start(ClientConfig::server(
        "dash",
        "127.0.0.1",
        server.local_addr().port(),
    ));
    let mut events = client.events();
    let _sub = client.subscribe::<f64>("/dash/target", Default::default());
    let publisher = client.publish::<f64>("/dash/target", Default::default());
    eventually("connected", || client.is_connected()).await;
    publisher.set(7.0);
    let heard = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = events.recv().await {
            if let nt4::Event::Value { name, value, .. } = event {
                if &*name == "/dash/target" && value.value == nt4::Value::Double(7.0) {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(heard, "no event for our own write");
}

// When a topic is deleted (its last publisher gone), the server's own
// subscriber goes back to no value, as ntcore's does.
#[tokio::test(flavor = "multi_thread")]
async fn the_servers_subscriber_forgets_a_deleted_topic() {
    let server = server().await;
    let sub = server.subscribe::<f64>("/dash/level", Default::default());
    let client = Client::start(ClientConfig::server(
        "dash",
        "127.0.0.1",
        server.local_addr().port(),
    ));
    let publisher = client.publish::<f64>("/dash/level", Default::default());
    publisher.set(4.0);
    eventually("the server gets 4.0", || sub.get() == Some(4.0)).await;
    drop(publisher);
    eventually("the server forgets it", || sub.get().is_none()).await;
}

// A server started after another one stopped has a later clock, as
// ntcore's does when robot code restarts: ntcore clients resend their last
// values stamped with the old server's time, so a clock that went back to
// 0 would make their later values look older and be ignored.
#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_servers_clock_carries_on() {
    let first = server().await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let last = first.now_us();
    drop(first);
    let second = server().await;
    assert!(second.now_us() > last, "{} <= {last}", second.now_us());
}
