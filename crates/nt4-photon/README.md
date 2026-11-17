# nt4-photon

[PhotonVision](https://photonvision.org) pipeline results as typed
NetworkTables topics, for the [`nt4`](https://docs.rs/nt4) crate: read a
camera's targets, tag poses and multi-tag pose from
`/photonvision/<camera>/rawBytes`, or publish results of your own that
robot code's photonlib accepts.

Unofficial: not affiliated with or endorsed by PhotonVision, WPILib or
FIRST.

**Not published yet.** It follows PhotonVision 2027's format, which only
PhotonVision's dev builds have so far. Use it from a clone of the
repository (a path or git dependency) until it is on crates.io.

```rust,no_run
# async fn demo() {
use nt4::{Client, ClientConfig};
use nt4_photon::{PhotonPipelineResult, result_topic};

let client = Client::start(ClientConfig::team("vision-reader", 3200));
let mut results = client.subscribe::<PhotonPipelineResult>(&result_topic("Front"), Default::default());
while let Ok(Some(result)) = results.changed().await {
    if let Some(best) = result.best_target() {
        println!("tag {} at yaw {:.1} deg", best.fiducial_id, best.yaw);
    }
}
# }
```

PhotonVision sends results in its own "photonstruct" format, whose
version is a hash in the topic type
(`photonstruct:PhotonPipelineResult:<hash>`). This release has
PhotonVision 2027's format (`fd353b11...`). A coprocessor on another
version publishes another hash; `nt4_photon::check_version` compares a
topic's `message_uuid` property with ours. Geometry is
[`nt4-wpimath`](https://docs.rs/nt4-wpimath)'s `Transform3d`.

Tested against a real PhotonVision 2027 build (test mode, with its 2026
field image): its results decode to the same best target it publishes on
its own `targetYaw` / `targetPose` topics, and encode back to the same
bytes.

To report a format mismatch or ask for something,
[open an issue](https://github.com/tuffinmuffin/nt4-rs/issues/new).

License: MIT. PhotonVision's message definitions are from its MIT-licensed
`photon-lib`.
