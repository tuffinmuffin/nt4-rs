//! Differential fuzzing of nt4-proto's decoders against ntcore's.
//!
//! [`generate`] makes frames that look like what a peer could send: valid
//! ones, valid ones with a byte changed, cut short, plain noise, and very
//! large ones (huge claimed lengths with no data behind them, megabyte
//! strings, deep JSON). [`check`] decodes a frame with nt4-proto and, when
//! the ntcore oracle is built in (see `build.rs`), with ntcore's own
//! `WireDecodeBinary` / `WireDecodeText`, and compares the results in one
//! canonical JSON form. Without the oracle it still checks that nt4-proto
//! doesn't panic and that what it decodes re-encodes to the same thing.
//!
//! The `nt4-fuzz` binary runs this in a loop; `fuzz/` has cargo-fuzz targets
//! that call [`check`] on coverage-guided inputs.
//!
//! Written by Claude.

use nt4_proto::{
    BinaryMessage, BinaryReader, ClientMessage, ServerMessage, Value, decode_client_text,
    decode_server_text, encode_binary, encode_text_frame,
};
use serde_json::{Value as Json, json};

mod generator;
pub use generator::{Rng, generate};

/// Which decoder a frame is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// MessagePack value frames (both directions use the same format).
    Binary,
    /// JSON text frames a client sends (the server decodes them).
    ClientText,
    /// JSON text frames the server sends (a client decodes them).
    ServerText,
}

impl Target {
    /// Every target.
    pub const ALL: [Target; 3] = [Target::Binary, Target::ClientText, Target::ServerText];

    /// Short name, as on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Target::Binary => "binary",
            Target::ClientText => "client-text",
            Target::ServerText => "server-text",
        }
    }
}

/// Whether the ntcore oracle is built in.
pub fn has_oracle() -> bool {
    cfg!(has_ntcore)
}

/// What [`check`] found wrong with one frame.
#[derive(Debug, Clone)]
pub struct Mismatch {
    /// The decoder.
    pub target: Target,
    /// A short, stable description, for grouping mismatches of one kind.
    pub signature: String,
    /// nt4-proto's result, canonical.
    pub ours: Json,
    /// ntcore's result, canonical (`null` for self-checks).
    pub theirs: Json,
}

/// What a frame decoded to, for statistics.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Messages decoded (or handler calls, for text).
    pub accepted: usize,
    /// Whether decoding stopped on an error, or warned.
    pub rejected: bool,
    /// ntcore threw this exception instead of returning (which ends an
    /// ntcore process). Not compared: a bug in ntcore, not nt4.
    pub ntcore_exception: Option<String>,
    /// nt4 differs from ntcore here on purpose; see [`known_deviation`].
    pub known_deviation: Option<&'static str>,
}

/// Decode `input` with nt4-proto and ntcore (when built in) and compare.
/// Text input that isn't UTF-8 is converted lossily first, as a WebSocket
/// text frame must be UTF-8.
pub fn check(target: Target, input: &[u8]) -> Result<Outcome, Mismatch> {
    let text = String::from_utf8_lossy(input);
    let ours = match std::panic::catch_unwind(|| match target {
        Target::Binary => canon_binary(input),
        Target::ClientText => canon_client_text(&text),
        Target::ServerText => canon_server_text(&text),
    }) {
        Ok(j) => j,
        Err(_) => {
            return Err(Mismatch {
                target,
                signature: "nt4 panicked".into(),
                ours: Json::Null,
                theirs: Json::Null,
            });
        }
    };
    let outcome = outcome(target, &ours);
    if let Some(problem) = self_check(target, input, &text) {
        return Err(Mismatch {
            target,
            signature: problem,
            ours,
            theirs: Json::Null,
        });
    }
    #[cfg(has_ntcore)]
    {
        let theirs = match target {
            Target::Binary => oracle::binary(input),
            Target::ClientText => oracle::client_text(&text),
            Target::ServerText => oracle::server_text(&text),
        };
        if let Some(e) = theirs.get("exception").and_then(Json::as_str) {
            return Ok(Outcome {
                ntcore_exception: Some(e.to_owned()),
                ..outcome
            });
        }
        if let Some(signature) = compare(target, &ours, &theirs).map(|s| collapse_digits(&s)) {
            if let Some(why) = known_deviation(target, &text, &ours, &theirs) {
                return Ok(Outcome {
                    known_deviation: Some(why),
                    ..outcome
                });
            }
            return Err(Mismatch {
                target,
                signature,
                ours,
                theirs,
            });
        }
    }
    Ok(outcome)
}

