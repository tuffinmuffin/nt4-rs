//! Struct schema parser cases ported from wpiutil's SchemaParserTest.

// Ported from allwpilib wpiutil/src/test/native/cpp/struct/SchemaParserTest.cpp
// at commit 4ccfe967ba1903b1f409b04b99db0922dd3b7318.
// Copyright (c) FIRST and other WPILib contributors.
// The test cases come from that file and stay under the WPILib BSD license;
// see tests/LICENSE-WPILIB.md. The Rust test code was written by Claude.

use nt4_proto::structs::{Declaration, parse_schema};

fn one(schema: &str) -> Declaration {
    let mut decls = parse_schema(schema).unwrap();
    assert_eq!(decls.len(), 1, "{schema}");
    decls.remove(0)
}

fn err(schema: &str) -> String {
    parse_schema(schema).unwrap_err()
}

// upstream: "StructParserTest Empty"
#[test]
fn empty() {
    assert!(parse_schema("").unwrap().is_empty());
}

// upstream: "StructParserTest EmptySemicolon"
#[test]
fn empty_semicolon() {
    assert!(parse_schema(";").unwrap().is_empty());
}

// upstream: "StructParserTest Simple"
#[test]
fn simple() {
    let decl = one("int32 a");
    assert_eq!(decl.type_name, "int32");
    assert_eq!(decl.name, "a");
    assert_eq!(decl.array_size, 1);
}

// upstream: "StructParserTest UTF8Identifiers"
#[test]
fn utf8_identifiers() {
    let decl = one("{😀=1,二=2} 🚀Type data💾;");
    assert_eq!(decl.type_name, "🚀Type");
    assert_eq!(decl.name, "data💾");
    assert_eq!(
        decl.enum_values,
        vec![("😀".to_string(), 1), ("二".to_string(), 2)]
    );
}

// upstream: "StructParserTest SimpleTrailingSemi"
#[test]
fn simple_trailing_semi() {
    one("int32 a;");
}

// upstream: "StructParserTest Array"
#[test]
fn array() {
    let decl = one("int32 a[2]");
    assert_eq!(decl.type_name, "int32");
    assert_eq!(decl.name, "a");
    assert_eq!(decl.array_size, 2);
}

// upstream: "StructParserTest ArrayTrailingSemi"
#[test]
fn array_trailing_semi() {
    one("int32 a[2];");
}

// upstream: "StructParserTest Bitfield"
#[test]
fn bitfield() {
    let decl = one("int32 a:2");
    assert_eq!(decl.type_name, "int32");
    assert_eq!(decl.name, "a");
    assert_eq!(decl.bit_width, 2);
}

// upstream: "StructParserTest BitfieldTrailingSemi"
#[test]
fn bitfield_trailing_semi() {
    one("int32 a:2;");
}

// upstream: "StructParserTest EnumKeyword"
#[test]
fn enum_keyword() {
    let decl = one("enum {x=1} int32 a;");
    assert_eq!(decl.type_name, "int32");
    assert_eq!(decl.name, "a");
    assert_eq!(decl.enum_values, vec![("x".to_string(), 1)]);
}

// upstream: "StructParserTest EnumNoKeyword"
#[test]
fn enum_no_keyword() {
    let decl = one("{x=1} int32 a;");
    assert_eq!(decl.type_name, "int32");
    assert_eq!(decl.name, "a");
    assert_eq!(decl.enum_values, vec![("x".to_string(), 1)]);
}

// upstream: "StructParserTest EnumNoValues"
#[test]
fn enum_no_values() {
    let decl = one("{} int32 a;");
    assert_eq!(decl.type_name, "int32");
    assert_eq!(decl.name, "a");
    assert!(decl.enum_values.is_empty());
}

// upstream: "StructParserTest EnumMultipleValues"
#[test]
fn enum_multiple_values() {
    let decl = one("{x=1,y=-2} int32 a;");
    assert_eq!(
        decl.enum_values,
        vec![("x".to_string(), 1), ("y".to_string(), -2)]
    );
}

// upstream: "StructParserTest EnumTrailingComma"
#[test]
fn enum_trailing_comma() {
    let decl = one("{x=1,y=2,} int32 a;");
    assert_eq!(
        decl.enum_values,
        vec![("x".to_string(), 1), ("y".to_string(), 2)]
    );
}

// upstream: "StructParserTest MultipleNoTrailingSemi"
#[test]
fn multiple_no_trailing_semi() {
    let decls = parse_schema("int32 a; int16 b").unwrap();
    assert_eq!(decls.len(), 2);
    assert_eq!(
        (decls[0].type_name.as_str(), decls[0].name.as_str()),
        ("int32", "a")
    );
    assert_eq!(
        (decls[1].type_name.as_str(), decls[1].name.as_str()),
        ("int16", "b")
    );
}

// upstream: "StructParserTest ErrBitfieldArray"
#[test]
fn err_bitfield_array() {
    assert_eq!(err("int32 a[1]:2"), "10: expected ';', got ':'");
}

// upstream: "StructParserTest ErrNoArrayValue"
#[test]
fn err_no_array_value() {
    assert_eq!(err("int32 a[]"), "8: expected integer, got ']'");
}

// upstream: "StructParserTest ErrNoBitfieldValue"
#[test]
fn err_no_bitfield_value() {
    assert_eq!(err("int32 a:"), "8: expected integer, got ''");
}

// upstream: "StructParserTest ErrNoNameArray"
#[test]
fn err_no_name_array() {
    assert_eq!(err("int32 [2]"), "6: expected identifier, got '['");
}

// upstream: "StructParserTest ErrNoNameBitField"
#[test]
fn err_no_name_bit_field() {
    assert_eq!(err("int32 :2"), "6: expected identifier, got ':'");
}

// upstream: "StructParserTest NegativeBitField"
#[test]
fn negative_bit_field() {
    assert_eq!(
        err("int32 a:-1"),
        "8: bitfield width '-1' is not a positive integer"
    );
}

// upstream: "StructParserTest NegativeArraySize"
#[test]
fn negative_array_size() {
    assert_eq!(
        err("int32 a[-1]"),
        "8: array size '-1' is not a positive integer"
    );
}

// upstream: "StructParserTest ZeroBitField"
#[test]
fn zero_bit_field() {
    assert_eq!(
        err("int32 a:0"),
        "8: bitfield width '0' is not a positive integer"
    );
}

// upstream: "StructParserTest ZeroArraySize"
#[test]
fn zero_array_size() {
    assert_eq!(
        err("int32 a[0]"),
        "8: array size '0' is not a positive integer"
    );
}

// Written by Claude: input wpiutil's tests don't cover.
#[test]
fn other_errors() {
    assert_eq!(err("{x=-} int32 a"), "3: could not parse enum value '-'");
    assert_eq!(err("int32 a b"), "8: expected ';', got 'b'");
    assert_eq!(
        err("int32 a[99999999999999999999]"),
        "8: array size '99999999999999999999' is not a positive integer"
    );
    assert_eq!(err("int32 a; ?"), "9: expected identifier, got '?'");
    assert_eq!(err("enum int32 a"), "5: expected '{', got 'int32'");
    // Whitespace of every kind separates tokens.
    let decls = parse_schema("\tdouble\r\nx ;\n").unwrap();
    assert_eq!(decls[0].name, "x");
}
