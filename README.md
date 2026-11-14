# nt4-rs

A NetworkTables 4.1 library for Rust, with both a client and a server, written
for FRC Team 3200 (Raptacon) and its [raptaboard](https://github.com/tuffinmuffin/raptaboard)
dashboard.

NetworkTables (NT) is how FRC robot code, dashboards and coprocessors share
values. NT4 runs over a WebSocket: JSON text frames for control messages,
MessagePack binary frames for values. The protocol is described in
[ntcore's networktables4.adoc](https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc).

## Status

Milestone 1 of 6: the wire format.

| Crate | What it does | Status |
| --- | --- | --- |
| `nt4-proto` | JSON and MessagePack encode/decode, value and type definitions. No I/O. | Done |
| `nt4-core` | Client and server state machines with no sockets (sans-I/O) | Milestones 2 and 4 |
| `nt4` | tokio runtime: `client` and `server` features | Milestones 3 and 5 |

The full plan, including how conformance is tested, is in the
[project plan doc](https://claude.ai/artifact/R1JvYBZZYPm6HuUuFM9QPh).

## Conformance testing

The tests are checked against WPILib's own ntcore, in three layers:

1. **Ported ntcore tests.** `crates/nt4-proto/tests/wire_encoder.rs` and
   `wire_decoder.rs` port every case of ntcore's `WireEncoderTest` and
   `WireDecoderTest`, with the same bytes, JSON and warning text. Each test
   names the upstream case it ports in an `// upstream: "..."` comment.
2. **Upstream audit.** `scripts/upstream_audit.py <allwpilib checkout>` lists
   upstream cases that aren't ported yet. CI runs it against the pinned
   allwpilib commit (blocking) and against allwpilib `main` (warning only).
3. **Interop with real ntcore** (milestone 3 onward): CI runs our client and
   server against ntcore from the `pyntcore` Python package.

## Development

```bash
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT, see [LICENSE](LICENSE). Test vectors ported from allwpilib stay under
WPILib's BSD license; see
[crates/nt4-proto/tests/LICENSE-WPILIB.md](crates/nt4-proto/tests/LICENSE-WPILIB.md).
