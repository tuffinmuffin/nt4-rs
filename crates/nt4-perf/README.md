# nt4-perf: instruction-count benchmarks

Benchmarks for nt4's hot paths that count CPU instructions with Callgrind
(through [gungraun](https://github.com/gungraun/gungraun)) instead of timing
them. Counts are the same on every run of the same code, so CI can compare a
pull request with its base branch on a shared runner without timing noise.
Not published.

Each benchmark is one 20 ms loop of the simulated robot in `bench/` (200
topics, 150 of them updated every loop):

| Benchmark | What | Instructions (2026-11-15) |
| --- | --- | --- |
| `decode_binary` | decode 150 values from one binary frame | 82,000 |
| `encode_binary_batch` | encode 150 values into one binary frame | 42,000 |
| `decode_announce` | decode the server's announce frame for 200 topics | 2,126,000 |
| `encode_publish` | encode the client's publish frame for 200 topics | 399,000 |
| `client_receive` | `ClientCore` takes 150 values and hands out the events | 192,000 |
| `client_publish` | `ClientCore` sets 150 values and builds the frames to send | 1,246,000 |
| `server_relay` | `ServerCore` takes a robot's 150 values and builds the frames for a dashboard subscribed to all of them | (new; see the PR's `perf` job) |

## Running

Linux only (needs valgrind). The runner version must match the `gungraun`
version in `Cargo.toml`.

```bash
cargo install gungraun-runner --version 0.20.0 --locked
cargo bench -p nt4-perf
```

To compare a change with what you had before:

```bash
cargo bench -p nt4-perf -- --save-baseline=before
# ...make the change...
cargo bench -p nt4-perf -- --baseline=before --callgrind-limits='ir=10%'
```

## In CI

The `perf` job in `.github/workflows/ci.yml` runs on pull requests. It
benchmarks the base branch, then the PR. A benchmark that uses more than 10%
more instructions gives a warning on the PR; more than 50% fails the job. The `client_core` benchmarks move by about 0.02%
between runs (hash map seeds); everything else is exact.
