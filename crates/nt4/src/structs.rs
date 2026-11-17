//! Struct and protobuf topics, with their schemas.
//!
//! WPILib sends structured values (poses, module states, ...) as raw bytes
//! on a topic typed `struct:<Name>` (or `struct:<Name>[]` for an array), or
//! `proto:<message name>` for protobuf. Dashboards decode them with a
//! schema the publisher puts on `/.schema/<name>`.
//!
//! Implement [`NtStruct`] for a type and it is an [`NtValue`]:
//! `publish::<T>` and `subscribe::<T>` work for `T` and `Vec<T>`, and
//! publishing registers the schemas (`T`'s and those of the structs it
//! contains) the way ntcore does: each once per client or server, on a
//! retained `/.schema/struct:<Name>` topic of type `structschema`.
//! The [`nt4-wpimath`](https://docs.rs/nt4-wpimath) crate has WPILib's
//! geometry and kinematics types ready-made.
//!
//! For protobuf, implement [`NtProtobuf`] and use [`Proto<T>`] as the value
//! type. Its schemas are the serialized `FileDescriptorProto`s, on
//! `/.schema/proto:<file name>` with type `proto:FileDescriptorProto`.
//!
//! To read struct values whose type isn't known at compile time, see
//! [`nt4_proto::structs`].
//!
//! ```
//! use std::borrow::Cow;
//! use nt4::structs::{NtStruct, StructField, StructReader};
//!
//! /// A struct of our own: two `int32`s.
//! #[derive(Debug, Clone, PartialEq, Default)]
//! struct Counts {
//!     good: i32,
//!     bad: i32,
//! }
//!
//! impl NtStruct for Counts {
//!     const SIZE: usize = 8;
//!     fn type_name() -> Cow<'static, str> {
//!         "Counts".into()
//!     }
//!     fn schema() -> Cow<'static, str> {
//!         "int32 good;int32 bad".into()
//!     }
//!     fn pack(&self, out: &mut Vec<u8>) {
//!         self.good.pack(out);
//!         self.bad.pack(out);
//!     }
//!     fn unpack(data: &[u8]) -> Self {
//!         let mut r = StructReader::new(data);
//!         Counts { good: r.read(), bad: r.read() }
//!     }
//! }
//!
//! // Check the schema against the layout (worth a unit test).
//! nt4::structs::check::<Counts>().unwrap();
//!
//! use nt4::{NtType, NtValue, Value};
//! assert_eq!(Counts::nt_type(), NtType::Struct { name: "Counts".into(), array: false });
//! let bytes = Counts { good: 1, bad: -1 }.into_value();
//! assert_eq!(bytes, Value::Raw(vec![1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]));
//! ```

use std::borrow::Cow;

use nt4_proto::structs::StructDatabase;
use nt4_proto::{NtType, Value};

use crate::NtValue;

/// Schema type of struct schemas.
pub const STRUCT_SCHEMA_TYPE: &str = "structschema";
/// Schema type of protobuf schemas.
pub const PROTOBUF_SCHEMA_TYPE: &str = "proto:FileDescriptorProto";
/// Prefix of schema topics: a schema named `n` is published at
/// `/.schema/n`.
pub const SCHEMA_PREFIX: &str = "/.schema/";

/// A Rust type with a fixed-size WPILib struct layout.
///
/// The layout is packed, little-endian, in schema order, as wpiutil's
/// `Struct<T>`. See the [module docs](self) for an example.
///
/// The name and schema are functions rather than constants so that generic
/// types can build them, as wpimath's `Matrix__<rows>_<cols>` does. They are
/// only called when a topic is published or subscribed, not per value.
pub trait NtStruct: Sized + Clone + Send + Sync + 'static {
    /// Packed size in bytes.
    const SIZE: usize;

    /// The struct name, e.g. `Pose2d`. The topic type is `struct:<name>`.
    fn type_name() -> Cow<'static, str>;

    /// The schema, e.g. `Translation2d translation;Rotation2d rotation`.
    fn schema() -> Cow<'static, str>;

    /// Append exactly [`SIZE`](Self::SIZE) bytes.
    fn pack(&self, out: &mut Vec<u8>);

    /// Read a value from exactly [`SIZE`](Self::SIZE) bytes.
    fn unpack(data: &[u8]) -> Self;

    /// Call [`for_each_struct_schema`](Self::for_each_struct_schema) of each struct type
    /// this one contains. The default is for structs with none.
    fn for_each_nested(f: &mut dyn FnMut(&str, &str)) {
        let _ = f;
    }

    /// The type string and schema of each struct this type needs: those it
    /// contains first, then its own (`struct:<name>`). Don't override.
    fn for_each_struct_schema(f: &mut dyn FnMut(&str, &str)) {
        Self::for_each_nested(f);
        f(&format!("struct:{}", Self::type_name()), &Self::schema());
    }
}