fn outcome(target: Target, canon: &Json) -> Outcome {
    match target {
        Target::Binary => Outcome {
            accepted: canon["msgs"].as_array().map_or(0, Vec::len),
            rejected: !canon["error"].is_null(),
            ntcore_exception: None,
            known_deviation: None,
        },
        _ => Outcome {
            accepted: canon["calls"].as_array().map_or(0, Vec::len),
            rejected: canon["warnings"].as_array().is_some_and(|w| !w.is_empty()),
            ntcore_exception: None,
            known_deviation: None,
        },
    }
}

/// Checks that need no oracle: what decodes must re-encode and decode to
/// the same messages.
fn self_check(target: Target, input: &[u8], text: &str) -> Option<String> {
    match target {
        Target::Binary => {
            let msgs: Vec<BinaryMessage> = BinaryReader::new(input).map_while(Result::ok).collect();
            let mut again = Vec::new();
            for m in &msgs {
                encode_binary(&mut again, m);
            }
            let back: Vec<BinaryMessage> =
                BinaryReader::new(&again).map_while(Result::ok).collect();
            // `{:?}` so NaN compares equal to itself.
            (format!("{back:?}") != format!("{msgs:?}")).then(|| "binary round trip".into())
        }
        Target::ClientText => {
            let msgs = decode_client_text(text).messages;
            let back = decode_client_text(&encode_text_frame(&msgs)).messages;
            (format!("{back:?}") != format!("{msgs:?}")).then(|| "client text round trip".into())
        }
        Target::ServerText => {
            let msgs = decode_server_text(text).messages;
            let back = decode_server_text(&encode_text_frame(&msgs)).messages;
            (format!("{back:?}") != format!("{msgs:?}")).then(|| "server text round trip".into())
        }
    }
}

/// Where nt4 differs from ntcore on purpose, the reason, given nt4's result
/// on the frame. Each one is listed in crates/nt4-fuzz/README.md.
pub fn known_deviation(
    target: Target,
    text: &str,
    ours: &Json,
    theirs: &Json,
) -> Option<&'static str> {
    if target == Target::Binary {
        return None;
    }
    let warned = |needle: &str| {
        ours["warnings"].as_array().is_some_and(|w| {
            w.iter()
                .any(|w| w.as_str().is_some_and(|w| w.contains(needle)))
        })
    };
    let ntcore_warned = |needle: &str| {
        theirs["warnings"].as_array().is_some_and(|w| {
            w.iter()
                .any(|w| w.as_str().is_some_and(|w| w.contains(needle)))
        })
    };
    if warned("subuid out of range") && ntcore_warned("pubuid out of range") {
        return Some("unsubscribe's range error says subuid (ntcore says pubuid, a typo)");
    }
    if warned("recursion limit exceeded") {
        return Some("JSON nested deeper than 128 levels is rejected");
    }
    if warned("periodic value out of range") {
        return Some(
            "a negative, huge or infinite periodic is rejected (ntcore converts it to unsigned ms, undefined behavior)",
        );
    }
    if has_integer_above_i64(text) && (warned("out of range") || warned("must be a number")) {
        return Some("integers above i64::MAX are out of range (ntcore wraps them)");
    }
    None
}

