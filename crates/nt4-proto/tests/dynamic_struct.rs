//! Struct layout and value cases ported from wpiutil's DynamicStructTest.

// Ported from allwpilib wpiutil/src/test/native/cpp/struct/DynamicStructTest.cpp
// at commit 4ccfe967ba1903b1f409b04b99db0922dd3b7318.
// Copyright (c) FIRST and other WPILib contributors.
// The test cases come from that file and stay under the WPILib BSD license;
// see tests/LICENSE-WPILIB.md. The Rust test code was written by Claude.
//
// Upstream's templated cases run once with DynamicStruct and once with
// MutableDynamicStruct; here each checks both views in one test.

use nt4_proto::structs::{
    DynamicStruct, DynamicStructMut, FieldType, StructDatabase, StructDescriptor,
};

fn valid<'a>(db: &'a StructDatabase, name: &str) -> &'a StructDescriptor {
    let desc = db.find(name).unwrap();
    assert!(desc.is_valid(), "{name}");
    desc
}

// upstream: "DynamicStructTest SignedBitfieldLiteralBytes"
#[test]
fn signed_bitfield_literal_bytes() {
    // Literal wire data ensures writer and reader bugs cannot cancel each other.
    let cases: &[(&str, &[u8], i64)] = &[
        ("int16 value:3", &[0x04, 0x00], -4),
        ("int16 value:3", &[0x07, 0x00], -1),
        ("int16 value:3", &[0x03, 0x00], 3),
        ("int8 value:1", &[0x00], 0),
        ("int8 value:1", &[0x01], -1),
        ("int8 value:3", &[0x04], -4),
        ("int32 value:3", &[0x07, 0x00, 0x00, 0x00], -1),
        ("int64 value:3", &[0x04, 0, 0, 0, 0, 0, 0, 0], -4),
        (
            "uint16 before:2; int16 value:3; uint16 after:11",
            &[0xf3, 0xff],
            -4,
        ),
        (
            "uint16 before:2; int16 value:3; uint16 after:11",
            &[0x1c, 0x00],
            -1,
        ),
        (
            "uint16 before:2; int16 value:3; uint16 after:11",
            &[0xef, 0xff],
            3,
        ),
        (
            "int64 value:63",
            &[0, 0, 0, 0, 0, 0, 0, 0x40],
            -4611686018427387904,
        ),
        (
            "int64 value:63",
            &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
            -1,
        ),
        (
            "int64 value:63",
            &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x3f],
            4611686018427387903,
        ),
        (
            "uint64 before:1; int64 value:63",
            &[1, 0, 0, 0, 0, 0, 0, 0x80],
            -4611686018427387904,
        ),
        ("int64 value:64", &[0, 0, 0, 0, 0, 0, 0, 0x80], i64::MIN),
        ("int64 value:64", &[0xff; 8], -1),
        (
            "int64 value:64",
            &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
            i64::MAX,
        ),
        ("int8 value", &[0x80], -128),
        ("int16 value", &[0, 0x80], -32768),
        ("int32 value", &[0, 0, 0, 0x80], -2147483648),
        ("int64 value", &[0, 0, 0, 0, 0, 0, 0, 0x80], i64::MIN),
    ];
    let mut db = StructDatabase::new();
    for &(schema, bytes, expected) in cases {
        let desc = db.add("test", schema).unwrap();
        assert_eq!(desc.size(), Some(bytes.len()), "{schema}");
        let view = DynamicStruct::new(&db, "test", bytes).unwrap();
        let field = view.field("value").unwrap();
        assert_eq!(
            view.get_int(field, 0),
            Some(expected),
            "{schema} {bytes:x?}"
        );
        let mut copy = bytes.to_vec();
        let mutable = DynamicStructMut::new(&db, "test", &mut copy).unwrap();
        assert_eq!(mutable.as_ref().get_int(field, 0), Some(expected));
    }
}

// upstream: "DynamicStructTest UnsignedBitfieldLiteralBytes"
#[test]
fn unsigned_bitfield_literal_bytes() {
    let cases: &[(&str, &[u8], u64)] = &[
        ("uint8 value:1", &[0x01], 1),
        ("uint16 value:3", &[0x07, 0x00], 7),
        (
            "uint32 before:2; uint32 value:3; uint32 after:27",
            &[0xff; 4],
            7,
        ),
        ("uint64 value:63", &[0xff; 8], 9223372036854775807),
        ("uint64 value:64", &[0xff; 8], u64::MAX),
    ];
    let mut db = StructDatabase::new();
    for &(schema, bytes, expected) in cases {
        db.add("test", schema).unwrap();
        let view = DynamicStruct::new(&db, "test", bytes).unwrap();
        let field = view.field("value").unwrap();
        assert_eq!(view.get_uint(field, 0), Some(expected), "{schema}");
    }
}

