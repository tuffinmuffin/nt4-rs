//! Trajectory samples (wpimath's `trajectory` structs), as path
//! followers and planners publish them.

use nt4::structs::NtStruct;

use crate::{ChassisAccelerations, ChassisVelocities, Pose2d};

/// One sample of a holonomic (swerve or mecanum) trajectory
/// (`HolonomicSample`). Velocity and acceleration are field-relative here,
/// unlike a robot's own [`ChassisVelocities`].
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct HolonomicSample {
    /// Seconds from the start of the trajectory.
    pub time: f64,
    /// The robot's pose on the field.
    pub pose: Pose2d,
    /// Field-relative velocity.
    pub velocity: ChassisVelocities,
    /// Field-relative acceleration.
    pub acceleration: ChassisAccelerations,
}

/// One sample of a differential drive trajectory (`DifferentialSample`):
/// a [`HolonomicSample`]'s fields, then the wheel velocities.
///
/// Its schema writes the `HolonomicSample` fields out, but registers
/// `HolonomicSample` as its nested struct, as wpimath does.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
#[nt4(nested(HolonomicSample))]
pub struct DifferentialSample {
    /// Seconds from the start of the trajectory.
    pub time: f64,
    /// The robot's pose on the field.
    pub pose: Pose2d,
    /// Field-relative velocity.
    pub velocity: ChassisVelocities,
    /// Field-relative acceleration.
    pub acceleration: ChassisAccelerations,
    /// Meters per second, left wheels.
    #[nt4(rename = "leftVelocity")]
    pub left_velocity: f64,
    /// Meters per second, right wheels.
    #[nt4(rename = "rightVelocity")]
    pub right_velocity: f64,
}
