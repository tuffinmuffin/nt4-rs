//! The persistent-values file (`networktables.json`), as ntcore's
//! `ServerStorage::DumpPersistent` and `LoadPersistent` write and read it.
//!
//! Output matches ntcore byte for byte: its JSON writer (wpiutil's `json`,
//! with double-conversion for numbers) has its own number format and
//! escapes more characters than most, so this module writes JSON itself
//! instead of through serde_json.

use nt4_proto::{NtType, Properties, Value};
use serde_json::Value as Json;

// ----- Writing -----

/// A double as double-conversion's `ToShortest` writes it with wpiutil's
/// settings: the shortest digits that read back the same, no trailing
/// `.0`, exponent notation below 1e-6 or from 1e21 up (`1e+21`, `1e-7`),
/// `-0` as `0`, infinity as `1e5000`, NaN as `null`.
pub(crate) fn fmt_double(v: f64) -> String {
    if v.is_nan() {
        return "null".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "1e5000" } else { "-1e5000" }.into();
    }
    shortest(&format!("{v:e}"))
}

/// A float as double-conversion's `ToShortestSingle` writes it: the same
/// format as [`fmt_double`] with the shortest digits for an `f32`.
pub(crate) fn fmt_float(v: f32) -> String {
    if v.is_nan() {
        return "null".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "1e5000" } else { "-1e5000" }.into();
    }
    shortest(&format!("{v:e}"))
}

/// Lay out Rust's shortest `{:e}` digits (`-1.25e3`) the way
/// double-conversion does.
fn shortest(sci: &str) -> String {
    let (mantissa, exp) = sci.split_once('e').expect("`{:e}` has an exponent");
    let exp: i32 = exp.parse().expect("`{:e}` exponent is a number");
    let (neg, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    if digits.bytes().all(|d| d == b'0') {
        return "0".into(); // UNIQUE_ZERO: no "-0"
    }
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    let len = digits.len() as i32;
    // Position of the decimal point within the digits.
    let point = exp + 1;
    if (-6..21).contains(&exp) {
        if point <= 0 {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-point) as usize));
            out.push_str(&digits);
        } else if point >= len {
            out.push_str(&digits);
            out.extend(std::iter::repeat_n('0', (point - len) as usize));
        } else {
            out.push_str(&digits[..point as usize]);
            out.push('.');
            out.push_str(&digits[point as usize..]);
        }
    } else {
        out.push_str(&digits[..1]);
        if len > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&exp.unsigned_abs().to_string());
    }
    out
}

/// A JSON string, escaped as wpiutil's `json::serialize` does: besides `"`
/// and `\`, it escapes `/`, `&`, `'`, `<`, `=`, `>`, DEL, control
/// characters and everything outside ASCII (as UTF-16 `\u` escapes).
pub(crate) fn push_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{0c}' => out.push_str("\\f"),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '"' => out.push_str("\\\""),
            ' '..='~' if !matches!(c, '&' | '\'' | '<' | '=' | '>') => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// A JSON value as wpiutil's `json::marshal(os, pretty = true, indent)`
/// writes it. Its pretty mode puts an object with more than one key on
/// several lines, indented from `indent` rather than from where the object
/// starts; ntcore's file has that layout, so this keeps it.
pub(crate) fn push_json(out: &mut String, v: &Json, indent: usize) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                out.push_str(&fmt_double(n.as_f64().unwrap_or(f64::NAN)));
            }
        }
        Json::String(s) => push_string(out, s),
        Json::Array(a) => {
            out.push('[');
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                push_json(out, e, indent);
            }
            out.push(']');
        }
        Json::Object(o) => push_object(out, o, indent),
    }
}

fn push_object(out: &mut String, o: &Properties, indent: usize) {
    let multi = o.len() > 1;
    out.push('{');
    for (i, (k, v)) in o.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        if multi {
            out.push('\n');
            out.push_str(&"  ".repeat(indent + 1));
        }
        push_string(out, k);
        out.push_str(": ");
        push_json(out, v, indent + usize::from(multi));
    }
    if multi {
        out.push('\n');
        out.push_str(&"  ".repeat(indent));
    }
    out.push('}');
}