/// A value that can be a field of an [`NtStruct`]: the schema's number
/// types (`bool`, `int8` to `uint64`, `float`, `double`), arrays of them
/// (`double x[3]` is `[f64; 3]`), and nested structs.
pub trait StructField: Sized {
    /// Packed size in bytes.
    const SIZE: usize;
    /// Append exactly [`SIZE`](Self::SIZE) bytes, little-endian.
    fn pack(&self, out: &mut Vec<u8>);
    /// Read from exactly [`SIZE`](Self::SIZE) bytes.
    fn unpack(data: &[u8]) -> Self;
}

macro_rules! number_field {
    ($($t:ty),*) => {$(
        impl StructField for $t {
            const SIZE: usize = std::mem::size_of::<$t>();
            fn pack(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
            fn unpack(data: &[u8]) -> Self {
                let mut buf = [0u8; std::mem::size_of::<$t>()];
                buf.copy_from_slice(&data[..Self::SIZE]);
                <$t>::from_le_bytes(buf)
            }
        }
    )*};
}

number_field!(u8, i8, u16, i16, u32, i32, u64, i64, f32, f64);

impl StructField for bool {
    const SIZE: usize = 1;
    fn pack(&self, out: &mut Vec<u8>) {
        out.push(*self as u8);
    }
    fn unpack(data: &[u8]) -> Self {
        data[0] != 0
    }
}

impl<T: StructField, const N: usize> StructField for [T; N] {
    const SIZE: usize = T::SIZE * N;
    fn pack(&self, out: &mut Vec<u8>) {
        for v in self {
            v.pack(out);
        }
    }
    fn unpack(data: &[u8]) -> Self {
        std::array::from_fn(|i| T::unpack(&data[i * T::SIZE..]))
    }
}

impl<T: NtStruct> StructField for T {
    const SIZE: usize = <T as NtStruct>::SIZE;
    fn pack(&self, out: &mut Vec<u8>) {
        NtStruct::pack(self, out);
    }
    fn unpack(data: &[u8]) -> Self {
        NtStruct::unpack(&data[..<T as NtStruct>::SIZE])
    }
}

/// Reads [`StructField`]s one after another, for [`NtStruct::unpack`].
#[derive(Debug, Clone)]
pub struct StructReader<'a> {
    data: &'a [u8],
}

impl<'a> StructReader<'a> {
    /// Start at the beginning of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Read the next field. Panics if `data` is too short, which
    /// [`NtStruct::unpack`]'s caller rules out when `SIZE` is right
    /// ([`check`] tests that).
    pub fn read<T: StructField>(&mut self) -> T {
        let (field, rest) = self.data.split_at(T::SIZE);
        self.data = rest;
        T::unpack(field)
    }
}

