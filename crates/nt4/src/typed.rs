//! Rust types that map to NT4 topic types.
//!
//! [`NtValue`] connects a Rust type to an NT4 type string and [`Value`]
//! variant, so [`Client::publish`](crate::Client::publish) and
//! [`Client::subscribe`](crate::Client::subscribe) can be typed:
//!
//! | Rust | NT4 type |
//! | --- | --- |
//! | `bool` | `boolean` |
//! | `f64` | `double` |
//! | `i64` | `int` |
//! | `f32` | `float` |
//! | `String` | `string` |
//! | `Vec<u8>` | `raw` |
//! | `Vec<bool>`, `Vec<f64>`, `Vec<i64>`, `Vec<f32>`, `Vec<String>` | the `[]` array types |
//! | `T` and `Vec<T>` for `T:` [`NtStruct`](crate::structs::NtStruct) | `struct:<name>` and `struct:<name>[]` |
//! | [`Proto<T>`](crate::structs::Proto) | `proto:<message name>` |
//!
//! ```
//! use nt4::NtValue;
//! use nt4_proto::{NtType, Value};
//! assert_eq!(f64::nt_type(), NtType::Double);
//! assert_eq!(2.5f64.into_value(), Value::Double(2.5));
//! assert_eq!(f64::from_value(&Value::Double(2.5)), Some(2.5));
//! assert_eq!(f64::from_value(&Value::Int(2)), None);
//! ```

use nt4_proto::{NtType, Value};

/// A Rust type that can be a topic's value.
pub trait NtValue: Sized + Clone + Send + Sync + 'static {
    /// The NT4 type to announce for this Rust type.
    fn nt_type() -> NtType;
    /// Convert to a wire value.
    fn into_value(self) -> Value;
    /// Convert from a wire value; `None` if the value has another type.
    fn from_value(value: &Value) -> Option<Self>;

    /// The schemas a topic of this type needs, as (name, schema type,
    /// data); publishing puts each on `/.schema/<name>` once. None for the
    /// plain types; structs and protobuf messages have theirs.
    fn for_each_schema(f: &mut dyn FnMut(&str, &str, &[u8])) {
        let _ = f;
    }
}

macro_rules! nt_value {
    ($rust:ty, $nt:ident, $variant:ident) => {
        impl NtValue for $rust {
            fn nt_type() -> NtType {
                NtType::$nt
            }
            fn into_value(self) -> Value {
                Value::$variant(self)
            }
            fn from_value(value: &Value) -> Option<Self> {
                match value {
                    Value::$variant(v) => Some(v.clone()),
                    _ => None,
                }
            }
        }
    };
}

nt_value!(bool, Boolean, Boolean);
nt_value!(f64, Double, Double);
nt_value!(i64, Int, Int);
nt_value!(f32, Float, Float);
nt_value!(String, String, String);
nt_value!(Vec<u8>, Raw, Raw);
nt_value!(Vec<bool>, BooleanArray, BooleanArray);
nt_value!(Vec<f64>, DoubleArray, DoubleArray);
nt_value!(Vec<i64>, IntArray, IntArray);
nt_value!(Vec<f32>, FloatArray, FloatArray);
nt_value!(Vec<String>, StringArray, StringArray);
