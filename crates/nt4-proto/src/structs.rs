//! WPILib struct schemas, read at run time.
//!
//! A topic of type `struct:Pose2d` carries packed little-endian bytes. Its
//! layout is described by a schema string (`Translation2d
//! translation;Rotation2d rotation`) that the publisher puts on the topic
//! `/.schema/struct:Pose2d`, of type `structschema`. This module lets a
//! program that doesn't know `Pose2d` at compile time (a dashboard, a
//! logger) decode such values:
//!
//! - [`parse_schema`] turns a schema string into [`Declaration`]s.
//! - [`StructDatabase`] holds the schemas seen so far and works out field
//!   offsets, including structs nested in structs whose schemas arrive in
//!   any order.
//! - [`DynamicStruct`] reads the fields of one value, [`DynamicStructMut`]
//!   writes them.
//!
//! This is a port of wpiutil's `SchemaParser` and `DynamicStruct`, with the
//! same layout rules (bit-field packing included) and error text, so a
//! schema means here what it means to ntcore. Types known at compile time
//! should implement `nt4::NtStruct` instead.
//!
//! ```
//! use nt4_proto::structs::{DynamicStruct, FieldValue, StructDatabase};
//!
//! let mut db = StructDatabase::new();
//! // Schemas can arrive in any order; Pose2d becomes usable once both of
//! // its nested types are known.
//! db.add("Pose2d", "Translation2d translation;Rotation2d rotation").unwrap();
//! db.add("Translation2d", "double x;double y").unwrap();
//! db.add("Rotation2d", "double value").unwrap();
//!
//! let bytes: Vec<u8> = [1.0f64, 2.0, 0.5].iter().flat_map(|v| v.to_le_bytes()).collect();
//! let pose = DynamicStruct::new(&db, "Pose2d", &bytes).unwrap();
//! let translation = pose.get_struct(pose.field("translation").unwrap(), 0).unwrap();
//! assert_eq!(translation.get_double(translation.field("y").unwrap(), 0), Some(2.0));
//! assert!(matches!(pose.values()[1].1, FieldValue::Struct(_)));
//! ```

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Parser (wpiutil SchemaParser)

/// One field declaration of a schema, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// The type as written, e.g. `int32`, `float64` or `Translation2d`.
    pub type_name: String,
    /// The field name.
    pub name: String,
    /// Named values from an `enum {a=1, b=2}` prefix, in order.
    pub enum_values: Vec<(String, i64)>,
    /// Number of elements; 1 unless declared as `name[n]`.
    pub array_size: usize,
    /// Bit-field width from `name:n`; 0 when not a bit field.
    pub bit_width: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    Identifier,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Colon,
    Semicolon,
    Comma,
    Equals,
    EndOfInput,
    Unknown,
}

impl Kind {
    fn describe(self) -> &'static str {
        match self {
            Kind::Integer => "integer",
            Kind::Identifier => "identifier",
            Kind::LeftBracket => "'['",
            Kind::RightBracket => "']'",
            Kind::LeftBrace => "'{'",
            Kind::RightBrace => "'}'",
            Kind::Colon => "':'",
            Kind::Semicolon => "';'",
            Kind::Comma => "','",
            Kind::Equals => "'='",
            Kind::EndOfInput => "<EOF>",
            Kind::Unknown => "unknown",
        }
    }
}

struct Token {
    kind: Kind,
    text: String,
}

