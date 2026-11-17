# Load benchmark: nt4 vs ntcore

Checks that nt4 keeps up with a robot's NetworkTables traffic, and compares it
with WPILib's own C++ ntcore (through pyntcore). Linux only, because CPU and
memory come from `/proc`.

```bash
cargo build --release -p nt4-bench
python3 bench/run.py --python <python with pyntcore> --seconds 20 --scale 1
python3 bench/run.py --python <python with pyntcore> --seconds 20 --scale 1 --scenario publish
# over the network address (ntcore batches; 127.x is its unbatched local path)
python3 bench/run.py --python <python with pyntcore> --host <this machine's address>
```

`--json FILE` also saves the raw numbers, and `--only a,b` picks clients.

## The simulated robot

`robot_sim.py` publishes 200 topics, the mix a robot program has:

| Topics | What | How often |
| --- | --- | --- |
| `/robot/config/*` | 40 one-time values (strings, doubles, booleans, double arrays), like constants and build info | once, at startup |
| `/robot/periodic/*` | 150 values: 100 doubles, 20 ints, 10 booleans, 10 `double[8]`, 10 48-byte `struct:Pose3d` | every 20 ms, then `flush_local()` like `TimedRobot` |
| `/robot/loop/*` | 10 values (5 ints, 5 doubles) from a busy thread | in a loop with a 1 ms sleep, about 9,000 sets/s |

Every publisher uses `send_all`, so every set must arrive. `--scale N`
multiplies the periodic and loop topics by N (scale 10 is 1,640 topics, about
155,000 values/s). Each value carries a sequence number, so a receiver can
count lost and out-of-order updates. Booleans can't carry one, so they are
counted as received but not checked.

## What is measured

- `config_ok`: the one-time values arrived with the right contents.
- `snapshot_ms`: from start until every topic had a value.
- `rate`, `lost`, `out_of_order`: updates per second, and gaps or reversals in
  the sequence numbers, measured after the snapshot.
- `p50`, `p99`, `max`: latency in ms, from the value's server timestamp to the
  client's estimate of server time when the value arrived. This includes the
  publisher's wait for its next send period, so it is mostly NT's batching,
  not the client.
- `cpu_pct`: CPU time over the run, as a percentage of one core.
  `peak_rss_mib`: peak memory.

Receive scenario (the default), where the client under test subscribes to
`/robot/` with `all` and a 5 ms period:

| Client | What |
| --- | --- |
| `nt4` | `crates/nt4-bench`: the nt4 event stream, with every value handled in Rust |
| `nt4-native` | nt4 caching values with no per-value code; reads `Client::topics()` at start and end |
| `ntcore-listener` | pyntcore with a value listener, so ntcore calls Python for every value (what Python dashboards do) |
| `ntcore-poller` | pyntcore with a listener poller read every 5 ms |
| `ntcore-native` | C++ ntcore caching values with no per-value Python, the fair comparison for `nt4-native` |

The publish scenario swaps the roles. The robot is the client under test,
publishing the same topics to an empty ntcore server, and an ntcore listener
checks what arrives. CPU and memory are the robot's.

| Robot | What |
| --- | --- |
| `nt4-robot` | `nt4-bench --mode robot`: nt4 publishers, every 20 ms, `send_all` |
| `ntcore-robot` | `robot_sim.py --connect`: pyntcore client with the same publishers |

## Results

2026-11-15, 4-core Xeon at 2.1 GHz (a cloud VM), pyntcore 2027.0.0a8, 20 s per
client, server and client on the same machine.