// upstream: "DynamicStructTest Empty"
#[test]
fn empty() {
    let mut db = StructDatabase::new();
    let desc = db.add("test", "").unwrap();
    assert_eq!(desc.name(), "test");
    assert_eq!(desc.schema(), "");
    assert!(desc.fields().is_empty());
    assert!(desc.is_valid());
    assert_eq!(desc.size(), Some(0));
}

// upstream: "DynamicStructTest NestedStruct"
#[test]
fn nested_struct() {
    let mut db = StructDatabase::new();
    assert!(db.add("test", "int32 a").unwrap().is_valid());
    let desc2 = db.add("test2", "test a").unwrap();
    assert!(desc2.is_valid());
    assert_eq!(desc2.size(), Some(4));
}

// upstream: "DynamicStructTest BoundedNestedArrayViews"
#[test]
fn bounded_nested_array_views() {
    let mut db = StructDatabase::new();
    db.add("Inner", "uint16 value").unwrap();
    db.add("Outer", "Inner children[2]; uint8 tail").unwrap();
    let field = valid(&db, "Outer").field("children").unwrap();
    for index in 0..2 {
        let mut data = vec![0x01, 0x00, 0x02, 0x00, 0x7f];
        let base = data.as_ptr();
        let view = DynamicStruct::new(&db, "Outer", &data).unwrap();
        let child = view.get_struct(field, index).unwrap();
        assert_eq!(child.descriptor().name(), "Inner");
        assert_eq!(child.data(), &[index as u8 + 1, 0x00]);
        assert_eq!(child.data().as_ptr(), base.wrapping_add(2 * index));

        let mut view = DynamicStructMut::new(&db, "Outer", &mut data).unwrap();
        view.get_struct_mut(field, index).unwrap().data().fill(0x55);
        assert_eq!(
            view.as_ref().get_struct(field, index).unwrap().data().len(),
            2
        );
        let expected = if index == 0 {
            vec![0x55, 0x55, 2, 0, 0x7f]
        } else {
            vec![1, 0, 0x55, 0x55, 0x7f]
        };
        assert_eq!(data, expected);
    }
}

// upstream: "DynamicStructTest BoundedNestedScalarViews"
#[test]
fn bounded_nested_scalar_views() {
    for schema in [
        "uint8 head; Inner child; uint8 tail",
        "uint8 head; Inner child",
    ] {
        let mut db = StructDatabase::new();
        db.add("Inner", "uint16 value").unwrap();
        db.add("Outer", schema).unwrap();
        let field = valid(&db, "Outer").field("child").unwrap();
        // Include bytes beyond the declared parent, even when child is at its end.
        let mut data = vec![0x7e, 0x01, 0x02, 0x7f];
        let base = data.as_ptr();
        let view = DynamicStruct::new(&db, "Outer", &data).unwrap();
        let child = view.get_struct(field, 0).unwrap();
        assert_eq!(child.data(), &[0x01, 0x02]);
        assert_eq!(child.data().as_ptr(), base.wrapping_add(1));

        let mut view = DynamicStructMut::new(&db, "Outer", &mut data).unwrap();
        view.get_struct_mut(field, 0).unwrap().data().fill(0x55);
        assert_eq!(data, vec![0x7e, 0x55, 0x55, 0x7f]);
    }
}

