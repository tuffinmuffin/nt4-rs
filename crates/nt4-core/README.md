# nt4-core

The [NetworkTables 4.1](https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc)
client protocol as a plain state machine, with no sockets and no clock
("sans-I/O"). Frames and the time go in; frames to send and events come out.
It runs the same under tokio, in a browser (WASM), on a blocking socket, or
in a unit test with a fake clock.

Most programs want the [`nt4`](https://docs.rs/nt4) crate, which wraps this
one with tokio and a typed API. Use `nt4-core` directly to bring your own
runtime or WebSocket.

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

What it covers: topic announcements, publish and subscribe, value
coalescing per publish period, server time sync, keepalive (WebSocket pings
on 4.1, RTT pings on 4.0), and sending publishers, last values and
subscriptions again after a reconnect.

Its behavior follows WPILib's ntcore, and its tests port ntcore's
`NetworkOutgoingQueueTest` and `TimeSyncTest`.

Example: `cargo run -p nt4-core --example blocking_client -- 127.0.0.1 /`.

License: MIT. Ported WPILib tests keep WPILib's BSD license
(`tests/LICENSE-WPILIB.md`).
