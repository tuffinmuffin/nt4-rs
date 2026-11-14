# CLAUDE.md

Guidance for Claude Code working in this repository.

## Project

nt4-rs is a general-purpose NetworkTables 4.1 (NT4) client and server library
in Rust, filling the gap for Rust programs that talk to WPILib. Keep it
independent of any one app. The plan and milestones are in the project plan doc
linked from README.md.

Protocol reference: allwpilib `ntcore/doc/networktables4.adoc`. Where the spec
is vague, match ntcore's behavior (allwpilib `ntcore/src/main/native/cpp/`).

## Layout

- `crates/nt4-proto`: wire format only (JSON text frames, MessagePack binary
  frames, `NtType`, `Value`). No I/O, no async, no tokio, so it stays usable
  from WASM.
- `crates/nt4-core`: sans-I/O state machines, same no-I/O rule (CI builds it
  for `wasm32-unknown-unknown`). `ClientCore` (client), `TimeSync`, and
  `OutgoingQueue` (ntcore's `NetworkOutgoingQueue`, shared with the server
  core later). Callers pass the time in microseconds; nothing reads a clock.
- `crates/nt4`: tokio runtime. `client` feature: `Client` (connect,
  reconnect, frame pump around `ClientCore`) and typed `Publisher<T>` /
  `Subscriber<T>` via `NtValue`. Interop tests in `tests/interop.rs` drive a
  real ntcore through `tests/interop/ntcore_peer.py` (JSON lines on stdin);
  they skip unless `NT4_INTEROP_PYTHON` points at a Python with pyntcore.
- Planned: `ServerCore` in `nt4-core` and the `server` feature in `nt4`.

## Rules

- Where ntcore has a bug, don't copy it: do the sensible thing and say so in
  a comment (e.g. the NT 4.0 RTT keepalive in `client.rs`).
- Encoders must match ntcore byte for byte; decoders accept and reject the
  same input as ntcore, with ntcore's warning text.
- Timestamps are integer microseconds, as on the wire. (ntcore 2027 uses
  nanoseconds internally; don't copy that.)
- Ported ntcore tests: keep the header naming the source file and allwpilib
  commit, the WPILib copyright line and the pointer to
  `tests/LICENSE-WPILIB.md`. Put an `// upstream: "<Catch2 name>"` marker on
  each test; list cases that can't apply in `scripts/upstream_na.txt` with the
  reason. When bumping `ALLWPILIB_COMMIT` in CI, update the headers too.
- Tests written by Claude say so in a comment.
- Public items need docs (`missing_docs` is on); add a `///` example to
  anything users call directly.

## Commands

```bash
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/upstream_audit.py <allwpilib checkout>
NT4_INTEROP_PYTHON=<python with pyntcore> cargo test -p nt4 --test interop
```

ntcore facts learned from interop: the server always renames a client to
`<name>@<n>`.
