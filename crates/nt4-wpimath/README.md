# nt4-wpimath

WPILib's `wpimath` geometry, kinematics, trajectory and shape types as
NetworkTables struct topics, for the [`nt4`](https://docs.rs/nt4) crate. Each type has the same
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

## Which types

- Geometry: `Translation2d` to `Twist3d`, `Quaternion`.
- Drive state: `ChassisVelocities`, `ChassisAccelerations`, swerve module
  positions, velocities and accelerations, differential and mecanum wheel
  positions, velocities and accelerations, and `SwerveDriveKinematics<N>`.
- Trajectory samples: `HolonomicSample`, `DifferentialSample`.
- Shapes: `Rectangle2d`, `Ellipse2d`.

They're in modules by group (`nt4_wpimath::kinematics`, ...) and all
re-exported at the top. CI checks every type here against WPILib's own
definitions (C++, Java and RobotPy) at a pinned allwpilib commit.

Not every WPILib struct is here, on purpose. Controller gains, motor
models, profile and spline internals, matrices and `ControlWord` are
rarely sent from Rust, and each type has to follow WPILib's renames. The
full list, with a reason for each, is
[`scripts/schema_audit.txt`](https://github.com/tuffinmuffin/nt4-rs/blob/main/scripts/schema_audit.txt).

For a type that isn't here, you don't have to wait:

- To read one, decode it at run time from its `/.schema/` topic with
  [`nt4::proto::structs`](https://docs.rs/nt4-proto/latest/nt4_proto/structs/),
  which handles any struct WPILib sends.
- To publish or read it typed, derive it yourself with matching field
  names, types and order (`nt4`'s `derive` feature):

```rust
use nt4::structs::NtStruct;

/// WPILib's `DifferentialDriveKinematics`: schema `double trackwidth`.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
struct DifferentialDriveKinematics {
    trackwidth: f64,
}

assert_eq!(DifferentialDriveKinematics::schema(), "double trackwidth");
```

To have a type added here, or to report one whose schema doesn't match
WPILib's, [open an issue](https://github.com/tuffinmuffin/nt4-rs/issues/new?template=struct-type.md)
with the WPILib type name and what you use it for.

## Examples

From a clone of the [repository](https://github.com/tuffinmuffin/nt4-rs):

| Example | What it does |
| --- | --- |
| `cargo run -p nt4-wpimath --example struct_topic -- 127.0.0.1` | publishes a `Pose2d` driving in a circle and four swerve module states, with their schemas |

License: MIT.
