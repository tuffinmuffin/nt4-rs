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
  team-number server lookup, and interop tests against pyntcore 2026.2.2.
  `Client::events()` is lossless (an unbounded queue per reader, like
  ntcore's listener queues). `Client::topic` and `Client::topics` read the
  cached topics and values.
- Load benchmark against ntcore in `bench/` (see `bench/README.md`). Fixes
  it led to: queued messages go out as soon as time sync completes, the send
  tick is at most 100 ms, local publish and subscribe changes are sent at
  once, `TCP_NODELAY` is on, and `nt4-proto` decodes MessagePack directly
  (no rmpv at run time).
