//! `#[derive(NtStruct)]` for the [`nt4`](https://docs.rs/nt4) crate.
//!
//! Unofficial: not affiliated with or endorsed by WPILib or FIRST.
//!
//! Use it through `nt4` with its `derive` feature, as
//! `nt4::structs::NtStruct`; this crate is not meant to be used on its own.
//! See [`NtStruct`](macro@NtStruct) for what it generates.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DataEnum, DeriveInput, Error, Expr, ExprLit, ExprUnary, Fields, Ident, Lit,
    LitStr, Path, Result, Type, UnOp, parse_macro_input,
};

/// Implements `nt4::structs::NtStruct` for a struct with named fields, or
/// for an enum without fields.
///
/// For a struct, the fields are packed in order, and the schema and the
/// list of nested schemas come from the field types, so they can't drift
/// from the layout. A field is any `nt4::structs::StructField`: `bool`,
/// the integer types, `f32`, `f64`, an array of one of those, or another
/// `NtStruct` (and arrays of those). Each field is declared as wpimath's
/// C++ schemas declare it: `double x`, `int16 raw[4]`, `Rotation2d angle`.
///
/// ```
/// use nt4::structs::NtStruct;
///
/// #[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
/// struct Rotation2d {
///     value: f64,
/// }
///
/// #[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
/// struct ModuleState {
///     velocity: f64,
///     angle: Rotation2d,
///     currents: [f32; 2],
/// }
///
/// assert_eq!(ModuleState::type_name(), "ModuleState");
/// assert_eq!(
///     ModuleState::schema(),
///     "double velocity;Rotation2d angle;float currents[2]"
/// );
/// assert_eq!(<ModuleState as NtStruct>::SIZE, 24);
/// nt4::structs::check::<ModuleState>().unwrap();
/// ```
///
/// An enum without fields becomes a one-byte struct, the way WPILib's
/// Java `StructGenerator.genEnum` writes one: the schema is
/// `enum {<variant>=<value>,...} int8 variant`, with the Rust
/// discriminants as the values (they must fit in an `i8`). A byte that
/// matches no variant unpacks as the enum's `Default`, so the enum must
/// implement `Default` (`#[derive(Default)]` with `#[default]` on a
/// variant).
///
/// ```
/// use nt4::structs::NtStruct;
///
/// #[derive(NtStruct, Debug, Clone, Copy, PartialEq, Default)]
/// enum Mode {
///     #[default]
///     Disabled,
///     Teleop,
///     Auto = 5,
/// }
///
/// assert_eq!(Mode::schema(), "enum {Disabled=0,Teleop=1,Auto=5} int8 variant");
/// nt4::structs::check::<Mode>().unwrap();
/// ```
///
/// Attributes on the type, all optional:
///
/// - `#[nt4(name = "..")]`: the struct name, if it isn't the Rust name
///   (the topic type is `struct:<name>`).
/// - `#[nt4(schema = "..")]`: the schema, written out, instead of the one
///   built from the fields. The layout and nested schemas still come from
///   the fields; `nt4::structs::check` tests that they agree.
/// - `#[nt4(crate = "..")]`: the path to the `nt4` crate, if it isn't
///   `::nt4` (say, when it is renamed in `Cargo.toml`).
///
/// ```
/// use nt4::structs::NtStruct;
///
/// #[derive(NtStruct, Clone, Default)]
/// #[nt4(name = "TrapezoidProfileConstraints", schema = "double maxVelocity;double maxAcceleration")]
/// struct Constraints {
///     max_velocity: f64,
///     max_acceleration: f64,
/// }
///
/// assert_eq!(Constraints::type_name(), "TrapezoidProfileConstraints");
/// nt4::structs::check::<Constraints>().unwrap();
/// ```
///
/// Not supported: tuple structs, unions, generic types (implement
/// `NtStruct` by hand; its `type_name()` can build a name such as
/// `Matrix__2_3`), enums with fields, and bit fields.
#[proc_macro_derive(NtStruct, attributes(nt4))]
pub fn derive_nt_struct(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// The `#[nt4(..)]` options on the type.
#[derive(Default)]
struct Options {
    name: Option<LitStr>,
    schema: Option<LitStr>,
    krate: Option<Path>,
}

fn options(attrs: &[Attribute]) -> Result<Options> {
    let mut out = Options::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("nt4")) {
        attr.parse_nested_meta(|meta| {
            let slot = if meta.path.is_ident("name") {
                &mut out.name
            } else if meta.path.is_ident("schema") {
                &mut out.schema
            } else if meta.path.is_ident("crate") {
                let lit: LitStr = meta.value()?.parse()?;
                if out.krate.is_some() {
                    return Err(meta.error("`crate` is given twice"));
                }
                out.krate = Some(lit.parse()?);
                return Ok(());
            } else {
                return Err(meta.error("unknown nt4 option; expected `name`, `schema` or `crate`"));
            };
            if slot.is_some() {
                return Err(meta.error("this option is given twice"));
            }
            *slot = Some(meta.value()?.parse()?);
            Ok(())
        })?;
    }
    Ok(out)
}