/// Works on bytes, as wpiutil does: every byte from 0x80 up counts as an
/// identifier character, so UTF-8 names lex whole.
struct Lexer<'a> {
    input: &'a [u8],
    current: Option<u8>,
    token_start: usize,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            current: None,
            token_start: 0,
            pos: 0,
        }
    }

    fn get(&mut self) {
        self.current = self.input.get(self.pos).copied();
        self.pos += 1;
    }

    fn unget(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
            self.current = self.input.get(self.pos).copied();
        } else {
            self.current = None;
        }
    }

    fn token(&self, kind: Kind) -> Token {
        let end = self.pos.min(self.input.len());
        let start = self.token_start.min(end);
        Token {
            kind,
            text: String::from_utf8_lossy(&self.input[start..end]).into_owned(),
        }
    }

    fn scan(&mut self) -> Token {
        loop {
            self.get();
            if !matches!(self.current, Some(b' ' | b'\t' | b'\n' | b'\r')) {
                break;
            }
        }
        self.token_start = self.pos - 1;
        let Some(c) = self.current else {
            return Token {
                kind: Kind::EndOfInput,
                text: String::new(),
            };
        };
        match c {
            b'[' => self.token(Kind::LeftBracket),
            b']' => self.token(Kind::RightBracket),
            b'{' => self.token(Kind::LeftBrace),
            b'}' => self.token(Kind::RightBrace),
            b':' => self.token(Kind::Colon),
            b';' => self.token(Kind::Semicolon),
            b',' => self.token(Kind::Comma),
            b'=' => self.token(Kind::Equals),
            b'-' | b'0'..=b'9' => {
                loop {
                    self.get();
                    if !matches!(self.current, Some(b'0'..=b'9')) {
                        break;
                    }
                }
                self.unget();
                self.token(Kind::Integer)
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 => {
                loop {
                    self.get();
                    match self.current {
                        Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 => {}
                        _ => break,
                    }
                }
                self.unget();
                self.token(Kind::Identifier)
            }
            _ => self.token(Kind::Unknown),
        }
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    token: Token,
}

type ParseResult<T> = Result<T, String>;

impl Parser<'_> {
    fn next(&mut self) -> Kind {
        self.token = self.lexer.scan();
        self.token.kind
    }

    fn fail<T>(&self, msg: String) -> ParseResult<T> {
        Err(format!("{}: {msg}", self.lexer.token_start))
    }

    fn expect(&self, kind: Kind) -> ParseResult<()> {
        if self.token.kind == kind {
            Ok(())
        } else {
            self.fail(format!(
                "expected {}, got '{}'",
                kind.describe(),
                self.token.text
            ))
        }
    }

    fn parse(&mut self) -> ParseResult<Vec<Declaration>> {
        let mut out = Vec::new();
        loop {
            match self.next() {
                Kind::Semicolon => continue,
                Kind::EndOfInput => break,
                _ => {}
            }
            out.push(self.declaration()?);
            if self.token.kind == Kind::EndOfInput {
                break;
            }
        }
        Ok(out)
    }

    fn declaration(&mut self) -> ParseResult<Declaration> {
        let mut enum_values = Vec::new();
        if self.token.kind == Kind::Identifier && self.token.text == "enum" {
            self.next();
            self.expect(Kind::LeftBrace)?;
            enum_values = self.enum_values()?;
            self.next();
        } else if self.token.kind == Kind::LeftBrace {
            enum_values = self.enum_values()?;
            self.next();
        }

        self.expect(Kind::Identifier)?;
        let type_name = std::mem::take(&mut self.token.text);
        self.next();

        self.expect(Kind::Identifier)?;
        let name = std::mem::take(&mut self.token.text);
        self.next();

        let mut array_size = 1;
        let mut bit_width = 0;
        if self.token.kind == Kind::LeftBracket {
            self.next();
            self.expect(Kind::Integer)?;
            match self.token.text.parse::<u64>() {
                Ok(n) if n > 0 => array_size = n as usize,
                _ => {
                    return self.fail(format!(
                        "array size '{}' is not a positive integer",
                        self.token.text
                    ));
                }
            }
            self.next();
            self.expect(Kind::RightBracket)?;
            self.next();
        } else if self.token.kind == Kind::Colon {
            self.next();
            self.expect(Kind::Integer)?;
            match self.token.text.parse::<u32>() {
                Ok(n) if n > 0 => bit_width = n,
                _ => {
                    return self.fail(format!(
                        "bitfield width '{}' is not a positive integer",
                        self.token.text
                    ));
                }
            }
            self.next();
        }

        // A declaration ends at a semicolon or the end of the schema.
        if self.token.kind != Kind::EndOfInput {
            self.expect(Kind::Semicolon)?;
        }
        Ok(Declaration {
            type_name,
            name,
            enum_values,
            array_size,
            bit_width,
        })
    }

    /// Called with the current token on `{`; returns on `}`.
    fn enum_values(&mut self) -> ParseResult<Vec<(String, i64)>> {
        let mut out = Vec::new();
        self.next();
        while self.token.kind != Kind::RightBrace {
            self.expect(Kind::Identifier)?;
            let name = std::mem::take(&mut self.token.text);
            self.next();
            self.expect(Kind::Equals)?;
            self.next();
            self.expect(Kind::Integer)?;
            let Ok(value) = self.token.text.parse::<i64>() else {
                return self.fail(format!("could not parse enum value '{}'", self.token.text));
            };
            out.push((name, value));
            self.next();
            if self.token.kind == Kind::RightBrace {
                break;
            }
            self.expect(Kind::Comma)?;
            self.next();
        }
        Ok(out)
    }
}

/// Parse a struct schema into its declarations. Errors read like
/// wpiutil's: `"<byte offset>: <problem>"`.
///
/// ```
/// use nt4_proto::structs::parse_schema;
/// let decls = parse_schema("enum {off=0, on=1} uint8 mode; double x[2]").unwrap();
/// assert_eq!(decls[0].enum_values[1], ("on".to_string(), 1));
/// assert_eq!(decls[1].array_size, 2);
/// assert_eq!(parse_schema("int32 a[]").unwrap_err(), "8: expected integer, got ']'");
/// ```
pub fn parse_schema(schema: &str) -> Result<Vec<Declaration>, String> {
    Parser {
        lexer: Lexer::new(schema),
        token: Token {
            kind: Kind::Unknown,
            text: String::new(),
        },
    }
    .parse()
}