// upstream: "DynamicStructTest BoundedMultilevelAndEmptyViews"
#[test]
fn bounded_multilevel_and_empty_views() {
    let mut db = StructDatabase::new();
    db.add("Empty", "").unwrap();
    db.add("Leaf", "uint16 value").unwrap();
    db.add("Inner", "Empty before[2]; Leaf value; Empty end")
        .unwrap();
    db.add("Outer", "uint8 head; Inner child; uint8 tail")
        .unwrap();
    let field = valid(&db, "Outer").field("child").unwrap();
    let inner = valid(&db, "Inner");
    let before = inner.field("before").unwrap();
    let end = inner.field("end").unwrap();
    let value = inner.field("value").unwrap();
    for empty_field in 0..3 {
        let mut data = vec![0x7e, 0x01, 0x02, 0x7f];
        let base = data.as_ptr();
        let view = DynamicStruct::new(&db, "Outer", &data).unwrap();
        let child = view.get_struct(field, 0).unwrap();
        assert_eq!(child.data(), &[0x01, 0x02]);
        let grandchild = child.get_struct(value, 0).unwrap();
        assert_eq!(grandchild.data(), &[0x01, 0x02]);
        assert_eq!(grandchild.data().as_ptr(), base.wrapping_add(1));
        let nested = if empty_field == 2 {
            child.get_struct(end, 0).unwrap()
        } else {
            child.get_struct(before, empty_field).unwrap()
        };
        assert!(nested.data().is_empty());
        let at = if empty_field == 2 { 3 } else { 1 };
        assert_eq!(nested.data().as_ptr(), base.wrapping_add(at));

        let mut view = DynamicStructMut::new(&db, "Outer", &mut data).unwrap();
        let mut child = view.get_struct_mut(field, 0).unwrap();
        let mut nested = if empty_field == 2 {
            child.get_struct_mut(end, 0).unwrap()
        } else {
            child.get_struct_mut(before, empty_field).unwrap()
        };
        nested.data().fill(0x55);
        assert_eq!(data, vec![0x7e, 0x01, 0x02, 0x7f]);
    }
}

// upstream: "DynamicStructTest DelayedValid"
#[test]
fn delayed_valid() {
    let mut db = StructDatabase::new();
    assert!(!db.add("test", "foo a").unwrap().is_valid());
    assert!(!db.add("test2", "foo a;foo b;").unwrap().is_valid());
    assert!(!db.add("test3", "foo a[2]").unwrap().is_valid());
    assert!(db.add("foo", "int32 a").unwrap().is_valid());
    assert_eq!(valid(&db, "test").size(), Some(4));
    assert_eq!(valid(&db, "test2").size(), Some(8));
    assert_eq!(valid(&db, "test3").size(), Some(8));
}

// upstream: "DynamicStructTest ReuseNestedStructDelayed"
#[test]
fn reuse_nested_struct_delayed() {
    let mut db = StructDatabase::new();
    db.add("test2", "test a;test b;").unwrap();
    assert!(
        db.add("test", "int32 a; uint16 b; int16 c;")
            .unwrap()
            .is_valid()
    );
    let desc2 = valid(&db, "test2");
    assert_eq!(desc2.size(), Some(16));
    let fields = desc2.fields();
    assert_eq!((fields[0].name(), fields[0].offset()), ("a", 0));
    assert_eq!((fields[1].name(), fields[1].offset()), ("b", 8));
}

// upstream: "DynamicStructTest InvalidBitfield"
#[test]
fn invalid_bitfield() {
    let mut db = StructDatabase::new();
    assert_eq!(
        db.add("test", "float a:1").unwrap_err(),
        "field a: type float cannot be bitfield"
    );
    assert_eq!(
        db.add("test", "double a:1").unwrap_err(),
        "field a: type double cannot be bitfield"
    );
    assert_eq!(
        db.add("test", "foo a:1").unwrap_err(),
        "field a: type foo cannot be bitfield"
    );
}

// upstream: "DynamicStructTest CircularStructReference"
#[test]
fn circular_struct_reference() {
    let mut db = StructDatabase::new();
    assert_eq!(
        db.add("test", "test a").unwrap_err(),
        "field a: recursive struct reference"
    );
}

// upstream: "DynamicStructTest NestedCircularStructRef"
#[test]
fn nested_circular_struct_ref() {
    let mut db = StructDatabase::new();
    db.add("test", "foo a").unwrap();
    db.add("foo", "bar a").unwrap();
    assert_eq!(
        db.add("bar", "test a").unwrap_err(),
        "circular struct reference: bar <- foo <- test"
    );
    // ok
    assert!(!db.add("baz", "bar a").unwrap().is_valid());
}

// upstream: "DynamicStructTest NestedCircularStructRef2"
#[test]
fn nested_circular_struct_ref2() {
    let mut db = StructDatabase::new();
    db.add("test", "foo a").unwrap();
    db.add("bar", "test a").unwrap();
    assert_eq!(
        db.add("foo", "bar a").unwrap_err(),
        "circular struct reference: foo <- test <- bar"
    );
}

/// (bit width, bit shift, bit mask, offset, size) of each field.
fn layout(desc: &StructDescriptor) -> Vec<(u32, u32, u64, usize, usize)> {
    desc.fields()
        .iter()
        .map(|f| {
            (
                f.bit_width(),
                f.bit_shift(),
                f.bit_mask(),
                f.offset(),
                f.size(),
            )
        })
        .collect()
}