ntcore 2027 treats a peer on `127.x` as local and sends it every value in its
own WebSocket frame, with no batching. A robot connection is not local, so the
main results below use the machine's network address (`run.py --host
<address>`), which takes the batched path. The localhost results follow.

### Receive over the network address, 200 topics (about 16,400 values/s)

| Client | config_ok | snapshot ms | values/s | lost | p50 ms | p99 ms | CPU % | memory MiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| nt4 | yes | 13 | 16,387 | 0 | 10.8 | 27.7 | 2.2 | 6.7 |
| nt4-native | yes | 18 | | | | | 0.8 | 4.2 |
| ntcore-listener | yes | 113 | 16,443 | 0 | 11.8 | 28.1 | 10.8 | 40.0 |
| ntcore-poller | yes | 114 | 16,445 | 0 | 15.4 | 31.4 | 12.5 | 39.9 |
| ntcore-native | yes | 114 | | | | | 1.1 | 27.7 |

### Receive over the network address, 1,640 topics (about 156,000 values/s)

| Client | config_ok | snapshot ms | values/s | lost | p50 ms | p99 ms | CPU % | memory MiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| nt4 | yes | 19 | 156,488 | 0 | 13.6 | 29.9 | 17.5 | 28.9 |
| nt4-native | yes | 19 | | | | | 3.7 | 4.9 |
| ntcore-listener | yes | 128 | 155,618 | 0 | 21.7 | 55.7 | 87.1 | 203.3 |
| ntcore-poller | yes | 131 | 155,218 | 0 | 116.3 | 316.0 | 99.1 | 196.6 |
| ntcore-native | yes | 135 | | | | | 5.0 | 31.1 |

### Publish over the network address, 200 and 840 topics

| Robot | topics | values/s received | lost | p50 ms | p99 ms | CPU % | memory MiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| nt4-robot | 200 | 16,526 | 0 | 18.3 | 31.1 | 3.6 | 4.4 |
| ntcore-robot | 200 | 16,418 | 0 | 13.9 | 29.7 | 5.7 | 27.8 |
| nt4-robot | 840 | 81,741 | 0 | 22.8 | 35.2 | 7.0 | 6.0 |
| ntcore-robot | 840 | 80,741 | 0 | 21.7 | 36.9 | 12.1 | 29.5 |

Above 840 topics the Python receiver that checks the robot runs out of CPU,
so publish runs stop there.

### Localhost (ntcore's unbatched local path)

Receive, 200 topics:

| Client | config_ok | snapshot ms | values/s | lost | p50 ms | p99 ms | CPU % | memory MiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| nt4 | yes | 4 | 16,402 | 0 | 2.2 | 18.4 | 11.4 | 6.7 |
| nt4-native | yes | 6 | | | | | 8.4 | 4.2 |
| ntcore-listener | yes | 5 | 16,391 | 0 | 3.1 | 17.7 | 21.4 | 43.4 |
| ntcore-poller | yes | 6 | 16,378 | 0 | 8.2 | 21.0 | 20.9 | 42.7 |
| ntcore-native | yes | 6 | | | | | 8.0 | 27.7 |

Receive, 1,640 topics:

| Client | config_ok | snapshot ms | values/s | lost | p50 ms | p99 ms | CPU % | memory MiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| nt4 | yes | 38 | 157,084 | 0 | 6.3 | 19.3 | 89.8 | 28.6 |
| nt4-native | yes | 18 | | | | | 71.0 | 4.9 |
| ntcore-listener | yes | 21 | 146,542 | 193,985 | 74.3 | 123.1 | 147.2 | 176.6 |
| ntcore-poller | yes | 31 | 122,719 | 593,427 | 118.9 | 253.5 | 130.8 | 163.0 |
| ntcore-native | yes | 33 | | | | | 48.5 | 30.6 |

Publish, 200 and 840 topics:

| Robot | topics | values/s received | lost | p50 ms | p99 ms | CPU % | memory MiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| nt4-robot | 200 | 16,520 | 0 | 12.9 | 22.6 | 3.6 | 4.7 |
| ntcore-robot | 200 | 16,390 | 0 | 2.8 | 16.3 | 11.4 | 27.7 |
| nt4-robot | 840 | 82,673 | 0 | 18.4 | 40.5 | 6.2 | 6.2 |
| ntcore-robot | 840 | 80,053 | 0 | 7.2 | 18.8 | 28.4 | 29.0 |

### Reading the results

- nt4 lost and reordered nothing in any run, and the one-time values always
  arrived.
- Over the network address, library alone (`nt4-native` against
  `ntcore-native`), nt4 uses less CPU than C++ ntcore: 3.7% against 5.0% of a
  core at 1,640 topics. It also uses 4 to 5 MiB of memory against about
  30 MiB, though some of ntcore's figure is the Python interpreter.
- nt4 has the first snapshot in 13 to 19 ms; ntcore takes about 115 ms,
  because it leaves its subscription for its next 100 ms timer after time
  sync and nt4 sends it with the time sync reply.
- With per-value work, nt4 delivering every value to Rust code (`nt4`) costs
  a fifth of what ntcore costs delivering it to Python. At 1,640 topics the
  Python clients use a whole core and fall behind (p99 56 to 316 ms), while
  nt4 stays at a p99 of 30 ms. The `nt4` run's extra memory at scale 10 is
  the bench storing every latency sample.
- Latency over the network address is about the same for every client,
  because it is mostly the send periods: the robot's 20 ms loop and the 5 ms
  subscription period. nt4 as a publisher is 1 to 4 ms higher at p50 and the
  same at p99.
- On localhost the server sends one frame per value, so everyone pays per
  frame instead of per batch: latency drops to a few ms and CPU goes up
  several times. Here nt4 is the slower library at high rates: 71% of a core
  against ntcore's 49% at 157,000 frames/s, and about equal at 16,400. (Since
  these runs, nt4's receive path costs about a quarter less here; see
  `docs/STATUS.md`.) At 1,640 topics the Python
  clients could not keep up and missed values (the `lost` column).
- nt4 does not have ntcore 2027's local mode as a sender, so on localhost
  `nt4-robot` still batches (p50 13 to 18 ms against ntcore's 3 to 7 ms) and
  uses a third to a fifth of the CPU.

## Fixed along the way

The first runs found three problems in nt4, fixed in the same change:

1. `Client::events()` used a bounded broadcast channel that dropped events
   when the reader fell behind, which lost one-time values under load. It is
   now an unbounded queue per reader, like ntcore's listener queues.
2. The first snapshot took about 200 ms. Messages queued before time sync
   waited for the next send tick, which could be 200 ms away. They now go
   out with the time sync reply (the 2027 rerun found a case where they still
   waited one tick, fixed in `nt4-core`), so the snapshot takes under 20 ms. The
   send tick is capped at 100 ms, like ntcore's local timer. Publish,
   subscribe and property changes wake the send loop at once. The client
   also sets `TCP_NODELAY`, as ntcore does.
3. Decoding values went through a generic MessagePack tree (rmpv), which was
   about half the client's own CPU. The decoder now reads MessagePack
   directly into `Value`. A test checks it against the old decoder on 20,000
   generated frames: valid, mutated, truncated and random.