// ---------------------------------------------------------------------------
// Descriptors (wpiutil StructDescriptor / StructDescriptorDatabase)

/// Largest struct accepted, in bytes (1 GiB): far beyond any real one, and
/// small enough that offsets can't overflow, even with a 32-bit `usize`.
const MAX_STRUCT_SIZE: usize = 1 << 30;

/// The type of a struct field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldType {
    /// `bool`, 1 byte (or one bit in a bit field).
    Bool,
    /// `char`, 1 byte; a `char` array is a UTF-8 string.
    Char,
    /// `int8`.
    Int8,
    /// `int16`.
    Int16,
    /// `int32`.
    Int32,
    /// `int64`.
    Int64,
    /// `uint8`.
    Uint8,
    /// `uint16`.
    Uint16,
    /// `uint32`.
    Uint32,
    /// `uint64`.
    Uint64,
    /// `float` or `float32`.
    Float,
    /// `double` or `float64`.
    Double,
    /// Any other name: a nested struct.
    Struct,
}

impl FieldType {
    fn from_name(name: &str) -> Self {
        match name {
            "bool" => FieldType::Bool,
            "char" => FieldType::Char,
            "int8" => FieldType::Int8,
            "int16" => FieldType::Int16,
            "int32" => FieldType::Int32,
            "int64" => FieldType::Int64,
            "uint8" => FieldType::Uint8,
            "uint16" => FieldType::Uint16,
            "uint32" => FieldType::Uint32,
            "uint64" => FieldType::Uint64,
            "float" | "float32" => FieldType::Float,
            "double" | "float64" => FieldType::Double,
            _ => FieldType::Struct,
        }
    }

    /// Size in bytes; 0 for [`FieldType::Struct`].
    fn size(self) -> usize {
        match self {
            FieldType::Bool | FieldType::Char | FieldType::Int8 | FieldType::Uint8 => 1,
            FieldType::Int16 | FieldType::Uint16 => 2,
            FieldType::Int32 | FieldType::Uint32 | FieldType::Float => 4,
            FieldType::Int64 | FieldType::Uint64 | FieldType::Double => 8,
            FieldType::Struct => 0,
        }
    }
}

/// One field of a [`StructDescriptor`].
#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    parent: usize,
    name: String,
    ty: FieldType,
    type_name: String,
    size: usize,
    offset: usize,
    array_size: usize,
    enum_values: Vec<(String, i64)>,
    nested: Option<usize>,
    bit_mask: u64,
    bit_width: u32,
    bit_shift: u32,
}

impl FieldDescriptor {
    /// The field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field type.
    pub fn field_type(&self) -> FieldType {
        self.ty
    }

    /// The type as written in the schema (the struct name for nested
    /// structs).
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// Whether the field is a signed integer.
    pub fn is_int(&self) -> bool {
        matches!(
            self.ty,
            FieldType::Int8 | FieldType::Int16 | FieldType::Int32 | FieldType::Int64
        )
    }

    /// Whether the field is an unsigned integer.
    pub fn is_uint(&self) -> bool {
        matches!(
            self.ty,
            FieldType::Uint8 | FieldType::Uint16 | FieldType::Uint32 | FieldType::Uint64
        )
    }

    /// Size of one element in bytes (for a bit field, of the integer it
    /// is packed into).
    pub fn size(&self) -> usize {
        self.size
    }

    /// Byte offset of the field in the struct.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Width in bits: the bit-field width, or the whole size.
    pub fn bit_width(&self) -> u32 {
        self.bit_width
    }

    /// Mask of the field's bits, before shifting.
    pub fn bit_mask(&self) -> u64 {
        self.bit_mask
    }

    /// Bit position of a bit field within its integer.
    pub fn bit_shift(&self) -> u32 {
        self.bit_shift
    }

    /// Whether the field is an array (more than one element).
    pub fn is_array(&self) -> bool {
        self.array_size > 1
    }

    /// Number of elements; 1 for a plain field.
    pub fn array_size(&self) -> usize {
        self.array_size
    }

    /// Named values from the schema's `enum {..}`, in order.
    pub fn enum_values(&self) -> &[(String, i64)] {
        &self.enum_values
    }

    /// The name of an enum value, if the schema gives one.
    ///
    /// ```
    /// use nt4_proto::structs::StructDatabase;
    /// let mut db = StructDatabase::new();
    /// let desc = db.add("Mode", "enum {off=0, on=1} uint8 mode").unwrap();
    /// assert_eq!(desc.field("mode").unwrap().enum_name(1), Some("on"));
    /// ```
    pub fn enum_name(&self, value: i64) -> Option<&str> {
        self.enum_values
            .iter()
            .find(|(_, v)| *v == value)
            .map(|(n, _)| n.as_str())
    }

    /// Smallest value an unsigned field holds (0).
    pub fn uint_min(&self) -> u64 {
        0
    }

    /// Largest value an unsigned field holds.
    pub fn uint_max(&self) -> u64 {
        self.bit_mask
    }

