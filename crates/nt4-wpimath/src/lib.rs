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

use nt4::structs::NtStruct;

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
    fn schemas_match_wpimath() {
        // GetSchema() in wpimath's *Struct.hpp at the pinned allwpilib
        // commit, copied by hand. (The derive builds ours from the fields.)
        let cases = [
            (
                "Translation2d",
                Translation2d::schema(),
                "double x;double y",
            ),
            ("Rotation2d", Rotation2d::schema(), "double value"),
            (
                "Pose2d",
                Pose2d::schema(),
                "Translation2d translation;Rotation2d rotation",
            ),
            (
                "Transform2d",
                Transform2d::schema(),
                "Translation2d translation;Rotation2d rotation",
            ),
            (
                "Twist2d",
                Twist2d::schema(),
                "double dx;double dy;double dtheta",
            ),
            (
                "Translation3d",
                Translation3d::schema(),
                "double x;double y;double z",
            ),
            (
                "Quaternion",
                Quaternion::schema(),
                "double w;double x;double y;double z",
            ),
            ("Rotation3d", Rotation3d::schema(), "Quaternion q"),
            (
                "Pose3d",
                Pose3d::schema(),
                "Translation3d translation;Rotation3d rotation",
            ),
            (
                "Transform3d",
                Transform3d::schema(),
                "Translation3d translation;Rotation3d rotation",
            ),
            (
                "Twist3d",
                Twist3d::schema(),
                "double dx;double dy;double dz;double rx;double ry;double rz",
            ),
            (
                "ChassisVelocities",
                ChassisVelocities::schema(),
                "double vx;double vy;double omega",
            ),
            (
                "SwerveModulePosition",
                SwerveModulePosition::schema(),
                "double distance;Rotation2d angle",
            ),
            (
                "SwerveModuleVelocity",
                SwerveModuleVelocity::schema(),
                "double velocity;Rotation2d angle",
            ),
        ];
        for (name, ours, wpimath) in cases {
            assert_eq!(ours, wpimath, "{name}");
        }
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
