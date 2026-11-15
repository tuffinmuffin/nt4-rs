# nt4

A [NetworkTables 4.1](https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc)
client for Rust and tokio. It talks to FRC robot code, the simulator, and
anything else that runs WPILib's ntcore, the same way AdvantageScope or
Elastic do.

- Connects by team number or address, keeps the connection alive, and
  reconnects on its own
- Typed publishers and subscribers for every NT4 value type
- Server clock sync, so timestamps line up with the robot's
- Publishers and subscriptions work before the first connection and come
  back after every reconnect
- Tested in CI against a real ntcore server (pyntcore) on Linux, Windows and
  macOS

## Quick start

```toml
[dependencies]
nt4 = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time"] }
```

```rust,no_run
use nt4::{Client, ClientConfig};

#[tokio::main]
async fn main() {
    // Tries 10.32.0.2, roborio-3200-frc.local and the USB address.
    let client = Client::start(ClientConfig::team("my-dashboard", 3200));

    let mut speed = client.subscribe::<f64>("/SmartDashboard/speed", Default::default());
    let heartbeat = client.publish::<i64>("/dashboard/heartbeat", Default::default());

    let mut beats = 0;
    while let Ok(value) = speed.changed().await {
        beats += 1;
        heartbeat.set(beats);
        println!("speed = {value:?}");
    }
}
```

For the simulator or a local server, use
`ClientConfig::server("my-dashboard", "127.0.0.1", 5810)`.

## Types

| Rust | NT4 type |
| --- | --- |
| `bool`, `f64`, `i64`, `f32`, `String` | `boolean`, `double`, `int`, `float`, `string` |
| `Vec<u8>` | `raw` |
| `Vec<bool>`, `Vec<f64>`, `Vec<i64>`, `Vec<f32>`, `Vec<String>` | `boolean[]`, `double[]`, `int[]`, `float[]`, `string[]` |

Structs, protobuf and other raw types use `Client::publish_generic` with an
`NtType` such as `NtType::Struct { name: "Pose2d".into(), array: false }`;
see the `struct_topic` example.

To watch many topics at once, `Client::subscribe_multiple` with
`SubscriptionOptions { prefix: true, .. }` and read `Client::events()`, which
reports every announcement, value, connection change and time sync update.

## Examples

Start a server first (robot code in simulation, or
`python -c "import ntcore,time; i=ntcore.NetworkTableInstance.getDefault(); i.startServer(); time.sleep(1e9)"`
with `pip install pyntcore`), then:

| Example | What it does |
| --- | --- |
| `cargo run -p nt4 --example sync_loop -- 127.0.0.1` | uses the client from a plain (non-async) 50 Hz loop, the way robot code uses ntcore |
| `cargo run -p nt4 --example watch -- 127.0.0.1 /` | prints every topic and value under a prefix; pass a team number instead of a host to find a robot |
| `cargo run -p nt4 --example publish -- 127.0.0.1` | publishes a counter and a sine wave, and reads a gain other clients can set |
| `cargo run -p nt4 --example struct_topic -- 127.0.0.1` | publishes a `Translation2d` struct with its schema |

## How it fits together

| Crate | Job |
| --- | --- |
| [`nt4-proto`](https://docs.rs/nt4-proto) | wire format: JSON and MessagePack encode and decode, no I/O |
| [`nt4-core`](https://docs.rs/nt4-core) | the protocol as a state machine with no sockets or clock (works in WASM) |
| `nt4` | this crate: tokio, WebSocket, clock, typed API |

A background task owns the socket and keeps a local copy of each
subscribed topic, so `Subscriber::get` and `Publisher::set` are plain,
non-blocking calls that never touch the network, as in ntcore for Java and
C++. Only starting the client needs a tokio runtime; the `sync_loop` example
shows a normal non-async program.

To run the protocol on another runtime, use `nt4-core` directly (its
`custom_driver` example shows the loop).

A server (`server` feature) is planned.

## Compatibility

NT 4.1, with fallback to 4.0. Tested against ntcore 2027 (pyntcore 2027.0.0a8);
ntcore 2026 and older are not tracked. Minimum Rust
version 1.95.

## License

MIT. Tests ported from WPILib's allwpilib stay under WPILib's BSD license;
see `tests/LICENSE-WPILIB.md` in the repository.
