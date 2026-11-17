//! `#[derive(NtStruct)]`: derived types against hand-written ones, and the
//! schemas, nested schemas and bytes the derive generates.
//!
//! Written by Claude.

#![cfg(feature = "derive")]

use std::borrow::Cow;

use nt4::proto::structs::{DynamicStruct, StructDatabase};
use nt4::structs::{NtStruct, StructField, StructReader, check};
use nt4::{NtType, NtValue, Value};

#[path = "support/wpi_types.rs"]
mod wpi_types;

/// The derived twins of `wpi_types`, which are hand-written.
mod derived {
    use nt4::structs::NtStruct;

    #[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
    pub struct Translation2d {
        pub x: f64,
        pub y: f64,
    }

    #[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
    pub struct Rotation2d {
        pub value: f64,
    }

    #[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
    pub struct Pose2d {
        pub translation: Translation2d,
        pub rotation: Rotation2d,
    }

    #[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
    pub struct SwerveModuleVelocity {
        pub velocity: f64,
        pub angle: Rotation2d,
    }
}

/// Every schema `T` registers, as (type string, schema), in order.
fn schemas<T: NtStruct>() -> Vec<(String, String)> {
    let mut out = Vec::new();
    T::for_each_struct_schema(&mut |ty, schema| out.push((ty.to_owned(), schema.to_owned())));
    out
}

fn packed<T: NtStruct>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    NtStruct::pack(v, &mut out);
    out
}

#[test]
fn derived_matches_hand_written() {
    use derived as d;
    use wpi_types as h;

    assert_eq!(schemas::<d::Translation2d>(), schemas::<h::Translation2d>());
    assert_eq!(schemas::<d::Rotation2d>(), schemas::<h::Rotation2d>());
    assert_eq!(schemas::<d::Pose2d>(), schemas::<h::Pose2d>());
    assert_eq!(
        schemas::<d::SwerveModuleVelocity>(),
        schemas::<h::SwerveModuleVelocity>()
    );
    assert_eq!(<d::Pose2d as NtStruct>::SIZE, <h::Pose2d as NtStruct>::SIZE);
    assert_eq!(d::Pose2d::nt_type(), h::Pose2d::nt_type());

    let dp = d::Pose2d {
        translation: d::Translation2d { x: 1.5, y: -2.0 },
        rotation: d::Rotation2d { value: 0.25 },
    };
    let hp = h::Pose2d {
        translation: h::Translation2d { x: 1.5, y: -2.0 },
        rotation: h::Rotation2d { value: 0.25 },
    };
    assert_eq!(packed(&dp), packed(&hp));
    assert_eq!(<d::Pose2d as NtStruct>::unpack(&packed(&hp)), dp);

    let ds = d::SwerveModuleVelocity {
        velocity: 3.0,
        angle: d::Rotation2d { value: 1.0 },
    };
    let hs = h::SwerveModuleVelocity {
        velocity: 3.0,
        angle: h::Rotation2d { value: 1.0 },
    };
    assert_eq!(vec![ds, ds].into_value(), vec![hs, hs].into_value());
}

/// Every number type, arrays, a raw identifier, and nested structs.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
struct Everything {
    a: bool,
    b: i8,
    c: u8,
    d: i16,
    e: u16,
    f: i32,
    g: u32,
    h: i64,
    i: u64,
    j: f32,
    k: f64,
    r#type: [u16; 3],
    modules: [derived::SwerveModuleVelocity; 2],
    heading: derived::Rotation2d,
    pose: derived::Pose2d,
}

#[test]
fn schema_declares_each_field_as_wpimath_does() {
    assert_eq!(
        Everything::schema(),
        "bool a;int8 b;uint8 c;int16 d;uint16 e;int32 f;uint32 g;int64 h;uint64 i;\
         float j;double k;uint16 type[3];SwerveModuleVelocity modules[2];\
         Rotation2d heading;Pose2d pose"
    );
    assert_eq!(<Everything as NtStruct>::SIZE, 43 + 6 + 32 + 8 + 24);
    check::<Everything>().unwrap();
}

#[test]
fn nested_schemas_each_once_in_first_use_order() {
    let names: Vec<String> = schemas::<Everything>()
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert_eq!(
        names,
        [
            "struct:Rotation2d",
            "struct:SwerveModuleVelocity",
            "struct:Translation2d",
            "struct:Pose2d",
            "struct:Everything",
        ]
    );
}

