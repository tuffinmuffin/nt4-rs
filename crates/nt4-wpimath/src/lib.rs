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

/// Implements [`NtStruct`] for a struct whose fields are all
/// [`StructField`]s, in order.
macro_rules! wpi_struct {
    (
        $(#[$meta:meta])*
        $ty:ident, $schema:literal, [$($nested:ty),*],
        { $($(#[$fmeta:meta])* $field:ident: $fty:ty),* $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Default)]
        pub struct $ty {
            $($(#[$fmeta])* pub $field: $fty),*
        }

        impl NtStruct for $ty {
            const SIZE: usize = 0 $(+ <$fty as StructField>::SIZE)*;
            fn type_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($ty))
            }
            fn schema() -> Cow<'static, str> {
                Cow::Borrowed($schema)
            }
            fn pack(&self, out: &mut Vec<u8>) {
                $(StructField::pack(&self.$field, out);)*
            }
            fn unpack(data: &[u8]) -> Self {
                let mut r = StructReader::new(data);
                Self { $($field: r.read()),* }
            }
            fn for_each_nested(f: &mut dyn FnMut(&str, &str)) {
                let _ = &f;
                $(<$nested as NtStruct>::for_each_struct_schema(f);)*
            }
        }
    };
}

wpi_struct!(
    /// A 2D position (`Translation2d`).
    Translation2d, "double x;double y", [], {
        /// Meters.
        x: f64,
        /// Meters.
        y: f64,
    }
);

wpi_struct!(
    /// A 2D rotation (`Rotation2d`).
    Rotation2d, "double value", [], {
        /// Radians, counterclockwise.
        value: f64,
    }
);

wpi_struct!(
    /// A 2D position and heading (`Pose2d`).
    Pose2d, "Translation2d translation;Rotation2d rotation", [Translation2d, Rotation2d], {
        /// Position.
        translation: Translation2d,
        /// Heading.
        rotation: Rotation2d,
    }
);

wpi_struct!(
    /// A 2D change of pose (`Transform2d`).
    Transform2d, "Translation2d translation;Rotation2d rotation", [Translation2d, Rotation2d], {
        /// Change in position.
        translation: Translation2d,
        /// Change in heading.
        rotation: Rotation2d,
    }
);

wpi_struct!(
    /// A 2D motion along an arc (`Twist2d`).
    Twist2d, "double dx;double dy;double dtheta", [], {
        /// Meters forward.
        dx: f64,
        /// Meters left.
        dy: f64,
        /// Radians.
        dtheta: f64,
    }
);

wpi_struct!(
    /// A 3D position (`Translation3d`).
    Translation3d, "double x;double y;double z", [], {
        /// Meters.
        x: f64,
        /// Meters.
        y: f64,
        /// Meters.
        z: f64,
    }
);

wpi_struct!(
    /// A quaternion (`Quaternion`).
    Quaternion, "double w;double x;double y;double z", [], {
        /// Real part.
        w: f64,
        /// i part.
        x: f64,
        /// j part.
        y: f64,
        /// k part.
        z: f64,
    }
);

wpi_struct!(
    /// A 3D rotation (`Rotation3d`), as a unit quaternion.
    Rotation3d, "Quaternion q", [Quaternion], {
        /// The rotation.
        q: Quaternion,
    }
);

wpi_struct!(
    /// A 3D position and orientation (`Pose3d`).
    Pose3d, "Translation3d translation;Rotation3d rotation", [Translation3d, Rotation3d], {
        /// Position.
        translation: Translation3d,
        /// Orientation.
        rotation: Rotation3d,
    }
);

wpi_struct!(
    /// A 3D change of pose (`Transform3d`).
    Transform3d, "Translation3d translation;Rotation3d rotation", [Translation3d, Rotation3d], {
        /// Change in position.
        translation: Translation3d,
        /// Change in orientation.
        rotation: Rotation3d,
    }
);

wpi_struct!(
    /// A 3D motion along an arc (`Twist3d`).
    Twist3d, "double dx;double dy;double dz;double rx;double ry;double rz", [], {
        /// Meters.
        dx: f64,
        /// Meters.
        dy: f64,
        /// Meters.
        dz: f64,
        /// Radians about x.
        rx: f64,
        /// Radians about y.
        ry: f64,
        /// Radians about z.
        rz: f64,
    }
);

wpi_struct!(
    /// Robot-relative chassis velocities (`ChassisVelocities`, called
    /// `ChassisSpeeds` before 2027).
    ChassisVelocities, "double vx;double vy;double omega", [], {
        /// Meters per second forward.
        vx: f64,
        /// Meters per second left.
        vy: f64,
        /// Radians per second counterclockwise.
        omega: f64,
    }
);

wpi_struct!(
    /// A swerve module's distance and angle (`SwerveModulePosition`).
    SwerveModulePosition, "double distance;Rotation2d angle", [Rotation2d], {
        /// Meters driven.
        distance: f64,
        /// Module angle.
        angle: Rotation2d,
    }
);

wpi_struct!(
    /// A swerve module's velocity and angle (`SwerveModuleVelocity`,
    /// called `SwerveModuleState` before 2027).
    SwerveModuleVelocity, "double velocity;Rotation2d angle", [Rotation2d], {
        /// Meters per second.
        velocity: f64,
        /// Module angle.
        angle: Rotation2d,
    }
);

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
