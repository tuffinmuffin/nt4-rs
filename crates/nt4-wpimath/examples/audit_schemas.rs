//! Prints each type's struct name, schema, size and nested schemas, one JSON
//! object per line, for `scripts/schema_audit.py` to compare with WPILib.
//! Not a usage example.
//!
//! Every `NtStruct` in this crate must be listed here; the audit checks
//! that against the source.
//!
//! Written by Claude.

use nt4::structs::NtStruct;
use nt4_wpimath::*;

fn print<T: NtStruct>(rust: &str) {
    let mut nested = Vec::new();
    T::for_each_nested(&mut |ty, _| {
        let name = ty.strip_prefix("struct:").unwrap_or(ty).to_owned();
        if !nested.contains(&name) {
            nested.push(name);
        }
    });
    let line = serde_json::json!({
        "rust": rust,
        "name": T::type_name(),
        "schema": T::schema(),
        "size": <T as NtStruct>::SIZE,
        "nested": nested,
    });
    println!("{line}");
}

macro_rules! print_all {
    ($($t:ty),* $(,)?) => {
        $(print::<$t>(stringify!($t));)*
    };
}

fn main() {
    print_all!(
        Translation2d,
        Rotation2d,
        Pose2d,
        Transform2d,
        Twist2d,
        Translation3d,
        Quaternion,
        Rotation3d,
        Pose3d,
        Transform3d,
        Twist3d,
        ChassisVelocities,
        SwerveModulePosition,
        SwerveModuleVelocity,
        // Generic: the audit matches each instance to WPILib's pattern.
        SwerveDriveKinematics<2>,
        SwerveDriveKinematics<4>,
    );
}