#[test]
fn packs_in_field_order_and_reads_back() {
    let v = Everything {
        a: true,
        b: -2,
        c: 200,
        d: -300,
        e: 60000,
        f: -70000,
        g: 4_000_000_000,
        h: -5,
        i: u64::MAX,
        j: 1.5,
        k: -0.125,
        r#type: [1, 2, 3],
        modules: [
            derived::SwerveModuleVelocity {
                velocity: 1.0,
                angle: derived::Rotation2d { value: 2.0 },
            },
            derived::SwerveModuleVelocity {
                velocity: 3.0,
                angle: derived::Rotation2d { value: 4.0 },
            },
        ],
        heading: derived::Rotation2d { value: 5.0 },
        pose: derived::Pose2d {
            translation: derived::Translation2d { x: 6.0, y: 7.0 },
            rotation: derived::Rotation2d { value: 8.0 },
        },
    };
    let bytes = packed(&v);
    assert_eq!(bytes.len(), <Everything as NtStruct>::SIZE);
    assert_eq!(&bytes[..3], &[1, 0xfe, 200]);
    assert_eq!(<Everything as NtStruct>::unpack(&bytes), v);

    // A dashboard reading the bytes through the schemas sees the same.
    let mut db = StructDatabase::new();
    for (ty, schema) in schemas::<Everything>() {
        db.add(ty.strip_prefix("struct:").unwrap(), &schema)
            .unwrap();
    }
    let view = DynamicStruct::new(&db, "Everything", &bytes).unwrap();
    let desc = db.find("Everything").unwrap();
    assert_eq!(view.get_uint(desc.field("type").unwrap(), 2), Some(3));
    assert_eq!(view.get_int(desc.field("d").unwrap(), 0), Some(-300));
}

#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
#[nt4(name = "TrapezoidProfileConstraints")]
struct Constraints {
    #[allow(dead_code)]
    max_velocity: f64,
    #[allow(dead_code)]
    max_acceleration: f64,
}

#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
#[nt4(schema = "double maxVelocity;double maxAcceleration")]
struct Written {
    max_velocity: f64,
    max_acceleration: f64,
}

#[test]
fn name_and_schema_options() {
    assert_eq!(Constraints::type_name(), "TrapezoidProfileConstraints");
    assert_eq!(
        Constraints::nt_type(),
        NtType::Struct {
            name: "TrapezoidProfileConstraints".into(),
            array: false
        }
    );
    assert_eq!(
        Constraints::schema(),
        "double max_velocity;double max_acceleration"
    );
    assert_eq!(
        Written::schema(),
        "double maxVelocity;double maxAcceleration"
    );
    check::<Constraints>().unwrap();
    check::<Written>().unwrap();
    let w = Written {
        max_velocity: 1.0,
        max_acceleration: 2.0,
    };
    assert_eq!(<Written as NtStruct>::unpack(&packed(&w)), w);
}

#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
enum Mode {
    #[default]
    Disabled,
    Teleop,
    Auto = 5,
    After,
    Test = -3,
}

#[test]
fn fieldless_enum() {
    assert_eq!(
        Mode::schema(),
        "enum {Disabled=0,Teleop=1,Auto=5,After=6,Test=-3} int8 variant"
    );
    assert_eq!(<Mode as NtStruct>::SIZE, 1);
    check::<Mode>().unwrap();
    assert_eq!(Mode::Auto.into_value(), Value::Raw(vec![5]));
    assert_eq!(Mode::Test.into_value(), Value::Raw(vec![0xfd]));
    for m in [
        Mode::Disabled,
        Mode::Teleop,
        Mode::Auto,
        Mode::After,
        Mode::Test,
    ] {
        assert_eq!(Mode::from_value(&m.into_value()), Some(m));
    }
    // A value no variant has unpacks as the default.
    assert_eq!(
        Mode::from_value(&Value::Raw(vec![42])),
        Some(Mode::Disabled)
    );

    let mut db = StructDatabase::new();
    let desc = db.add("Mode", &Mode::schema()).unwrap();
    assert_eq!(desc.field("variant").unwrap().enum_name(6), Some("After"));
}

/// An enum as a field of a struct.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
struct State {
    mode: Mode,
    enabled: bool,
}

#[test]
fn enum_field() {
    assert_eq!(State::schema(), "Mode mode;bool enabled");
    let names: Vec<String> = schemas::<State>().into_iter().map(|(t, _)| t).collect();
    assert_eq!(names, ["struct:Mode", "struct:State"]);
    check::<State>().unwrap();
    let s = State {
        mode: Mode::Teleop,
        enabled: true,
    };
    assert_eq!(s.into_value(), Value::Raw(vec![1, 1]));
}

#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
struct Empty;

#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
struct AlsoEmpty {}

