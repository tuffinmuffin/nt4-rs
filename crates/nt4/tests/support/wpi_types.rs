//! Test copies of four WPILib struct types, with wpimath's names and
//! schemas, so nt4's tests can check schema registration against ntcore
//! without depending on the nt4-wpimath crate (which depends on nt4).
//!
//! Written by Claude.

#![allow(dead_code)] // each test file uses part of it

use std::borrow::Cow;

use nt4::structs::{NtStruct, StructField, StructReader};

macro_rules! test_struct {
    ($ty:ident, $schema:literal, [$($nested:ty),*], { $($field:ident: $fty:ty),* }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Default)]
        pub struct $ty {
            $(pub $field: $fty),*
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

test_struct!(Translation2d, "double x;double y", [], { x: f64, y: f64 });
test_struct!(Rotation2d, "double value", [], { value: f64 });
test_struct!(
    Pose2d, "Translation2d translation;Rotation2d rotation", [Translation2d, Rotation2d],
    { translation: Translation2d, rotation: Rotation2d }
);
test_struct!(
    SwerveModuleVelocity, "double velocity;Rotation2d angle", [Rotation2d],
    { velocity: f64, angle: Rotation2d }
);
