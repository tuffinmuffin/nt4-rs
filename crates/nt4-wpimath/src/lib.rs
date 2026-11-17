//! WPILib's `wpimath` types as [`NtStruct`]s, for the [`nt4`] crate.
//!
//! Unofficial: not affiliated with or endorsed by WPILib or FIRST.
//!
//! Plain data with the same names, fields, units and struct schemas as
//! WPILib 2027's `wpimath`, so robot code, AdvantageScope and Elastic read
//! them as their own. Angles are radians, distances meters, speeds per
//! second. There is no math here; convert to your own geometry types as
//! needed.
//!
//! ```
//! use nt4_wpimath::{Pose2d, Rotation2d, Translation2d};
//! let pose = Pose2d {
//!     translation: Translation2d { x: 1.0, y: 2.0 },
//!     rotation: Rotation2d { value: std::f64::consts::FRAC_PI_2 },
//! };
//! // client.publish::<Pose2d>("/vision/pose", Default::default()).set(pose);
//! # let _ = pose;
//! ```

use std::borrow::Cow;

use nt4::structs::{NtStruct, StructField, StructReader};

// Compile and run the code in README.md as doc tests.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;

/// A 2D position (`Translation2d`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Translation2d {
    /// Meters.
    pub x: f64,
    /// Meters.
    pub y: f64,
}

/// A 2D rotation (`Rotation2d`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Rotation2d {
    /// Radians, counterclockwise.
    pub value: f64,
}

/// A 2D position and heading (`Pose2d`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose2d {
    /// Position.
    pub translation: Translation2d,
    /// Heading.
    pub rotation: Rotation2d,
}

/// A 2D change of pose (`Transform2d`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Transform2d {
    /// Change in position.
    pub translation: Translation2d,
    /// Change in heading.
    pub rotation: Rotation2d,
}

/// A 2D motion along an arc (`Twist2d`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Twist2d {
    /// Meters forward.
    pub dx: f64,
    /// Meters left.
    pub dy: f64,
    /// Radians.
    pub dtheta: f64,
}

/// A 3D position (`Translation3d`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Translation3d {
    /// Meters.
    pub x: f64,
    /// Meters.
    pub y: f64,
    /// Meters.
    pub z: f64,
}

/// A quaternion (`Quaternion`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Quaternion {
    /// Real part.
    pub w: f64,
    /// i part.
    pub x: f64,
    /// j part.
    pub y: f64,
    /// k part.
    pub z: f64,
}

/// A 3D rotation (`Rotation3d`), as a unit quaternion.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Rotation3d {
    /// The rotation.
    pub q: Quaternion,
}

/// A 3D position and orientation (`Pose3d`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose3d {
    /// Position.
    pub translation: Translation3d,
    /// Orientation.
    pub rotation: Rotation3d,
}

/// A 3D change of pose (`Transform3d`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Transform3d {
    /// Change in position.
    pub translation: Translation3d,
    /// Change in orientation.
    pub rotation: Rotation3d,
}

/// A 3D motion along an arc (`Twist3d`).
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Twist3d {
    /// Meters.
    pub dx: f64,
    /// Meters.
    pub dy: f64,
    /// Meters.
    pub dz: f64,
    /// Radians about x.
    pub rx: f64,
    /// Radians about y.
    pub ry: f64,
    /// Radians about z.
    pub rz: f64,
}

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

#[cfg(test)]
mod tests {
    // Written by Claude.
    use super::*;
    use nt4::structs::check;

    #[test]
    fn layouts_match_schemas() {
        check::<Translation2d>().unwrap();
        check::<Rotation2d>().unwrap();
        check::<Pose2d>().unwrap();
        check::<Transform2d>().unwrap();
        check::<Twist2d>().unwrap();
        check::<Translation3d>().unwrap();
        check::<Quaternion>().unwrap();
        check::<Rotation3d>().unwrap();
        check::<Pose3d>().unwrap();
        check::<Transform3d>().unwrap();
        check::<Twist3d>().unwrap();
        check::<ChassisVelocities>().unwrap();
        check::<SwerveModulePosition>().unwrap();
        check::<SwerveModuleVelocity>().unwrap();
    }

    #[test]
    fn swerve_drive_kinematics() {
        check::<SwerveDriveKinematics<4>>().unwrap();
        check::<SwerveDriveKinematics<3>>().unwrap();
        assert_eq!(<SwerveDriveKinematics<4> as NtStruct>::SIZE, 64);
        assert_eq!(
            SwerveDriveKinematics::<3>::type_name(),
            "SwerveDriveKinematics__3"
        );
        let mut seen = Vec::new();
        SwerveDriveKinematics::<4>::for_each_struct_schema(&mut |ty, _| seen.push(ty.to_owned()));
        assert_eq!(
            seen,
            ["struct:Translation2d", "struct:SwerveDriveKinematics__4"]
        );
        let k = SwerveDriveKinematics {
            modules: [
                Translation2d { x: 1.0, y: 2.0 },
                Translation2d { x: 3.0, y: 4.0 },
            ],
        };
        let mut out = Vec::new();
        NtStruct::pack(&k, &mut out);
        let expected: Vec<u8> = [1.0f64, 2.0, 3.0, 4.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(out, expected);
        assert_eq!(<SwerveDriveKinematics<2> as NtStruct>::unpack(&out), k);
    }

    #[test]
    fn sizes_match_wpilib() {
        // GetSize() in wpimath's *Struct.hpp.
        assert_eq!(<Pose2d as NtStruct>::SIZE, 24);
        assert_eq!(<Pose3d as NtStruct>::SIZE, 56);
        assert_eq!(<Twist3d as NtStruct>::SIZE, 48);
        assert_eq!(<SwerveModuleVelocity as NtStruct>::SIZE, 16);
    }

    #[test]
    fn nested_schemas_come_first_in_wpilib_order() {
        let mut seen = Vec::new();
        Pose3d::for_each_struct_schema(&mut |ty, _| seen.push(ty.to_owned()));
        assert_eq!(
            seen,
            [
                "struct:Translation3d",
                "struct:Quaternion",
                "struct:Rotation3d",
                "struct:Pose3d"
            ]
        );
    }

    #[test]
    fn packs_little_endian_in_field_order() {
        let pose = Pose2d {
            translation: Translation2d { x: 1.0, y: 2.0 },
            rotation: Rotation2d { value: 3.0 },
        };
        let mut out = Vec::new();
        NtStruct::pack(&pose, &mut out);
        let expected: Vec<u8> = [1.0f64, 2.0, 3.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(out, expected);
        assert_eq!(<Pose2d as NtStruct>::unpack(&out), pose);
    }
}
