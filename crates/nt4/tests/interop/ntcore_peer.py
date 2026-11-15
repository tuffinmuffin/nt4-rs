"""A real ntcore (pyntcore) peer for nt4's interop tests.

Reads one JSON command per line on stdin and writes one JSON reply per line
on stdout. The Rust tests in tests/interop.rs drive it; run it by hand with
`python -u ntcore_peer.py` and type commands to try it.

Commands (the peer is an ntcore server or an ntcore client):
  {"cmd": "start_server", "port": 5810}
  {"cmd": "stop_server"}
  {"cmd": "start_client", "name": "dash", "port": 5810, "host": "127.0.0.1"}
  {"cmd": "stop_client"}
  {"cmd": "connected"}
  {"cmd": "time_offset"}
  {"cmd": "unpublish", "name": "/x"}
  {"cmd": "set_properties", "name": "/x", "properties": {"retained": true}}
  {"cmd": "properties", "name": "/x"}
  {"cmd": "topics", "prefix": "/"}
  {"cmd": "publish", "name": "/x", "type": "double", "value": 1.5}
  {"cmd": "subscribe", "name": "/x"}
  {"cmd": "get", "name": "/x", "timeout": 5, "not": <value to wait past>}
  {"cmd": "connections"}
  {"cmd": "now"}

Written by Claude. Uses the pyntcore 2027 API (snake_case names; times are
nanoseconds there, reported here in microseconds as on the wire).
"""

import gc
import json
import sys
import time

import ntcore

inst = ntcore.NetworkTableInstance.create()
publishers = {}
subscribers = {}

SETTERS = {
    "boolean": "set_boolean",
    "double": "set_double",
    "int": "set_integer",
    "float": "set_float",
    "string": "set_string",
    "raw": "set_raw",
    "boolean[]": "set_boolean_array",
    "double[]": "set_double_array",
    "int[]": "set_integer_array",
    "float[]": "set_float_array",
    "string[]": "set_string_array",
}


def to_json(v):
    if isinstance(v, (bytes, bytearray)):
        return list(v)
    if isinstance(v, (list, tuple)):
        return [to_json(x) for x in v]
    return v


def subscriber(name):
    if name not in subscribers:
        subscribers[name] = inst.get_topic(name).generic_subscribe(
            options=ntcore.PubSubOptions(send_all=False)
        )
    return subscribers[name]


def handle(cmd):
    c = cmd["cmd"]
    if c == "start_server":
        inst.start_server("", "127.0.0.1", "", cmd["port"])
        return {"ok": True}
    if c == "stop_server":
        inst.stop_server()
        return {"ok": True}
    if c == "start_client":
        inst.start_client(cmd["name"])
        inst.set_server(cmd.get("host", "127.0.0.1"), cmd["port"])
        return {"ok": True}
    if c == "stop_client":
        inst.stop_client()
        return {"ok": True}
    if c == "connected":
        return {"ok": True, "connected": inst.is_connected()}
    if c == "time_offset":
        offset = inst.get_server_time_offset()
        return {"ok": True, "offset": None if offset is None else offset // 1000}
    if c == "unpublish":
        # pyntcore 2027 publishers have no close(); they unpublish when freed.
        del publishers[cmd["name"]]
        gc.collect()
        return {"ok": True}
    if c == "set_properties":
        inst.get_topic(cmd["name"]).set_properties(cmd["properties"])
        return {"ok": True}
    if c == "properties":
        return {"ok": True, "properties": inst.get_topic(cmd["name"]).get_properties()}
    if c == "topics":
        names = [t.get_name() for t in inst.get_topics(cmd.get("prefix", ""))]
        return {"ok": True, "names": sorted(names)}
    if c == "publish":
        name, ty = cmd["name"], cmd["type"]
        if name not in publishers:
            publishers[name] = inst.get_topic(name).generic_publish(ty)
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
            if v.is_valid() and ("not" not in cmd or to_json(v.value()) != cmd["not"]):
                return {
                    "ok": True,
                    "type": sub.get_topic().get_type_string(),
                    "value": to_json(v.value()),
                    "time": v.time() // 1000,
                    "server_time": v.server_time() // 1000,
                }
            time.sleep(0.01)
        return {"ok": False, "error": "timeout"}
    if c == "connections":
        return {"ok": True, "names": [ci.remote_id for ci in inst.get_connections()]}
    if c == "now":
        return {"ok": True, "now": ntcore._now() // 1000}
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
