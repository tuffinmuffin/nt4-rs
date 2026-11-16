#!/usr/bin/env python3
"""Check that every ntcore (and wpiutil struct) test case we track has been ported.

Usage: python3 scripts/upstream_audit.py <path to allwpilib checkout>

Reads the Catch2 test names from the ntcore test files listed in TRACKED,
then the `// upstream: "<name>"` markers in this repo's tests, plus the
cases listed as not applicable in scripts/upstream_na.txt. Prints anything
upstream that we don't cover, and anything we cite that upstream no longer
has, and exits non-zero if either list is not empty.

Written by Claude.
"""

import pathlib
import re
import sys

# ntcore test files whose cases this repo ports, relative to allwpilib.
TRACKED = [
    "ntcore/src/test/native/cpp/net/WireEncoderTest.cpp",
    "ntcore/src/test/native/cpp/net/WireDecoderTest.cpp",
    "ntcore/src/test/native/cpp/net/NetworkOutgoingQueueTest.cpp",
    "ntcore/src/test/native/cpp/TimeSyncTest.cpp",
    "ntcore/src/test/native/cpp/server/ServerImplTest.cpp",
    "ntcore/src/test/native/cpp/NetworkServerTest.cpp",
    "ntcore/src/test/native/cpp/StructTest.cpp",
    "wpiutil/src/test/native/cpp/struct/SchemaParserTest.cpp",
    "wpiutil/src/test/native/cpp/struct/DynamicStructTest.cpp",
]

REPO = pathlib.Path(__file__).resolve().parent.parent
# The name may be split into adjacent string literals ("a " "b"), which C++
# joins; capture all of them.
CASE = re.compile(r'TEST_CASE(?:_METHOD)?\s*\((?:[^,"]*,)?\s*((?:"[^"]*"\s*)+)', re.S)
LITERAL = re.compile(r'"([^"]*)"')
MARKER = re.compile(r'//\s*upstream:\s*"([^"]+)"')


def upstream_cases(allwpilib: pathlib.Path) -> set[str]:
    names = set()
    for rel in TRACKED:
        for literals in CASE.findall((allwpilib / rel).read_text()):
            names.add("".join(LITERAL.findall(literals)))
    return names


def ported_cases() -> set[str]:
    names = set()
    for path in REPO.glob("crates/*/tests/*.rs"):
        names |= set(MARKER.findall(path.read_text()))
    na = REPO / "scripts" / "upstream_na.txt"
    for line in na.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            names.add(line)
    return names


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    upstream = upstream_cases(pathlib.Path(sys.argv[1]))
    ours = ported_cases()
    missing = sorted(upstream - ours)
    stale = sorted(ours - upstream)
    print(f"{len(upstream)} upstream cases, {len(upstream & ours)} covered")
    for name in missing:
        print(f"NOT PORTED: {name}")
    for name in stale:
        print(f"NOT UPSTREAM (renamed or removed?): {name}")
    return 1 if missing or stale else 0


if __name__ == "__main__":
    sys.exit(main())
