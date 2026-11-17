# CLAUDE.md

Guidance for Claude Code working in this repository.

## Project

nt4-rs is a general-purpose NetworkTables 4.1 (NT4) client and server library
in Rust, filling the gap for Rust programs that talk to WPILib. Keep it
independent of any one app. The plan and milestones are in the project plan doc
linked from README.md.

Protocol reference: allwpilib `ntcore/doc/networktables4.adoc`. Where the spec
is vague, match ntcore's behavior (allwpilib `ntcore/src/main/native/cpp/`).
"ntcore" means ntcore 2027 and later (interop tests, fuzzer oracle and
benchmark all use pyntcore 2027.0.0a8); ntcore 2026 behavior is not tracked.

## Layout

- `crates/nt4-proto`: wire format only (JSON text frames, MessagePack binary
  frames, `NtType`, `Value`). No I/O, no async, no tokio, so it stays usable
  from WASM.
- `crates/nt4-core`: sans-I/O state machines, same no-I/O rule (CI builds it
  for `wasm32-unknown-unknown`). `ClientCore` (client), `ServerCore`
  (ntcore's `ServerImpl`, with the server's own client as slot 0),
  `TimeSync`, and `OutgoingQueue` (ntcore's `NetworkOutgoingQueue`, used by
  both). Callers pass the time in microseconds; nothing reads a clock.
- `crates/nt4`: tokio runtime. `client` feature: `Client` (connect,
  reconnect, frame pump around `ClientCore`) and typed `Publisher<T>` /
  `Subscriber<T>` via `NtValue`. Interop tests in `tests/interop.rs` drive a
  real ntcore through `tests/interop/ntcore_peer.py` (JSON lines on stdin);
  they skip unless `NT4_INTEROP_PYTHON` points at a Python with pyntcore.
- `nt4`'s `server` feature: `Server` (listen, handshake, one task per
  connection around a shared `ServerCore`, persistent file). The handle
  types (`Publisher<T>`, `Subscriber<T>`, `Events`) live in `handles.rs` and
  serve both runtimes. Server interop tests are `tests/interop_server.rs`
  (pyntcore as the client). Build and test with `--all-features`.
- Structs: `nt4_proto::structs` parses schemas and decodes values at run
  time (port of wpiutil's `SchemaParser` / `DynamicStruct`; tests in
  `nt4-proto/tests/struct_schema.rs` and `dynamic_struct.rs`). `nt4::structs`
  has the typed `NtStruct` / `NtProtobuf` traits and schema registration.
- `crates/nt4-derive`: `#[derive(NtStruct)]` (proc macro), re-exported as
  `nt4::structs::NtStruct` behind `nt4`'s `derive` feature. The generated
  code calls helpers in `nt4::structs::__private`. Tests:
  `nt4/tests/derive.rs`, plus the error cases in `nt4-derive`'s own unit
  tests.
- `crates/nt4-wpimath`: the WPILib types, built only on `nt4`'s public
  traits. Their schemas must match wpimath's `*Struct.hpp` exactly;
  `scripts/schema_audit.py` checks that, and every new type goes in
  `examples/audit_schemas.rs` (and off `scripts/schema_audit.txt`). nt4's
  own tests use the copies in `nt4/tests/support/wpi_types.rs` instead
  (nt4 can't depend on nt4-wpimath).

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
- The published crates support Rust 1.85 (`rust-version`, checked by the CI
  `msrv` job): no let chains (`if let .. && let ..`, 1.88) or newer std
  APIs. Clippy's `incompatible_msrv` flags the APIs. The dev tools
  (`nt4-fuzz`, `nt4-perf`) have their own, newer `rust-version`.

## Commands

```bash
cargo test --workspace --all-features
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
python3 scripts/upstream_audit.py <allwpilib checkout>
python3 scripts/schema_audit.py <allwpilib checkout> [--robotpy <python with robotpy-wpimath, robotpy-hal>]   # nt4-wpimath against WPILib's schemas
NT4_INTEROP_PYTHON=<python with pyntcore> cargo test -p nt4 --all-features --test interop --test interop_server
# coverage, as the CI `coverage` job measures it (needs cargo-llvm-cov)
cargo llvm-cov -p nt4-proto -p nt4-core -p nt4-derive -p nt4 -p nt4-wpimath --all-features --ignore-filename-regex '(tests|examples|benches)/' --html
```

ntcore facts learned from interop: the server always renames a client to
`<name>@<n>`. pyntcore 2027 uses snake_case names, has no NT3 port, and
reports times in nanoseconds.
