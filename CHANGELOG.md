# Changelog

The library crates (`nt4-proto`, `nt4-core`, `nt4-derive`, `nt4`,
`nt4-wpimath`) share one version number. Pushing the tag `v<version>` publishes them; see "Releasing"
in README.md.

## 0.2.0 (2026-11-16)

- `#[derive(NtStruct)]`, in the new `nt4-derive` crate: turn on `nt4`'s
  `derive` feature and use `nt4::structs::NtStruct`. For a struct with
  named fields it builds the schema and the nested schemas from the field
  types, declared as wpimath's C++ schemas declare them, so they can't
  drift from the layout. A fieldless enum becomes a one-byte struct,
  `enum {A=0,B=1} int8 variant`, as WPILib's Java `StructGenerator` writes
  one. Options on the type: `#[nt4(name = "..")]`,
  `#[nt4(schema = "..")]`, `#[nt4(nested(A, ..))]` (the nested structs, in
  place of the fields', as `DifferentialSample` registers
  `HolonomicSample`) and `#[nt4(crate = "..")]`; on a field,
  `#[nt4(rename = "..")]` (its schema name, such as WPILib's camelCase
  `xWidth`).
- `StructField` has two new methods with defaults (not a breaking change):
  `schema_field(name)` gives a field's schema declaration (`double x`,
  `int16 raw[4]`, `Rotation2d angle`), and `for_each_field_schema` the
  schemas of the struct in a field. Using a type that isn't a
  `StructField` as a field now says so in the compiler error.
- `NtValue::properties()` (default: none) gives the properties a typed
  topic is published with; `publish` uses it.
- `nt4-wpimath`: new types `SwerveDriveKinematics<N>`
  (`struct:SwerveDriveKinematics__N`, the module positions, generic over
  the number of modules), `ChassisAccelerations`,
  `SwerveModuleAcceleration` (struct `SwerveModuleAccelerations`, as
  WPILib names it), differential and mecanum wheel positions, velocities
  and accelerations, `HolonomicSample` and `DifferentialSample`,
  `Rectangle2d` and `Ellipse2d`. The types are in modules by group
  (`geometry`, `kinematics`, `trajectory`, `shape`), re-exported at the
  crate root.
- `nt4-wpimath`'s types are built with the derive; the existing ones'
  schemas, sizes and nested schema order are unchanged. CI checks every
  type against WPILib's own struct definitions (C++, Java and RobotPy) at
  a pinned allwpilib commit, and a weekly job checks each newer allwpilib
  release. Types WPILib has that aren't here are listed, with reasons, in
  `scripts/schema_audit.txt`; the crate README says how to read or derive
  them, and how to ask for one.
- In the repository, not published yet: `nt4-photon`, PhotonVision
  pipeline results (`PhotonPipelineResult`, its targets, PnP and
  multi-tag results) as typed topics, in PhotonVision 2027's photonstruct
  format (hash `fd353b11...`). It will be published once PhotonVision 2027
  is released.

## 0.1.0 (2026-11-16)

First release.

- `nt4-proto`: NT 4.1 wire format. JSON control messages and MessagePack
  value frames, matching ntcore byte for byte; all of ntcore's
  `WireEncoderTest` and `WireDecoderTest` cases ported.
- `nt4-core`: sans-I/O `ClientCore` (announcements, publish and subscribe,
  time sync, keepalive, reconnect replay), `TimeSync`, and `OutgoingQueue`
  (port of ntcore's `NetworkOutgoingQueue`); ntcore's
  `NetworkOutgoingQueueTest` and the client cases of `TimeSyncTest` ported.
- `nt4`: tokio `Client` with typed `Publisher<T>` / `Subscriber<T>`,
  team-number server lookup, and interop tests against pyntcore 2027.0.0a8.
  `Client::events()` is lossless (an unbounded queue per reader, like
  ntcore's listener queues). `Client::topic` and `Client::topics` read the
  cached topics and values.
- Load benchmark against ntcore in `bench/` (see `bench/README.md`). Fixes
  it led to: queued messages go out as soon as time sync completes, the send
  tick is at most 100 ms, local publish and subscribe changes are sent at
  once, `TCP_NODELAY` is on, and `nt4-proto` decodes MessagePack directly
  (no rmpv at run time).
- Differential fuzzer in `crates/nt4-fuzz/` (see `crates/nt4-fuzz/README.md`).
  Generates structure-aware frames, runs nt4-proto and ntcore's own decoders
  on the same bytes, and compares. Found and fixed: invalid UTF-8 in string
  values was rejected (ntcore accepts them, using replacement characters);
  `-0` integer literals were parsed as `-0.0` (ntcore treats them as 0); JSON
  numbers are now parsed exactly (serde_json's `float_roundtrip`), as ntcore
  does; JSON nesting stops at 20 levels and the first of a repeated key wins,
  as in ntcore; a binary timestamp that doesn't fit in nanoseconds is
  rejected, as in ntcore. It also found an ntcore parser bug (a `null` member
  after a repeated key takes the repeated key's value), which nt4 does not
  copy.
- `nt4-wpimath`: WPILib's geometry and kinematics struct types, split out
  of `nt4` (where they were `nt4::wpimath`) so `nt4` stays a general
  NetworkTables library and the types can follow WPILib's yearly renames.
  `NtStruct::type_name()` and `schema()` are functions (they were the
  constants `TYPE_NAME` and `SCHEMA`), so generic struct types can be
  written.
- Publishing a value costs about half what it did (17 against 33 ns per
  value in a steady loop), and relaying one through the server about 40%
  less: no per-value copy of the topic name, value or encode buffer, no
  per-value target list on the server, and a cheaper hash for queue ids.
  `nt4_proto::encode_binary_parts` encodes a value message from borrowed
  parts.
- Receiving costs a quarter less CPU when frames are small and many, as on
  ntcore's unbatched localhost path (nt4-native 36.5% against 27.8% of a
  core at 157,000 frames/s, on a 2-core arm64 Linux VM; ntcore 23.2%).
  tungstenite now reads 8 KiB at a time instead of zero-filling 128 KiB
  before every read, and the client and server connection loops handle all
  frames already read under one lock, keep one timer per connection instead
  of one per frame, and flush only when they sent something.
- Two ordering bugs nt4 had copied from ntcore are fixed (found while
  raising test coverage; both in `docs/UPSTREAM_BUGS.md`): a client's
  `set_properties` right after `publish` could reach the server first and
  be ignored, when the publisher's period isn't the default; and a
  `flush()` within 5 ms of the previous send was dropped, so the value
  waited for its publisher's period.
- Minimum Rust version 1.85 (the first with edition 2024, and what the
  dependencies need), checked in CI.
- ntcore 2027 only: the interop tests, the fuzzer oracle and the benchmark
  use pyntcore 2027.0.0a8. ntcore 2026 behavior is not tracked.
