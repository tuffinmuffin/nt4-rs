//! Frame generators. MessagePack and JSON are written by hand, so frames can
//! be anything a peer might send: non-minimal integer widths, lengths that
//! promise more data than there is, duplicate JSON keys, numbers no type
//! holds.
//!
//! Written by Claude.

use crate::Target;

/// xorshift64*: small, fast and repeatable from a seed.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
    /// Very large items still allowed in this frame.
    big: u32,
}

impl Rng {
    /// Start from `seed` (0 is replaced, as xorshift can't leave it).
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.max(1),
            big: 1,
        }
    }
    /// The next 64 random bits.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// With probability 1 in `n`, and at most once per frame: whether to
    /// make the next item very large.
    fn big(&mut self, n: u64) -> bool {
        if self.big > 0 && self.one_in(n) {
            self.big -= 1;
            true
        } else {
            false
        }
    }
    /// A number in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    /// True with probability 1 in `n`.
    pub fn one_in(&mut self, n: u64) -> bool {
        self.below(n) == 0
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// One frame for `target`.
pub fn generate(target: Target, rng: &mut Rng) -> Vec<u8> {
    rng.big = 1;
    let mut frame = match target {
        Target::Binary => {
            let mut out = Vec::new();
            for _ in 0..1 + rng.below(4) {
                message(rng, &mut out);
            }
            out
        }
        Target::ClientText => text_frame(rng, true).into_bytes(),
        Target::ServerText => text_frame(rng, false).into_bytes(),
    };
    mutate(rng, &mut frame, target != Target::Binary);
    frame
}

/// Damage a frame some of the time: change bytes, cut it short, or replace
/// it with noise.
fn mutate(rng: &mut Rng, frame: &mut Vec<u8>, text: bool) {
    match rng.below(10) {
        0 | 1 if !frame.is_empty() => {
            for _ in 0..1 + rng.below(3) {
                let i = rng.below(frame.len() as u64) as usize;
                frame[i] = if text {
                    *rng.pick(b"{}[]\",:0-.e\\ aZ\x00\x7f")
                } else {
                    rng.next() as u8
                };
            }
        }
        2 => {
            let n = rng.below(frame.len() as u64 + 1) as usize;
            frame.truncate(n);
        }
        3 if rng.one_in(4) => {
            *frame = (0..rng.below(64)).map(|_| rng.next() as u8).collect();
        }
        _ => {}
    }
}

// ----- MessagePack -----

/// An integer in a randomly chosen encoding that can hold it, often not the
/// smallest.
fn int(rng: &mut Rng, out: &mut Vec<u8>, v: i128) {
    let mut forms: Vec<u8> = Vec::new();
    if (0..=127).contains(&v) {
        forms.push(0x00);
    }
    if (-32..0).contains(&v) {
        forms.push(0xe0);
    }
    for (m, lo, hi) in [
        (0xcc, 0i128, u8::MAX as i128),
        (0xcd, 0, u16::MAX as i128),
        (0xce, 0, u32::MAX as i128),
        (0xcf, 0, u64::MAX as i128),
        (0xd0, i8::MIN as i128, i8::MAX as i128),
        (0xd1, i16::MIN as i128, i16::MAX as i128),
        (0xd2, i32::MIN as i128, i32::MAX as i128),
        (0xd3, i64::MIN as i128, i64::MAX as i128),
    ] {
        if (lo..=hi).contains(&v) {
            forms.push(m);
        }
    }
    let Some(&m) = (!forms.is_empty()).then(|| rng.pick(&forms)) else {
        out.push(0xc0); // nothing holds it: nil
        return;
    };
    match m {
        0x00 => out.push(v as u8),
        0xe0 => out.push(v as i8 as u8),
        0xcc | 0xd0 => out.extend([m, v as u8]),
        0xcd | 0xd1 => {
            out.push(m);
            out.extend((v as u16).to_be_bytes());
        }
        0xce | 0xd2 => {
            out.push(m);
            out.extend((v as u32).to_be_bytes());
        }
        _ => {
            out.push(m);
            out.extend((v as u64).to_be_bytes());
        }
    }
}

/// An interesting integer: small, at a type boundary, or random.
fn some_int(rng: &mut Rng) -> i128 {
    const EDGES: [i128; 16] = [
        0,
        1,
        -1,
        127,
        128,
        -32,
        -33,
        i32::MAX as i128,
        i32::MAX as i128 + 1,
        i32::MIN as i128,
        i32::MIN as i128 - 1,
        i64::MAX as i128,
        i64::MAX as i128 + 1,
        i64::MIN as i128,
        u64::MAX as i128,
        u32::MAX as i128,
    ];
    match rng.below(4) {
        0 => *rng.pick(&EDGES),
        1 => (rng.next() as i64 >> rng.below(64)) as i128,
        2 => rng.below(20) as i128,
        _ => (rng.next() >> rng.below(64)) as i128,
    }
}

fn some_f64(rng: &mut Rng) -> f64 {
    const EDGES: [f64; 9] = [
        0.0,
        -0.0,
        1.5,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MIN_POSITIVE,
        5e-324,
        f64::MAX,
    ];
    if rng.one_in(3) {
        *rng.pick(&EDGES)
    } else {
        f64::from_bits(rng.next())
    }
}

fn len_header(rng: &mut Rng, out: &mut Vec<u8>, fix: Option<(u8, usize)>, ms: [u8; 3], n: usize) {
    // fix: (marker base, max) for the fix form, if the type has one.
    let mut forms: Vec<usize> = Vec::new();
    if let Some((_, max)) = fix
        && n <= max
    {
        forms.push(0);
    }
    if n <= u8::MAX as usize && ms[0] != 0 {
        forms.push(1);
    }
    if n <= u16::MAX as usize {
        forms.push(2);
    }
    forms.push(3);
    match *rng.pick(&forms) {
        0 => out.push(fix.unwrap().0 | n as u8),
        1 => out.extend([ms[0], n as u8]),
        2 => {
            out.push(ms[1]);
            out.extend((n as u16).to_be_bytes());
        }
        _ => {
            out.push(ms[2]);
            out.extend((n as u32).to_be_bytes());
        }
    }
}

fn str_header(rng: &mut Rng, out: &mut Vec<u8>, n: usize) {
    len_header(rng, out, Some((0xa0, 31)), [0xd9, 0xda, 0xdb], n);
}
fn bin_header(rng: &mut Rng, out: &mut Vec<u8>, n: usize) {
    len_header(rng, out, None, [0xc4, 0xc5, 0xc6], n);
}
fn array_header(rng: &mut Rng, out: &mut Vec<u8>, n: usize) {
    len_header(rng, out, Some((0x90, 15)), [0, 0xdc, 0xdd], n);
}

/// A length for a string, binary or array: usually small, sometimes large.
fn some_len(rng: &mut Rng) -> usize {
    if rng.big(100) {
        return 1 << 20 | rng.below(1 << 20) as usize; // 1 to 2 MiB
    }
    if rng.one_in(50) {
        return rng.below(70_000) as usize; // past the 16-bit forms
    }
    rng.below(40) as usize
}

fn some_str(rng: &mut Rng) -> Vec<u8> {
    let n = some_len(rng);
    match rng.below(6) {
        0 => vec![0xff; n.min(4)], // not UTF-8
        1 => "é€😀".repeat(n / 9 + 1).into_bytes(),
        2 if n > 0 => {
            // A multi-byte character cut in half.
            let mut s = "é".repeat(n).into_bytes();
            s.pop();
            s
        }
        _ => vec![b'a' + rng.below(26) as u8; n],
    }
}

/// Any item, of any kind.
fn any_item(rng: &mut Rng, out: &mut Vec<u8>, depth: u32) {
    match rng.below(if depth > 3 { 13 } else { 15 }) {
        0 => out.push(0xc2 + rng.below(2) as u8),
        1 => {
            out.push(0xcb);
            out.extend(some_f64(rng).to_be_bytes());
        }
        2 => {
            out.push(0xca);
            out.extend((some_f64(rng) as f32).to_be_bytes());
        }
        3 | 4 => {
            let v = some_int(rng);
            int(rng, out, v)
        }
        5 => {
            let s = some_str(rng);
            str_header(rng, out, s.len());
            out.extend(s);
        }
        6 => {
            let n = some_len(rng);
            bin_header(rng, out, n);
            out.extend(std::iter::repeat_n(7, n));
        }
        7 => out.push(0xc0),
        8 => out.push(0xc1), // never used
        9 => {
            // A map.
            out.push(0x81);
            out.push(0xc0);
            out.push(0xc0);
        }
        10 => {
            // Extensions: fixext and ext8.
            if rng.one_in(2) {
                out.extend([0xd4, 1, 0]);
            } else {
                out.extend([0xc7, 2, 1, 0, 0]);
            }
        }
        11 | 12 => {
            // A length far beyond the data behind it.
            let n = u32::MAX - rng.below(1 << 20) as u32;
            match rng.below(4) {
                0 => out.push(0xdb),
                1 => out.push(0xc6),
                2 => out.push(0xdd),
                _ => out.push(0xdf),
            }
            out.extend(n.to_be_bytes());
            out.extend([0xc3; 3]);
        }
        _ => {
            // A nested array, sometimes very deep.
            if rng.big(30) {
                let d = 1_000 + rng.below(50_000) as usize;
                out.extend(std::iter::repeat_n(0x91, d));
                out.push(0xc3);
            } else {
                let n = rng.below(6) as usize;
                array_header(rng, out, n);
                for _ in 0..n {
                    any_item(rng, out, depth + 1);
                }
            }
        }
    }
}

const TYPES: [i128; 16] = [0, 1, 2, 3, 4, 5, 16, 17, 18, 19, 20, 6, 15, 21, -1, 255];

/// A value meant to fit `type_id`, mostly.
fn value(rng: &mut Rng, out: &mut Vec<u8>, type_id: i128) {
    if rng.one_in(12) {
        return any_item(rng, out, 0);
    }
    match type_id {
        0 => out.push(0xc2 + rng.below(2) as u8),
        1 | 3 => match rng.below(3) {
            0 => {
                out.push(0xcb);
                out.extend(some_f64(rng).to_be_bytes());
            }
            1 => {
                out.push(0xca);
                out.extend((some_f64(rng) as f32).to_be_bytes());
            }
            _ => {
                let v = some_int(rng);
                int(rng, out, v)
            }
        },
        2 => {
            let v = some_int(rng);
            int(rng, out, v)
        }
        4 => {
            let s = if rng.one_in(3) {
                some_str(rng)
            } else {
                b"value".to_vec()
            };
            str_header(rng, out, s.len());
            out.extend(s);
        }
        5 => {
            let n = some_len(rng);
            bin_header(rng, out, n);
            out.extend((0..n).map(|i| i as u8));
        }
        16..=20 => {
            let n = if rng.big(50) {
                100_000 + rng.below(200_000) as usize
            } else {
                rng.below(20) as usize
            };
            array_header(rng, out, n);
            for _ in 0..n {
                // Mostly the right element type.
                if rng.one_in(40) {
                    any_item(rng, out, 3);
                } else {
                    small_element(rng, out, type_id - 16);
                }
            }
        }
        _ => any_item(rng, out, 0),
    }
}

/// An array element of `type_id`, kept small so big arrays stay fast.
fn small_element(rng: &mut Rng, out: &mut Vec<u8>, type_id: i128) {
    match type_id {
        0 => out.push(0xc2 + rng.below(2) as u8),
        1 | 3 => {
            if rng.one_in(2) {
                out.push(0xcb);
                out.extend(some_f64(rng).to_be_bytes());
            } else {
                {
                    let v = rng.below(300) as i128 - 150;
                    int(rng, out, v)
                };
            }
        }
        2 => {
            let v = some_int(rng);
            int(rng, out, v)
        }
        _ => {
            let n = rng.below(6) as usize;
            str_header(rng, out, n);
            out.extend(std::iter::repeat_n(b'x', n));
        }
    }
}

fn message(rng: &mut Rng, out: &mut Vec<u8>) {
    let n = if rng.one_in(25) {
        rng.below(7) as usize
    } else {
        4
    };
    array_header(rng, out, n);
    let type_id = *rng.pick(&TYPES);
    for i in 0..n {
        if rng.one_in(30) {
            any_item(rng, out, 0);
            continue;
        }
        match i {
            0 => {
                let id = if rng.one_in(4) {
                    some_int(rng)
                } else {
                    rng.below(1000) as i128 - 1
                };
                int(rng, out, id);
            }
            1 => {
                let v = some_int(rng);
                int(rng, out, v)
            }
            2 => int(rng, out, type_id),
            _ => value(rng, out, type_id),
        }
    }
}

// ----- JSON -----

fn json_string(rng: &mut Rng, s: &mut String) {
    s.push('"');
    let n = if rng.big(60) {
        100_000 + rng.below(1_000_000) as usize
    } else {
        rng.below(12) as usize
    };
    if n > 1000 {
        s.extend(std::iter::repeat_n('a', n));
    } else {
        for _ in 0..n {
            match rng.below(14) {
                0 => s.push_str("\\n"),
                1 => s.push_str("\\u00e9"),
                2 => s.push_str("\\ud83d\\ude00"),
                3 => s.push_str(rng.pick(&["\\ud800", "\\udc00", "\\u0000", "\\/", "\\\""])),
                4 => s.push(*rng.pick(&['é', '😀', '\u{7f}', '\u{ffff}', '/'])),
                _ => s.push((b'a' + rng.below(26) as u8) as char),
            }
        }
    }
    s.push('"');
}

fn json_number(rng: &mut Rng, s: &mut String) {
    const EDGES: [&str; 22] = [
        "0",
        "-0",
        "1",
        "-1",
        "1.0",
        "1.5",
        "-1.5",
        "1e2",
        "1E+2",
        "2.5e-3",
        "1e400",
        "-1e400",
        "1e-400",
        "2147483647",
        "2147483648",
        "-2147483648",
        "-2147483649",
        "9223372036854775807",
        "9223372036854775808",
        "18446744073709551615",
        "18446744073709551616",
        "123456789012345678901234567890",
    ];
    if rng.one_in(2) {
        s.push_str(rng.pick(&EDGES));
    } else {
        s.push_str(&(rng.below(100) as i64 - 10).to_string());
    }
}

fn json_any(rng: &mut Rng, s: &mut String, depth: u32) {
    match rng.below(if depth > 4 { 6 } else { 8 }) {
        0 => s.push_str("null"),
        1 => s.push_str(if rng.one_in(2) { "true" } else { "false" }),
        2 | 3 => json_number(rng, s),
        4 | 5 => json_string(rng, s),
        6 => json_object(rng, s, depth + 1),
        _ => {
            if rng.big(40) {
                // Deep nesting.
                let d = 100 + rng.below(20_000) as usize;
                s.extend(std::iter::repeat_n('[', d));
                s.extend(std::iter::repeat_n(']', d));
                return;
            }
            s.push('[');
            for i in 0..rng.below(4) {
                if i > 0 {
                    s.push(',');
                }
                json_any(rng, s, depth + 1);
            }
            s.push(']');
        }
    }
}

fn json_object(rng: &mut Rng, s: &mut String, depth: u32) {
    s.push('{');
    let n = if rng.big(60) { 2_000 } else { rng.below(4) };
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        let key = rng.pick(&["persistent", "retained", "cached", "a", "", "x\\u00e9"]);
        s.push_str(&format!(
            "\"{key}{}\":",
            if n > 10 { i.to_string() } else { String::new() }
        ));
        json_any(rng, s, depth);
    }
    s.push('}');
}

