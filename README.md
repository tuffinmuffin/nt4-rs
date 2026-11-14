# nt4-rs

A NetworkTables 4.1 library for Rust, with both a client and a server, so Rust
programs can talk to WPILib robot code, dashboards and coprocessors.

NetworkTables (NT) is how FRC robot code, dashboards and coprocessors share
values. NT4 runs over a WebSocket: JSON text frames for control messages,
MessagePack binary frames for values. The protocol is described in
[ntcore's networktables4.adoc](https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc).

## Status

Milestone 2 of 6: the wire format and the client core.

| Crate | What it does | Status |
| --- | --- | --- |
| `nt4-proto` | JSON and MessagePack encode/decode, value and type definitions. No I/O. | Done |
| `nt4-core` | Client and server state machines with no sockets (sans-I/O) | Client done; server in milestone 4 |
| `nt4` | tokio runtime: `client` and `server` features | Milestones 3 and 5 |

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
3. **Interop with real ntcore** (milestone 3 onward): CI runs our client and
   server against ntcore from the `pyntcore` Python package.

## Using the client core

`nt4-core`'s `ClientCore` is the NT4 client with no socket and no clock: you
open the WebSocket, feed it frames and the time, and send the frames it
returns. It handles announcements, publish and subscribe, time sync,
keepalive, and sends publishers, last values and subscriptions again after a
reconnect. The tokio client in milestone 3 wraps it. See the crate docs
(`cargo doc -p nt4-core --open`) for an example.

## Development

```bash
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT, see [LICENSE](LICENSE). Test vectors ported from allwpilib stay under
WPILib's BSD license; see
`LICENSE-WPILIB.md` in each crate's `tests/` folder.
