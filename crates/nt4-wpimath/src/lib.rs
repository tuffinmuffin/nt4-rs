//! WPILib's `wpimath` types as [`NtStruct`](nt4::structs::NtStruct)s, for the [`nt4`] crate.
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

pub mod geometry;
pub mod kinematics;
pub mod shape;
pub mod trajectory;

pub use geometry::*;
pub use kinematics::*;
pub use shape::*;
pub use trajectory::*;

// Compile and run the code in README.md as doc tests.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;

#[cfg(test)]
mod tests {
    // Written by Claude.
    use super::*;
    use nt4::structs::{NtStruct, check};

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
        check::<ChassisAccelerations>().unwrap();
        check::<SwerveModuleAcceleration>().unwrap();
        check::<DifferentialDriveWheelPositions>().unwrap();
        check::<DifferentialDriveWheelVelocities>().unwrap();
        check::<DifferentialDriveWheelAccelerations>().unwrap();
        check::<MecanumDriveWheelPositions>().unwrap();
        check::<MecanumDriveWheelVelocities>().unwrap();
        check::<MecanumDriveWheelAccelerations>().unwrap();
        check::<HolonomicSample>().unwrap();
        check::<DifferentialSample>().unwrap();
        check::<Rectangle2d>().unwrap();
        check::<Ellipse2d>().unwrap();
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
        assert_eq!(<HolonomicSample as NtStruct>::SIZE, 80);
        assert_eq!(<DifferentialSample as NtStruct>::SIZE, 96);
        assert_eq!(<Rectangle2d as NtStruct>::SIZE, 40);
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
