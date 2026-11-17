//! Positions, rotations, poses and their changes, in 2D and 3D
//! (wpimath's `geometry` structs).

use nt4::structs::NtStruct;

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
