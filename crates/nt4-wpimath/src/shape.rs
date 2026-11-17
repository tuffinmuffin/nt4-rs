//! 2D shapes on the field (wpimath's `geometry` shapes), as used for
//! zones and obstacles.

use nt4::structs::NtStruct;

use crate::Pose2d;

/// A rectangle (`Rectangle2d`): a center pose and the size along its own
/// x and y axes.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Rectangle2d {
    /// Center and rotation.
    pub center: Pose2d,
    /// Meters, the full size along the rectangle's x axis.
    #[nt4(rename = "xWidth")]
    pub x_width: f64,
    /// Meters, the full size along the rectangle's y axis.
    #[nt4(rename = "yWidth")]
    pub y_width: f64,
}

/// An ellipse (`Ellipse2d`): a center pose and its semi-axes.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
pub struct Ellipse2d {
    /// Center and rotation.
    pub center: Pose2d,
    /// Meters, half the size along the ellipse's x axis.
    #[nt4(rename = "xSemiAxis")]
    pub x_semi_axis: f64,
    /// Meters, half the size along the ellipse's y axis.
    #[nt4(rename = "ySemiAxis")]
    pub y_semi_axis: f64,
}
