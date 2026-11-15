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
- ntcore 2027 only: the interop tests, the fuzzer oracle and the benchmark
  use pyntcore 2027.0.0a8. ntcore 2026 behavior is not tracked.