    /// Smallest value a signed field holds.
    pub fn int_min(&self) -> i64 {
        -((self.bit_mask >> 1) as i64) - 1
    }

    /// Largest value a signed field holds.
    pub fn int_max(&self) -> i64 {
        (self.bit_mask >> 1) as i64
    }

    /// Whether the field is a bit field (packed with others into one
    /// integer).
    pub fn is_bit_field(&self) -> bool {
        (self.bit_shift != 0 || self.bit_width as usize != self.size * 8) && self.nested.is_none()
    }
}

/// The layout of one struct type.
#[derive(Debug, Clone)]
pub struct StructDescriptor {
    id: usize,
    name: String,
    schema: String,
    references: Vec<usize>,
    fields: Vec<FieldDescriptor>,
    by_name: HashMap<String, usize>,
    size: usize,
    valid: bool,
}

impl StructDescriptor {
    fn placeholder(id: usize, name: &str) -> Self {
        Self {
            id,
            name: name.to_owned(),
            schema: String::new(),
            references: Vec::new(),
            fields: Vec::new(),
            by_name: HashMap::new(),
            size: 0,
            valid: false,
        }
    }

    /// The struct's type name, e.g. `Pose2d`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The schema string it was added with (empty for a struct that so far
    /// is only referred to).
    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// Whether the layout is known: its schema and those of every struct it
    /// contains have been added.
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Size in bytes, once [valid](Self::is_valid).
    pub fn size(&self) -> Option<usize> {
        self.valid.then_some(self.size)
    }

    /// A field by name.
    pub fn field(&self, name: &str) -> Option<&FieldDescriptor> {
        self.by_name.get(name).map(|&i| &self.fields[i])
    }

    /// All fields, in schema order.
    pub fn fields(&self) -> &[FieldDescriptor] {
        &self.fields
    }
}

/// The struct schemas seen so far, by type name.
///
/// Add each `/.schema/struct:<Name>` value with
/// [`add`](Self::add)`("<Name>", schema)`. A struct that contains another
/// one stays invalid until that one has been added too.
#[derive(Debug, Clone, Default)]
pub struct StructDatabase {
    structs: Vec<StructDescriptor>,
    by_name: HashMap<String, usize>,
}

impl StructDatabase {
    /// An empty database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add (or replace) the schema of struct `name`.
    ///
    /// Errors use wpiutil's wording, e.g. `"duplicate field a"` or
    /// `"circular struct reference: bar <- foo <- test"`.
    ///
    /// ```
    /// use nt4_proto::structs::StructDatabase;
    /// let mut db = StructDatabase::new();
    /// assert_eq!(db.add("Outer", "Inner i").unwrap().is_valid(), false);
    /// assert_eq!(db.add("Inner", "int32 a").unwrap().size(), Some(4));
    /// assert_eq!(db.find("Outer").unwrap().size(), Some(4));
    /// assert_eq!(db.add("Bad", "float f:1").unwrap_err(), "field f: type float cannot be bitfield");
    /// ```
    pub fn add(&mut self, name: &str, schema: &str) -> Result<&StructDescriptor, String> {
        let decls = parse_schema(schema).map_err(|e| format!("parse error: {e}"))?;
        // wpiutil leaves a half-updated struct behind when a schema is
        // rejected (fields cleared, references to it kept). Undo the
        // whole call instead, so a bad schema changes nothing.
        let before = self.clone();
        match self.add_parsed(name, schema, decls) {
            Ok(id) => Ok(&self.structs[id]),
            Err(e) => {
                *self = before;
                Err(e)
            }
        }
    }

    /// A struct by type name (valid or not).
    pub fn find(&self, name: &str) -> Option<&StructDescriptor> {
        self.by_name.get(name).map(|&i| &self.structs[i])
    }

    /// The struct a [`FieldType::Struct`] field contains.
    pub fn nested(&self, field: &FieldDescriptor) -> Option<&StructDescriptor> {
        field.nested.map(|i| &self.structs[i])
    }

    fn get_or_create(&mut self, name: &str) -> usize {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = self.structs.len();
        self.structs.push(StructDescriptor::placeholder(id, name));
        self.by_name.insert(name.to_owned(), id);
        id
    }

