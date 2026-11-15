# Project status

Where nt4-rs stands, so work can be picked up from any machine. Last updated
2026-11-15. The full plan is in the
[project plan doc](https://claude.ai/artifact/R1JvYBZZYPm6HuUuFM9QPh).

## Milestones

| # | What | State |
| --- | --- | --- |
| 1 | `nt4-proto`: wire format, all ntcore `WireEncoderTest` / `WireDecoderTest` cases ported | Merged (PR #1) |
| 2 | `nt4-core`: sans-I/O `ClientCore`, `TimeSync`, `OutgoingQueue`; `NetworkOutgoingQueueTest` and client `TimeSyncTest` cases ported | Merged (PR #2) |
| 3 | `nt4`: tokio `Client`, typed publishers and subscribers, pyntcore interop tests, load benchmark, decoder fuzzer, crates.io readiness | Merged (PRs #3 to #7) |
| 4 | `ServerCore` in `nt4-core`, port of ntcore `ServerImplTest` | In review (branch `claude/server-core`) |
| 5 | `nt4` server feature (tokio), persistence file, interop with pyntcore clients, the `TimeSyncTest` server case | Not started |
| 6 | Release and publish to crates.io | Not started |

## Milestone 4: `ServerCore`

`crates/nt4-core/src/server.rs` ports ntcore's `ServerImpl`, `ServerStorage`,
`ServerClient4`, `ServerClientLocal` and the topic, publisher and subscriber
types into one sans-I/O `ServerCore`: retained last values, a send queue per
client at each subscription's period, `sendAll` and topics-only
subscriptions, RTT replies, WebSocket keepalive, the `$` topic rules, the
meta topics (MessagePack written byte for byte as mpack does), and the
server's own publishers and subscribers as client 0. Topic and client ids
come out as ntcore's (freed topic ids are reused once 16 are free, as in
`UidVector`).

Tests: `tests/server.rs` ports all 16 applicable `ServerImplTest` cases
(`AddClient3` is NT3 and listed as not applicable); `tests/server_core.rs`
adds 21 more for meta topic contents, keepalive, properties, retained
topics and id reuse. The fuzzer now feeds every frame to `ServerCore` as
well (60 s locally, about 127,000 frames: no panics).

Where it departs from ntcore, on purpose:

- `setproperties` answers the sender with `ack: true`, as the spec says.
  ntcore passes no client there, so nobody gets an ack. ntcore clients
  ignore acks, so this is safe for them.
- When a topic is deleted, every client forgets that it was announced.
  ntcore keeps the flag (keyed by pointer) for clients that only published,
  which with reused ids would hide the next topic with the same id.
- When a client leaves, its topics' "last set by" marks are cleared, so a
  later client in the same slot isn't treated as the same publisher.
- `tick_period_ms` is at most 100 ms. ntcore runs a client's timer only at
  its subscription period, so a client that only publishes gets its
  announcements and pings only when the server flushes.
- Incoming frames are handled at once. ntcore handles 10 messages, stops
  reading the socket and queues the rest (backpressure between its
  threads); here the caller decides when to read. The client's meta topics
  update once per frame instead of once per batch.
- Map iteration is in slot and uid order where ntcore uses hash order, so
  meta arrays with several entries can list them in a different order.
- `$clients` updates on `add_client` and `remove_client`; in ntcore the
  network layer calls `ConnectionsChanged`.

Left for milestone 5: the persistence file (`DumpPersistent` /
`LoadPersistent` and the persistent-changed flag, with `NetworkServerTest`),
connections that use only the RTT subprotocol, the `TimeSyncTest`
server case, server benchmarks in `nt4-perf`, and interop with pyntcore
clients.

## What PR #3 found

The load benchmark (`bench/`, results in `bench/README.md`) runs nt4 and
pyntcore 2027.0.0a8 against a simulated robot of 200 topics (one-time values,
values set every 20 ms, and a busy loop), scalable to 1,640 topics. nt4 lost
no values in any run. Over a network address, with no per-value code, nt4
used less CPU and memory than C++ ntcore. On localhost ntcore 2027 sends one
frame per value, and there nt4 uses more CPU than ntcore at high rates (71%
against 49% of a core at 157,000 frames/s); nt4's cost per frame is worth a
look.

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

## Decoder robustness testing (done, in PR #3)

`crates/nt4-fuzz` is a differential fuzzer for our own parser (see
`crates/nt4-fuzz/README.md`). It generates structure-aware frames (MessagePack
binary, client and server JSON text), decodes each one with `nt4-proto` and
with ntcore's own decoders from the pyntcore 2027.0.0a8 wheel, and compares the
results. It also feeds every frame to `ClientCore` to check for panics, and
shrinks any failing input. Without `NT4_NTCORE_PYTHON` set it runs only nt4's
own checks.

Fixed in `nt4-proto` as a result:

- Invalid UTF-8 in binary string values is now accepted with replacement
  characters, as ntcore does.
- `-0` integer literals in JSON text frames now parse as 0, as in ntcore.
- JSON numbers now parse exactly (serde_json `float_roundtrip`), as ntcore's
  do; some doubles decoded one step low before.
- JSON nesting stops at 20 levels, the first of a repeated key wins, and an
  integer above `i64::MAX` is not a valid id, all as in ntcore 2027.
- A binary timestamp too large to hold in nanoseconds is rejected, as in
  ntcore 2027.

`crates/nt4-proto/tests/fuzz_regressions.rs` keeps the inputs behind each fix.

nt4 tracks ntcore 2027 and later only (ntcore 2026 goes out of support), so
the oracle, the interop tests and the benchmark all use pyntcore 2027.0.0a8.
Against it the fuzzer reports no mismatches. The intentional differences are
listed in the fuzzer README: nt4 rejects a few kinds of text that are not
JSON but that ntcore's parser takes, and it does not copy two ntcore bugs.

Those two ntcore bugs are on allwpilib `main`: the JSON parser gives a `null`
member that follows a repeated key the repeated key's value, and
`unsubscribe` logs "pubuid out of range" for a bad `subuid`. A write-up for
WPILib is in
[this doc](https://claude.ai/artifact/QZfgEuNvsBzrsHrimL2qUG).

### Running it

CI runs the fuzzer with the ntcore oracle for 60 seconds on each push (the
`fuzz` job, Linux only). On GitHub's runners that is about 230,000 frames
over the three targets, and the whole job takes about 1.5 minutes. The seed
is the run id, printed on the first line, so a failure reproduces locally
with `--seed`. See `crates/nt4-fuzz/README.md` for longer local runs.

## Performance regression checks

`crates/nt4-perf` counts CPU instructions (Callgrind, through gungraun) for
the hot paths: binary decode and encode, announce decode, publish encode, and
`ClientCore` receiving and publishing one 20 ms robot loop. The `perf` CI job
benchmarks the base branch and the PR in the same job. A benchmark that uses
more than 10% more instructions gives a warning; more than 50% fails the job. Counts don't depend on how busy
the runner is, so a failure means the code does more work. See
`crates/nt4-perf/README.md` for the numbers and how to run it locally.

One thing it shows: `ClientCore` spends about 8,300 instructions per value
when publishing, against about 1,300 when receiving. That may be worth a look.

The `bench/` load benchmark against pyntcore stays a manual tool and is not
run in CI. nt4 is meant for hosts and coprocessors, so the instruction-count
check is enough.

## Next steps

1. **Review and merge the milestone 4 PR** (`claude/server-core`).
2. **Milestone 5: the server runtime.** A tokio server on 5810 around
   `ServerCore`, the persistence file, pyntcore clients against it in CI,
   and server benchmarks in `nt4-perf`.
3. **Report the ntcore bugs to allwpilib** (`main`): the two parser bugs
   below, and the two server ones above (the missing `ack`, and the stale
   announce flag).
4. **Look at nt4's cost per frame** on localhost and per published value
   (see the benchmark notes below).

## Picking this up locally

```bash
git clone https://github.com/tuffinmuffin/nt4-rs && cd nt4-rs
git switch claude/server-core   # milestone 4, until it merges
cargo test --workspace
```

The interop tests and the benchmark need a Python with `pyntcore` installed
(`pip install --pre pyntcore==2027.0.0a8`); see `crates/nt4/tests/interop` and
`bench/README.md`.
