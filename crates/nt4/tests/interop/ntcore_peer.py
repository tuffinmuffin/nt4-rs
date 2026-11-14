"""A real ntcore (pyntcore) peer for nt4's interop tests.

Reads one JSON command per line on stdin and writes one JSON reply per line
on stdout. The Rust tests in tests/interop.rs drive it; run it by hand with
`python -u ntcore_peer.py` and type commands to try it.

Commands:
  {"cmd": "start_server", "port4": 5810, "port3": 1735}
  {"cmd": "stop_server"}
  {"cmd": "publish", "name": "/x", "type": "double", "value": 1.5}
  {"cmd": "subscribe", "name": "/x"}
  {"cmd": "get", "name": "/x", "timeout": 5, "not": <value to wait past>}
  {"cmd": "connections"}
  {"cmd": "now"}

Written by Claude.
"""

import json
import sys
import time

import ntcore

inst = ntcore.NetworkTableInstance.create()
publishers = {}
subscribers = {}

SETTERS = {
    "boolean": "setBoolean",
    "double": "setDouble",
    "int": "setInteger",
    "float": "setFloat",
    "string": "setString",
    "raw": "setRaw",
    "boolean[]": "setBooleanArray",
    "double[]": "setDoubleArray",
    "int[]": "setIntegerArray",
    "float[]": "setFloatArray",
    "string[]": "setStringArray",
}


def to_json(v):
    if isinstance(v, (bytes, bytearray)):
        return list(v)
    if isinstance(v, (list, tuple)):
        return [to_json(x) for x in v]
    return v


def subscriber(name):
    if name not in subscribers:
        subscribers[name] = inst.getTopic(name).genericSubscribe(
            ntcore.PubSubOptions(sendAll=False)
        )
    return subscribers[name]


def handle(cmd):
    c = cmd["cmd"]
    if c == "start_server":
        inst.startServer("", "127.0.0.1", cmd["port3"], cmd["port4"])
        return {"ok": True}
    if c == "stop_server":
        inst.stopServer()
        return {"ok": True}
    if c == "publish":
        name, ty = cmd["name"], cmd["type"]
        if name not in publishers:
            publishers[name] = inst.getTopic(name).genericPublish(ty)
        value = cmd["value"]
        if ty == "raw":
            value = bytes(value)
        getattr(publishers[name], SETTERS[ty])(value)
        return {"ok": True}
    if c == "subscribe":
        subscriber(cmd["name"])
        return {"ok": True}
    if c == "get":
        sub = subscriber(cmd["name"])
        deadline = time.monotonic() + cmd.get("timeout", 5)
        while time.monotonic() < deadline:
            v = sub.get()
            if v.isValid() and ("not" not in cmd or to_json(v.value()) != cmd["not"]):
                return {
                    "ok": True,
                    "type": sub.getTopic().getTypeString(),
                    "value": to_json(v.value()),
                    "time": v.time(),
                    "server_time": v.server_time(),
                }
            time.sleep(0.01)
        return {"ok": False, "error": "timeout"}
    if c == "connections":
        return {"ok": True, "names": [ci.remote_id for ci in inst.getConnections()]}
    if c == "now":
        return {"ok": True, "now": ntcore._now()}
    return {"ok": False, "error": f"unknown command {c}"}


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        reply = handle(json.loads(line))
    except Exception as e:  # report, don't die
        reply = {"ok": False, "error": repr(e)}
    print(json.dumps(reply), flush=True)