    fn add_parsed(
        &mut self,
        name: &str,
        schema: &str,
        decls: Vec<Declaration>,
    ) -> Result<usize, String> {
        let id = self.get_or_create(name);
        let mut fields = Vec::with_capacity(decls.len());
        let mut by_name = HashMap::new();
        let mut valid = true;
        for decl in decls {
            let ty = FieldType::from_name(&decl.type_name);
            let mut size = ty.size();

            if decl.bit_width != 0 {
                if matches!(
                    ty,
                    FieldType::Char | FieldType::Float | FieldType::Double | FieldType::Struct
                ) {
                    return Err(format!(
                        "field {}: type {} cannot be bitfield",
                        decl.name, decl.type_name
                    ));
                }
                if decl.bit_width as usize > size * 8 {
                    return Err(format!(
                        "field {}: bit width {} exceeds type size",
                        decl.name, decl.bit_width
                    ));
                }
                if ty == FieldType::Bool && decl.bit_width != 1 {
                    return Err(format!(
                        "field {}: bit width must be 1 for bool type",
                        decl.name
                    ));
                }
                // The parser can't produce this, but check as wpiutil does.
                if decl.array_size > 1 {
                    return Err(format!(
                        "field {}: cannot combine array and bitfield",
                        decl.name
                    ));
                }
            }

            let mut nested = None;
            if ty == FieldType::Struct {
                if decl.type_name == name {
                    return Err(format!("field {}: recursive struct reference", decl.name));
                }
                let sid = self.get_or_create(&decl.type_name);
                let inner = &mut self.structs[sid];
                if inner.valid {
                    size = inner.size;
                } else {
                    valid = false;
                }
                // So the inner struct can update us once it becomes valid.
                if !inner.references.contains(&id) {
                    inner.references.push(id);
                }
                nested = Some(sid);
            }

            if by_name.insert(decl.name.clone(), fields.len()).is_some() {
                return Err(format!("duplicate field {}", decl.name));
            }
            let (bit_width, bit_mask) = if nested.is_some() {
                // Masks mean nothing for a nested struct, and a struct over
                // 8 bytes would overflow wpiutil's shift.
                (0, 0)
            } else {
                let width = if decl.bit_width == 0 {
                    size as u32 * 8
                } else {
                    decl.bit_width
                };
                (width, u64::MAX >> (64 - width))
            };
            fields.push(FieldDescriptor {
                parent: id,
                name: decl.name,
                ty,
                type_name: decl.type_name,
                size,
                offset: 0,
                array_size: decl.array_size,
                enum_values: decl.enum_values,
                nested,
                bit_mask,
                bit_width,
                bit_shift: 0,
            });
        }

        let s = &mut self.structs[id];
        s.schema = schema.to_owned();
        s.fields = fields;
        s.by_name = by_name;
        s.valid = valid;
        let mut stack = Vec::new();
        if valid {
            self.calculate_offsets(id, &mut stack)?;
        } else if !self.check_circular(id, &mut stack) {
            let path: Vec<&str> = stack.iter().map(|&i| self.structs[i].name()).collect();
            return Err(format!("circular struct reference: {}", path.join(" <- ")));
        }
        Ok(id)
    }

    /// Walks up through the structs that contain `id`; false (leaving the
    /// loop in `stack`) if it comes back to one already on the path.
    fn check_circular(&self, id: usize, stack: &mut Vec<usize>) -> bool {
        stack.push(id);
        for &r in &self.structs[id].references {
            if stack.contains(&r) || !self.check_circular(r, stack) {
                return false;
            }
        }
        stack.pop();
        true
    }

