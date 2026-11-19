# nt4-rs

A NetworkTables 4.1 library for Rust, with both a client and a server, so Rust
programs can talk to WPILib robot code, dashboards and coprocessors.

Unofficial: not affiliated with or endorsed by WPILib or FIRST. It is tested
against WPILib's ntcore (see [Conformance testing](#conformance-testing)).

NetworkTables (NT) is how FRC robot code, dashboards and coprocessors share
values. NT4 runs over a WebSocket: JSON text frames for control messages,
MessagePack binary frames for values. The protocol is described in
[ntcore's networktables4.adoc](https://github.com/wpilibsuite/allwpilib/blob/main/ntcore/doc/networktables4.adoc).

## Status

Client and server are done and tested against ntcore, and published on
crates.io ([CHANGELOG.md](CHANGELOG.md) lists each release). Current state
and next steps are in
[docs/STATUS.md](docs/STATUS.md).

| Crate | What it does |
| --- | --- |
| [`nt4`](crates/nt4) | The library most programs want: tokio client (`client` feature, on by default) and server (`server` feature), typed topics and structs |
| [`nt4-wpimath`](crates/nt4-wpimath) | WPILib's wpimath types (`Pose2d`, `Pose3d`, swerve module states, ...) as struct topics for `nt4` |
| [`nt4-photon`](crates/nt4-photon) | PhotonVision pipeline results (targets, tag poses, multi-tag pose) as typed topics for `nt4`. In progress, not published yet |
| [`nt4-derive`](crates/nt4-derive) | `#[derive(NtStruct)]`, used through `nt4`'s `derive` feature |
| [`nt4-core`](crates/nt4-core) | Building block: client and server state machines with no sockets or clock (sans-I/O), for other runtimes and WASM |
| [`nt4-proto`](crates/nt4-proto) | Building block: JSON and MessagePack encode and decode, value and type definitions, struct schemas. No I/O |

`nt4` re-exports the two building blocks as `nt4::core` and `nt4::proto`.

The full plan, including how conformance is tested, is in the
[project plan doc](https://claude.ai/artifact/R1JvYBZZYPm6HuUuFM9QPh).

## Conformance testing

The tests are checked against WPILib's own ntcore, in three layers:

1. **Ported ntcore tests.** `crates/nt4-proto/tests/wire_encoder.rs` and
   `wire_decoder.rs` port every case of ntcore's `WireEncoderTest` and
   `WireDecoderTest`, with the same bytes, JSON and warning text.
   `crates/nt4-core/tests/` ports `NetworkOutgoingQueueTest` and the client
   cases of `TimeSyncTest`. Each test names the upstream case it ports in an
   `// upstream: "..."` comment.
2. **Upstream audit.** `scripts/upstream_audit.py <allwpilib checkout>` lists
   upstream cases that aren't ported yet. CI runs it against the pinned
   allwpilib commit (blocking) and against allwpilib `main` (warning only).
3. **Interop with real ntcore.** `crates/nt4/tests/interop.rs` runs our
   client against an ntcore server from the `pyntcore` Python package
   (connect, time sync, every value type both ways, prefix subscriptions,
   reconnect after a server restart). CI runs it on Linux, Windows and macOS.
   Locally it needs `NT4_INTEROP_PYTHON` pointing at a Python with
   `pyntcore` installed, and it skips without it. From milestone 5, CI also
   runs ntcore clients against our server.

## Performance

`bench/` runs nt4 and ntcore against a simulated robot (200 topics: one-time
values, values set every 20 ms, and a busy loop), up to 1,640 topics and
155,000 values/s. No run lost or reordered an update. With no per-value code
in either, nt4 used less CPU than C++ ntcore (3.8% against 4.7% of a core at
1,640 topics) and about 5 MiB of memory. See [bench/README.md](bench/README.md)
for the setup and full results.

## Using the client

```toml
[dependencies]
nt4 = { git = "https://github.com/tuffinmuffin/nt4-rs" }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time"] }
```

```rust,no_run
use nt4::{Client, ClientConfig};

#[tokio::main]
async fn main() {
    // Tries 10.32.0.2, the mDNS names and USB, like ntcore's SetServerTeam.
    let client = Client::start(ClientConfig::team("my-dashboard", 3200));
    let mut speed = client.subscribe::<f64>("/SmartDashboard/speed", Default::default());
    let heartbeat = client.publish::<i64>("/dashboard/heartbeat", Default::default());
    let mut beats = 0;
    while let Ok(v) = speed.changed().await {
        beats += 1;
        heartbeat.set(beats);
        println!("speed = {v:?}");
    }
}
```

Publishers and subscribers work before the first connection and survive
reconnects; drop one to unpublish or unsubscribe. `client.events()` gives
every announcement, value and time sync change.

Under the hood, `nt4-core`'s `ClientCore` is the protocol with no socket and
no clock, so it can be driven from any runtime or from WASM.

## Releasing

The five published crates share one version and are published together by
CI. A release is one button and one merge:

1. While working, add what changed to the `## Unreleased` section at the
   top of `CHANGELOG.md` (create it if there isn't one).
2. **Actions > Prepare release > Run workflow**, and type the version
   (`0.4.0`; `0.4.0-alpha.1` makes a pre-release). It runs
   `scripts/prepare_release.py`, which sets the version (all in the root
   `Cargo.toml`: the workspace version and the published crates'
   `[workspace.dependencies]`), renames `## Unreleased` to
   `## <version> (<date>)` and updates the READMEs' install lines, then
   opens a PR "Release <version>". It refuses a version that isn't newer,
   or an empty or missing Unreleased section.
3. Merge the PR. The merge's push to `main` starts `release.yml`, which
   sees a version with no tag yet: it waits for CI to pass on that commit,
   checks the changelog and a `cargo publish --dry-run`, tags the commit
   `v<version>`, publishes `nt4-proto`, `nt4-core`,
   `nt4-derive`, `nt4` and `nt4-wpimath` in that order (with a short-lived
   token from crates.io trusted publishing: each crate trusts this
   repository's `release.yml` in the `Deploy` environment), and creates the
   GitHub release from the changelog section.

GitHub doesn't run CI on a PR a workflow opens, so the release PR shows no
checks: CI runs on `main` after the merge, and nothing is published unless
it passes. Opening the PR needs **Settings > Actions > General > Allow GitHub
Actions to create and approve pull requests**; without it the workflow
pushes the branch and links to opening the PR by hand.

The script also runs locally (`python3 scripts/prepare_release.py 0.4.0`,
then `cargo update --workspace`) if you'd rather make the release PR
yourself; merging it releases the same way. Pushing a tag `v<version>` on a
commit on `main` still works too.

If publishing fails partway, run it again with **Actions > Release > Run
workflow**: it releases the version on `main`, from the commit its tag
points at if the tag was already made, and skips crates already on
crates.io at that version. (crates.io's trusted publishing only accepts
runs started by a push or by Run workflow, so `release.yml` is started by
the merge's push and waits for CI, rather than being started by CI
finishing.) A published version can't be
changed or reused (only yanked), so fix forward with the next patch
version. CI runs `cargo publish --dry-run` on every push, so packaging
problems show up before a release.

## Development

```bash
cargo test --workspace
# interop tests (optional): python -m pip install --pre pyntcore==2027.0.0a8
NT4_INTEROP_PYTHON=$(which python) cargo test -p nt4 --test interop
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT, see [LICENSE](LICENSE). Test vectors ported from allwpilib stay under
WPILib's BSD license; see
`LICENSE-WPILIB.md` in each crate's `tests/` folder.
