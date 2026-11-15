# nt4-fuzz: differential fuzzer

Differential fuzzer that runs nt4-proto's decoders and ntcore's own decoders
(from the pyntcore wheel) on the same generated frames and compares the
results. It also feeds every frame to `ClientCore` and to `ServerCore` (as a
client's frame) to check for panics.

## Building

Without `NT4_NTCORE_PYTHON` the fuzzer only runs nt4's own checks (no panics,
round trips). Set it to a Python interpreter with pyntcore 2027.0.0a8 installed
(`pip install --pre pyntcore==2027.0.0a8`)
to enable the oracle:

```bash
NT4_NTCORE_PYTHON=<python with pyntcore> cargo build --release -p nt4-fuzz
```

The oracle builds on Linux only. It links ntcore's own shared libraries
directly out of the pyntcore wheel. Build.rs finds them by running Python to locate `site-packages/native`,
and stops if the wheel is another version. The shim needs a C++23 compiler
(ntcore 2027's headers use `std::expected`).

## Running

```bash
# Default: all targets, 30 seconds, random seed
./target/release/nt4-fuzz

# Specific options
./target/release/nt4-fuzz --seconds 60 --seed 42
./target/release/nt4-fuzz --target binary       # binary|client-text|server-text
./target/release/nt4-fuzz --out /tmp/findings   # save shrunken failing inputs
```

Set `NT4_FUZZ_DEBUG` to see panics in nt4-proto on stderr.

CI runs it with the oracle for 60 seconds on every push (the `fuzz` job in
`.github/workflows/ci.yml`), seeded with the run id.

## What it checks

Three targets:

- **binary**: MessagePack frames (NT4 binary protocol). Checks that nt4-proto
  and ntcore produce the same decoded messages.
- **client-text**: JSON text frames from a client. Checks that nt4-proto and
  ntcore fire the same handler callbacks.
- **server-text**: JSON text frames from a server. Same as above, server side.

For each frame the fuzzer verifies:
1. nt4-proto does not panic.
2. `ClientCore` and `ServerCore` do not panic on the same frame.
3. nt4-proto's decoded output matches ntcore's output exactly (when the oracle
   is built in).

Outputs are compared in a canonical form: strings as hex, doubles and floats
as 64-bit or 32-bit bit patterns in hex, integers as decimal, so no precision
is lost through JSON.

## Known deviations

Cases where nt4 intentionally differs from ntcore 2027. The fuzzer counts
these separately and does not report them as mismatches.

| Case | nt4 behavior | ntcore behavior | Why |
| --- | --- | --- | --- |
| Text that isn't JSON but ntcore's parser takes: a `\u` escape that is half of a surrogate pair, a number too large for a double (`1e400`), a `\x41` escape | rejects the frame | accepts it (the number becomes infinity) | nt4 parses strict JSON; no real peer sends these |
| A `null` member right after a repeated key, `{"a":1,"a":2,"b":null}` | `b` is null | `b` is 2 | ntcore parser bug, see below |
| `unsubscribe` bad uid error text | "subuid out of range" | "pubuid out of range" | ntcore has a typo |

Everything else matches, including the things that are particular to ntcore
2027: JSON nesting stops at 20 levels, the first of a repeated key wins, `-0`
is the integer 0, an integer above `i64::MAX` is not a valid id, an
out-of-range `periodic` is rejected, and a binary timestamp too large to hold
in nanoseconds is rejected.

## ntcore bugs found

Both are in ntcore 2027 (alpha 8) and on allwpilib `main`.

- The JSON parser (wpiutil `json.cpp`) reuses one `value` variable for every
  member of an object. Reading `null` doesn't assign it, and a repeated key's
  value is dropped without clearing it, so a `null` member right after a
  repeated key gets the dropped value: `{"a":1,"a":2,"b":null}` reads as
  `{"a":1,"b":2}`.
- `unsubscribe` with a `subuid` out of range logs "pubuid out of range".

ntcore 2026.2.2 had more (a process exit on `[1e400]`, ids above `i64::MAX`
wrapping to negative ids, an unchecked `periodic`). ntcore 2027 has none of
those, and nt4 only tracks 2027 and later.

## Structure

```
oracle/
  oracle.cpp               # C shim over ntcore's WireDecodeBinary/WireDecodeText
  ntcore-v2027.0.0-alpha-8/  # ntcore internal headers from that allwpilib tag
src/
  lib.rs                   # check(), shrink(), generate(), known_deviation(), …
  generator.rs             # structure-aware frame generator (Rng, generate)
  main.rs                  # CLI runner: loop, stats, mismatch report
build.rs                   # builds oracle.cpp; links ntcore .so from pyntcore wheel
```