    fn calculate_offsets(&mut self, id: usize, stack: &mut Vec<usize>) -> Result<(), String> {
        let nested_sizes: Vec<Option<Option<usize>>> = self.structs[id]
            .fields
            .iter()
            .map(|f| f.nested.map(|n| self.structs[n].size()))
            .collect();
        let s = &mut self.structs[id];
        let mut offset = 0;
        let mut shift = 0u32;
        let mut prev_bitfield_size = 0;
        for (field, nested) in s.fields.iter_mut().zip(nested_sizes) {
            if !field.is_bit_field() {
                shift = 0;
                offset += prev_bitfield_size;
                prev_bitfield_size = 0;
                field.offset = offset;
                if let Some(nested) = nested {
                    match nested {
                        Some(size) => field.size = size,
                        None => {
                            s.valid = false;
                            return Ok(());
                        }
                    }
                }
                // Schemas come off the network: wpiutil lets a huge array
                // size wrap around; refuse it instead.
                offset = field
                    .size
                    .checked_mul(field.array_size)
                    .and_then(|n| n.checked_add(offset))
                    .filter(|&n| n <= MAX_STRUCT_SIZE)
                    .ok_or_else(|| format!("field {}: struct too large", field.name))?;
            } else {
                if field.ty == FieldType::Bool
                    && prev_bitfield_size != 0
                    && (shift + 1) as usize <= prev_bitfield_size * 8
                {
                    // A one-bit bool joins the integer before it, if it fits.
                    field.size = prev_bitfield_size;
                } else if field.size != prev_bitfield_size
                    || (shift + field.bit_width) as usize > field.size * 8
                {
                    shift = 0;
                    offset += prev_bitfield_size;
                }
                prev_bitfield_size = field.size;
                field.offset = offset;
                field.bit_shift = shift;
                shift += field.bit_width;
            }
        }
        s.size = offset + prev_bitfield_size;
        s.valid = true;

        // Structs that contain this one may be complete now.
        stack.push(id);
        for r in self.structs[id].references.clone() {
            if stack.contains(&r) {
                return Err(format!(
                    "internal error (inconsistent data): circular struct reference between {} and {}",
                    self.structs[id].name, self.structs[r].name
                ));
            }
            self.calculate_offsets(r, stack)?;
        }
        stack.pop();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Values (wpiutil DynamicStruct / MutableDynamicStruct)

/// One field's value, from [`DynamicStruct::value`].
#[derive(Debug, Clone)]
pub enum FieldValue<'a> {
    /// A `bool`.
    Bool(bool),
    /// A signed integer of any width.
    Int(i64),
    /// An unsigned integer of any width.
    Uint(u64),
    /// A `float`.
    Float(f32),
    /// A `double`.
    Double(f64),
    /// A `char` field or array, as text (see [`DynamicStruct::get_string`]).
    String(Cow<'a, str>),
    /// A nested struct.
    Struct(DynamicStruct<'a>),
    /// An array of any of the above except strings.
    Array(Vec<FieldValue<'a>>),
}

fn read_raw(data: &[u8], field: &FieldDescriptor, index: usize) -> Option<u64> {
    if index >= field.array_size {
        return None;
    }
    let start = field.offset + index * field.size;
    let bytes = data.get(start..start + field.size)?;
    let mut buf = [0u8; 8];
    buf[..bytes.len()].copy_from_slice(bytes);
    Some((u64::from_le_bytes(buf) >> field.bit_shift) & field.bit_mask)
}

/// A read-only view of one struct value.
#[derive(Clone, Copy)]
pub struct DynamicStruct<'a> {
    db: &'a StructDatabase,
    desc: &'a StructDescriptor,
    data: &'a [u8],
}

impl fmt::Debug for DynamicStruct<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct(&self.desc.name);
        for (name, value) in self.values() {
            s.field(name, &value);
        }
        s.finish()
    }
}

impl<'a> DynamicStruct<'a> {
    /// View `data` as a value of struct `name`. `None` if the struct is
    /// unknown or not yet [valid](StructDescriptor::is_valid), or `data`
    /// is shorter than it. Bytes past its size are ignored.
    pub fn new(db: &'a StructDatabase, name: &str, data: &'a [u8]) -> Option<Self> {
        Self::with_descriptor(db, db.find(name)?, data)
    }

    /// Like [`new`](Self::new), with a descriptor from `db`.
    pub fn with_descriptor(
        db: &'a StructDatabase,
        desc: &'a StructDescriptor,
        data: &'a [u8],
    ) -> Option<Self> {
        let size = desc.size()?;
        Some(Self {
            db,
            desc,
            data: data.get(..size)?,
        })
    }

    /// View the value of a `struct:<name>[]` topic: one struct after
    /// another. `None` if the struct is unknown or the length is not a
    /// multiple of its size.
    ///
    /// ```
    /// use nt4_proto::structs::{DynamicStruct, StructDatabase};
    /// let mut db = StructDatabase::new();
    /// db.add("Rotation2d", "double value").unwrap();
    /// let bytes: Vec<u8> = [0.1f64, 0.2].iter().flat_map(|v| v.to_le_bytes()).collect();
    /// assert_eq!(DynamicStruct::array(&db, "Rotation2d", &bytes).unwrap().len(), 2);
    /// ```
    pub fn array(db: &'a StructDatabase, name: &str, data: &'a [u8]) -> Option<Vec<Self>> {
        let desc = db.find(name)?;
        let size = desc.size()?;
        if size == 0 {
            return data.is_empty().then(Vec::new);
        }
        if data.len() % size != 0 {
            return None;
        }
        data.chunks_exact(size)
            .map(|chunk| Self::with_descriptor(db, desc, chunk))
            .collect()
    }

    /// The struct's descriptor.
    pub fn descriptor(&self) -> &'a StructDescriptor {
        self.desc
    }

