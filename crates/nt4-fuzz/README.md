# nt4-fuzz: differential fuzzer

Differential fuzzer that runs nt4-proto's decoders and ntcore's own decoders
(from the pyntcore wheel) on the same generated frames and compares the
results. It also feeds every frame to `ClientCore` to check for panics.

## Building

Without `NT4_NTCORE_PYTHON` the fuzzer only runs nt4's own checks (no panics,
round trips). Set it to a Python interpreter with pyntcore 2026.2.2 installed
to enable the oracle:

```bash
NT4_NTCORE_PYTHON=<python with pyntcore> cargo build --release -p nt4-fuzz
```

The oracle links ntcore's own shared libraries directly out of the pyntcore
wheel. Build.rs finds them by running Python to locate `site-packages/native`.

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

## What it checks

Three targets:

- **binary**: MessagePack frames (NT4 binary protocol). Checks that nt4-proto
  and ntcore produce the same decoded messages.
- **client-text**: JSON text frames from a client. Checks that nt4-proto and
  ntcore fire the same handler callbacks.
- **server-text**: JSON text frames from a server. Same as above, server side.

For each frame the fuzzer verifies:
1. nt4-proto does not panic.
2. `ClientCore` does not panic on the same frame.
3. nt4-proto's decoded output matches ntcore's output exactly (when the oracle
   is built in).

Outputs are compared in a canonical form: strings as hex, doubles and floats
as 64-bit or 32-bit bit patterns in hex, integers as decimal, so no precision
is lost through JSON.

## Known deviations

Cases where nt4 intentionally differs from ntcore:

| Case | nt4 behavior | ntcore behavior | Why |
| --- | --- | --- | --- |
| JSON nested > 128 levels | rejects | accepts | Stack overflow risk; serde_json's default limit |
| `periodic` out of range (negative, huge, ∞) | rejects with warning | accepts; converts to unsigned ms (UB) | Undefined behavior in ntcore |
| Integers above `i64::MAX` | rejects as "out of range" | wraps (two's complement) | Wrapping is unspecified behavior in C++ |
| `unsubscribe` bad uid error text | "subuid out of range" | "pubuid out of range" | ntcore has a typo |

## ntcore bug found

Numbers like `1e400` or `-1e400` in JSON text frames cause ntcore to crash
with an unhandled C++ exception (`json.exception.out_of_range`). The pyntcore
wheel's `WireDecodeText` throws `std::out_of_range` from nlohmann JSON's
integer overflow path and does not catch it, which terminates the process.

Reproduced by sending `[{"method":"announce","params":{"name":"","id":1,"type":"","pubuid":1e400,"properties":{}}}]`
to an ntcore server. nt4-proto handles these frames without crashing.

This should be reported to WPILib (allwpilib ntcore).

## Structure

```
oracle/
  oracle.cpp               # C shim over ntcore's WireDecodeBinary/WireDecodeText
  ntcore-v2026.2.2/        # ntcore internal headers from allwpilib v2026.2.2
src/
  lib.rs                   # check(), shrink(), generate(), known_deviation(), …
  generator.rs             # structure-aware frame generator (Rng, generate)
  main.rs                  # CLI runner: loop, stats, mismatch report
build.rs                   # builds oracle.cpp; links ntcore .so from pyntcore wheel
```
