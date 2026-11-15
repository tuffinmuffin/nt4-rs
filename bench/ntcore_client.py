"""Benchmark client using ntcore (C++ via pyntcore), for comparison with nt4.

    python -u bench/ntcore_client.py --port 5810 --seconds 20 --mode listener

Subscribes to /robot/ with sendAll and a 5 ms period, then records the same
numbers as crates/nt4-bench (see bench/README.md) and prints them as one
JSON line.

--mode listener: a value listener; ntcore calls Python on its own thread for
                 every value (what most Python dashboards do).
--mode poller:   a NetworkTableListenerPoller read every 5 ms; ntcore queues
                 values in C++ and Python takes them in batches. Latency
                 includes up to 5 ms of polling delay.
--mode native:   no Python per value: ntcore's C++ receives and caches every
                 value, and Python only checks the cache at the start and end.
                 This is the C++ ntcore's own cost (CPU, memory, snapshot,
                 one-time values); it can't count lost updates or latency.

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
ap.add_argument("--seconds", type=float, default=20)
ap.add_argument("--mode", choices=["listener", "poller", "native"], default="listener")
ap.add_argument("--topics", type=int, default=200)
args = ap.parse_args()

t0 = time.monotonic()
inst = ntcore.NetworkTableInstance.create()
inst.set_server("127.0.0.1", args.port)
inst.start_client(f"ntcore-{args.mode}")
sub = ntcore.MultiSubscriber(
    inst, ["/robot/"], ntcore.PubSubOptions(send_all=True, periodic=0.005)
)

lock = threading.Lock()
values = {}  # name -> latest python value
last_seq = {}
stats = {"received": 0, "lost": 0, "out_of_order": 0, "seq_values": 0}
latencies = []  # microseconds
measuring = [False]
snapshot_at = [None]
synced_at = [None]


def seq_of(v):
    if isinstance(v, bool):
        return None
    if isinstance(v, (int, float)):
        return int(v)
    if isinstance(v, (bytes, bytearray)) and len(v) >= 8:
        return struct.unpack_from("<q", v)[0]
    if isinstance(v, (list, tuple)) and v and isinstance(v[0], float):
        return int(v[0])
    return None


def on_value(topic, value):
    name = topic.get_name()
    v = value.value()
    values[name] = v
    if snapshot_at[0] is None and len(values) >= args.topics:
        snapshot_at[0] = time.monotonic()
    if not measuring[0] or name.startswith("/robot/config/"):
        return
    stats["received"] += 1
    seq = seq_of(v)
    if seq is None:
        return
    stats["seq_values"] += 1
    prev = last_seq.get(name)
    if prev is not None:
        if seq > prev + 1:
            stats["lost"] += seq - prev - 1
        elif seq <= prev:
            stats["out_of_order"] += 1
    last_seq[name] = max(seq, prev or seq)
    offset = inst.get_server_time_offset()
    if offset is not None:
        # pyntcore 2027 times are nanoseconds.
        latencies.append((ntcore._now() + offset - value.server_time()) / 1000)


mask = ntcore.EventFlags.VALUE_ALL
if args.mode == "listener":

    def cb(event):
        with lock:
            on_value(event.data.topic, event.data.value)

    inst.add_listener(sub, mask, cb)
elif args.mode == "poller":
    poller = ntcore.NetworkTableListenerPoller(inst)
    poller.add_listener(sub, mask)


def read_cache():
    """Native mode: copy ntcore's cached values into `values`."""
    for topic in inst.get_topics("/robot/"):
        v = inst.get_entry(topic.get_name()).get_value()
        if v.is_valid():
            values[topic.get_name()] = v.value()
    if snapshot_at[0] is None and len(values) >= args.topics:
        snapshot_at[0] = time.monotonic()


stop_at = None
while True:
    now = time.monotonic()
    if synced_at[0] is None and inst.get_server_time_offset() is not None:
        synced_at[0] = now
    if stop_at is None and snapshot_at[0] is not None:
        # Measure for --seconds after the snapshot arrived.
        with lock:
            measuring[0] = True
            last_seq.clear()
        measure_start = now
        stop_at = now + args.seconds
    if stop_at is not None and now >= stop_at:
        break
    if now - t0 > args.seconds + 30:
        break
    if args.mode == "native":
        if snapshot_at[0] is None:
            read_cache()
            time.sleep(0.005)
        else:
            time.sleep(0.05)
    elif args.mode == "poller":
        for e in poller.read_queue():
            if e.data is not None and hasattr(e.data, "value"):
                on_value(e.data.topic, e.data.value)
        time.sleep(0.005)
    else:
        time.sleep(0.05)

if args.mode == "native":
    read_cache()
with lock:
    measuring[0] = False
    elapsed = time.monotonic() - measure_start if stop_at else 0.0
    lat = sorted(latencies)


def pct(p):
    return lat[min(len(lat) - 1, int(p / 100 * len(lat)))] / 1000 if lat else None


config_ok = all(
    values.get(f"/robot/config/str{i}") == f"value-{i}" for i in range(20)
) and all(values.get(f"/robot/config/num{i}") == i * 1.5 for i in range(10))
print(
    json.dumps(
        {
            "client": f"ntcore-{args.mode}",
            "synced_ms": (synced_at[0] - t0) * 1000 if synced_at[0] else None,
            "snapshot_ms": (snapshot_at[0] - t0) * 1000 if snapshot_at[0] else None,
            "topics_seen": len(values),
            "config_ok": config_ok,
            "seconds": elapsed,
            "received": None if args.mode == "native" else stats["received"],
            "rate": None
            if args.mode == "native"
            else stats["received"] / elapsed
            if elapsed
            else 0,
            "seq_values": stats["seq_values"],
            "lost": None if args.mode == "native" else stats["lost"],
            "out_of_order": None if args.mode == "native" else stats["out_of_order"],
            "latency_ms": {
                "p50": pct(50),
                "p95": pct(95),
                "p99": pct(99),
                "max": pct(100),
            },
        }
    ),
    flush=True,
)

# pyntcore 2027.0.0a8 can hang at interpreter exit while a listener or
# publisher is alive, so leave without running Python's shutdown.
sys.stdout.flush()
os._exit(0)