    /// The struct's bytes (exactly its size).
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// A field by name.
    pub fn field(&self, name: &str) -> Option<&'a FieldDescriptor> {
        self.desc.field(name)
    }

    fn raw(&self, field: &FieldDescriptor, index: usize, ok: bool) -> Option<u64> {
        if !ok || field.parent != self.desc.id {
            return None;
        }
        read_raw(self.data, field, index)
    }

    /// A `bool` field. `None` if the field is another type, belongs to
    /// another struct, or `index` is past its end (likewise for the other
    /// getters).
    pub fn get_bool(&self, field: &FieldDescriptor, index: usize) -> Option<bool> {
        self.raw(field, index, field.ty == FieldType::Bool)
            .map(|v| v != 0)
    }

    /// A signed integer field, sign-extended from its declared width.
    pub fn get_int(&self, field: &FieldDescriptor, index: usize) -> Option<i64> {
        let mut raw = self.raw(field, index, field.is_int())?;
        if raw & (1u64 << (field.bit_width - 1)) != 0 {
            raw |= !field.bit_mask;
        }
        Some(raw as i64)
    }

    /// An unsigned integer field.
    pub fn get_uint(&self, field: &FieldDescriptor, index: usize) -> Option<u64> {
        self.raw(field, index, field.is_uint())
    }

    /// A `float` field.
    pub fn get_float(&self, field: &FieldDescriptor, index: usize) -> Option<f32> {
        self.raw(field, index, field.ty == FieldType::Float)
            .map(|v| f32::from_bits(v as u32))
    }

    /// A `double` field.
    pub fn get_double(&self, field: &FieldDescriptor, index: usize) -> Option<f64> {
        self.raw(field, index, field.ty == FieldType::Double)
            .map(f64::from_bits)
    }

    /// A `char` field as text: trailing zero bytes are dropped, and so is a
    /// UTF-8 sequence cut short at the end (as wpiutil does). Other bad
    /// UTF-8 becomes U+FFFD.
    pub fn get_string(&self, field: &FieldDescriptor) -> Option<Cow<'a, str>> {
        if field.ty != FieldType::Char || field.parent != self.desc.id {
            return None;
        }
        let bytes = &self.data[field.offset..field.offset + field.array_size];
        let mut len = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        if len > 0 && bytes[len - 1] & 0x80 != 0 {
            // Ends inside a multi-byte sequence? Find where it starts.
            match bytes[..len].iter().rposition(|&b| b & 0x40 != 0) {
                None => len = 0,
                Some(start) => {
                    let need = match bytes[start] {
                        b if b & 0xE0 == 0xC0 => Some(2),
                        b if b & 0xF0 == 0xE0 => Some(3),
                        b if b & 0xF8 == 0xF0 => Some(4),
                        _ => None,
                    };
                    if need.is_some_and(|n| start + n != len) {
                        len = start;
                    }
                }
            }
        }
        Some(String::from_utf8_lossy(&bytes[..len]))
    }

    /// A nested struct field.
    pub fn get_struct(&self, field: &FieldDescriptor, index: usize) -> Option<DynamicStruct<'a>> {
        if field.parent != self.desc.id || index >= field.array_size {
            return None;
        }
        let desc = self.db.nested(field)?;
        let size = desc.size()?;
        let start = field.offset + index * size;
        Some(DynamicStruct {
            db: self.db,
            desc,
            data: self.data.get(start..start + size)?,
        })
    }

    /// A field's value, whatever its type; arrays (other than `char`)
    /// become [`FieldValue::Array`].
    pub fn value(&self, field: &FieldDescriptor) -> Option<FieldValue<'a>> {
        if field.ty == FieldType::Char {
            return self.get_string(field).map(FieldValue::String);
        }
        let one = |i| -> Option<FieldValue<'a>> {
            Some(match field.ty {
                FieldType::Bool => FieldValue::Bool(self.get_bool(field, i)?),
                FieldType::Float => FieldValue::Float(self.get_float(field, i)?),
                FieldType::Double => FieldValue::Double(self.get_double(field, i)?),
                FieldType::Struct => FieldValue::Struct(self.get_struct(field, i)?),
                _ if field.is_int() => FieldValue::Int(self.get_int(field, i)?),
                _ => FieldValue::Uint(self.get_uint(field, i)?),
            })
        };
        if field.is_array() {
            (0..field.array_size)
                .map(one)
                .collect::<Option<_>>()
                .map(FieldValue::Array)
        } else {
            one(0)
        }
    }

    /// Every field's name and value, in schema order.
    pub fn values(&self) -> Vec<(&'a str, FieldValue<'a>)> {
        self.desc
            .fields
            .iter()
            .filter_map(|f| Some((f.name(), self.value(f)?)))
            .collect()
    }
}

/// A view of one struct value that can also change it.
pub struct DynamicStructMut<'a> {
    db: &'a StructDatabase,
    desc: &'a StructDescriptor,
    data: &'a mut [u8],
}

impl<'a> DynamicStructMut<'a> {
    /// View `data` as a value of struct `name`, for writing. `None` as for
    /// [`DynamicStruct::new`].
    ///
    /// ```
    /// use nt4_proto::structs::{DynamicStructMut, StructDatabase};
    /// let mut db = StructDatabase::new();
    /// db.add("Flags", "bool a:1; uint8 b:3").unwrap();
    /// let mut data = [0u8; 1];
    /// let mut flags = DynamicStructMut::new(&db, "Flags", &mut data).unwrap();
    /// let b = flags.field("b").unwrap();
    /// flags.set_uint(b, 5, 0).unwrap();
    /// assert_eq!(flags.as_ref().get_uint(b, 0), Some(5));
    /// assert_eq!(data, [0b1010]);
    /// ```
    pub fn new(db: &'a StructDatabase, name: &str, data: &'a mut [u8]) -> Option<Self> {
        Self::with_descriptor(db, db.find(name)?, data)
    }