// upstream: "DynamicStructTest BitfieldBasic"
#[test]
fn bitfield_basic() {
    let mut db = StructDatabase::new();
    let desc = db.add("test", "int32 a:2; uint32 b:30").unwrap();
    assert_eq!(desc.size(), Some(4));
    assert_eq!(
        layout(desc),
        vec![(2, 0, 0x3, 0, 4), (30, 2, 0x3fffffff, 0, 4)]
    );
}

// upstream: "DynamicStructTest BitfieldDiffType"
#[test]
fn bitfield_diff_type() {
    let mut db = StructDatabase::new();
    let desc = db.add("test", "int32 a:2; int16 b:2").unwrap();
    assert_eq!(desc.size(), Some(6));
    assert_eq!(layout(desc), vec![(2, 0, 0x3, 0, 4), (2, 0, 0x3, 4, 2)]);
}

// upstream: "DynamicStructTest BitfieldOverflow"
#[test]
fn bitfield_overflow() {
    let mut db = StructDatabase::new();
    let desc = db.add("test", "int8 a:4; int8 b:5").unwrap();
    assert_eq!(desc.size(), Some(2));
    assert_eq!(layout(desc), vec![(4, 0, 0xf, 0, 1), (5, 0, 0x1f, 1, 1)]);
}

// upstream: "DynamicStructTest BitfieldBoolBegin8"
#[test]
fn bitfield_bool_begin8() {
    let mut db = StructDatabase::new();
    let desc = db.add("test", "bool a:1; int8 b:5").unwrap();
    assert_eq!(desc.size(), Some(1));
    assert_eq!(layout(desc), vec![(1, 0, 0x1, 0, 1), (5, 1, 0x1f, 0, 1)]);
}

// upstream: "DynamicStructTest BitfieldBoolBegin16"
#[test]
fn bitfield_bool_begin16() {
    let mut db = StructDatabase::new();
    let desc = db.add("test", "bool a:1; int16 b:5").unwrap();
    assert_eq!(desc.size(), Some(3));
    assert_eq!(layout(desc), vec![(1, 0, 0x1, 0, 1), (5, 0, 0x1f, 1, 2)]);
}

// upstream: "DynamicStructTest BitfieldBoolMid"
#[test]
fn bitfield_bool_mid() {
    let mut db = StructDatabase::new();
    let desc = db
        .add("test", "int16 a:2; bool b:1; bool c:1; uint16 d:5")
        .unwrap();
    assert_eq!(desc.size(), Some(2));
    assert_eq!(
        layout(desc),
        vec![
            (2, 0, 0x3, 0, 2),
            (1, 2, 0x1, 0, 2),
            (1, 3, 0x1, 0, 2),
            (5, 4, 0x1f, 0, 2)
        ]
    );
}

// upstream: "DynamicStructTest BitfieldBoolEnd"
#[test]
fn bitfield_bool_end() {
    let mut db = StructDatabase::new();
    let desc = db.add("test", "int16 a:15; bool b:1").unwrap();
    assert_eq!(desc.size(), Some(2));
    assert_eq!(
        layout(desc),
        vec![(15, 0, 0x7fff, 0, 2), (1, 15, 0x1, 0, 2)]
    );
}

// upstream: "DynamicStructTest BitfieldBoolEnd2"
#[test]
fn bitfield_bool_end2() {
    let mut db = StructDatabase::new();
    let desc = db.add("test", "int16 a:16; bool b:1").unwrap();
    assert_eq!(desc.size(), Some(3));
    assert_eq!(layout(desc), vec![(16, 0, 0xffff, 0, 2), (1, 0, 0x1, 2, 1)]);
}

// upstream: "DynamicStructTest BitfieldBoolWrongSize"
#[test]
fn bitfield_bool_wrong_size() {
    let mut db = StructDatabase::new();
    assert_eq!(
        db.add("test", "bool a:2").unwrap_err(),
        "field a: bit width must be 1 for bool type"
    );
}

// upstream: "DynamicStructTest BitfieldTooBig"
#[test]
fn bitfield_too_big() {
    let mut db = StructDatabase::new();
    assert_eq!(
        db.add("test", "int16 a:17").unwrap_err(),
        "field a: bit width 17 exceeds type size"
    );
}

