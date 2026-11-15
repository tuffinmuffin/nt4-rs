# Project status

Where nt4-rs stands, so work can be picked up from any machine. Last updated
2026-11-15. The full plan is in the
[project plan doc](https://claude.ai/artifact/R1JvYBZZYPm6HuUuFM9QPh).

## Milestones

| # | What | State |
| --- | --- | --- |
| 1 | `nt4-proto`: wire format, all ntcore `WireEncoderTest` / `WireDecoderTest` cases ported | Merged (PR #1) |
| 2 | `nt4-core`: sans-I/O `ClientCore`, `TimeSync`, `OutgoingQueue`; `NetworkOutgoingQueueTest` and client `TimeSyncTest` cases ported | Merged (PR #2) |
| 3 | `nt4`: tokio `Client`, typed publishers and subscribers, pyntcore interop tests, load benchmark, decoder fuzzer, crates.io readiness | Open (PR #3, branch `claude/project-thread-vtt68a`) |
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

## Decoder robustness testing (done, in PR #3)

`crates/nt4-fuzz` is a differential fuzzer for our own parser (see
`crates/nt4-fuzz/README.md`). It generates structure-aware frames (MessagePack
binary, client and server JSON text), decodes each one with `nt4-proto` and
with ntcore's own decoders from the pyntcore 2026.2.2 wheel, and compares the
results. It also feeds every frame to `ClientCore` to check for panics, and
shrinks any failing input. Without `NT4_NTCORE_PYTHON` set it runs only nt4's
own checks.

Fixed in `nt4-proto` as a result:

- Invalid UTF-8 in binary string values is now accepted with replacement
  characters, as ntcore does.
- `-0` integer literals in JSON text frames now parse as 0, as in ntcore.
- Integer id fields above `i64::MAX` are now rejected as out of range instead
  of wrapping.
- JSON numbers now parse exactly (serde_json `float_roundtrip`), as ntcore's
  do; some doubles decoded one step low before.

`crates/nt4-proto/tests/fuzz_regressions.rs` keeps the inputs behind each fix.

Intentional differences from ntcore (JSON nesting limit, out-of-range
`periodic`, ids above `i64::MAX`, ntcore's unsubscribe error-text typo) are
listed in the fuzzer README.

It also found an ntcore bug: ntcore 2026.2.2 ends its process on a JSON text
frame holding a number too large for a double, such as `[1e400]`. Its JSON
parser throws `out_of_range`, and `WireDecodeText` only catches
`parse_error`. ntcore 2027 (alpha 8) uses a different JSON parser and is not
affected. `nt4-proto` handles these frames. A write-up for WPILib is in
progress.

### Running it

CI runs the fuzzer with the ntcore oracle for 60 seconds on each push (the
`fuzz` job, Linux only). On GitHub's runners that is about 230,000 frames
over the three targets, and the whole job takes about 1.5 minutes. The seed
is the run id, printed on the first line, so a failure reproduces locally
with `--seed`. See `crates/nt4-fuzz/README.md` for longer local runs.

## Next steps

1. **Review and merge PR #3.**
2. **Report the `1e400` ntcore bug to allwpilib** (2026 branch only).
3. **Milestone 4: `ServerCore`.** Extend the fuzzer to server-side handling
   once it exists.

## Picking this up locally

```bash
git clone https://github.com/tuffinmuffin/nt4-rs && cd nt4-rs
git switch claude/project-thread-vtt68a   # PR #3, until it merges
cargo test --workspace
```

The interop tests and the benchmark need a Python with `pyntcore` installed
(`pip install pyntcore==2026.2.2`); see `crates/nt4/tests/interop` and
`bench/README.md`.
