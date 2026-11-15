//! JSON control messages (text frames).
//!
//! A text frame is a JSON array of messages, each shaped like
//! `{"method": "subscribe", "params": {...}}`. Clients send
//! [`ClientMessage`]s; servers send [`ServerMessage`]s.
//!
//! Encoding writes keys in the same order as ntcore, so frames match it byte
//! for byte. Decoding follows the spec's rule that a bad message is skipped
//! without dropping the rest of the frame; each skipped message adds a warning
//! in ntcore's wording, such as `"0: no pubuid key"` (the number is the
//! message's index in the frame).
//!
//! ```
//! use nt4_proto::{encode_text_frame, ClientMessage, SubscriptionOptions};
//! let sub = ClientMessage::Subscribe {
//!     subuid: 1,
//!     topics: vec!["/Telemetry/".into()],
//!     options: SubscriptionOptions { prefix: true, ..Default::default() },
//! };
//! assert_eq!(
//!     encode_text_frame(&[sub]),
//!     r#"[{"method":"subscribe","params":{"options":{"prefix":true},"topics":["/Telemetry/"],"subuid":1}}]"#
//! );
//! ```

use std::fmt::Write as _;

use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Value as Json;

use crate::NtType;

/// Topic properties: a JSON object. Keys the spec defines are `persistent`,
/// `retained` and `cached`; any others must be kept as they are.
pub type Properties = serde_json::Map<String, Json>;

/// Options sent with a `subscribe` message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubscriptionOptions {
    /// How often the server should send changes, in seconds.
    /// `None` means the default of 0.1 s.
    pub periodic: Option<f64>,
    /// Send every value change, not only the latest per period.
    pub all: bool,
    /// Only announce topics; send no values.
    pub topics_only: bool,
    /// Treat each entry in `topics` as a name prefix.
    pub prefix: bool,
}

/// A control message sent from a client to the server.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientMessage {
    /// Start publishing a topic (`publish`).
    Publish {
        /// Topic name.
        name: String,
        /// Client-chosen publisher id, used in this client's binary frames.
        pubuid: i32,
        /// Requested type; ignored by the server if the topic already exists.
        ty: NtType,
        /// Initial properties; ignored by the server if the topic already exists.
        properties: Properties,
    },
    /// Stop publishing (`unpublish`).
    Unpublish {
        /// The id from the matching `publish`.
        pubuid: i32,
    },
    /// Change a topic's properties (`setproperties`). A `null` value deletes
    /// that property.
    SetProperties {
        /// Topic name.
        name: String,
        /// Properties to change.
        update: Properties,
    },
    /// Subscribe to topics (`subscribe`). Reusing a `subuid` replaces that
    /// subscription.
    Subscribe {
        /// Client-chosen subscription id.
        subuid: i32,
        /// Topic names, or prefixes when `options.prefix` is set.
        topics: Vec<String>,
        /// Subscription options.
        options: SubscriptionOptions,
    },
    /// End a subscription (`unsubscribe`).
    Unsubscribe {
        /// The id from the matching `subscribe`.
        subuid: i32,
    },
}

/// A control message sent from the server to a client.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    /// A topic exists (`announce`).
    Announce {
        /// Topic name.
        name: String,
        /// Server-chosen topic id, used in the server's binary frames.
        id: i32,
        /// The topic's actual type.
        ty: NtType,
        /// Set when this answers the client's own `publish`.
        pubuid: Option<i32>,
        /// The topic's properties.
        properties: Properties,
    },
    /// A topic was deleted (`unannounce`).
    Unannounce {
        /// Topic name.
        name: String,
        /// The id it was announced with.
        id: i32,
    },
    /// A topic's properties changed (`properties`).
    Properties {
        /// Topic name.
        name: String,
        /// Properties that changed; `null` means deleted.
        update: Properties,
        /// True when this answers the client's own `setproperties`.
        ack: bool,
    },
}

