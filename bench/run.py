"""Run the nt4 vs ntcore benchmark and print a table (and optional JSON).

    python bench/run.py --python <python with pyntcore> --seconds 20 --scale 1

Starts bench/robot_sim.py (an ntcore server with a simulated robot's ~200
topics), then runs each client against it, one at a time:

  nt4              crates/nt4-bench (build with cargo build --release -p nt4-bench)
  nt4-native       crates/nt4-bench --mode native (library only, no per-value user code)
  ntcore-listener  bench/ntcore_client.py --mode listener
  ntcore-poller    bench/ntcore_client.py --mode poller
  ntcore-native    bench/ntcore_client.py --mode native (C++ only, no per-value Python)

While a client runs, its CPU time (user + system) and peak memory (VmHWM)
are read from /proc, so this needs Linux. Each client gets a fresh server.

--scenario publish swaps the roles: the robot is the client under test,
publishing the same topics to an empty ntcore server, and an ntcore listener
(ntcore_client.py --mode listener) checks what arrives:

  nt4-robot        crates/nt4-bench --mode robot
  ntcore-robot     bench/robot_sim.py --connect (ntcore client, Python loop)

CPU and memory are then the robot's.

Written by Claude.
"""

import argparse
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def proc_stats(pid):
    """(cpu seconds, peak RSS in MiB) of a running process, from /proc."""
    with open(f"/proc/{pid}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    ticks = os.sysconf("SC_CLK_TCK")
    cpu = (int(fields[11]) + int(fields[12])) / ticks
    rss = 0.0
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            if line.startswith("VmHWM:"):
                rss = int(line.split()[1]) / 1024
    return cpu, rss


class Sampler:
    """Track a process's CPU time and peak memory while it runs."""

    def __init__(self, proc):
        self.proc, self.cpu, self.rss, self.start = proc, 0.0, 0.0, time.monotonic()

    def sample(self):
        try:
            c, r = proc_stats(self.proc.pid)
            # A sample taken as the process exits can read 0.
            self.cpu, self.rss = max(self.cpu, c), max(self.rss, r)
        except (FileNotFoundError, ProcessLookupError, IndexError):
            pass

    def into(self, result):
        wall = time.monotonic() - self.start
        # CPU over the whole run (connect, snapshot and measurement), as a
        # percentage of one core.
        result["cpu_pct"] = 100 * self.cpu / wall if wall else None
        result["peak_rss_mib"] = self.rss
        return result


def wait_sampling(proc, samplers):
    while proc.poll() is None:
        for s in samplers:
            s.sample()
        time.sleep(0.1)


def last_json(proc, name):
    out = proc.stdout.read().strip().splitlines()
    result = json.loads(out[-1]) if out else {"error": "no output"}
    result["client"] = name
    return result


def run_publish(name, robot_cmd, args, port):
    """Publish scenario: `robot_cmd` publishes to an empty ntcore server and
    an ntcore listener checks what arrives. CPU and memory are the robot's."""
    server = subprocess.Popen(
        [
            args.python,
            "-u",
            os.path.join(HERE, "robot_sim.py"),
            "--port",
            str(port),
            "--server-only",
            "--seconds",
            str(args.seconds + 90),
        ],
        stdout=subprocess.PIPE,
        text=True,
    )
    robot = None
    try:
        json.loads(server.stdout.readline())
        robot = subprocess.Popen(
            robot_cmd
            + ["--seconds", str(args.seconds + 45), "--scale", str(args.scale)],
            stdout=subprocess.PIPE,
            text=True,
        )
        topics = json.loads(robot.stdout.readline())["topics"]
        sampler = Sampler(robot)
        receiver = subprocess.Popen(
            [
                args.python,
                "-u",
                os.path.join(HERE, "ntcore_client.py"),
                "--mode",
                "listener",
                "--port",
                str(port),
                "--seconds",
                str(args.seconds),
                "--topics",
                str(topics),
            ],
            stdout=subprocess.PIPE,
            text=True,
        )
        wait_sampling(receiver, [sampler])
        result = last_json(receiver, name)
        result["topics"] = topics
        return sampler.into(result)
    finally:
        for p in (robot, server):
            if p is not None:
                p.terminate()
                p.wait()


def run_one(name, cmd, args, port):
    server = subprocess.Popen(
        [
            args.python,
            "-u",
            os.path.join(HERE, "robot_sim.py"),
            "--port",
            str(port),
            "--seconds",
            str(args.seconds + 60),
            "--scale",
            str(args.scale),
        ],
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        ready = json.loads(server.stdout.readline())
        topics = ready["topics"]
        time.sleep(0.5)
        client = subprocess.Popen(
            cmd
            + [
                "--port",
                str(port),
                "--seconds",
                str(args.seconds),
                "--topics",
                str(topics),
            ],
            stdout=subprocess.PIPE,
            text=True,
        )
        sampler = Sampler(client)
        wait_sampling(client, [sampler])
        result = last_json(client, name)
        result["topics"] = topics
        return sampler.into(result)
    finally:
        server.terminate()
        server.wait()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--python", default=sys.executable, help="a Python with pyntcore")
    ap.add_argument("--seconds", type=float, default=20)
    ap.add_argument("--scale", type=int, default=1)
    ap.add_argument("--port", type=int, default=5810)
    ap.add_argument("--json", help="also write the results here")
    ap.add_argument("--only", help="comma-separated client names")
    ap.add_argument("--scenario", choices=["receive", "publish"], default="receive")
    ap.add_argument(
        "--host",
        default="127.0.0.1",
        help="address the server listens on and clients connect to; use this "
        "machine's network address to measure ntcore's batched (non-local) path",
    )
    args = ap.parse_args()
    # Every program in bench/ reads the address from here.
    os.environ["NT4_BENCH_HOST"] = args.host

    bench = os.path.join(ROOT, "target", "release", "nt4-bench")
    if args.scenario == "publish":
        robots = {
            "nt4-robot": lambda port: [bench, "--mode", "robot", "--port", str(port)],
            "ntcore-robot": lambda port: [
                args.python,
                "-u",
                os.path.join(HERE, "robot_sim.py"),
                "--connect",
                str(port),
                "--port",
                "0",
            ],
        }
        if args.only:
            robots = {k: v for k, v in robots.items() if k in args.only.split(",")}
        results = []
        for i, (name, cmd) in enumerate(robots.items()):
            port = args.port + 2 * i
            r = run_publish(name, cmd(port), args, port)
            results.append(r)
            print(json.dumps(r), file=sys.stderr, flush=True)
        report(results, args)
        return

    clients = {
        "nt4": [bench],
        "nt4-native": [bench, "--mode", "native"],
        "ntcore-listener": [
            args.python,
            "-u",
            os.path.join(HERE, "ntcore_client.py"),
            "--mode",
            "listener",
        ],
        "ntcore-poller": [
            args.python,
            "-u",
            os.path.join(HERE, "ntcore_client.py"),
            "--mode",
            "poller",
        ],
        "ntcore-native": [
            args.python,
            "-u",
            os.path.join(HERE, "ntcore_client.py"),
            "--mode",
            "native",
        ],
    }
    if args.only:
        clients = {k: v for k, v in clients.items() if k in args.only.split(",")}

    results = []
    for i, (name, cmd) in enumerate(clients.items()):
        r = run_one(name, cmd, args, args.port + 2 * i)
        results.append(r)
        print(json.dumps(r), file=sys.stderr, flush=True)
    report(results, args)


def report(results, args):
    cols = [
        "client",
        "topics",
        "config_ok",
        "snapshot_ms",
        "rate",
        "lost",
        "out_of_order",
        "p50",
        "p99",
        "max",
        "cpu_pct",
        "peak_rss_mib",
    ]
    print("| " + " | ".join(cols) + " |")
    print("|" + "---|" * len(cols))
    for r in results:
        lat = r.get("latency_ms", {})
        row = {**r, "p50": lat.get("p50"), "p99": lat.get("p99"), "max": lat.get("max")}
        cells = []
        for c in cols:
            v = row.get(c)
            cells.append(f"{v:.1f}" if isinstance(v, float) else str(v))
        print("| " + " | ".join(cells) + " |")
    if args.json:
        with open(args.json, "w") as f:
            json.dump(results, f, indent=2)


if __name__ == "__main__":
    main()