/// Check an [`NtStruct`] implementation: that its schemas parse, that the
/// layout they describe is [`SIZE`](NtStruct::SIZE) bytes, and that
/// [`pack`](NtStruct::pack) writes that many. Meant for unit tests.
///
/// ```
/// use std::borrow::Cow;
/// use nt4::structs::{NtStruct, StructReader};
///
/// #[derive(Clone, Default)]
/// struct Flag(bool);
///
/// impl NtStruct for Flag {
///     const SIZE: usize = 1;
///     fn type_name() -> Cow<'static, str> {
///         "Flag".into()
///     }
///     fn schema() -> Cow<'static, str> {
///         "bool value".into()
///     }
///     fn pack(&self, out: &mut Vec<u8>) {
///         out.push(self.0 as u8);
///     }
///     fn unpack(data: &[u8]) -> Self {
///         Flag(StructReader::new(data).read())
///     }
/// }
///
/// nt4::structs::check::<Flag>().unwrap();
/// ```
pub fn check<T: NtStruct + Default>() -> Result<(), String> {
    let mut db = StructDatabase::new();
    let mut err = None;
    T::for_each_struct_schema(&mut |ty, schema| {
        let name = ty.strip_prefix("struct:").unwrap_or(ty);
        if let Err(e) = db.add(name, schema) {
            err.get_or_insert(format!("{ty}: {e}"));
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    let name = T::type_name();
    let desc = db.find(&name).ok_or("own schema not registered")?;
    let size = desc
        .size()
        .ok_or_else(|| format!("{name}: a nested schema is missing"))?;
    if size != T::SIZE {
        return Err(format!("schema is {size} bytes, SIZE is {}", T::SIZE));
    }
    let mut out = Vec::new();
    NtStruct::pack(&T::default(), &mut out);
    if out.len() != T::SIZE {
        return Err(format!(
            "pack wrote {} bytes, SIZE is {}",
            out.len(),
            T::SIZE
        ));
    }
    Ok(())
}

/// `{"retained": true}`, as ntcore gives schema topics.
#[cfg(any(feature = "client", feature = "server"))]
pub(crate) fn schema_properties() -> nt4_proto::Properties {
    let mut props = nt4_proto::Properties::new();
    props.insert("retained".into(), true.into());
    props
}

fn struct_schemas<T: NtStruct>(f: &mut dyn FnMut(&str, &str, &[u8])) {
    T::for_each_struct_schema(&mut |name, schema| f(name, STRUCT_SCHEMA_TYPE, schema.as_bytes()));
}

impl<T: NtStruct> NtValue for T {
    fn nt_type() -> NtType {
        NtType::Struct {
            name: T::type_name().into_owned(),
            array: false,
        }
    }
    fn into_value(self) -> Value {
        let mut out = Vec::with_capacity(T::SIZE);
        NtStruct::pack(&self, &mut out);
        Value::Raw(out)
    }
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Raw(data) if data.len() == T::SIZE => Some(NtStruct::unpack(data)),
            _ => None,
        }
    }
    fn for_each_schema(f: &mut dyn FnMut(&str, &str, &[u8])) {
        struct_schemas::<T>(f);
    }
}

impl<T: NtStruct> NtValue for Vec<T> {
    fn nt_type() -> NtType {
        NtType::Struct {
            name: T::type_name().into_owned(),
            array: true,
        }
    }
    fn into_value(self) -> Value {
        let mut out = Vec::with_capacity(T::SIZE * self.len());
        for v in &self {
            NtStruct::pack(v, &mut out);
        }
        Value::Raw(out)
    }
    fn from_value(value: &Value) -> Option<Self> {
        let Value::Raw(data) = value else {
            return None;
        };
        if T::SIZE == 0 {
            return data.is_empty().then(Vec::new);
        }
        if !data.len().is_multiple_of(T::SIZE) {
            return None;
        }
        Some(data.chunks_exact(T::SIZE).map(NtStruct::unpack).collect())
    }
    fn for_each_schema(f: &mut dyn FnMut(&str, &str, &[u8])) {
        struct_schemas::<T>(f);
    }
}

/// A Rust type that is a protobuf message, for [`Proto<T>`] topics.
///
/// With [prost](https://docs.rs/prost), `encode` and `decode` are
/// `prost::Message`'s, and the descriptors come from the
/// `FileDescriptorSet` that `prost_build::Config::file_descriptor_set_path`
/// writes (one entry per `.proto` file, serialized again on its own).
///
/// ```
/// use nt4::structs::{NtProtobuf, Proto};
/// use nt4::{NtType, NtValue, Value};
///
/// /// A message with one `double` field (field 1), by hand.
/// #[derive(Debug, Clone, PartialEq)]
/// struct Speed(f64);
///
/// impl NtProtobuf for Speed {
///     const TYPE_NAME: &'static str = "demo.Speed";
///     fn for_each_descriptor(f: &mut dyn FnMut(&str, &[u8])) {
///         // The serialized FileDescriptorProto of demo.proto would go here.
///         f("demo.proto", b"...");
///     }
///     fn encode(&self, out: &mut Vec<u8>) {
///         out.push(0x09); // field 1, 64-bit
///         out.extend_from_slice(&self.0.to_le_bytes());
///     }
///     fn decode(data: &[u8]) -> Option<Self> {
///         let bytes = data.strip_prefix(&[0x09])?.try_into().ok()?;
///         Some(Speed(f64::from_le_bytes(bytes)))
///     }
/// }
///
/// assert_eq!(Proto::<Speed>::nt_type(), NtType::Proto { name: "demo.Speed".into() });
/// let v = Proto(Speed(1.5)).into_value();
/// assert_eq!(Proto::<Speed>::from_value(&v), Some(Proto(Speed(1.5))));
/// ```
pub trait NtProtobuf: Sized + Clone + Send + Sync + 'static {
    /// The full message name, e.g. `wpi.proto.ProtobufPose2d`. The topic
    /// type is `proto:<name>`.
    const TYPE_NAME: &'static str;

    /// Each `.proto` file the message needs, as (file name, serialized
    /// `FileDescriptorProto`), dependencies before the files that import
    /// them.
    fn for_each_descriptor(f: &mut dyn FnMut(&str, &[u8]));

    /// Append the encoded message.
    fn encode(&self, out: &mut Vec<u8>);

    /// Decode a message; `None` if the bytes don't parse.
    fn decode(data: &[u8]) -> Option<Self>;
}

/// The value type of a protobuf topic: `publish::<Proto<T>>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Proto<T>(pub T);

impl<T: NtProtobuf> NtValue for Proto<T> {
    fn nt_type() -> NtType {
        NtType::Proto {
            name: T::TYPE_NAME.into(),
        }
    }
    fn into_value(self) -> Value {
        let mut out = Vec::new();
        self.0.encode(&mut out);
        Value::Raw(out)
    }
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Raw(data) => T::decode(data).map(Proto),
            _ => None,
        }
    }
    fn for_each_schema(f: &mut dyn FnMut(&str, &str, &[u8])) {
        T::for_each_descriptor(&mut |file, descriptor| {
            f(&format!("proto:{file}"), PROTOBUF_SCHEMA_TYPE, descriptor)
        });
    }
}