/// A message that can be written into a text frame.
pub trait TextMessage {
    /// Append this message's JSON object to `out`.
    fn encode(&self, out: &mut String);
}

/// Build one text frame (a JSON array) from messages.
pub fn encode_text_frame<'a, M: TextMessage + 'a>(msgs: impl IntoIterator<Item = &'a M>) -> String {
    let mut out = String::from("[");
    for (i, m) in msgs.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        m.encode(&mut out);
    }
    out.push(']');
    out
}

fn push_str(out: &mut String, s: &str) {
    out.push_str(&serde_json::to_string(s).expect("strings always serialize"));
}

fn push_json(out: &mut String, v: &Properties) {
    out.push_str(&serde_json::to_string(v).expect("JSON values always serialize"));
}

fn push_float(out: &mut String, f: f64) {
    match serde_json::Number::from_f64(f) {
        Some(n) => {
            let _ = write!(out, "{n}");
        }
        // NaN and infinity are not JSON; send null so the peer rejects it.
        None => out.push_str("null"),
    }
}

impl TextMessage for ClientMessage {
    fn encode(&self, out: &mut String) {
        match self {
            ClientMessage::Publish {
                name,
                pubuid,
                ty,
                properties,
            } => {
                out.push_str(r#"{"method":"publish","params":{"name":"#);
                push_str(out, name);
                out.push_str(r#","properties":"#);
                push_json(out, properties);
                let _ = write!(out, r#","pubuid":{pubuid},"type":"#);
                push_str(out, &ty.to_string());
                out.push_str("}}");
            }
            ClientMessage::Unpublish { pubuid } => {
                let _ = write!(
                    out,
                    r#"{{"method":"unpublish","params":{{"pubuid":{pubuid}}}}}"#
                );
            }
            ClientMessage::SetProperties { name, update } => {
                out.push_str(r#"{"method":"setproperties","params":{"name":"#);
                push_str(out, name);
                out.push_str(r#","update":"#);
                push_json(out, update);
                out.push_str("}}");
            }
            ClientMessage::Subscribe {
                subuid,
                topics,
                options,
            } => {
                out.push_str(r#"{"method":"subscribe","params":{"options":{"#);
                let mut fields: Vec<&str> = Vec::new();
                if options.all {
                    fields.push(r#""all":true"#);
                }
                if options.topics_only {
                    fields.push(r#""topicsonly":true"#);
                }
                if options.prefix {
                    fields.push(r#""prefix":true"#);
                }
                out.push_str(&fields.join(","));
                if let Some(p) = options.periodic {
                    if !fields.is_empty() {
                        out.push(',');
                    }
                    out.push_str(r#""periodic":"#);
                    push_float(out, p);
                }
                out.push_str(r#"},"topics":["#);
                for (i, t) in topics.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    push_str(out, t);
                }
                let _ = write!(out, r#"],"subuid":{subuid}}}}}"#);
            }
            ClientMessage::Unsubscribe { subuid } => {
                let _ = write!(
                    out,
                    r#"{{"method":"unsubscribe","params":{{"subuid":{subuid}}}}}"#
                );
            }
        }
    }
}

impl TextMessage for ServerMessage {
    fn encode(&self, out: &mut String) {
        match self {
            ServerMessage::Announce {
                name,
                id,
                ty,
                pubuid,
                properties,
            } => {
                let _ = write!(out, r#"{{"method":"announce","params":{{"id":{id},"name":"#);
                push_str(out, name);
                out.push_str(r#","properties":"#);
                push_json(out, properties);
                if let Some(p) = pubuid {
                    let _ = write!(out, r#","pubuid":{p}"#);
                }
                out.push_str(r#","type":"#);
                push_str(out, &ty.to_string());
                out.push_str("}}");
            }
            ServerMessage::Unannounce { name, id } => {
                let _ = write!(
                    out,
                    r#"{{"method":"unannounce","params":{{"id":{id},"name":"#
                );
                push_str(out, name);
                out.push_str("}}");
            }
            ServerMessage::Properties { name, update, ack } => {
                out.push_str(r#"{"method":"properties","params":{"name":"#);
                push_str(out, name);
                out.push_str(r#","update":"#);
                push_json(out, update);
                if *ack {
                    out.push_str(r#","ack":true"#);
                }
                out.push_str("}}");
            }
        }
    }
}

/// The result of decoding one text frame.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDecode<M> {
    /// Messages that decoded, in frame order.
    pub messages: Vec<M>,
    /// One line per problem, in ntcore's wording. Messages with a problem are
    /// skipped, except an `announce` whose properties are not an object
    /// (ntcore warns and treats them as empty).
    pub warnings: Vec<String>,
}

/// Decode a text frame sent by a client (the server side reads these).
///
/// ```
/// let d = nt4_proto::decode_client_text(r#"[{"method":"unpublish","params":{}}]"#);
/// assert!(d.messages.is_empty());
/// assert_eq!(d.warnings, vec!["0: no pubuid key"]);
/// ```
pub fn decode_client_text(text: &str) -> TextDecode<ClientMessage> {
    decode_frame(text, |method, params, _warnings| {
        decode_client(method, params)
    })
}

/// Decode a text frame sent by the server (the client side reads these).
pub fn decode_server_text(text: &str) -> TextDecode<ServerMessage> {
    decode_frame(text, decode_server)
}

type Params = serde_json::Map<String, Json>;

fn decode_frame<M>(
    text: &str,
    decode: impl Fn(&str, &mut Params, &mut Vec<String>) -> Result<M, String>,
) -> TextDecode<M> {
    let mut out = TextDecode {
        messages: Vec::new(),
        warnings: Vec::new(),
    };
    let json = match parse_json(text) {
        Ok(j) => j,
        Err(e) => {
            out.warnings
                .push(format!("could not decode JSON message: {e}"));
            return out;
        }
    };
    let Json::Array(items) = json else {
        out.warnings.push("expected JSON array at top level".into());
        return out;
    };
    for (i, item) in items.into_iter().enumerate() {
        let result = (|| {
            let Json::Object(mut msg) = item else {
                return Err("expected message to be an object".to_string());
            };
            let method = match msg.remove("method") {
                None => return Err("no method key".into()),
                Some(Json::String(s)) => s,
                Some(_) => return Err("method must be a string".into()),
            };
            let mut params = match msg.remove("params") {
                None => return Err("no params key".into()),
                Some(Json::Object(p)) => p,
                Some(_) => return Err("params must be an object".into()),
            };
            decode(&method, &mut params, &mut out.warnings)
        })();
        match result {
            Ok(m) => out.messages.push(m),
            Err(e) => out.warnings.push(format!("{i}: {e}")),
        }
    }
    out
}

/// ntcore's JSON parser gives up below this many nested arrays and objects.
const MAX_JSON_DEPTH: usize = 20;

/// Parse a frame's JSON the way ntcore's parser reads it, where that differs
/// from serde_json's defaults:
///
/// - nothing may sit inside more than [`MAX_JSON_DEPTH`] arrays and objects;
/// - when an object repeats a key, the first value is kept;
/// - `-0` is the integer 0, not the double -0.0.
///
/// ntcore's parser also accepts some input that isn't JSON; that is rejected
/// here, as serde_json does.
fn parse_json(text: &str) -> Result<Json, String> {
    if too_deep(text) {
        return Err("depth_exceeded".into());
    }
    let text = negative_zero_as_integer(text);
    let mut de = serde_json::Deserializer::from_str(&text);
    let json = FirstKeyWins
        .deserialize(&mut de)
        .map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())?;
    Ok(json)
}

/// `text` with each `-0` number (not `-0.0`) written as `0`. ntcore's JSON
/// library reads `-0` as the integer 0, serde_json as the double -0.0,
/// which would make `"id": -0` "not a number" and change `-0` properties.
fn negative_zero_as_integer(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains("-0") {
        return text.into();
    }
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut in_string, mut escaped, mut last) = (false, false, 0);
    for i in 0..b.len() {
        let c = b[i];
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
        } else if c == b'"' {
            in_string = true;
        } else if c == b'-'
            && b.get(i + 1) == Some(&b'0')
            && !matches!(b.get(i + 2), Some(b'.' | b'e' | b'E' | b'0'..=b'9'))
            && !matches!(
                i.checked_sub(1).map(|j| b[j]),
                Some(b'0'..=b'9' | b'e' | b'E' | b'.')
            )
        {
            out.push_str(&text[last..i]);
            last = i + 1; // drop the '-'
        }
    }
    out.push_str(&text[last..]);
    out.into()
}

/// Whether any value or key in `text` is inside more than
/// [`MAX_JSON_DEPTH`] arrays and objects.
fn too_deep(text: &str) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for &c in text.as_bytes() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            b' ' | b'\t' | b'\n' | b'\r' | b',' | b':' => continue,
            b']' | b'}' => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        // Anything else starts a value or a key.
        if depth >= MAX_JSON_DEPTH {
            return true;
        }
        match c {
            b'[' | b'{' => depth += 1,
            b'"' => in_string = true,
            _ => {}
        }
    }
    false
}

/// Deserializes any JSON into a [`Json`], see [`parse_json`].
struct FirstKeyWins;

impl<'de> DeserializeSeed<'de> for FirstKeyWins {
    type Value = Json;

    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Json, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for FirstKeyWins {
    type Value = Json;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Json, E> {
        Ok(Json::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Json, E> {
        Ok(v.into())
    }

    fn visit_u64<E>(self, v: u64) -> Result<Json, E> {
        Ok(v.into())
    }

    fn visit_f64<E>(self, v: f64) -> Result<Json, E> {
        Ok(v.into())
    }

    fn visit_str<E>(self, v: &str) -> Result<Json, E> {
        Ok(Json::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Json, E> {
        Ok(Json::String(v))
    }

    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(FirstKeyWins)? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut object = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value_seed(FirstKeyWins)?;
            object.entry(key).or_insert(value);
        }
        Ok(Json::Object(object))
    }
}

fn get_string(params: &mut Params, key: &str) -> Result<String, String> {
    match params.remove(key) {
        None => Err(format!("no {key} key")),
        Some(Json::String(s)) => Ok(s),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

/// The id in `key`, checked as ntcore does; `what` names it in the range
/// error.
fn get_id(params: &Params, key: &str, what: &str) -> Result<i32, String> {
    match params.get(key) {
        None => Err(format!("no {key} key")),
        Some(v) => id_value(v, &format!("{key} must be a number"), what),
    }
}

/// An id value. An integer above `i64::MAX` is a double by now (see
/// [`parse_json`]), so it is "not a number" here, as in ntcore.
fn id_value(v: &Json, not_a_number: &str, what: &str) -> Result<i32, String> {
    match v.as_i64() {
        Some(i) => id_range(i, what),
        None => Err(not_a_number.to_owned()),
    }
}

/// ntcore limits ids to the 32-bit range, excluding both end points.
fn id_range(v: i64, what: &str) -> Result<i32, String> {
    if v >= 0x7fff_ffff || v <= -0x8000_0000 {
        Err(format!("{what} out of range"))
    } else {
        Ok(v as i32)
    }
}

fn get_object(params: &mut Params, key: &str) -> Result<Properties, String> {
    match params.remove(key) {
        None => Err(format!("no {key} key")),
        Some(Json::Object(o)) => Ok(o),
        Some(_) => Err(format!("{key} must be an object")),
    }
}

fn decode_client(method: &str, params: &mut Params) -> Result<ClientMessage, String> {
    Ok(match method {
        "publish" => {
            let name = get_string(params, "name")?;
            let ty = get_string(params, "type")?;
            let pubuid = get_id(params, "pubuid", "pubuid")?;
            // A missing properties key is treated as empty.
            let properties = match params.remove("properties") {
                None => Properties::new(),
                Some(Json::Object(o)) => o,
                Some(_) => return Err("properties must be an object".into()),
            };
            ClientMessage::Publish {
                name,
                pubuid,
                ty: ty.as_str().into(),
                properties,
            }
        }
        "unpublish" => ClientMessage::Unpublish {
            pubuid: get_id(params, "pubuid", "pubuid")?,
        },
        "setproperties" => {
            let name = get_string(params, "name")?;
            let update = get_object(params, "update")?;
            ClientMessage::SetProperties { name, update }
        }
        "subscribe" => {
            let subuid = get_id(params, "subuid", "subuid")?;
            let options = match params.remove("options") {
                None => SubscriptionOptions::default(),
                Some(Json::Object(o)) => decode_options(&o)?,
                Some(_) => return Err("options must be an object".into()),
            };
            let topics = match params.remove("topics") {
                None => return Err("no topics key".into()),
                Some(Json::Array(a)) => a
                    .into_iter()
                    .enumerate()
                    .map(|(n, t)| match t {
                        Json::String(s) => Ok(s),
                        _ => Err(format!("topics/{n} must be a string")),
                    })
                    .collect::<Result<_, _>>()?,
                Some(_) => return Err("topics must be an array".into()),
            };
            ClientMessage::Subscribe {
                subuid,
                topics,
                options,
            }
        }
        // ntcore says "pubuid out of range" here; this says subuid.
        "unsubscribe" => ClientMessage::Unsubscribe {
            subuid: get_id(params, "subuid", "subuid")?,
        },
        other => return Err(format!("unrecognized method '{other}'")),
    })
}

fn decode_options(o: &serde_json::Map<String, Json>) -> Result<SubscriptionOptions, String> {
    let mut options = SubscriptionOptions::default();
    if let Some(p) = o.get("periodic") {
        let val = p
            .as_f64()
            .ok_or_else(|| "periodic value must be a number".to_string())?;
        if !val.is_finite() || val < 0.0 || val > f64::from(u32::MAX) / 1000.0 {
            return Err("periodic value out of range".into());
        }
        options.periodic = Some(val);
    }
    let flag = |key: &str| -> Result<bool, String> {
        match o.get(key) {
            None => Ok(false),
            Some(Json::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("{key} value must be a boolean")),
        }
    };
    options.all = flag("all")?;
    options.topics_only = flag("topicsonly")?;
    options.prefix = flag("prefix")?;
    Ok(options)
}

fn decode_server(
    method: &str,
    params: &mut Params,
    warnings: &mut Vec<String>,
) -> Result<ServerMessage, String> {
    Ok(match method {
        "announce" => {
            let name = get_string(params, "name")?;
            let id = get_id(params, "id", "id")?;
            let ty = get_string(params, "type")?;
            let pubuid = match params.get("pubuid") {
                None => None,
                Some(v) => Some(id_value(v, "pubuid value must be a number", "pubuid")?),
            };
            let properties = match params.remove("properties") {
                None => return Err("no properties key".into()),
                Some(Json::Object(o)) => o,
                Some(_) => {
                    warnings.push(format!("{name}: properties is not an object"));
                    Properties::new()
                }
            };
            ServerMessage::Announce {
                name,
                id,
                ty: ty.as_str().into(),
                pubuid,
                properties,
            }
        }
        "unannounce" => {
            let name = get_string(params, "name")?;
            let id = get_id(params, "id", "id")?;
            ServerMessage::Unannounce { name, id }
        }
        "properties" => {
            let name = get_string(params, "name")?;
            let update = get_object(params, "update")?;
            let ack = match params.get("ack") {
                None => false,
                Some(Json::Bool(b)) => *b,
                Some(_) => return Err("ack must be a boolean".into()),
            };
            ServerMessage::Properties { name, update, ack }
        }
        other => return Err(format!("unrecognized method '{other}'")),
    })
}