// upstream: "DynamicStructTest DuplicateFieldName"
#[test]
fn duplicate_field_name() {
    let mut db = StructDatabase::new();
    assert_eq!(
        db.add("test", "int16 a; int8 a").unwrap_err(),
        "duplicate field a"
    );
}

/// A `char a[n]` struct: set `value`, then read it back.
fn string_round_trip(n: usize, value: &str) -> (bool, String) {
    let mut db = StructDatabase::new();
    db.add("test", &format!("char a[{n}]")).unwrap();
    let mut data = vec![0u8; n];
    let mut dynamic = DynamicStructMut::new(&db, "test", &mut data).unwrap();
    let field = dynamic.field("a").unwrap();
    let full = dynamic.set_string(field, value).unwrap();
    let got = dynamic.as_ref().get_string(field).unwrap().into_owned();
    (full, got)
}

// upstream: "DynamicStructTest StringAllZeros"
#[test]
fn string_all_zeros() {
    let mut db = StructDatabase::new();
    db.add("test", "char a[32]").unwrap();
    let data = [0u8; 32];
    let dynamic = DynamicStruct::new(&db, "test", &data).unwrap();
    assert_eq!(dynamic.get_string(dynamic.field("a").unwrap()).unwrap(), "");
}

// upstream: "DynamicStructTest StringRoundTrip"
#[test]
fn string_round_trip_ascii() {
    assert_eq!(string_round_trip(32, "abc"), (true, "abc".into()));
}

// upstream: "DynamicStructTest StringRoundTripEmbeddedNull"
#[test]
fn string_round_trip_embedded_null() {
    assert_eq!(string_round_trip(32, "ab\0c"), (true, "ab\0c".into()));
}

// upstream: "DynamicStructTest StringRoundTripTooLong"
#[test]
fn string_round_trip_too_long() {
    assert_eq!(string_round_trip(2, "abc"), (false, "ab".into()));
}

// upstream: "DynamicStructTest StringRoundTripPartial2ByteUtf8"
#[test]
fn string_round_trip_partial_2byte_utf8() {
    assert_eq!(string_round_trip(2, "aȴ"), (false, "a".into()));
}

// upstream: "DynamicStructTest StringRoundTrip2ByteUtf8"
#[test]
fn string_round_trip_2byte_utf8() {
    assert_eq!(string_round_trip(3, "aȴ"), (true, "aȴ".into()));
}

// upstream: "DynamicStructTest StringRoundTrip3ByteUtf8"
#[test]
fn string_round_trip_3byte_utf8() {
    assert_eq!(string_round_trip(4, "aሴ"), (true, "aሴ".into()));
}

// upstream: "DynamicStructTest StringRoundTrip3ByteUtf8PartialFirstByte"
#[test]
fn string_round_trip_3byte_utf8_partial_first_byte() {
    assert_eq!(string_round_trip(2, "aሴ"), (false, "a".into()));
}

// upstream: "DynamicStructTest StringRoundTrip3ByteUtf8PartialSecondByte"
#[test]
fn string_round_trip_3byte_utf8_partial_second_byte() {
    assert_eq!(string_round_trip(3, "aሴ"), (false, "a".into()));
}

/// "a" and U+1F400, built from bytes as upstream does.
fn four_byte() -> String {
    String::from_utf8(vec![0x61, 0xf0, 0x9f, 0x90, 0x80]).unwrap()
}

// upstream: "DynamicStructTest StringRoundTrip4ByteUtf8"
#[test]
fn string_round_trip_4byte_utf8() {
    assert_eq!(string_round_trip(5, &four_byte()), (true, four_byte()));
}

// upstream: "DynamicStructTest StringRoundTrip4ByteUtf8PartialFirstByte"
#[test]
fn string_round_trip_4byte_utf8_partial_first_byte() {
    assert_eq!(string_round_trip(2, &four_byte()), (false, "a".into()));
}

// upstream: "DynamicStructTest StringRoundTrip4ByteUtf8PartialSecondByte"
#[test]
fn string_round_trip_4byte_utf8_partial_second_byte() {
    assert_eq!(string_round_trip(3, &four_byte()), (false, "a".into()));
}

// upstream: "DynamicStructTest StringRoundTrip4ByteUtf8PartialThirdByte"
#[test]
fn string_round_trip_4byte_utf8_partial_third_byte() {
    assert_eq!(string_round_trip(4, &four_byte()), (false, "a".into()));
}

/// One row of upstream's simpleTests table.
struct Simple {
    schema: &'static str,
    size: usize,
    ty: FieldType,
    is_int: bool,
    is_uint: bool,
    bit_width: u32,
    bit_mask: u64,
    min: u64,
    max: u64,
}