/// A value as ntcore's `DumpValue` writes it.
pub(crate) fn push_value(out: &mut String, v: &Value) {
    fn list<T>(out: &mut String, items: &[T], f: impl Fn(&mut String, &T)) {
        out.push('[');
        for (i, x) in items.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            f(out, x);
        }
        out.push(']');
    }
    match v {
        Value::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Double(d) => out.push_str(&fmt_double(*d)),
        Value::Float(f) => out.push_str(&fmt_float(*f)),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::String(s) => push_string(out, s),
        Value::Raw(r) => {
            out.push('"');
            out.push_str(&base64_encode(r));
            out.push('"');
        }
        Value::BooleanArray(a) => {
            list(out, a, |o, b| o.push_str(if *b { "true" } else { "false" }))
        }
        Value::DoubleArray(a) => list(out, a, |o, d| o.push_str(&fmt_double(*d))),
        Value::FloatArray(a) => list(out, a, |o, f| o.push_str(&fmt_float(*f))),
        Value::IntArray(a) => list(out, a, |o, i| o.push_str(&i.to_string())),
        Value::StringArray(a) => list(out, a, |o, s| push_string(o, s)),
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with `=` padding, as wpiutil's `Base64Encode`.
pub(crate) fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Base64 as wpiutil's `Base64Decode` reads it: decode up to the first
/// character that isn't base64 (such as the `=` padding) and ignore the
/// rest.
pub(crate) fn base64_decode(s: &str) -> Vec<u8> {
    let sextet = |c: u8| BASE64.iter().position(|&b| b == c).map(|p| p as u32);
    let vals: Vec<u32> = s.bytes().map_while(sextet).collect();
    let mut out = Vec::with_capacity(vals.len() * 3 / 4);
    for chunk in vals.chunks(4) {
        let mut n = 0;
        for (i, v) in chunk.iter().enumerate() {
            n |= v << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        // 2 sextets give 1 byte, 3 give 2, 4 give 3.
        out.extend_from_slice(&bytes[..chunk.len().saturating_sub(1)]);
    }
    out
}

// ----- Reading -----

/// One entry of the file, checked as ntcore's `LoadPersistent` checks it.
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) ty: NtType,
    pub(crate) properties: Properties,
    pub(crate) value: Value,
    /// Array elements of the wrong type were skipped, as in ntcore (which
    /// sets this error but loads the entry anyway and never reports it).
    pub(crate) skipped: Option<String>,
}