/// `#[nt4(..)]` is only for the type, for now.
fn no_options(attrs: &[Attribute], on: &str) -> Result<()> {
    match attrs.iter().find(|a| a.path().is_ident("nt4")) {
        Some(attr) => Err(Error::new_spanned(
            attr,
            format!("#[nt4(..)] goes on the type, not on a {on}"),
        )),
        None => Ok(()),
    }
}

/// An identifier as it appears in a schema: without `r#`.
fn schema_ident(ident: &Ident) -> String {
    let s = ident.to_string();
    s.strip_prefix("r#").map(str::to_owned).unwrap_or(s)
}

fn expand(input: &DeriveInput) -> Result<TokenStream2> {
    let opts = options(&input.attrs)?;
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "NtStruct can't be derived for a generic type; implement it by hand \
             (type_name() can build a name such as `Matrix__2_3`)",
        ));
    }
    let krate = opts
        .krate
        .clone()
        .map(|p| quote!(#p))
        .unwrap_or_else(|| quote!(::nt4));
    let name = opts
        .name
        .clone()
        .unwrap_or_else(|| LitStr::new(&schema_ident(&input.ident), input.ident.span()));
    match &input.data {
        Data::Struct(data) => expand_struct(input, &opts, &krate, &name, &data.fields),
        Data::Enum(data) => expand_enum(input, &opts, &krate, &name, data),
        Data::Union(data) => Err(Error::new(
            data.union_token.span(),
            "NtStruct can't be derived for a union",
        )),
    }
}

fn expand_struct(
    input: &DeriveInput,
    opts: &Options,
    krate: &TokenStream2,
    name: &LitStr,
    fields: &Fields,
) -> Result<TokenStream2> {
    let named: Vec<(&Ident, &Type)> = match fields {
        Fields::Named(f) => f
            .named
            .iter()
            .map(|f| {
                no_options(&f.attrs, "field")?;
                Ok((f.ident.as_ref().expect("named field"), &f.ty))
            })
            .collect::<Result<_>>()?,
        Fields::Unit => Vec::new(),
        Fields::Unnamed(f) => {
            return Err(Error::new_spanned(
                f,
                "NtStruct needs named fields (schema fields have names); \
                 use `struct Name { field: Type }`",
            ));
        }
    };
    let ident = &input.ident;
    let s = quote!(#krate::structs);
    let p = quote!(#krate::structs::__private);
    let idents: Vec<_> = named.iter().map(|(i, _)| *i).collect();
    let types: Vec<_> = named.iter().map(|(_, t)| *t).collect();
    let field_names: Vec<_> = idents.iter().map(|i| schema_ident(i)).collect();
    let type_strings: Vec<_> = types.iter().map(|t| quote!(#t).to_string()).collect();

    let schema = match &opts.schema {
        Some(lit) => quote!(#p::Cow::Borrowed(#lit)),
        None => quote! {
            let mut schema = #p::String::new();
            #(#p::field::<#types>(&mut schema, #field_names, #type_strings);)*
            #p::Cow::Owned(schema)
        },
    };
    let unpack = match fields {
        Fields::Unit => quote! {
            let _ = data;
            Self
        },
        _ if named.is_empty() => quote! {
            let _ = data;
            Self {}
        },
        _ => quote! {
            let mut reader = #s::StructReader::new(data);
            Self { #(#idents: reader.read::<#types>(),)* }
        },
    };

    Ok(quote! {
        #[automatically_derived]
        impl #s::NtStruct for #ident {
            const SIZE: usize = 0 #(+ <#types as #s::StructField>::SIZE)*;

            fn type_name() -> #p::Cow<'static, str> {
                #p::Cow::Borrowed(#name)
            }

            fn schema() -> #p::Cow<'static, str> {
                #schema
            }

            fn pack(&self, out: &mut #p::Vec<u8>) {
                let _ = &out;
                #(#s::StructField::pack(&self.#idents, out);)*
            }

            fn unpack(data: &[u8]) -> Self {
                #unpack
            }

            fn for_each_nested(f: &mut dyn FnMut(&str, &str)) {
                #p::nested(f, &[#(<#types as #s::StructField>::for_each_field_schema,)*]);
            }
        }
    })
}

/// A discriminant written as an integer literal, possibly negated.
fn discriminant(expr: &Expr) -> Result<i64> {
    let (negative, lit) = match expr {
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr,
            ..
        }) => (true, &**expr),
        e => (false, e),
    };
    match lit {
        Expr::Lit(ExprLit {
            lit: Lit::Int(int), ..
        }) => {
            let v: i64 = int.base10_parse()?;
            Ok(if negative { -v } else { v })
        }
        _ => Err(Error::new_spanned(
            expr,
            "NtStruct needs each discriminant written as an integer literal",
        )),
    }
}

fn expand_enum(
    input: &DeriveInput,
    opts: &Options,
    krate: &TokenStream2,
    name: &LitStr,
    data: &DataEnum,
) -> Result<TokenStream2> {
    if data.variants.is_empty() {
        return Err(Error::new_spanned(
            &input.ident,
            "NtStruct can't be derived for an enum with no variants",
        ));
    }
    let mut next: i64 = 0;
    let mut variants = Vec::new();
    for v in &data.variants {
        no_options(&v.attrs, "variant")?;
        if !matches!(v.fields, Fields::Unit) {
            return Err(Error::new_spanned(
                &v.fields,
                "NtStruct can only be derived for an enum whose variants have no fields",
            ));
        }
        let value = match &v.discriminant {
            Some((_, expr)) => discriminant(expr)?,
            None => next,
        };
        let Ok(byte) = i8::try_from(value) else {
            return Err(Error::new_spanned(
                v,
                format!(
                    "the value of `{}` is {value}; an NtStruct enum is an int8 (-128 to 127)",
                    v.ident
                ),
            ));
        };
        next = value + 1;
        variants.push((&v.ident, byte));
    }

    let ident = &input.ident;
    let s = quote!(#krate::structs);
    let p = quote!(#krate::structs::__private);
    let schema = match &opts.schema {
        Some(lit) => lit.clone(),
        None => {
            let values: Vec<String> = variants
                .iter()
                .map(|(i, v)| format!("{}={v}", schema_ident(i)))
                .collect();
            LitStr::new(
                &format!("enum {{{}}} int8 variant", values.join(",")),
                input.ident.span(),
            )
        }
    };
    let idents: Vec<_> = variants.iter().map(|(i, _)| *i).collect();
    let bytes: Vec<_> = variants.iter().map(|(_, b)| *b).collect();

    Ok(quote! {
        #[automatically_derived]
        impl #s::NtStruct for #ident {
            const SIZE: usize = 1;

            fn type_name() -> #p::Cow<'static, str> {
                #p::Cow::Borrowed(#name)
            }

            fn schema() -> #p::Cow<'static, str> {
                #p::Cow::Borrowed(#schema)
            }

            fn pack(&self, out: &mut #p::Vec<u8>) {
                let value: i8 = match self {
                    #(Self::#idents => #bytes,)*
                };
                #s::StructField::pack(&value, out);
            }

            fn unpack(data: &[u8]) -> Self {
                match <i8 as #s::StructField>::unpack(data) {
                    #(#bytes => Self::#idents,)*
                    _ => <Self as #p::Default>::default(),
                }
            }
        }
    })
}