#[allow(clippy::too_many_arguments)]
const fn simple(
    schema: &'static str,
    size: usize,
    ty: FieldType,
    is_int: bool,
    is_uint: bool,
    bit_width: u32,
    bit_mask: u64,
    min: u64,
    max: u64,
) -> Simple {
    Simple {
        schema,
        size,
        ty,
        is_int,
        is_uint,
        bit_width,
        bit_mask,
        min,
        max,
    }
}

const SIMPLE: &[Simple] = &[
    simple(
        "bool a",
        1,
        FieldType::Bool,
        false,
        false,
        8,
        u8::MAX as u64,
        0,
        0,
    ),
    simple(
        "char a",
        1,
        FieldType::Char,
        false,
        false,
        8,
        u8::MAX as u64,
        0,
        0,
    ),
    simple(
        "int8 a",
        1,
        FieldType::Int8,
        true,
        false,
        8,
        u8::MAX as u64,
        i8::MIN as u64,
        i8::MAX as u64,
    ),
    simple(
        "int16 a",
        2,
        FieldType::Int16,
        true,
        false,
        16,
        u16::MAX as u64,
        i16::MIN as u64,
        i16::MAX as u64,
    ),
    simple(
        "int32 a",
        4,
        FieldType::Int32,
        true,
        false,
        32,
        u32::MAX as u64,
        i32::MIN as u64,
        i32::MAX as u64,
    ),
    simple(
        "int64 a",
        8,
        FieldType::Int64,
        true,
        false,
        64,
        u64::MAX,
        i64::MIN as u64,
        i64::MAX as u64,
    ),
    simple(
        "uint8 a",
        1,
        FieldType::Uint8,
        false,
        true,
        8,
        u8::MAX as u64,
        0,
        u8::MAX as u64,
    ),
    simple(
        "uint16 a",
        2,
        FieldType::Uint16,
        false,
        true,
        16,
        u16::MAX as u64,
        0,
        u16::MAX as u64,
    ),
    simple(
        "uint32 a",
        4,
        FieldType::Uint32,
        false,
        true,
        32,
        u32::MAX as u64,
        0,
        u32::MAX as u64,
    ),
    simple(
        "uint64 a",
        8,
        FieldType::Uint64,
        false,
        true,
        64,
        u64::MAX,
        0,
        u64::MAX,
    ),
    simple(
        "float a",
        4,
        FieldType::Float,
        false,
        false,
        32,
        u32::MAX as u64,
        0,
        0,
    ),
    simple(
        "float32 a",
        4,
        FieldType::Float,
        false,
        false,
        32,
        u32::MAX as u64,
        0,
        0,
    ),
    simple(
        "double a",
        8,
        FieldType::Double,
        false,
        false,
        64,
        u64::MAX,
        0,
        0,
    ),
    simple(
        "float64 a",
        8,
        FieldType::Double,
        false,
        false,
        64,
        u64::MAX,
        0,
        0,
    ),
    simple("foo a", 0, FieldType::Struct, false, false, 0, 0, 0, 0),
];

// upstream: "DynamicSimpleStructTest Check"
#[test]
fn simple_check() {
    for p in SIMPLE {
        let mut db = StructDatabase::new();
        db.add("test", p.schema).unwrap();
        let desc = db.find("test").unwrap();
        assert_eq!(desc.name(), "test");
        assert_eq!(desc.schema(), p.schema);
        let fields = desc.fields();
        assert_eq!(fields.len(), 1);
        let f = &fields[0];
        assert_eq!(f.name(), "a");
        assert_eq!(f.field_type(), p.ty);
        assert_eq!(
            (f.is_int(), f.is_uint()),
            (p.is_int, p.is_uint),
            "{}",
            p.schema
        );
        assert!(!f.is_array());
        if p.ty != FieldType::Struct {
            assert_eq!(desc.size(), Some(p.size), "{}", p.schema);
            assert_eq!(f.size(), p.size);
            assert_eq!(f.bit_width(), p.bit_width);
            assert_eq!(f.bit_mask(), p.bit_mask);
        } else {
            assert!(!desc.is_valid());
            assert!(db.nested(f).is_some());
        }
    }
}

