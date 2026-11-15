# Load benchmark: nt4 vs ntcore

Checks that nt4 keeps up with a robot's NetworkTables traffic, and compares it
with WPILib's own C++ ntcore (through pyntcore). Linux only, because CPU and
memory come from `/proc`.

```bash
cargo build --release -p nt4-bench
python3 bench/run.py --python <python with pyntcore> --seconds 20 --scale 1
python3 bench/run.py --python <python with pyntcore> --seconds 20 --scale 1 --scenario publish
```

`--json FILE` also saves the raw numbers, and `--only a,b` picks clients.

## The simulated robot

`robot_sim.py` publishes 200 topics, the mix a robot program has:

| Topics | What | How often |
| --- | --- | --- |
| `/robot/config/*` | 40 one-time values (strings, doubles, booleans, double arrays), like constants and build info | once, at startup |
| `/robot/periodic/*` | 150 values: 100 doubles, 20 ints, 10 booleans, 10 `double[8]`, 10 48-byte `struct:Pose3d` | every 20 ms, then `flushLocal()` like `TimedRobot` |
| `/robot/loop/*` | 10 values (5 ints, 5 doubles) from a busy thread | in a loop with a 1 ms sleep, about 9,000 sets/s |

Every publisher uses `sendAll`, so every set must arrive. `--scale N`
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

2026-11-15, 4-core Xeon at 2.1 GHz (a cloud VM), pyntcore 2026.2.2, 20 s per
client, server and client on the same machine.

### Receive, 200 topics (about 16,400 values/s)

| Client | config_ok | snapshot ms | values/s | lost | p50 ms | p99 ms | CPU % | memory MiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| nt4 | yes | 114 | 16,391 | 0 | 10.0 | 27.9 | 2.1 | 6.8 |
| nt4-native | yes | 118 | | | | | 0.8 | 4.2 |
| ntcore-listener | yes | 117 | 16,354 | 0 | 12.4 | 28.2 | 13.5 | 45.8 |
| ntcore-poller | yes | 119 | 16,373 | 0 | 14.4 | 30.9 | 15.0 | 42.3 |
| ntcore-native | yes | 118 | | | | | 1.2 | 30.1 |

### Receive, 1,640 topics (about 155,000 values/s)

| Client | config_ok | snapshot ms | values/s | lost | p50 ms | p99 ms | CPU % | memory MiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| nt4 | yes | 121 | 154,707 | 0 | 13.4 | 29.6 | 18.5 | 28.7 |
| nt4-native | yes | 119 | | | | | 3.8 | 5.0 |
| ntcore-listener | yes | 132 | 156,393 | 0 | 20.8 | 68.6 | 85.3 | 195.5 |
| ntcore-poller | yes | 138 | 153,709 | 0 | 197.9 | 350.1 | 100.2 | 230.3 |
| ntcore-native | yes | 150 | | | | | 4.7 | 33.3 |

### Publish, 200 and 840 topics

| Robot | topics | values/s received | lost | p50 ms | p99 ms | CPU % | memory MiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| nt4-robot | 200 | 16,521 | 0 | 18.1 | 31.3 | 3.8 | 4.5 |
| ntcore-robot | 200 | 16,384 | 0 | 15.8 | 29.4 | 6.1 | 30.2 |
| nt4-robot | 840 | 81,807 | 0 | 23.9 | 35.8 | 7.2 | 6.2 |
| ntcore-robot | 840 | 80,430 | 0 | 21.7 | 35.6 | 13.3 | 31.9 |

Above 840 topics the Python receiver that checks the robot runs out of CPU,
so publish runs stop there.

### Reading the results

- Nothing was lost or reordered in any run, and the one-time values always
  arrived.
- Library alone (`nt4-native` against `ntcore-native`), nt4 uses less CPU
  than C++ ntcore: 3.8% against 4.7% of a core at 1,640 topics. It also uses
  4 to 5 MiB of memory against about 30 MiB, though some of ntcore's figure is
  the Python interpreter.
- With per-value work, nt4 delivering every value to Rust code (`nt4`) costs
  a fifth of what ntcore costs delivering it to Python. At 1,640 topics the
  Python clients use a whole core and fall behind (p99 69 to 350 ms), while
  nt4 stays at a p99 of 30 ms. The `nt4` run's extra memory at scale 10 is
  the bench storing every latency sample.
- Latency is about the same for every client, because it is mostly the send
  periods: the robot's 20 ms loop and the 5 ms subscription period. nt4 as a
  publisher is 2 to 3 ms higher at p50 and the same at p99.

## Fixed along the way

The first runs found three problems in nt4, fixed in the same change:

1. `Client::events()` used a bounded broadcast channel that dropped events
   when the reader fell behind, which lost one-time values under load. It is
   now an unbounded queue per reader, like ntcore's listener queues.
2. The first snapshot took about 200 ms instead of about 115 ms. Messages
   queued before time sync waited for the next send tick, which could be
   200 ms away. They now go out as soon as the time sync reply arrives. The
   send tick is capped at 100 ms, like ntcore's local timer. Publish,
   subscribe and property changes wake the send loop at once. The client
   also sets `TCP_NODELAY`, as ntcore does.
3. Decoding values went through a generic MessagePack tree (rmpv), which was
   about half the client's own CPU. The decoder now reads MessagePack
   directly into `Value`. A test checks it against the old decoder on 20,000
   generated frames: valid, mutated, truncated and random.