#[test]
fn structs_without_fields() {
    assert_eq!(Empty::schema(), "");
    assert_eq!(<Empty as NtStruct>::SIZE, 0);
    check::<Empty>().unwrap();
    check::<AlsoEmpty>().unwrap();
    assert_eq!(Empty::from_value(&Value::Raw(vec![])), Some(Empty));
}

mod renamed {
    // The nt4 crate under another name, as `#[nt4(crate = "..")]` is for.
    pub use nt4 as other_name;
}

#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
#[nt4(crate = "renamed::other_name")]
struct ViaPath {
    x: f32,
}

#[test]
fn crate_option() {
    assert_eq!(ViaPath::schema(), "float x");
    check::<ViaPath>().unwrap();
    assert_eq!(
        <ViaPath as NtStruct>::unpack(&packed(&ViaPath { x: 2.0 })).x,
        2.0
    );
}

/// A field type implementing `StructField` by hand, without a schema
/// declaration.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Opaque(u8);

impl StructField for Opaque {
    const SIZE: usize = 1;
    fn pack(&self, out: &mut Vec<u8>) {
        out.push(self.0);
    }
    fn unpack(data: &[u8]) -> Self {
        Opaque(data[0])
    }
}

#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
struct HasOpaque {
    raw: Opaque,
}

#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
#[nt4(schema = "uint8 raw")]
struct HasOpaqueWritten {
    raw: Opaque,
}

#[test]
#[should_panic(expected = "NtStruct field `raw`: `Opaque` has no schema declaration")]
fn field_without_declaration_panics_naming_it() {
    let _ = HasOpaque::schema();
}

#[test]
fn field_without_declaration_with_written_schema() {
    check::<HasOpaqueWritten>().unwrap();
    let mut r = StructReader::new(&[7]);
    assert_eq!(r.read::<Opaque>(), Opaque(7));
}

#[test]
fn arrays_of_arrays_have_no_declaration() {
    assert_eq!(<[[f64; 2]; 3]>::schema_field("m"), None);
    assert_eq!(
        <[f64; 2]>::schema_field("m").as_deref(),
        Some("double m[2]")
    );
}

/// Generated code doesn't depend on what the caller has imported (no
/// `use nt4::structs::...` here) and doesn't clash with a local `Cow`.
mod hygiene {
    #[allow(dead_code)]
    struct Cow;
    #[allow(dead_code)]
    struct String;

    #[derive(nt4::structs::NtStruct, Clone, Default)]
    pub struct Lonely {
        pub v: u32,
    }
}

#[test]
fn hygiene() {
    let schema: Cow<'static, str> = <hygiene::Lonely as NtStruct>::schema();
    assert_eq!(schema, "uint32 v");
}

/// A schema name that isn't the Rust name, as WPILib's camelCase fields.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
struct Shape {
    center: derived::Pose2d,
    #[nt4(rename = "xWidth")]
    x_width: f64,
    #[nt4(rename = "yWidth")]
    y_width: f64,
}

#[test]
fn rename_sets_the_schema_name() {
    assert_eq!(Shape::schema(), "Pose2d center;double xWidth;double yWidth");
    check::<Shape>().unwrap();
    let s = Shape {
        x_width: 1.0,
        y_width: 2.0,
        ..Default::default()
    };
    assert_eq!(<Shape as NtStruct>::unpack(&packed(&s)), s);
}

/// The fields of `Shape` written out, but registering `Shape` itself as
/// nested, as WPILib's `DifferentialSample` does with `HolonomicSample`.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
#[nt4(nested(Shape))]
struct Flattened {
    center: derived::Pose2d,
    #[nt4(rename = "xWidth")]
    x_width: f64,
    #[nt4(rename = "yWidth")]
    y_width: f64,
    extra: f64,
}

#[test]
fn nested_lists_the_nested_structs() {
    let names: Vec<String> = schemas::<Flattened>().into_iter().map(|(t, _)| t).collect();
    assert_eq!(
        names,
        [
            "struct:Translation2d",
            "struct:Rotation2d",
            "struct:Pose2d",
            "struct:Shape",
            "struct:Flattened"
        ]
    );
    assert_eq!(
        Flattened::schema(),
        "Pose2d center;double xWidth;double yWidth;double extra"
    );
    check::<Flattened>().unwrap();
}

/// `nested()` with nothing in it: no nested schemas, so `check` reports
/// the one the schema needs.
#[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
#[nt4(nested())]
struct ForgotNested {
    pose: derived::Pose2d,
}

#[test]
fn check_catches_a_missing_nested_schema() {
    assert_eq!(schemas::<ForgotNested>().len(), 1);
    assert!(check::<ForgotNested>().is_err());
}