/// Whether `text` has an integer literal above `i64::MAX`.
fn has_integer_above_i64(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit()
            && (i == 0 || !matches!(b[i - 1], b'0'..=b'9' | b'.' | b'e' | b'E' | b'-'))
        {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let integer = !matches!(b.get(i), Some(b'.' | b'e' | b'E'));
            if integer
                && i - start >= 19
                && text[start..i]
                    .parse::<u128>()
                    .map_or(true, |v| v > i64::MAX as u128)
            {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// Where the two results differ, or `None` if they agree.
pub fn compare(target: Target, ours: &Json, theirs: &Json) -> Option<String> {
    match target {
        Target::Binary => {
            let (a, b) = (&ours["msgs"], &theirs["msgs"]);
            if a != b {
                let (a, b) = (a.as_array()?, b.as_array()?);
                let i = a.iter().zip(b).position(|(x, y)| x != y);
                return Some(match i {
                    Some(i) => format!(
                        "binary: message {i} differs ({} vs ntcore {})",
                        a[i]["value"]["t"], b[i]["value"]["t"]
                    ),
                    None if a.len() < b.len() => format!(
                        "binary: nt4 rejects a message ntcore accepts as {}",
                        b[a.len()]["value"]["t"]
                    ),
                    None => format!(
                        "binary: nt4 accepts a message ntcore rejects ({})",
                        theirs["error"]
                    ),
                });
            }
            (ours["error"].is_null() != theirs["error"].is_null())
                .then(|| "binary: one reports an error, the other doesn't".into())
        }
        _ => {
            if ours["calls"] != theirs["calls"] {
                let (a, b) = (ours["calls"].as_array()?, theirs["calls"].as_array()?);
                let i = a.iter().zip(b).position(|(x, y)| x != y);
                return Some(match i {
                    Some(i) => {
                        let field = a[i]
                            .as_object()
                            .and_then(|o| o.keys().find(|k| a[i][k.as_str()] != b[i][k.as_str()]))
                            .cloned()
                            .unwrap_or_default();
                        format!("text: {} differs in {field}", b[i]["m"])
                    }
                    None if a.len() < b.len() => format!("text: nt4 skips a {}", b[a.len()]["m"]),
                    None => format!("text: nt4 accepts an extra {}", a[b.len()]["m"]),
                });
            }
            // Only the start of a JSON syntax error is compared: the rest is
            // the JSON library's own words (serde_json here, nlohmann's in
            // ntcore 2026, another in ntcore 2027).
            let syntax = |w: &Json| -> Vec<String> {
                w.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Json::as_str)
                    // ntcore's logger passes lines as C strings, so they end
                    // at a NUL.
                    .map(|w| w.split('\0').next().unwrap_or_default())
                    .map(|w| match w.split_once("could not decode JSON message: ") {
                        Some((a, _)) => format!("{a}could not decode JSON message: ..."),
                        None => w.to_owned(),
                    })
                    .collect()
            };
            (syntax(&ours["warnings"]) != syntax(&theirs["warnings"])).then(|| {
                let (a, b) = (
                    ours["warnings"].as_array().cloned().unwrap_or_default(),
                    theirs["warnings"].as_array().cloned().unwrap_or_default(),
                );
                let i = a
                    .iter()
                    .zip(&b)
                    .position(|(x, y)| x != y)
                    .unwrap_or(a.len().min(b.len()));
                format!(
                    "text: warning differs: {} vs ntcore {}",
                    short(a.get(i)),
                    short(b.get(i))
                )
            })
        }
    }
}

/// Digits replaced by `#`, so mismatches group by kind, not position.
pub fn collapse_digits(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !out.ends_with('#') {
                out.push('#');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A warning with its message index or topic name taken out, for grouping.
fn short(w: Option<&Json>) -> String {
    let Some(w) = w.and_then(Json::as_str) else {
        return "(none)".into();
    };
    match w.split_once(": ") {
        Some((_, rest)) => format!("{rest:?}"),
        None => format!("{w:?}"),
    }
}

// ----- Canonical form (see oracle/oracle.cpp) -----

/// Bring ntcore's result to what nt4 does on purpose: ntcore keeps strings
/// that aren't UTF-8 as they are; nt4 replaces bad sequences with U+FFFD.
#[cfg_attr(not(has_ntcore), allow(dead_code))]
fn normalize(mut j: Json) -> Json {
    fn fix(s: &mut Json) {
        if let Some(h) = s.as_str() {
            let bytes: Vec<u8> = (0..h.len() / 2)
                .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap_or(0))
                .collect();
            *s = Json::String(hex(String::from_utf8_lossy(&bytes).as_bytes()));
        }
    }
    if let Some(msgs) = j.get_mut("msgs").and_then(Json::as_array_mut) {
        for m in msgs {
            let v = &mut m["value"];
            match v["t"].as_str() {
                Some("string") => fix(&mut v["v"]),
                Some("string[]") => v["v"].as_array_mut().into_iter().flatten().for_each(fix),
                _ => {}
            }
        }
    }
    j
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn value_json(v: &Value) -> Json {
    let bits64 = |d: f64| hex(&d.to_le_bytes());
    let bits32 = |f: f32| hex(&f.to_le_bytes());
    let b = |b: bool| if b { "true" } else { "false" };
    match v {
        Value::Boolean(x) => json!({"t": "boolean", "v": b(*x)}),
        Value::Double(x) => json!({"t": "double", "v": bits64(*x)}),
        Value::Int(x) => json!({"t": "int", "v": x.to_string()}),
        Value::Float(x) => json!({"t": "float", "v": bits32(*x)}),
        Value::String(x) => json!({"t": "string", "v": hex(x.as_bytes())}),
        Value::Raw(x) => json!({"t": "raw", "v": hex(x)}),
        Value::BooleanArray(a) => {
            json!({"t": "boolean[]", "v": a.iter().map(|x| b(*x)).collect::<Vec<_>>()})
        }
        Value::DoubleArray(a) => {
            json!({"t": "double[]", "v": a.iter().map(|x| bits64(*x)).collect::<Vec<_>>()})
        }
        Value::IntArray(a) => {
            json!({"t": "int[]", "v": a.iter().map(i64::to_string).collect::<Vec<_>>()})
        }
        Value::FloatArray(a) => {
            json!({"t": "float[]", "v": a.iter().map(|x| bits32(*x)).collect::<Vec<_>>()})
        }
        Value::StringArray(a) => {
            json!({"t": "string[]", "v": a.iter().map(|x| hex(x.as_bytes())).collect::<Vec<_>>()})
        }
    }
}

/// nt4-proto's decoding of a binary frame, in the oracle's form.
pub fn canon_binary(frame: &[u8]) -> Json {
    let mut msgs = Vec::new();
    let mut error = Json::Null;
    for r in BinaryReader::new(frame) {
        match r {
            Ok(m) => msgs.push(json!({
                "id": m.id.to_string(),
                "time": m.timestamp_us.to_string(),
                "value": value_json(&m.value),
            })),
            Err(e) => error = Json::String(e.to_string()),
        }
    }
    json!({"msgs": msgs, "error": error})
}

/// nt4-proto's decoding of a client text frame, in the oracle's form.
pub fn canon_client_text(text: &str) -> Json {
    let d = decode_client_text(text);
    let calls: Vec<Json> = d
        .messages
        .iter()
        .map(|m| match m {
            ClientMessage::Publish {
                name,
                pubuid,
                ty,
                properties,
            } => json!({
                "m": "publish", "pubuid": pubuid.to_string(), "name": hex(name.as_bytes()),
                "type": hex(ty.to_string().as_bytes()), "props": properties,
            }),
            ClientMessage::Unpublish { pubuid } => {
                json!({"m": "unpublish", "pubuid": pubuid.to_string()})
            }
            ClientMessage::SetProperties { name, update } => {
                json!({"m": "setproperties", "name": hex(name.as_bytes()), "update": update})
            }
            ClientMessage::Subscribe {
                subuid,
                topics,
                options,
            } => {
                // No period: ntcore's PubSubOptionsImpl default.
                let periodic = options.periodic.unwrap_or(0.1);
                json!({
                    "m": "subscribe", "subuid": subuid.to_string(),
                    "topics": topics.iter().map(|t| hex(t.as_bytes())).collect::<Vec<_>>(),
                    "options": {
                        "periodic": hex(&periodic.to_le_bytes()), "all": options.all,
                        "topicsOnly": options.topics_only, "prefix": options.prefix,
                    },
                })
            }
            ClientMessage::Unsubscribe { subuid } => {
                json!({"m": "unsubscribe", "subuid": subuid.to_string()})
            }
        })
        .collect();
    json!({"calls": calls, "warnings": d.warnings})
}

/// nt4-proto's decoding of a server text frame, in the oracle's form.
pub fn canon_server_text(text: &str) -> Json {
    let d = decode_server_text(text);
    let calls: Vec<Json> = d
        .messages
        .iter()
        .map(|m| match m {
            ServerMessage::Announce {
                name,
                id,
                ty,
                pubuid,
                properties,
            } => json!({
                "m": "announce", "name": hex(name.as_bytes()), "id": id.to_string(),
                "type": hex(ty.to_string().as_bytes()),
                "pubuid": pubuid.map(|p| p.to_string()), "props": properties,
            }),
            ServerMessage::Unannounce { name, id } => {
                json!({"m": "unannounce", "name": hex(name.as_bytes()), "id": id.to_string()})
            }
            ServerMessage::Properties { name, update, ack } => json!({
                "m": "properties", "name": hex(name.as_bytes()), "update": update, "ack": ack,
            }),
        })
        .collect();
    json!({"calls": calls, "warnings": d.warnings})
}

// ----- Shrinking -----

/// Make a failing input smaller while it still fails with the same
/// signature: drop chunks of bytes, halving the chunk size each pass.
pub fn shrink(target: Target, input: &[u8], signature: &str, budget: usize) -> Vec<u8> {
    let fails = |b: &[u8]| matches!(check(target, b), Err(m) if m.signature == signature);
    let mut best = input.to_vec();
    let mut tries = 0;
    let mut chunk = best.len().div_ceil(2).max(1);
    while chunk >= 1 && tries < budget {
        let mut i = 0;
        let mut changed = false;
        while i < best.len() && tries < budget {
            let end = (i + chunk).min(best.len());
            let mut candidate = best[..i].to_vec();
            candidate.extend_from_slice(&best[end..]);
            tries += 1;
            if fails(&candidate) {
                best = candidate;
                changed = true;
            } else {
                i += chunk;
            }
        }
        if !changed {
            if chunk == 1 {
                break;
            }
            chunk /= 2;
        }
    }
    best
}

// ----- ClientCore -----

/// Feed one frame to a connected, time-synced `ClientCore` and drain what it
/// produces; panics are the failure this looks for.
pub fn drive_client_core(target: Target, input: &[u8]) {
    use nt4_core::{ClientCore, ProtocolVersion};
    let mut core = ClientCore::new();
    let _ = core.subscribe(
        vec![String::new()],
        nt4_proto::SubscriptionOptions {
            prefix: true,
            ..Default::default()
        },
    );
    core.connect(1_000, ProtocolVersion::V4_1);
    let mut reply = Vec::new();
    encode_binary(
        &mut reply,
        &BinaryMessage {
            id: nt4_proto::RTT_ID,
            timestamp_us: 5_000,
            value: Value::Int(1_000),
        },
    );
    core.handle_binary(2_000, &reply);
    // Announce some ids so value frames have topics to land on.
    let announce: String = (0..8)
        .map(|i| {
            format!(r#"{{"method":"announce","params":{{"name":"/t{i}","id":{i},"type":"double","properties":{{}}}}}}"#)
        })
        .collect::<Vec<_>>()
        .join(",");
    core.handle_text(2_000, &format!("[{announce}]"));
    match target {
        Target::Binary => core.handle_binary(3_000, input),
        _ => core.handle_text(3_000, &String::from_utf8_lossy(input)),
    }
    core.send_outgoing(400_000, true);
    while core.poll_event().is_some() {}
    while core.poll_transmit().is_some() {}
}

#[cfg(has_ntcore)]
#[allow(unsafe_code)]
mod oracle {
    //! Safe wrappers around oracle/oracle.cpp.
    use serde_json::Value as Json;
    use std::ffi::{CStr, c_char};

    unsafe extern "C" {
        fn nt4_oracle_binary(data: *const u8, len: usize) -> *mut c_char;
        fn nt4_oracle_client_text(data: *const c_char, len: usize) -> *mut c_char;
        fn nt4_oracle_server_text(data: *const c_char, len: usize) -> *mut c_char;
        fn nt4_oracle_free(s: *mut c_char);
    }

    fn take(p: *mut c_char) -> Json {
        // SAFETY: the oracle returns a NUL-terminated malloc'd string that we
        // free once.
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        unsafe { nt4_oracle_free(p) };
        let mut de = serde_json::Deserializer::from_str(&s);
        de.disable_recursion_limit();
        let j = serde_json::Value::deserialize(&mut de).expect("oracle writes JSON");
        super::normalize(j)
    }

    pub fn binary(b: &[u8]) -> Json {
        // SAFETY: pointer and length describe `b`, which outlives the call.
        take(unsafe { nt4_oracle_binary(b.as_ptr(), b.len()) })
    }

    pub fn client_text(t: &str) -> Json {
        // SAFETY: as above.
        take(unsafe { nt4_oracle_client_text(t.as_ptr().cast(), t.len()) })
    }

    pub fn server_text(t: &str) -> Json {
        // SAFETY: as above.
        take(unsafe { nt4_oracle_server_text(t.as_ptr().cast(), t.len()) })
    }
}
