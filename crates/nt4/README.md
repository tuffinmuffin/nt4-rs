# nt4

A [NetworkTables 4.1](https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc)
client and server for Rust and tokio. The client talks to FRC robot code,
the simulator, and anything else that runs WPILib's ntcore, the same way
AdvantageScope or Elastic do. The server (`server` feature) is what those
connect to, as robot code's ntcore server is.

Unofficial: not affiliated with or endorsed by WPILib or FIRST.

- Connects by team number or address, keeps the connection alive, and
  reconnects on its own
- Typed publishers and subscribers for every NT4 value type
- Server clock sync, so timestamps line up with the robot's
- Publishers and subscriptions work before the first connection and come
  back after every reconnect
- A server that ntcore clients connect to, with retained values, meta
  topics and the same typed handles for its own topics
- Tested in CI against real ntcore (pyntcore), as both client and server, on
  Linux, Windows and macOS

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

## Server

```toml
nt4 = { version = "0.1", features = ["server"] }
```

```rust,ignore
use nt4::{Server, ServerConfig};

let server = Server::start(ServerConfig::default()).await?; // port 5810
let mode = server.publish::<String>("/robot/mode");
let speed = server.subscribe::<f64>("/dashboard/speed", Default::default());
mode.set("teleop".into());
println!("{:?} from {} clients", speed.get(), server.clients().len());
```

Clients on the same machine (`127.x`) get every value right away, others
get them batched per subscription period, as with ntcore 2027. Set
`ServerConfig::persist_file` (robot code uses `networktables.json`) to keep
topics with the `persistent` property across restarts, in ntcore's file
format.
`Server::events()` reports what reaches the server's own subscriptions, and
warnings about what clients send.

## Types

| Rust | NT4 type |
| --- | --- |
| `bool`, `f64`, `i64`, `f32`, `String` | `boolean`, `double`, `int`, `float`, `string` |
| `Vec<u8>` | `raw` |
| `Vec<bool>`, `Vec<f64>`, `Vec<i64>`, `Vec<f32>`, `Vec<String>` | `boolean[]`, `double[]`, `int[]`, `float[]`, `string[]` |

Structs are typed too. The [`nt4-wpimath`](https://docs.rs/nt4-wpimath)
crate has WPILib's geometry and kinematics types (`Pose2d`, `Pose3d`,
`SwerveModuleVelocity`, ...), and your own types implement `NtStruct` (see
`nt4::structs`). Publishing one also publishes its schema under
`/.schema/`, as ntcore does, so AdvantageScope and Elastic can decode it:

```toml
[dependencies]
nt4 = "0.1"
nt4-wpimath = "0.1"
```

```rust,ignore
use nt4_wpimath::{Pose2d, SwerveModuleVelocity};
let pose = client.publish::<Pose2d>("/vision/pose", Default::default());
let modules = client.publish::<Vec<SwerveModuleVelocity>>("/drive/modules", Default::default());
pose.set(Pose2d::default());
```

Protobuf messages implement `NtProtobuf` and publish as `Proto<T>`. To
decode structs whose type you only learn at run time (a dashboard), read the
`/.schema/` topics into `nt4::proto::structs::StructDatabase`. Other raw
types use `publish_generic` with any `NtType`.

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
| `cargo run -p nt4-wpimath --example struct_topic -- 127.0.0.1` | publishes a `Pose2d` and swerve module states, with their schemas (in the `nt4-wpimath` crate) |
| `cargo run -p nt4 --features server --example server` | runs a server on 5810 with a few topics; point AdvantageScope, Elastic or the other examples at it |

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

## Compatibility

NT 4.1, with fallback to 4.0, and the RTT subprotocol on the server.
Tested against ntcore 2027 (pyntcore 2027.0.0a8);
ntcore 2026 and older are not tracked. Minimum Rust
version 1.95.

## License

MIT. Tests ported from WPILib's allwpilib stay under WPILib's BSD license;
see `tests/LICENSE-WPILIB.md` in the repository.