/// A field type that isn't a `StructField` is refused by the compiler,
/// with nt4's message:
///
/// ```compile_fail
/// #[derive(nt4::structs::NtStruct, Clone, Default)]
/// struct Named {
///     name: String, // `String` can't be a field of an NtStruct
/// }
/// ```
///
/// And without `Default`, an enum can't be derived:
///
/// ```compile_fail
/// #[derive(nt4::structs::NtStruct, Clone)]
/// enum Mode {
///     Off,
///     On,
/// }
/// ```
#[cfg(doctest)]
struct CompileFailures;

#[cfg(test)]
mod tests {
    // Written by Claude: each input the derive refuses, and its message.
    use super::*;

    fn error(src: &str) -> String {
        let input: DeriveInput = syn::parse_str(src).unwrap();
        match expand(&input) {
            Ok(_) => panic!("expected an error for {src}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn refuses_tuple_structs() {
        assert!(error("struct A(f64, f64);").contains("needs named fields"));
    }

    #[test]
    fn refuses_generics() {
        let e = error("struct M<const R: usize> { v: [f64; R] }");
        assert!(e.contains("generic type; implement it by hand"), "{e}");
        assert!(error("struct L<'a> { v: &'a f64 }").contains("generic"));
    }

    #[test]
    fn refuses_unions() {
        assert!(error("union U { a: u8, b: i8 }").contains("union"));
    }

    #[test]
    fn refuses_enums_with_fields_or_no_variants() {
        assert!(error("enum E { A(u8), B }").contains("variants have no fields"));
        assert!(error("enum E { A { x: u8 } }").contains("variants have no fields"));
        assert!(error("enum E {}").contains("no variants"));
    }

    #[test]
    fn refuses_discriminants_outside_int8() {
        let e = error("enum E { A = 127, B }");
        assert!(e.contains("value of `B` is 128"), "{e}");
        assert!(error("enum E { A = -129 }").contains("-128 to 127"));
        assert!(error("enum E { A = 1 << 2 }").contains("integer literal"));
    }

    #[test]
    fn accepts_the_int8_range() {
        let input: DeriveInput = syn::parse_str("enum E { A = -128, B = 127 }").unwrap();
        let out = expand(&input).unwrap().to_string();
        assert!(out.contains("enum {A=-128,B=127} int8 variant"), "{out}");
    }

    fn expanded(src: &str) -> String {
        expand(&syn::parse_str(src).unwrap()).unwrap().to_string()
    }

    #[test]
    fn expands_structs() {
        let out = expanded("struct P { r#type: f64, n: [u8; 2] }");
        assert!(out.contains("\"type\""), "{out}");
        assert!(out.contains("Cow :: Borrowed (\"P\")"), "{out}");
        let out = expanded(
            "#[nt4(name = \"Q\", schema = \"double v\", crate = \"my::nt\")] struct P { v: f64 }",
        );
        assert!(
            out.contains("Borrowed (\"Q\")") && out.contains("\"double v\""),
            "{out}"
        );
        assert!(out.contains("my :: nt :: structs :: NtStruct"), "{out}");
        assert!(
            expanded("struct U;").contains("Self\n") || expanded("struct U;").contains("Self }")
        );
        assert!(expanded("struct E {}").contains("Self { }"));
        assert!(
            expanded("#[nt4(schema = \"x\")] #[derive(Default)] enum E { #[default] A }")
                .contains("\"x\"")
        );
    }

    #[test]
    fn refuses_bad_options() {
        assert!(error("#[nt4(size = 3)] struct A { x: u8 }").contains("unknown nt4 option"));
        assert!(error("#[nt4(name = \"A\", name = \"B\")] struct A { x: u8 }").contains("twice"));
        assert!(error("#[nt4(name = 3)] struct A { x: u8 }").contains("expected string literal"));
        assert!(error("#[nt4(crate = \"a\", crate = \"b\")] struct A { x: u8 }").contains("twice"));
        assert!(error("struct A { #[nt4(name = \"y\")] x: u8 }").contains("not on a field"));
        assert!(
            error("#[derive(Default)] enum E { #[nt4(name = \"y\")] A }")
                .contains("not on a variant")
        );
    }
}