    /// Like [`new`](Self::new), with a descriptor from `db`.
    pub fn with_descriptor(
        db: &'a StructDatabase,
        desc: &'a StructDescriptor,
        data: &'a mut [u8],
    ) -> Option<Self> {
        let size = desc.size()?;
        Some(Self {
            db,
            desc,
            data: data.get_mut(..size)?,
        })
    }

    /// Read the value as it is now.
    pub fn as_ref(&self) -> DynamicStruct<'_> {
        DynamicStruct {
            db: self.db,
            desc: self.desc,
            data: self.data,
        }
    }

    /// The struct's descriptor.
    pub fn descriptor(&self) -> &'a StructDescriptor {
        self.desc
    }

    /// The struct's bytes, to change directly.
    pub fn data(&mut self) -> &mut [u8] {
        self.data
    }

    /// A field by name.
    pub fn field(&self, name: &str) -> Option<&'a FieldDescriptor> {
        self.desc.field(name)
    }

    /// Copy in a whole value. `None` if `data` is shorter than the struct.
    pub fn set_data(&mut self, data: &[u8]) -> Option<()> {
        let len = self.data.len();
        self.data.copy_from_slice(data.get(..len)?);
        Some(())
    }

    fn write_raw(
        &mut self,
        field: &FieldDescriptor,
        value: u64,
        index: usize,
        ok: bool,
    ) -> Option<()> {
        if !ok || field.parent != self.desc.id || index >= field.array_size {
            return None;
        }
        let start = field.offset + index * field.size;
        let bytes = self.data.get_mut(start..start + field.size)?;
        let value = if field.is_bit_field() {
            let mut buf = [0u8; 8];
            buf[..bytes.len()].copy_from_slice(bytes);
            let mut current = u64::from_le_bytes(buf);
            current &= !(field.bit_mask << field.bit_shift);
            current |= (value & field.bit_mask) << field.bit_shift;
            current
        } else {
            value
        };
        let len = bytes.len();
        bytes.copy_from_slice(&value.to_le_bytes()[..len]);
        Some(())
    }

    /// Set a `bool` field. `None` (changing nothing) if the field is
    /// another type, belongs to another struct, or `index` is past its end
    /// (likewise for the other setters).
    pub fn set_bool(&mut self, field: &FieldDescriptor, value: bool, index: usize) -> Option<()> {
        self.write_raw(field, value as u64, index, field.ty == FieldType::Bool)
    }

    /// Set a signed integer field (cut to its width).
    pub fn set_int(&mut self, field: &FieldDescriptor, value: i64, index: usize) -> Option<()> {
        self.write_raw(field, value as u64, index, field.is_int())
    }

    /// Set an unsigned integer field (cut to its width).
    pub fn set_uint(&mut self, field: &FieldDescriptor, value: u64, index: usize) -> Option<()> {
        self.write_raw(field, value, index, field.is_uint())
    }

    /// Set a `float` field.
    pub fn set_float(&mut self, field: &FieldDescriptor, value: f32, index: usize) -> Option<()> {
        self.write_raw(
            field,
            value.to_bits() as u64,
            index,
            field.ty == FieldType::Float,
        )
    }

    /// Set a `double` field.
    pub fn set_double(&mut self, field: &FieldDescriptor, value: f64, index: usize) -> Option<()> {
        self.write_raw(field, value.to_bits(), index, field.ty == FieldType::Double)
    }

    /// Set a `char` field, padding with zero bytes. `Some(false)` if the
    /// text was too long and got cut (possibly mid-character, as in
    /// wpiutil; [`DynamicStruct::get_string`] drops such a tail).
    pub fn set_string(&mut self, field: &FieldDescriptor, value: &str) -> Option<bool> {
        if field.ty != FieldType::Char || field.parent != self.desc.id {
            return None;
        }
        let dest = &mut self.data[field.offset..field.offset + field.array_size];
        let len = value.len().min(dest.len());
        dest[..len].copy_from_slice(&value.as_bytes()[..len]);
        dest[len..].fill(0);
        Some(len == value.len())
    }

    /// Set a nested struct field from a value of that struct.
    pub fn set_struct(
        &mut self,
        field: &FieldDescriptor,
        value: &DynamicStruct<'_>,
        index: usize,
    ) -> Option<()> {
        if field.parent != self.desc.id
            || index >= field.array_size
            || field.nested != Some(value.desc.id)
        {
            return None;
        }
        let size = value.data.len();
        let start = field.offset + index * size;
        self.data
            .get_mut(start..start + size)?
            .copy_from_slice(value.data);
        Some(())
    }

    /// A writable view of a nested struct field.
    pub fn get_struct_mut(
        &mut self,
        field: &FieldDescriptor,
        index: usize,
    ) -> Option<DynamicStructMut<'_>> {
        if field.parent != self.desc.id || index >= field.array_size {
            return None;
        }
        let desc = self.db.nested(field)?;
        let size = desc.size()?;
        let start = field.offset + index * size;
        Some(DynamicStructMut {
            db: self.db,
            desc,
            data: self.data.get_mut(start..start + size)?,
        })
    }
}