fn get_string<'a>(o: &'a Properties, key: &str) -> Result<&'a str, String> {
    match o.get(key) {
        None => Err(format!("no {key} key")),
        Some(Json::String(s)) => Ok(s),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

/// Check one entry; the error is in ntcore's words.
pub(crate) fn parse_entry(item: &Json) -> Result<Entry, String> {
    let Json::Object(o) = item else {
        return Err("expected item to be an object".into());
    };
    let name = get_string(o, "name")?;
    let ty = get_string(o, "type")?;
    let properties = match o.get("properties") {
        None => return Err("no properties key".into()),
        Some(Json::Object(p)) => p,
        Some(_) => return Err("properties must be an object".into()),
    };
    match properties.get("persistent") {
        None => return Err("no persistent property".into()),
        Some(Json::Bool(true)) => {}
        Some(Json::Bool(false)) => return Err("persistent property is false".into()),
        Some(_) => return Err("persistent property is not boolean".into()),
    }
    let Some(v) = o.get("value") else {
        return Err("no value key".into());
    };

    let mismatch = |expected: &str| format!("value type mismatch, expected {expected}");
    let mut skipped = None;
    // Elements of the right kind, noting when some were not.
    let mut elems = |expected: &str, keep: fn(&Json) -> bool| -> Result<Vec<Json>, String> {
        let Json::Array(a) = v else {
            return Err(mismatch("array"));
        };
        let kept: Vec<Json> = a.iter().filter(|e| keep(e)).cloned().collect();
        if kept.len() != a.len() {
            skipped = Some(mismatch(expected));
        }
        Ok(kept)
    };
    let value = match ty {
        "boolean" => Value::Boolean(v.as_bool().ok_or_else(|| mismatch("boolean"))?),
        "int" => Value::Int(v.as_i64().ok_or_else(|| mismatch("int"))?),
        "float" => Value::Float(v.as_f64().ok_or_else(|| mismatch("float"))? as f32),
        "double" => Value::Double(v.as_f64().ok_or_else(|| mismatch("double"))?),
        "string" | "json" => {
            Value::String(v.as_str().ok_or_else(|| mismatch("string"))?.to_owned())
        }
        "boolean[]" => Value::BooleanArray(
            elems("boolean", Json::is_boolean)?
                .iter()
                .filter_map(Json::as_bool)
                .collect(),
        ),
        "int[]" => Value::IntArray(
            elems("int", Json::is_i64)?
                .iter()
                .filter_map(Json::as_i64)
                .collect(),
        ),
        "double[]" => Value::DoubleArray(
            elems("double", Json::is_number)?
                .iter()
                .filter_map(Json::as_f64)
                .collect(),
        ),
        "float[]" => Value::FloatArray(
            elems("float", Json::is_number)?
                .iter()
                .filter_map(Json::as_f64)
                .map(|f| f as f32)
                .collect(),
        ),
        "string[]" => Value::StringArray(
            elems("string", Json::is_string)?
                .iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect(),
        ),
        // Everything else (raw, msgpack, structs, ...) is base64 bytes.
        _ => Value::Raw(base64_decode(v.as_str().ok_or_else(|| mismatch("string"))?)),
    };
    Ok(Entry {
        name: name.to_owned(),
        ty: ty.into(),
        properties: properties.clone(),
        value,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    // Written by Claude: expected strings from double-conversion's and
    // wpiutil's documented behavior.
    use super::*;

    #[test]
    fn doubles_like_double_conversion() {
        let cases: [(f64, &str); 14] = [
            (0.0, "0"),
            (-0.0, "0"),
            (42.0, "42"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1.25e22, "1.25e+22"),
            (123456.789, "123456.789"),
            (f64::INFINITY, "1e5000"),
            (f64::NAN, "null"),
        ];
        for (v, want) in cases {
            assert_eq!(fmt_double(v), want, "{v}");
        }
        assert_eq!(fmt_float(0.1), "0.1");
        assert_eq!(fmt_float(1.0 / 3.0), "0.33333334");
    }

    #[test]
    fn strings_like_wpiutil() {
        let mut s = String::new();
        push_string(&mut s, "a/b\"c\\d<e>&'=\u{7f}\té😀");
        // `\u` escapes spelled out, as `\` + `u` + hex.
        let u = |hex: &str| format!("{}u{hex}", '\\');
        let want = format!(
            r#""a\/b\"c\\d{}e{}{}{}{}{}\t{}{}{}""#,
            u("003c"),
            u("003e"),
            u("0026"),
            u("0027"),
            u("003d"),
            u("007f"),
            u("00e9"),
            u("d83d"),
            u("de00"),
        );
        assert_eq!(s, want);
    }

    #[test]
    fn objects_like_wpiutil_pretty() {
        let mut s = String::new();
        let one: Json = serde_json::json!({"persistent": true});
        push_json(&mut s, &one, 0);
        assert_eq!(s, r#"{"persistent": true}"#);
        let mut s = String::new();
        let two: Json = serde_json::json!({"a": [1, 2.5], "persistent": true});
        push_json(&mut s, &two, 0);
        assert_eq!(s, "{\n  \"a\": [1, 2.5],\n  \"persistent\": true\n}");
    }

    #[test]
    fn base64_round_trip_and_lenient_decode() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", &[0, 255, 128, 7]] {
            assert_eq!(base64_decode(&base64_encode(data)), data);
        }
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_decode("Zm9v!ignored"), b"foo");
    }
}