// upstream: "DynamicSimpleStructTest Array"
#[test]
fn simple_array() {
    for p in SIMPLE {
        let mut db = StructDatabase::new();
        let schema = format!("{}[2]", p.schema);
        db.add("test", &schema).unwrap();
        let desc = db.find("test").unwrap();
        assert_eq!(desc.schema(), schema);
        let f = &desc.fields()[0];
        assert_eq!(f.name(), "a");
        assert_eq!((f.is_int(), f.is_uint()), (p.is_int, p.is_uint));
        assert!(f.is_array());
        assert_eq!(f.array_size(), 2);
        if p.ty != FieldType::Struct {
            assert_eq!(desc.size(), Some(p.size * 2), "{schema}");
        } else {
            assert!(!desc.is_valid());
            assert!(db.nested(f).is_some());
        }
    }
}

// upstream: "DynamicSimpleStructTest IntRoundTrip"
#[test]
fn simple_int_round_trip() {
    fn sign_extend(value: u64, size: usize) -> i64 {
        match size {
            1 => value as i8 as i64,
            2 => value as i16 as i64,
            4 => value as i32 as i64,
            _ => value as i64,
        }
    }
    for p in SIMPLE.iter().filter(|p| p.ty != FieldType::Struct) {
        let mut db = StructDatabase::new();
        let size = db.add("test", p.schema).unwrap().size().unwrap();
        let mut dest = vec![0u8; size];
        let mut dynamic = DynamicStructMut::new(&db, "test", &mut dest).unwrap();
        let field = dynamic.field("a").unwrap();
        if p.is_int {
            for v in [p.min, p.max] {
                let value = sign_extend(v, field.size());
                dynamic.set_int(field, value, 0).unwrap();
                assert_eq!(
                    dynamic.as_ref().get_int(field, 0),
                    Some(value),
                    "{}",
                    p.schema
                );
            }
        } else if p.is_uint {
            for value in [p.min, p.max] {
                dynamic.set_uint(field, value, 0).unwrap();
                assert_eq!(
                    dynamic.as_ref().get_uint(field, 0),
                    Some(value),
                    "{}",
                    p.schema
                );
            }
        } else if p.ty == FieldType::Bool {
            dynamic.set_bool(field, false, 0).unwrap();
            assert_eq!(dynamic.as_ref().get_bool(field, 0), Some(false));
            dynamic.set_bool(field, true, 0).unwrap();
            assert_eq!(dynamic.as_ref().get_bool(field, 0), Some(true));
        }
    }
}

// Written by Claude: what nt4 adds or does differently.
#[test]
fn rejected_schema_changes_nothing() {
    let mut db = StructDatabase::new();
    db.add("test", "foo a").unwrap();
    db.add("foo", "bar a").unwrap();
    db.add("bar", "test a").unwrap_err();
    // wpiutil keeps the rejected "bar" fields and its link from "test";
    // nt4 undoes the call, so bar is still just a placeholder.
    assert_eq!(db.find("bar").unwrap().schema(), "");
    assert!(db.find("bar").unwrap().fields().is_empty());
    db.add("bar", "int8 x").unwrap();
    assert_eq!(valid(&db, "test").size(), Some(1));
}

// Written by Claude.
#[test]
fn getters_check_type_struct_and_bounds() {
    let mut db = StructDatabase::new();
    db.add("A", "double x; int32 n[2]").unwrap();
    db.add("B", "double x").unwrap();
    let data = [0u8; 16];
    let a = DynamicStruct::new(&db, "A", &data).unwrap();
    let b_x = db.find("B").unwrap().field("x").unwrap();
    let a_x = a.field("x").unwrap();
    let n = a.field("n").unwrap();
    assert_eq!(a.get_double(a_x, 0), Some(0.0));
    assert_eq!(a.get_double(b_x, 0), None, "field of another struct");
    assert_eq!(a.get_int(a_x, 0), None, "wrong type");
    assert_eq!(a.get_int(n, 1), Some(0));
    assert_eq!(a.get_int(n, 2), None, "past the end");
    assert!(
        DynamicStruct::new(&db, "A", &data[..15]).is_none(),
        "short data"
    );
    assert!(
        DynamicStruct::new(&db, "C", &data).is_none(),
        "unknown struct"
    );
}

