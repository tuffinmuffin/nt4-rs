# Project status

Where nt4-rs stands, so work can be picked up from any machine. Last updated
2026-11-16. The full plan is in the
[project plan doc](https://claude.ai/artifact/R1JvYBZZYPm6HuUuFM9QPh).

## Milestones

| # | What | State |
| --- | --- | --- |
| 1 | `nt4-proto`: wire format, all ntcore `WireEncoderTest` / `WireDecoderTest` cases ported | Merged (PR #1) |
| 2 | `nt4-core`: sans-I/O `ClientCore`, `TimeSync`, `OutgoingQueue`; `NetworkOutgoingQueueTest` and client `TimeSyncTest` cases ported | Merged (PR #2) |
| 3 | `nt4`: tokio `Client`, typed publishers and subscribers, pyntcore interop tests, load benchmark, decoder fuzzer, crates.io readiness | Merged (PRs #3 to #7) |
| 4 | `ServerCore` in `nt4-core`, port of ntcore `ServerImplTest` | Merged (PR #8) |
| 5 | `nt4` server feature (tokio), persistence file, interop with pyntcore clients | Merged (PR #9) |
| 6 | Structs and schemas, then release and publish to crates.io | Struct and protobuf code merged (PR #10); release not started |

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

## Milestone 5: the server runtime

`nt4`'s `server` feature adds `nt4::Server` (`crates/nt4/src/server.rs`):

- It listens (port 5810 by default) and checks each upgrade as ntcore's
  `NetworkServer` does: the path must be `/nt/<name>` (percent-decoded, not
  empty) or it gets a 404, and the subprotocol is NT 4.1, then 4.0, then
  RTT-only (anything else gets a 400; none at all means 4.0). RTT-only
  connections just answer time pings and aren't clients. Peers on loopback
  are local (sent to without batching, as ntcore 2027 does).
- One task per connection, all sharing one `ServerCore`. A task takes frames
  from the core only when its socket has room, so a slow client's values
  keep coalescing in the core's queue instead of piling up in a channel.
- The server's own code uses the same typed `Publisher` / `Subscriber`
  handles as `Client` (they moved to `handles.rs`), plus `Server::events`.
- Persistent values: `ServerConfig::persist_file` loads and saves ntcore's
  file format. `ServerCore::dump_persistent` matches ntcore byte for byte
  (wpiutil's number format and escaping, and its odd layout for properties
  with several keys); an interop test has ntcore save a file of awkward
  values and checks that loading and dumping it gives the same bytes. The
  runtime follows `NetworkServer`: restore `<file>.bck` if the file is
  missing, back up an unreadable file to `.bak`, save once a second through
  `.tmp` and `.bck`, CRLF line ends on Windows as ntcore's text mode gives.

Tests: `tests/server.rs` (our client against our server, the RTT
subprotocol, a refused handshake); `tests/interop_server.rs` (pyntcore
clients against our server: connect and time sync, every type both ways,
a relay between two ntcore clients, unpublish, retained topics, meta topics,
reconnect after a restart, the batched path over the machine's network
address, and the persistent file); `tests/persistent.rs` ports
`NetworkServerTest`'s four cases and adds saving tests. `nt4-perf` has a
`server_relay` benchmark. CI now builds, lints, documents and tests with
`--all-features`, since nt4's client and server are both optional.

Where it departs from ntcore, on purpose:

- When the server is dropped, it saves persistent values one last time;
  ntcore only saves on its 1 s timer.
- An unreadable persistent file is backed up and replaced as in ntcore, but
  a missing one is just created (ntcore also "backs up" the missing file,
  which does nothing), and a file with bad UTF-8 is read with replacement
  characters instead of being replaced.
- An array in the persistent file with elements of the wrong type loads
  without them, as in ntcore, but the error is reported (ntcore sets it and
  then drops it).
- No HTTP pages: ntcore answers a plain `GET /` and
  `GET /nt/persistent.json`; nt4 only takes WebSocket upgrades.
- `TimeSyncTest TestServer` is listed as not applicable: it switches one
  ntcore instance to server mode, while nt4's `Server` is its own type and
  always runs on server time.

Found in ntcore along the way: on Windows, ntcore's persistent file writes
characters above U+FFFF wrongly (😀 becomes a lone `\uf600`), because
wpiutil's JSON writer decodes UTF-8 into `wint_t`, which is 16 bits there.
nt4 writes the surrogate pair. A persistent infinite double is written as
`1e5000` by both; nt4's loader (serde_json) can't read that number back, so
such a file fails to load in nt4.

## Milestone 6: structs, protobuf and schemas

Typed struct topics (`nt4::structs`, and the `nt4-wpimath` crate):

- `NtStruct` is wpiutil's `Struct<T>`: a type name, a packed size, a schema
  string, pack and unpack. The name and schema are functions returning
  `Cow<'static, str>`, so generic types such as `Matrix__<r>_<c>` can
  build them; they're called only when publishing or subscribing. Any `T: NtStruct` is an `NtValue`, so
  `publish::<T>` / `subscribe::<T>` work for `T` (`struct:<Name>`) and
  `Vec<T>` (`struct:<Name>[]`). `StructField` and `StructReader` make the
  pack and unpack code short, and `structs::check::<T>()` tests a hand
  implementation against its own schema.
- `nt4-wpimath` (its own crate since the split; it was `nt4::wpimath`)
  has WPILib 2027's geometry and kinematics structs
  (`Translation2d` to `Twist3d`, `ChassisVelocities`,
  `SwerveModulePosition`, `SwerveModuleVelocity`), with the same names,
  schemas and nested-schema order as wpimath's `*Struct.hpp`.
- `NtProtobuf` plus the `Proto<T>` wrapper give `proto:<message>` topics.
  There's no protobuf dependency: with prost, encode and decode are
  `prost::Message`'s and the descriptors come from prost-build's
  `FileDescriptorSet`.
- Publishing a typed struct or protobuf topic registers its schemas as
  ntcore's `AddSchema` does: once per client or server, on a retained
  `/.schema/<name>` topic (`structschema` or `proto:FileDescriptorProto`)
  with the schema as a default value, nested structs first. `add_schema` /
  `has_schema` are public for raw topics. One difference: ntcore registers
  them on a publisher's first `Set`, nt4 when publishing.

Run-time decoding (`nt4_proto::structs`, no I/O): a port of wpiutil's
`SchemaParser` and `DynamicStruct` (`StructDatabase`, `DynamicStruct`,
`DynamicStructMut`, `FieldValue`), so a dashboard can decode structs it
learns about from `/.schema/` topics. Same layout rules, bit fields
included, and the same error text. Where it differs, on purpose:

- A rejected schema changes nothing (wpiutil leaves the struct
  half-updated, with its links to other structs kept).
- A struct over 1 GiB is refused (wpiutil lets a huge array size wrap
  around), and a nested struct field gets no bit mask (wpiutil computes one
  with an out-of-range shift for structs over 8 bytes).
- Getters and setters return `None` for a field of another type or struct,
  or an index past the end, where wpiutil asserts; views of short data
  aren't made at all.

Tests: wpiutil's `SchemaParserTest` and `DynamicStructTest` and ntcore's
`StructTest` are ported and tracked by the audit (175 of 175 cases). The
not-applicable `StructTest` cases are C++ template variants, plus
`std::array<T, N>` structs: wpiutil gives those the schema of `T` with
`[N]` appended, which only makes the last field an array (another bug to
report). Interop tests check, against pyntcore, that ntcore sees our schema
topics exactly as it makes its own (type, bytes, properties) in both
directions, and that we read ntcore's struct topics typed and through
their schemas. The fuzzer also feeds every input to the schema parser and
the dynamic views.

## What PR #3 found

The load benchmark (`bench/`, results in `bench/README.md`) runs nt4 and
pyntcore 2027.0.0a8 against a simulated robot of 200 topics (one-time values,
values set every 20 ms, and a busy loop), scalable to 1,640 topics. nt4 lost
no values in any run. Over a network address, with no per-value code, nt4
used less CPU and memory than C++ ntcore. On localhost ntcore 2027 sends one
frame per value, and there nt4 uses more CPU than ntcore at high rates (71%
against 49% of a core at 157,000 frames/s); nt4's cost per frame is worth a
look. (Looked at on 2026-11-16, below: the gap is now about a fifth.)

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

## Test coverage

The `coverage` CI job measures line, function and region coverage of
`nt4-proto`, `nt4-core` and `nt4` with cargo-llvm-cov, including the
pyntcore interop tests (test, example and bench files are not counted).
`scripts/coverage_summary.py` turns the result into a table per crate and
per file, with the change from the base branch on a PR. It goes in the job
summary and in one PR comment that each push updates; the HTML report is
the `coverage-html` artifact. Fork PRs get only the job summary (their token
can't comment). Nothing fails on coverage yet.

At the start (milestone 4, locally without interop): `nt4-proto` 96.6% and
`nt4-core` 90.0% of lines.

## Performance regression checks

`crates/nt4-perf` counts CPU instructions (Callgrind, through gungraun) for
the hot paths: binary decode and encode, announce decode, publish encode, and
`ClientCore` receiving and publishing one 20 ms robot loop. The `perf` CI job
benchmarks the base branch and the PR in the same job. A benchmark that uses
more than 10% more instructions gives a warning; more than 50% fails the job. Counts don't depend on how busy
the runner is, so a failure means the code does more work. See
`crates/nt4-perf/README.md` for the numbers and how to run it locally.

One thing it showed: `ClientCore` spent about 8,300 instructions per value
when publishing, against about 1,300 when receiving. Fixed on
`claude/publish-perf`: the client copied the topic name on every set, the
outgoing queue cloned each value and encoded it into a new buffer (now
`nt4_proto::encode_binary_parts` into a reused one), the server collected
its target clients into a list for every relayed value, and the queue's id
map used SipHash. Measured on a Mac, a steady publish loop went from 33 to
17 ns per value and a relay through `ServerCore` from 70 to 39 ns; allocations
per published value from 4.3 to 0.2. `client_publish` measures a new
client's first loop, which creates every topic's bookkeeping; a steady-state
benchmark would track a running robot better.

The localhost cost per frame, profiled with `perf` on a 2-core arm64 Ubuntu
26.04 VM (Parallels): 13% of nt4-native's samples were tungstenite
zero-filling its 128 KiB read buffer before each read (a localhost read
brings in about one 50-byte frame), and 6% were atomics from `tokio::select!`
setting up its wakeups again for every frame, plus a new timer per frame.
With an 8 KiB read buffer, frames batched under one lock, one timer per
connection and no empty flushes, nt4-native went from 36.5% to 27.8% of a
core at scale 10 (ntcore-native 23.2%), and the event-stream client from
48.8% to 34.5%, still with no values lost; over the network address it was
the same or lower (4.8% both; `nt4` 15.8% to 13.1%). The rest is mostly
kernel and syscall time. On macOS the smaller buffer gave no net gain, since
there the extra, smaller reads cost what the zero-fill saved.

The `bench/` load benchmark against pyntcore stays a manual tool and is not
run in CI. nt4 is meant for hosts and coprocessors, so the instruction-count
check is enough.

## Next steps

1. **Manual checks** with AdvantageScope and Elastic against the `server`
   example. Done 2026-11-16 on Windows, localhost: both connect and show
   the topics, and AdvantageScope draws `/server/pose` (`struct:Pose2d`)
   driving its circle on the field. AdvantageScope first needed the server
   to answer plain HTTP as ntcore does (it checks `GET /` before opening
   its WebSocket; fixed, and `plain_http_matches_ntcore` in
   `crates/nt4/tests/interop_server.rs` checks the answers against
   ntcore's byte for byte). Not checked: the struct array view (run
   the `struct_topic` example against the server for
   `struct:SwerveModuleVelocity[]`), Elastic's Field widget, and a robot
   on a real network.
2. **Release prep, in this order** (decided 2026-11-16: publish to
   crates.io as `nt4-proto`, `nt4-core`, `nt4`, `nt4-wpimath`, the names
   they have now, each marked unofficial in its description and README):
   1. Done: `NtStruct`'s name and schema are functions, so generic types
      can implement it.
   2. Done: `nt4::wpimath` is now the `nt4-wpimath` crate. nt4's doc
      examples use a small hand-written struct, its tests use test copies
      of four WPILib types (`tests/support/wpi_types.rs`), and the
      `struct_topic` example moved to `nt4-wpimath`. The `server` example
      no longer publishes a pose; run `struct_topic` against it for
      AdvantageScope's field view.
   3. A schema audit next to `scripts/upstream_audit.py`: pull every
      `GetTypeName()` / `GetSchema()` from wpimath's `*Struct.hpp` (plus
      Java-only ones such as HAL's `ControlWord`) at `ALLWPILIB_COMMIT`,
      compare with what nt4-wpimath's types report, and fail on a schema
      that differs, an upstream type that is missing and not listed as not
      applicable, or one of ours upstream no longer has.
   4. The rest of wpimath's structs, one file per group: kinematics
      (`ChassisAccelerations`, `SwerveModuleAcceleration`, differential and
      mecanum wheel positions, velocities and accelerations, the
      kinematics structs), shapes (`Rectangle2d`, `Ellipse2d`),
      trajectory samples (`HolonomicSample`, `DifferentialSample`),
      controllers (the four feedforwards, `DifferentialDriveWheelVoltages`),
      splines, `DCMotor`; then the generic ones (`Matrix__R_C`,
      `Vector__N`, `SwerveDriveKinematics__N`, `LinearSystem__S_I_O`), the
      three `TrapezoidProfileConstraints` names, and `ControlWord` (bit
      fields and an enum).
   5. A weekly CI job that runs both audits against allwpilib's newest
      release and opens an issue when something changed.
   6. Publish-path performance (item 4 below).
   7. `cargo publish --workspace --dry-run`, then publish, then tag.
   Later: `#[derive(NtStruct)]` in an `nt4-derive` crate, like Java's
   `StructGenerator.genRecord`.
3. **Report the ntcore bugs to allwpilib** (side todo, tracked
   separately; notes in `docs/UPSTREAM_BUGS.md`): the two parser bugs
   below, the two server ones above (the missing `ack`, and the stale
   announce flag), the Windows persistent-file bug, the
   `std::array` struct schema, and two client queue-ordering bugs (a
   `setproperties` overtaking its `publish`, and a dropped flush).
4. **nt4's cost per frame** on localhost and per published value: done in
   PR #13 (see the benchmark notes below). Localhost receive is now within
   about a fifth of ntcore on a 2-core VM; the rest is mostly kernel time.
   Worth rerunning `bench/` on the 4-core x86 VM the README tables came
   from. (No CI `perf` warning fired on PRs #8 to #12 because the job reran
   the base's binary for the PR; fixed in #13.)

## Picking this up locally

```bash
git clone https://github.com/tuffinmuffin/nt4-rs && cd nt4-rs
cargo test --workspace
```

The interop tests and the benchmark need a Python with `pyntcore` installed
(`pip install --pre pyntcore==2027.0.0a8`); see `crates/nt4/tests/interop` and
`bench/README.md`.
