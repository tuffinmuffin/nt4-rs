# nt4-rs

A NetworkTables 4.1 library for Rust, with both a client and a server, so Rust
programs can talk to WPILib robot code, dashboards and coprocessors.

NetworkTables (NT) is how FRC robot code, dashboards and coprocessors share
values. NT4 runs over a WebSocket: JSON text frames for control messages,
MessagePack binary frames for values. The protocol is described in
[ntcore's networktables4.adoc](https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc).

## Status

Milestone 3 of 6: the client is usable. The server comes next. Current state
and next steps are in [docs/STATUS.md](docs/STATUS.md).

| Crate | What it does | Status |
| --- | --- | --- |
| `nt4-proto` | JSON and MessagePack encode/decode, value and type definitions. No I/O. | Done |
| `nt4-core` | Client and server state machines with no sockets (sans-I/O) | Client done; server in milestone 4 |
| `nt4` | tokio runtime: `client` and `server` features | Client done; server in milestone 5 |

The full plan, including how conformance is tested, is in the
[project plan doc](https://claude.ai/artifact/R1JvYBZZYPm6HuUuFM9QPh).

## Conformance testing

The tests are checked against WPILib's own ntcore, in three layers:

1. **Ported ntcore tests.** `crates/nt4-proto/tests/wire_encoder.rs` and
   `wire_decoder.rs` port every case of ntcore's `WireEncoderTest` and
   `WireDecoderTest`, with the same bytes, JSON and warning text.
   `crates/nt4-core/tests/` ports `NetworkOutgoingQueueTest` and the client
   cases of `TimeSyncTest`. Each test names the upstream case it ports in an
   `// upstream: "..."` comment.
2. **Upstream audit.** `scripts/upstream_audit.py <allwpilib checkout>` lists
   upstream cases that aren't ported yet. CI runs it against the pinned
   allwpilib commit (blocking) and against allwpilib `main` (warning only).
3. **Interop with real ntcore.** `crates/nt4/tests/interop.rs` runs our
   client against an ntcore server from the `pyntcore` Python package
   (connect, time sync, every value type both ways, prefix subscriptions,
   reconnect after a server restart). CI runs it on Linux, Windows and macOS.
   Locally it needs `NT4_INTEROP_PYTHON` pointing at a Python with
   `pyntcore` installed, and it skips without it. From milestone 5, CI also
   runs ntcore clients against our server.

## Performance

`bench/` runs nt4 and ntcore against a simulated robot (200 topics: one-time
values, values set every 20 ms, and a busy loop), up to 1,640 topics and
155,000 values/s. No run lost or reordered an update. With no per-value code
in either, nt4 used less CPU than C++ ntcore (3.8% against 4.7% of a core at
1,640 topics) and about 5 MiB of memory. See [bench/README.md](bench/README.md)
for the setup and full results.

## Using the client

```toml
[dependencies]
nt4 = { git = "https://github.com/tuffinmuffin/nt4-rs" }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time"] }
```

```rust,no_run
use nt4::{Client, ClientConfig};

#[tokio::main]
async fn main() {
    // Tries 10.32.0.2, the mDNS names and USB, like ntcore's SetServerTeam.
    let client = Client::start(ClientConfig::team("my-dashboard", 3200));
    let mut speed = client.subscribe::<f64>("/SmartDashboard/speed", Default::default());
    let heartbeat = client.publish::<i64>("/dashboard/heartbeat", Default::default());
    let mut beats = 0;
    while let Ok(v) = speed.changed().await {
        beats += 1;
        heartbeat.set(beats);
        println!("speed = {v:?}");
    }
}
```

Publishers and subscribers work before the first connection and survive
reconnects; drop one to unpublish or unsubscribe. `client.events()` gives
every announcement, value and time sync change.

Under the hood, `nt4-core`'s `ClientCore` is the protocol with no socket and
no clock, so it can be driven from any runtime or from WASM.

## Releasing

The three crates share one version (workspace `version`) and are published
together:

1. Update `CHANGELOG.md`, and bump `version` in the root `Cargo.toml` and on
   each `nt4-proto` / `nt4-core` path dependency.
2. `cargo publish --workspace --dry-run`, then `cargo publish --workspace`
   (Cargo publishes `nt4-proto`, `nt4-core`, `nt4` in dependency order).
3. Tag the release `v<version>`.

CI runs the dry run on every push, so packaging problems show up early.

## Development

```bash
cargo test --workspace
# interop tests (optional): python -m pip install pyntcore==2026.2.2
NT4_INTEROP_PYTHON=$(which python) cargo test -p nt4 --test interop
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT, see [LICENSE](LICENSE). Test vectors ported from allwpilib stay under
WPILib's BSD license; see
`LICENSE-WPILIB.md` in each crate's `tests/` folder.
