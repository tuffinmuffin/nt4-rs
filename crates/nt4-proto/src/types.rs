//! Topic type strings and their binary type ids.

use std::fmt;

/// The data type a topic announces.
///
/// NT4 sends the type as a string in `announce` and `publish` messages, and
/// as a small integer id in each binary value update. Any type string not in
/// the spec's table is sent as raw bytes (id 5).
///
/// ```
/// use nt4_proto::NtType;
/// let t: NtType = "struct:Pose2d[]".parse().unwrap();
/// assert_eq!(t, NtType::Struct { name: "Pose2d".into(), array: true });
/// assert_eq!(t.to_string(), "struct:Pose2d[]");
/// assert_eq!(t.type_id(), 5);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NtType {
    /// `boolean`, id 0.
    Boolean,
    /// `double` (64-bit float), id 1.
    Double,
    /// `int` (64-bit integer), id 2.
    Int,
    /// `float` (32-bit float), id 3.
    Float,
    /// `string`, id 4.
    String,
    /// `json` text, sent as a string (id 4).
    Json,
    /// `raw` bytes, id 5.
    Raw,
    /// `rpc`, kept for NT3 compatibility, sent as raw (id 5).
    Rpc,
    /// `msgpack` nested data, sent as raw (id 5).
    MsgPack,
    /// `protobuf` as named in the spec table, sent as raw (id 5).
    Protobuf,
    /// `boolean[]`, id 16.
    BooleanArray,
    /// `double[]`, id 17.
    DoubleArray,
    /// `int[]`, id 18.
    IntArray,
    /// `float[]`, id 19.
    FloatArray,
    /// `string[]`, id 20.
    StringArray,
    /// A WPILib struct (`struct:<name>` or `struct:<name>[]`), sent as raw.
    /// Its schema is published at `/.schema/struct:<name>`.
    Struct {
        /// Struct type name, e.g. `Pose2d`.
        name: String,
        /// True for `struct:<name>[]`.
        array: bool,
    },
    /// A WPILib protobuf message (`proto:<name>`), sent as raw.
    Proto {
        /// Protobuf message name.
        name: String,
    },
    /// The schema text of a struct (`structschema`), sent as raw.
    StructSchema,
    /// Any other type string, sent as raw.
    Other(String),
}

impl NtType {
    /// Binary type id used in value updates.
    pub fn type_id(&self) -> u8 {
        match self {
            NtType::Boolean => 0,
            NtType::Double => 1,
            NtType::Int => 2,
            NtType::Float => 3,
            NtType::String | NtType::Json => 4,
            NtType::BooleanArray => 16,
            NtType::DoubleArray => 17,
            NtType::IntArray => 18,
            NtType::FloatArray => 19,
            NtType::StringArray => 20,
            _ => 5,
        }
    }
}

impl std::str::FromStr for NtType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "boolean" => NtType::Boolean,
            "double" => NtType::Double,
            "int" => NtType::Int,
            "float" => NtType::Float,
            "string" => NtType::String,
            "json" => NtType::Json,
            "raw" => NtType::Raw,
            "rpc" => NtType::Rpc,
            "msgpack" => NtType::MsgPack,
            "protobuf" => NtType::Protobuf,
            "boolean[]" => NtType::BooleanArray,
            "double[]" => NtType::DoubleArray,
            "int[]" => NtType::IntArray,
            "float[]" => NtType::FloatArray,
            "string[]" => NtType::StringArray,
            "structschema" => NtType::StructSchema,
            other => {
                if let Some(rest) = other.strip_prefix("struct:") {
                    match rest.strip_suffix("[]") {
                        Some(name) => NtType::Struct {
                            name: name.into(),
                            array: true,
                        },
                        None => NtType::Struct {
                            name: rest.into(),
                            array: false,
                        },
                    }
                } else if let Some(name) = other.strip_prefix("proto:") {
                    NtType::Proto { name: name.into() }
                } else {
                    NtType::Other(other.into())
                }
            }
        })
    }
}

impl From<&str> for NtType {
    fn from(s: &str) -> Self {
        s.parse().unwrap_or_else(|never| match never {})
    }
}

impl fmt::Display for NtType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            NtType::Boolean => "boolean",
            NtType::Double => "double",
            NtType::Int => "int",
            NtType::Float => "float",
            NtType::String => "string",
            NtType::Json => "json",
            NtType::Raw => "raw",
            NtType::Rpc => "rpc",
            NtType::MsgPack => "msgpack",
            NtType::Protobuf => "protobuf",
            NtType::BooleanArray => "boolean[]",
            NtType::DoubleArray => "double[]",
            NtType::IntArray => "int[]",
            NtType::FloatArray => "float[]",
            NtType::StringArray => "string[]",
            NtType::StructSchema => "structschema",
            NtType::Struct { name, array } => {
                return write!(f, "struct:{name}{}", if *array { "[]" } else { "" });
            }
            NtType::Proto { name } => return write!(f, "proto:{name}"),
            NtType::Other(s) => s,
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    // Tests written by Claude.
    use super::*;

    #[test]
    fn every_named_type_round_trips() {
        for s in [
            "boolean",
            "double",
            "int",
            "float",
            "string",
            "json",
            "raw",
            "rpc",
            "msgpack",
            "protobuf",
            "boolean[]",
            "double[]",
            "int[]",
            "float[]",
            "string[]",
            "structschema",
            "struct:Pose2d",
            "struct:SwerveModuleState[]",
            "proto:Pose3d",
            "weird",
        ] {
            let t: NtType = s.into();
            assert_eq!(t.to_string(), s);
        }
    }

    #[test]
    fn type_ids_match_the_spec_table() {
        let cases = [
            ("boolean", 0),
            ("double", 1),
            ("int", 2),
            ("float", 3),
            ("string", 4),
            ("json", 4),
            ("raw", 5),
            ("rpc", 5),
            ("msgpack", 5),
            ("protobuf", 5),
            ("struct:Pose2d", 5),
            ("anything else", 5),
            ("boolean[]", 16),
            ("double[]", 17),
            ("int[]", 18),
            ("float[]", 19),
            ("string[]", 20),
        ];
        for (s, id) in cases {
            assert_eq!(NtType::from(s).type_id(), id, "{s}");
        }
    }
}
