# nt4-core

The [NetworkTables 4.1](https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc)
client and server protocols as plain state machines, with no sockets and no
clock ("sans-I/O"). Frames and the time go in; frames to send and events
come out. They run the same under tokio, in a browser (WASM), on a blocking
socket, or in a unit test with a fake clock.

Unofficial: not affiliated with or endorsed by WPILib or FIRST.

Most programs want the [`nt4`](https://docs.rs/nt4) crate, which wraps this
one with tokio and a typed API. Use `nt4-core` directly to bring your own
runtime or WebSocket.

## Client

```rust
use nt4_core::{ClientCore, ProtocolVersion, Transmit};
use nt4_proto::SubscriptionOptions;

let mut client = ClientCore::new();
client.subscribe(vec!["/SmartDashboard/".into()], SubscriptionOptions { prefix: true, ..Default::default() });

// When the WebSocket opens (time in microseconds from your own clock):
client.connect(1, ProtocolVersion::V4_1);
// Then: pass received frames to handle_text / handle_binary / handle_pong,
// call send_outgoing every tick_period_ms(), send what poll_transmit()
// returns, and read poll_event().
assert!(matches!(client.poll_transmit(), Some(Transmit::Binary(_)))); // time sync ping
```

`ClientCore` covers topic announcements, publish and subscribe, value
coalescing per publish period, server time sync, keepalive (WebSocket pings
on 4.1, RTT pings on 4.0), and sending publishers, last values and
subscriptions again after a reconnect.

## Server

```rust
use nt4_core::{ProtocolVersion, ServerCore};
use nt4_proto::{NtType, Properties, Value};

let mut server = ServerCore::new();
let p = server.publish("/robot/mode", NtType::String, Properties::new());
server.set_value(p, Value::String("auto".into()), 1).unwrap();

// When a client connects (name from its /nt/<name> URL):
let (id, name) = server.add_client("dashboard", "10.0.0.5:51234", false, ProtocolVersion::V4_1);
assert_eq!(name, "dashboard@1");
// Then: pass its frames to handle_text / handle_binary / handle_pong, call
// send_outgoing(id, ..) every tick_period_ms(id), send what
// poll_transmit(id) returns, and call remove_client when it disconnects.
```

`ServerCore` covers retained last values, per-client send queues at each
subscription's period, `sendAll` and topics-only subscriptions, RTT replies
and WebSocket keepalive, the `$` topic rules, and the meta topics
(`$clients`, `$pub$<topic>`, `$sub$<topic>`, `$clientpub$<client>`, ...).
The server's own code publishes and subscribes through it too.

## Conformance

The behavior follows WPILib's ntcore, and the tests port ntcore's
`ServerImplTest`, `NetworkOutgoingQueueTest` and `TimeSyncTest`.

Example (advanced): `cargo run -p nt4-core --example custom_driver -- 127.0.0.1 /`
drives the client core by hand, to show what a runtime has to do. For normal
use, see the `nt4` crate's examples.

License: MIT. Ported WPILib tests keep WPILib's BSD license
(`tests/LICENSE-WPILIB.md`).
