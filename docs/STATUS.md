# Project status

Where nt4-rs stands, so work can be picked up from any machine. Last updated
2026-11-15. The full plan is in the
[project plan doc](https://claude.ai/artifact/R1JvYBZZYPm6HuUuFM9QPh).

## Milestones

| # | What | State |
| --- | --- | --- |
| 1 | `nt4-proto`: wire format, all ntcore `WireEncoderTest` / `WireDecoderTest` cases ported | Merged (PR #1) |
| 2 | `nt4-core`: sans-I/O `ClientCore`, `TimeSync`, `OutgoingQueue`; `NetworkOutgoingQueueTest` and client `TimeSyncTest` cases ported | Merged (PR #2) |
| 3 | `nt4`: tokio `Client`, typed publishers and subscribers, pyntcore interop tests, load benchmark, crates.io readiness | Open (PR #3, branch `claude/project-thread-vtt68a`) |
| 4 | `ServerCore` in `nt4-core`, port of ntcore `ServerImplTest` and the `TimeSyncTest` server case | Not started |
| 5 | `nt4` server feature (tokio) and interop with pyntcore clients | Not started |
| 6 | Release and publish to crates.io | Not started |

## What PR #3 found

The load benchmark (`bench/`, results in `bench/README.md`) runs nt4 and
pyntcore 2026.2.2 against a simulated robot of 200 topics (one-time values,
values set every 20 ms, and a busy loop), scalable to 1,640 topics. No values
were lost in any run, and with no per-value code nt4 used less CPU and memory
than C++ ntcore.

It also found three problems, fixed in PR #3:

- `Client::events()` could drop events, including one-time values, under
  load. It is now an unbounded queue per reader, like ntcore's listener
  queues.
- The first snapshot was slow (~215 ms). Messages queued before time sync now
  go out as soon as the reply arrives (~115 ms), the send tick is at most
  100 ms, local changes wake the send loop, and `TCP_NODELAY` is on.
- The MessagePack value decoder was about half of the client's CPU. It now
  decodes straight into `Value` instead of through rmpv; a unit test checks
  it against the old decoder on 20,000 generated frames.

## Next steps

1. **Review and merge PR #3.**
2. **Robustness tests for our own parser.** Add property-based tests
   (`proptest`) and a `cargo-fuzz` target for `nt4-proto`'s decoders (JSON
   text frames and MessagePack binary frames), so malformed or truncated
   input is rejected cleanly with no panic, hang or runaway allocation.
   Where a case is rejected or accepted, compare the result with ntcore
   through pyntcore and record any differences: per `CLAUDE.md`, our decoder
   should accept and reject the same input as ntcore. Not started.
3. **Milestone 4: `ServerCore`.**

## Picking this up locally

```bash
git clone https://github.com/tuffinmuffin/nt4-rs && cd nt4-rs
git switch claude/project-thread-vtt68a   # PR #3, until it merges
cargo test --workspace
```

The interop tests and the benchmark need a Python with `pyntcore` installed
(`pip install pyntcore==2026.2.2`); see `crates/nt4/tests/interop` and
`bench/README.md`.
