# nt4-wpimath

WPILib's `wpimath` geometry and kinematics types as NetworkTables struct
topics, for the [`nt4`](https://docs.rs/nt4) crate. Each type has the same
name, fields, units and struct schema as WPILib 2027's `wpimath`, so robot
code, AdvantageScope and Elastic read them as their own.

Unofficial: not affiliated with or endorsed by WPILib or FIRST.

```rust,no_run
# async fn demo(client: nt4::Client) {
use nt4_wpimath::{Pose2d, SwerveModuleVelocity};
let pose = client.publish::<Pose2d>("/vision/pose", Default::default());
let modules = client.publish::<Vec<SwerveModuleVelocity>>("/drive/modules", Default::default());
pose.set(Pose2d::default());
# }
```

The types are plain data: angles in radians, distances in meters, speeds
per second, and no math. Convert to your own geometry types as needed.

This crate is built only on `nt4`'s public `NtStruct` trait, and follows
WPILib's yearly renames (`ChassisSpeeds` became `ChassisVelocities` in
2027) on its own. This release matches WPILib 2027; the changelog says
which WPILib year each release follows.

From a clone of the [repository](https://github.com/tuffinmuffin/nt4-rs):

| Example | What it does |
| --- | --- |
| `cargo run -p nt4-wpimath --example struct_topic -- 127.0.0.1` | publishes a `Pose2d` driving in a circle and four swerve module states, with their schemas |


License: MIT.