#[derive(Clone, Copy)]
enum Kind {
    Int,
    Str,
    Obj,
    Bool,
    Strs,
    Opts,
}

fn field_value(rng: &mut Rng, s: &mut String, kind: Kind) {
    if rng.one_in(10) {
        return json_any(rng, s, 1);
    }
    match kind {
        Kind::Int => {
            if rng.one_in(3) {
                json_number(rng, s);
            } else {
                s.push_str(&rng.below(50).to_string());
            }
        }
        Kind::Str => {
            if rng.one_in(3) {
                json_string(rng, s);
            } else {
                s.push_str(rng.pick(&[
                    "\"/a\"",
                    "\"/robot/x\"",
                    "\"double\"",
                    "\"\"",
                    "\"int[]\"",
                ]));
            }
        }
        Kind::Obj => json_object(rng, s, 1),
        Kind::Bool => s.push_str(if rng.one_in(2) { "true" } else { "false" }),
        Kind::Strs => {
            s.push('[');
            for i in 0..rng.below(4) {
                if i > 0 {
                    s.push(',');
                }
                field_value(rng, s, Kind::Str);
            }
            s.push(']');
        }
        Kind::Opts => {
            let fields = [
                ("periodic", Kind::Int),
                ("all", Kind::Bool),
                ("topicsonly", Kind::Bool),
                ("prefix", Kind::Bool),
            ];
            object(rng, s, &fields);
        }
    }
}

