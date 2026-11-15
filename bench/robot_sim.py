"""Simulated robot: an ntcore server (C++ ntcore via pyntcore) with ~200 topics.

Run by bench/run.py; by hand:
    python -u bench/robot_sim.py --port 5810 --seconds 60 --scale 1

Topics (scale 1, 200 total):
  /robot/config/*   40 one-time values set at startup (strings, doubles,
                    booleans, double arrays), like constants and build info
  /robot/periodic/* 150 values set every 20 ms, like a robot's periodic():
                    100 doubles, 20 ints, 10 booleans, 10 double[8],
                    10 raw 48-byte structs (a Pose3d-sized blob)
  /robot/loop/*     10 values set in a tight loop as fast as a sleep(0.001)
                    loop allows (5 ints, 5 doubles), like a busy vision or
                    logging thread

--scale N multiplies the periodic and loop topic counts by N.

--connect PORT makes it a client of another server instead (the "publish"
scenario in bench/run.py, compared with nt4-bench --mode robot), and
--server-only runs an empty server for that.

After each 20 ms cycle it calls flush_local(), as TimedRobot does, so periodic
values reach the network right away. The tight loop doesn't flush, so its
values wait for ntcore's 100 ms local timer or the next cycle's flush.

Sequence-carrying values: every periodic and loop value (except booleans)
encodes an increasing sequence number, so a client can count lost updates:
doubles/ints are the sequence, arrays start with it, raw structs start with
it as a little-endian int64.

Prints one JSON line when ready ({"ready": true, "topics": N}) and one at the
end with how many values it set.

Written by Claude.
"""

import argparse
import json
import os
import struct
import sys
import threading
import time

import ntcore

ap = argparse.ArgumentParser()
ap.add_argument("--port", type=int, default=5810)
ap.add_argument("--seconds", type=float, default=60)
ap.add_argument("--scale", type=int, default=1)
ap.add_argument(
    "--connect",
    type=int,
    default=0,
    metavar="PORT",
    help="be a client of the server on 127.0.0.1:PORT instead of a server",
)
ap.add_argument(
    "--server-only",
    action="store_true",
    help="just run an empty server (for nt4-bench --mode robot)",
)
ap.add_argument(
    "--no-flush-local",
    action="store_true",
    help="don't call flush_local() after each 20 ms cycle",
)
args = ap.parse_args()

inst = ntcore.NetworkTableInstance.create()
if args.connect:
    inst.set_server("127.0.0.1", args.connect)
    inst.start_client("robot-sim")
    while not inst.is_connected():
        time.sleep(0.01)
else:
    inst.start_server("", "127.0.0.1", "", args.port)
if args.server_only:
    print(json.dumps({"ready": True, "topics": 0}), flush=True)
    time.sleep(args.seconds)
    os._exit(0)

# As a client, send every 20 ms (the default is 100 ms); a server sends at
# each subscriber's period whatever this says.
ALL = ntcore.PubSubOptions(send_all=True, keep_duplicates=True, periodic=0.02)

# One-time values. Keep the publishers: a dropped publisher unpublishes its
# topic, and the value goes with it.
config = []
for i in range(20):
    config.append(inst.get_string_topic(f"/robot/config/str{i}").publish())
    config[-1].set(f"value-{i}")
for i in range(10):
    config.append(inst.get_double_topic(f"/robot/config/num{i}").publish())
    config[-1].set(i * 1.5)
for i in range(5):
    config.append(inst.get_boolean_topic(f"/robot/config/flag{i}").publish())
    config[-1].set(i % 2 == 0)
for i in range(5):
    config.append(inst.get_double_array_topic(f"/robot/config/arr{i}").publish())
    config[-1].set([float(j) for j in range(10)])
config_count = len(config)

s = args.scale
periodic = []  # (kind, publisher)
for i in range(100 * s):
    periodic.append(
        ("double", inst.get_double_topic(f"/robot/periodic/double{i}").publish(ALL))
    )
for i in range(20 * s):
    periodic.append(
        ("int", inst.get_integer_topic(f"/robot/periodic/int{i}").publish(ALL))
    )
for i in range(10 * s):
    periodic.append(
        ("bool", inst.get_boolean_topic(f"/robot/periodic/bool{i}").publish(ALL))
    )
for i in range(10 * s):
    periodic.append(
        ("darr", inst.get_double_array_topic(f"/robot/periodic/darr{i}").publish(ALL))
    )
for i in range(10 * s):
    t = inst.get_raw_topic(f"/robot/periodic/pose{i}")
    periodic.append(("raw", t.publish("struct:Pose3d", ALL)))

loop = []
for i in range(5 * s):
    loop.append(("int", inst.get_integer_topic(f"/robot/loop/int{i}").publish(ALL)))
for i in range(5 * s):
    loop.append(("double", inst.get_double_topic(f"/robot/loop/double{i}").publish(ALL)))


def put(kind, pub, seq):
    if kind == "double":
        pub.set(float(seq))
    elif kind == "int":
        pub.set(seq)
    elif kind == "bool":
        pub.set(seq % 2 == 0)
    elif kind == "darr":
        pub.set([float(seq)] + [0.5] * 7)
    elif kind == "raw":
        pub.set(struct.pack("<q", seq) + bytes(40))


stop = threading.Event()
sent = {"periodic": 0, "loop": 0}


def loop_thread():
    seq = 0
    while not stop.is_set():
        seq += 1
        for kind, pub in loop:
            put(kind, pub, seq)
        sent["loop"] += len(loop)
        time.sleep(0.001)


print(
    json.dumps({"ready": True, "topics": config_count + len(periodic) + len(loop)}),
    flush=True,
)
th = threading.Thread(target=loop_thread, daemon=True)
th.start()

start = time.monotonic()
next_t = start
seq = 0
late = 0
while time.monotonic() - start < args.seconds:
    seq += 1
    for kind, pub in periodic:
        put(kind, pub, seq)
    sent["periodic"] += len(periodic)
    if not args.no_flush_local:
        # TimedRobot does this at the end of every loop, so values go to the
        # network now instead of on ntcore's 100 ms local timer.
        inst.flush_local()
    next_t += 0.020
    delay = next_t - time.monotonic()
    if delay > 0:
        time.sleep(delay)
    else:
        late += 1
stop.set()
th.join()
elapsed = time.monotonic() - start
print(
    json.dumps(
        {
            "done": True,
            "seconds": elapsed,
            "periodic_sets": sent["periodic"],
            "loop_sets": sent["loop"],
            "periodic_rate": sent["periodic"] / elapsed,
            "loop_rate": sent["loop"] / elapsed,
            "late_cycles": late,
        }
    ),
    flush=True,
)

# pyntcore 2027.0.0a8 can hang at interpreter exit while a listener or
# publisher is alive, so leave without running Python's shutdown.
sys.stdout.flush()
os._exit(0)
