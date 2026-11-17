# Changelog

All three crates (`nt4-proto`, `nt4-core`, `nt4`) share one version number.

## Unreleased (0.1.0)

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
- ntcore 2027 only: the interop tests, the fuzzer oracle and the benchmark
  use pyntcore 2027.0.0a8. ntcore 2026 behavior is not tracked.
