//! Drive state: chassis and module velocities, accelerations and
//! positions, and swerve kinematics (wpimath's `kinematics` structs).

use std::borrow::Cow;

use nt4::structs::{NtStruct, StructField, StructReader};

use crate::{Rotation2d, Translation2d};

/// Robot-relative chassis velocities (`ChassisVelocities`, called
/// `ChassisSpeeds` before 2027).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct ChassisVelocities {
    /// Meters per second forward.
    pub vx: f64,
    /// Meters per second left.
    pub vy: f64,
    /// Radians per second counterclockwise.
    pub omega: f64,
}

/// A swerve module's distance and angle (`SwerveModulePosition`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct SwerveModulePosition {
    /// Meters driven.
    pub distance: f64,
    /// Module angle.
    pub angle: Rotation2d,
}

/// A swerve module's velocity and angle (`SwerveModuleVelocity`,
/// called `SwerveModuleState` before 2027).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct SwerveModuleVelocity {
    /// Meters per second.
    pub velocity: f64,
    /// Module angle.
    pub angle: Rotation2d,
}

/// Robot-relative chassis accelerations (`ChassisAccelerations`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct ChassisAccelerations {
    /// Meters per second squared forward.
    pub ax: f64,
    /// Meters per second squared left.
    pub ay: f64,
    /// Radians per second squared counterclockwise.
    pub alpha: f64,
}

/// A swerve module's acceleration and its direction
/// (`SwerveModuleAcceleration`; WPILib's struct name has an `s`,
/// `SwerveModuleAccelerations`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
#[nt4(name = "SwerveModuleAccelerations")]
pub struct SwerveModuleAcceleration {
    /// Meters per second squared, of the wheel.
    pub acceleration: f64,
    /// Direction of the acceleration.
    pub angle: Rotation2d,
}

/// Distance driven by each side of a differential drive
/// (`DifferentialDriveWheelPositions`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct DifferentialDriveWheelPositions {
    /// Meters, left side.
    pub left: f64,
    /// Meters, right side.
    pub right: f64,
}

/// Wheel velocities of a differential drive
/// (`DifferentialDriveWheelVelocities`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct DifferentialDriveWheelVelocities {
    /// Meters per second, left side.
    pub left: f64,
    /// Meters per second, right side.
    pub right: f64,
}

/// Wheel accelerations of a differential drive
/// (`DifferentialDriveWheelAccelerations`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct DifferentialDriveWheelAccelerations {
    /// Meters per second squared, left side.
    pub left: f64,
    /// Meters per second squared, right side.
    pub right: f64,
}

/// Distance driven by each wheel of a mecanum drive
/// (`MecanumDriveWheelPositions`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct MecanumDriveWheelPositions {
    /// Meters.
    pub front_left: f64,
    /// Meters.
    pub front_right: f64,
    /// Meters.
    pub rear_left: f64,
    /// Meters.
    pub rear_right: f64,
}

/// Wheel velocities of a mecanum drive (`MecanumDriveWheelVelocities`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct MecanumDriveWheelVelocities {
    /// Meters per second.
    pub front_left: f64,
    /// Meters per second.
    pub front_right: f64,
    /// Meters per second.
    pub rear_left: f64,
    /// Meters per second.
    pub rear_right: f64,
}

/// Wheel accelerations of a mecanum drive
/// (`MecanumDriveWheelAccelerations`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct MecanumDriveWheelAccelerations {
    /// Meters per second squared.
    pub front_left: f64,
    /// Meters per second squared.
    pub front_right: f64,
    /// Meters per second squared.
    pub rear_left: f64,
    /// Meters per second squared.
    pub rear_right: f64,
}

/// A swerve drive's module positions (`SwerveDriveKinematics<N>`), for
/// `N` modules: struct `SwerveDriveKinematics__<N>`.
///
/// ```
/// use nt4::structs::NtStruct;
/// use nt4_wpimath::{SwerveDriveKinematics, Translation2d};
///
/// let half = 0.3;
/// let kinematics = SwerveDriveKinematics {
///     modules: [
///         Translation2d { x: half, y: half },
///         Translation2d { x: half, y: -half },
///         Translation2d { x: -half, y: half },
///         Translation2d { x: -half, y: -half },
///     ],
/// };
/// assert_eq!(SwerveDriveKinematics::<4>::type_name(), "SwerveDriveKinematics__4");
/// assert_eq!(SwerveDriveKinematics::<4>::schema(), "Translation2d modules[4]");
/// # let _ = kinematics;
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwerveDriveKinematics<const N: usize> {
    /// Each module's position relative to the robot's center, in the
    /// order the robot code gives them.
    pub modules: [Translation2d; N],
}

impl<const N: usize> Default for SwerveDriveKinematics<N> {
    fn default() -> Self {
        Self {
            modules: [Translation2d::default(); N],
        }
    }
}

// By hand: the derive doesn't take generic types.
impl<const N: usize> NtStruct for SwerveDriveKinematics<N> {
    const SIZE: usize = <[Translation2d; N] as StructField>::SIZE;
    fn type_name() -> Cow<'static, str> {
        format!("SwerveDriveKinematics__{N}").into()
    }
    fn schema() -> Cow<'static, str> {
        format!("Translation2d modules[{N}]").into()
    }
    fn pack(&self, out: &mut Vec<u8>) {
        StructField::pack(&self.modules, out);
    }
    fn unpack(data: &[u8]) -> Self {
        Self {
            modules: StructReader::new(data).read(),
        }
    }
    fn for_each_nested(f: &mut dyn FnMut(&str, &str)) {
        Translation2d::for_each_struct_schema(f);
    }
}