/// An object with (mostly) these fields, sometimes missing one, adding an
/// unknown one or repeating one.
fn object(rng: &mut Rng, s: &mut String, fields: &[(&str, Kind)]) {
    s.push('{');
    let mut first = true;
    let mut emit = |rng: &mut Rng, s: &mut String, name: &str, kind: Kind| {
        if !first {
            s.push(',');
        }
        first = false;
        s.push_str(&format!("\"{name}\":"));
        field_value(rng, s, kind);
    };
    for &(name, kind) in fields {
        if rng.one_in(12) {
            continue;
        }
        emit(rng, s, name, kind);
        if rng.one_in(25) {
            emit(rng, s, name, kind); // duplicate key
        }
    }
    if rng.one_in(10) {
        emit(rng, s, "extra", Kind::Obj);
    }
    s.push('}');
}

fn text_frame(rng: &mut Rng, client: bool) -> String {
    let client_msgs: [(&str, &[(&str, Kind)]); 5] = [
        (
            "publish",
            &[
                ("name", Kind::Str),
                ("pubuid", Kind::Int),
                ("type", Kind::Str),
                ("properties", Kind::Obj),
            ],
        ),
        ("unpublish", &[("pubuid", Kind::Int)]),
        (
            "setproperties",
            &[("name", Kind::Str), ("update", Kind::Obj)],
        ),
        (
            "subscribe",
            &[
                ("subuid", Kind::Int),
                ("topics", Kind::Strs),
                ("options", Kind::Opts),
            ],
        ),
        ("unsubscribe", &[("subuid", Kind::Int)]),
    ];
    let server_msgs: [(&str, &[(&str, Kind)]); 3] = [
        (
            "announce",
            &[
                ("name", Kind::Str),
                ("id", Kind::Int),
                ("type", Kind::Str),
                ("pubuid", Kind::Int),
                ("properties", Kind::Obj),
            ],
        ),
        ("unannounce", &[("name", Kind::Str), ("id", Kind::Int)]),
        (
            "properties",
            &[
                ("name", Kind::Str),
                ("update", Kind::Obj),
                ("ack", Kind::Bool),
            ],
        ),
    ];
    let msgs: &[(&str, &[(&str, Kind)])] = if client != rng.one_in(15) {
        &client_msgs
    } else {
        &server_msgs
    };

    if rng.one_in(40) {
        let mut s = String::new();
        json_any(rng, &mut s, 0); // not an array of messages
        return s;
    }
    let mut s = String::from(if rng.one_in(10) { " [ " } else { "[" });
    for i in 0..rng.below(4) + u64::from(!rng.one_in(20)) {
        if i > 0 {
            s.push(',');
        }
        if rng.one_in(30) {
            json_any(rng, &mut s, 1); // not a message object
            continue;
        }
        let (method, fields) = *rng.pick(msgs);
        s.push('{');
        if !rng.one_in(30) {
            s.push_str("\"method\":");
            if rng.one_in(20) {
                field_value(rng, &mut s, Kind::Str);
            } else {
                s.push_str(&format!("\"{method}\""));
            }
            s.push(',');
        }
        s.push_str("\"params\":");
        if rng.one_in(30) {
            json_any(rng, &mut s, 1);
        } else {
            object(rng, &mut s, fields);
        }
        s.push('}');
    }
    s.push(']');
    s
}