// Written by Claude.
#[test]
fn big_nested_struct_fields() {
    // A nested struct over 8 bytes as a field: wpiutil computes a bit mask
    // for it with an out-of-range shift; nt4 doesn't.
    let mut db = StructDatabase::new();
    db.add("Translation3d", "double x;double y;double z")
        .unwrap();
    db.add("Quaternion", "double w;double x;double y;double z")
        .unwrap();
    db.add("Rotation3d", "Quaternion q").unwrap();
    db.add("Pose3d", "Translation3d translation;Rotation3d rotation")
        .unwrap();
    assert_eq!(valid(&db, "Pose3d").size(), Some(56));
    let values: Vec<u8> = (1..=7).flat_map(|v| (v as f64).to_le_bytes()).collect();
    let pose = DynamicStruct::new(&db, "Pose3d", &values).unwrap();
    let rotation = pose.get_struct(pose.field("rotation").unwrap(), 0).unwrap();
    let q = rotation
        .get_struct(rotation.field("q").unwrap(), 0)
        .unwrap();
    assert_eq!(q.get_double(q.field("w").unwrap(), 0), Some(4.0));
    assert_eq!(format!("{pose:?}").matches("Double").count(), 7);
}

// Written by Claude.
#[test]
fn struct_arrays_and_set_struct() {
    let mut db = StructDatabase::new();
    db.add("Inner", "uint16 v").unwrap();
    db.add("Outer", "Inner items[2]").unwrap();
    let inner_bytes = [0x34, 0x12];
    let inner = DynamicStruct::new(&db, "Inner", &inner_bytes).unwrap();
    let mut data = [0u8; 4];
    let mut outer = DynamicStructMut::new(&db, "Outer", &mut data).unwrap();
    let items = outer.field("items").unwrap();
    outer.set_struct(items, &inner, 1).unwrap();
    assert!(outer.set_struct(items, &inner, 2).is_none());
    assert_eq!(data, [0, 0, 0x34, 0x12]);
    assert_eq!(
        DynamicStruct::array(&db, "Outer", &[0; 8]).unwrap().len(),
        2
    );
    assert!(DynamicStruct::array(&db, "Outer", &[0; 7]).is_none());
}

// Written by Claude: schemas come off the network, so no schema and no
// value bytes may make the database or the views panic.
#[test]
fn random_schemas_never_panic() {
    use nt4_proto::structs::FieldValue;
    fn walk(v: &FieldValue<'_>) {
        match v {
            FieldValue::Struct(s) => s.values().iter().for_each(|(_, v)| walk(v)),
            FieldValue::Array(items) => items.iter().for_each(walk),
            _ => {}
        }
    }
    let words = [
        "bool",
        "char",
        "int8",
        "int16",
        "int32",
        "int64",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "float",
        "double",
        "float64",
        "A",
        "B",
        "C",
        "D",
        "a",
        "b",
        "enum",
        "{",
        "}",
        "[",
        "]",
        ":",
        ";",
        ",",
        "=",
        "0",
        "1",
        "3",
        "7",
        "8",
        "17",
        "63",
        "64",
        "65",
        "-1",
        "99999999999",
        "18446744073709551615",
        "x=1",
        "🚀",
        "?",
    ];
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut db = StructDatabase::new();
    for _ in 0..20_000 {
        let len = next(12);
        let schema: Vec<&str> = (0..len).map(|_| words[next(words.len())]).collect();
        let _ = db.add(["A", "B", "C", "D"][next(4)], &schema.join(" "));
        let bytes: Vec<u8> = (0..next(64)).map(|_| next(256) as u8).collect();
        for name in ["A", "B", "C", "D"] {
            if let Some(view) = DynamicStruct::new(&db, name, &bytes) {
                view.values().iter().for_each(|(_, v)| walk(v));
            }
            let mut data = bytes.clone();
            if let Some(mut view) = DynamicStructMut::new(&db, name, &mut data) {
                for field in view.descriptor().fields() {
                    let _ = view.set_int(field, -1, 0);
                    let _ = view.set_uint(field, u64::MAX, 0);
                    let _ = view.set_string(field, "héllo");
                    if let Some(mut inner) = view.get_struct_mut(field, 0) {
                        inner.data().fill(0xff);
                    }
                }
            }
        }
    }
}

// Written by Claude: wpiutil wraps a huge size around; nt4 refuses it.
#[test]
fn huge_structs_are_refused() {
    let mut db = StructDatabase::new();
    assert_eq!(
        db.add("A", "int64 a[18446744073709551615]").unwrap_err(),
        "field a: struct too large"
    );
    db.add("Big", "uint8 a[1000000000]").unwrap();
    assert_eq!(
        db.add("Two", "Big b[2]").unwrap_err(),
        "field b: struct too large"
    );
    assert!(
        db.find("Two").is_none(),
        "the refused add left nothing behind"
    );
}
