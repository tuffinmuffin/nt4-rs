//! Run the differential fuzzer.
//!
//! ```text
//! NT4_NTCORE_PYTHON=<python with pyntcore> cargo run --release -p nt4-fuzz -- --seconds 60
//! ```
//!
//! Options: `--seconds N` (default 30), `--seed S` (default: from the
//! clock), `--target binary|client-text|server-text` (default: all),
//! `--out DIR` to save each new kind of mismatch, shrunk, as a file.
//! Without `NT4_NTCORE_PYTHON` at build time it only runs nt4's own checks
//! (no panics, round trips). Exits 1 if anything was found.
//!
//! Written by Claude.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use nt4_fuzz::{Mismatch, Rng, Target, check, drive_client_core, generate, has_oracle, shrink};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

#[derive(Default)]
struct Stats {
    cases: u64,
    accepted: u64,
    rejected: u64,
    bytes: u64,
    largest: usize,
    slowest: Duration,
}

fn main() {
    // Deep inputs make deep JSON values, which drop recursively.
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap();
}

fn run() {
    let seconds: f64 = arg("--seconds").map_or(30.0, |s| s.parse().expect("--seconds"));
    let seed: u64 = arg("--seed").map_or_else(
        || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
                | 1
        },
        |s| s.parse().expect("--seed"),
    );
    let targets: Vec<Target> = match arg("--target") {
        Some(t) => vec![
            *Target::ALL
                .iter()
                .find(|x| x.name() == t)
                .expect("--target binary|client-text|server-text"),
        ],
        None => Target::ALL.to_vec(),
    };
    let out_dir = arg("--out");

    println!(
        "seed {seed}, {seconds} s, oracle: {}",
        if has_oracle() {
            "ntcore (pyntcore 2026.2.2)"
        } else {
            "none (self-checks only)"
        }
    );
    // A panic in nt4 is reported as a finding, not printed (set
    // NT4_FUZZ_DEBUG to see them).
    if std::env::var_os("NT4_FUZZ_DEBUG").is_none() {
        std::panic::set_hook(Box::new(|_| {}));
    }

    let mut rng = Rng::new(seed);
    let mut stats: BTreeMap<&str, Stats> = BTreeMap::new();
    // signature -> (count, first mismatch, its input)
    let mut found: BTreeMap<String, (u64, Mismatch, Vec<u8>)> = BTreeMap::new();
    // ntcore exception text -> (count, an input that throws it)
    let mut ntcore_threw: BTreeMap<String, (u64, Target, Vec<u8>)> = BTreeMap::new();
    let mut deviations: BTreeMap<&str, u64> = BTreeMap::new();
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    let mut i = 0u64;
    while Instant::now() < deadline {
        let target = targets[(i % targets.len() as u64) as usize];
        i += 1;
        let input = generate(target, &mut rng);
        let t = Instant::now();
        let result = check(target, &input);
        // ClientCore must survive the same frame.
        let core_panicked = std::panic::catch_unwind(|| drive_client_core(target, &input)).is_err();
        let took = t.elapsed();

        let s = stats.entry(target.name()).or_default();
        s.cases += 1;
        s.bytes += input.len() as u64;
        s.largest = s.largest.max(input.len());
        s.slowest = s.slowest.max(took);
        let result = match (result, core_panicked) {
            (Ok(_), true) => Err(Mismatch {
                target,
                signature: "ClientCore panicked".into(),
                ours: serde_json::Value::Null,
                theirs: serde_json::Value::Null,
            }),
            (r, _) => r,
        };
        match result {
            Ok(o) => {
                s.accepted += o.accepted as u64;
                s.rejected += u64::from(o.rejected);
                if let Some(why) = o.known_deviation {
                    *deviations.entry(why).or_default() += 1;
                }
                if let Some(e) = o.ntcore_exception {
                    let e = ntcore_threw
                        .entry(nt4_fuzz::collapse_digits(&e))
                        .or_insert_with(|| (0, target, input.clone()));
                    e.0 += 1;
                }
            }
            Err(m) => {
                let entry = found
                    .entry(m.signature.clone())
                    .or_insert_with(|| (0, m.clone(), input.clone()));
                entry.0 += 1;
            }
        }
        if took > Duration::from_secs(1) {
            println!("slow: {} ms on a {}-byte {} frame", took.as_millis(), input.len(), target.name());
        }
    }

    for (name, s) in &stats {
        println!(
            "{name}: {} frames ({:.1} MB, largest {} KB), {} messages accepted, {} frames rejected or warned, slowest {} ms",
            s.cases,
            s.bytes as f64 / 1e6,
            s.largest / 1024,
            s.accepted,
            s.rejected,
            s.slowest.as_millis()
        );
    }
    for (why, count) in &deviations {
        println!("known deviation on {count} frames: {why}");
    }
    for (what, (count, target, input)) in &ntcore_threw {
        // One example is enough; these aren't nt4 bugs.
        println!(
            "\nntcore threw {what:?} on {count} {} frames (nt4 handled them), e.g. {}",
            target.name(),
            String::from_utf8_lossy(input).chars().take(300).collect::<String>()
        );
    }
    if found.is_empty() {
        println!("no mismatches");
        return;
    }
    println!("\n{} kinds of mismatch:", found.len());
    for (n, (signature, (count, m, input))) in found.iter().enumerate() {
        let small = if signature.contains("panicked") {
            input.clone()
        } else {
            shrink(m.target, input, signature, 3_000)
        };
        println!("\n[{n}] {signature} ({count} frames)");
        println!("  {} input, shrunk to {} bytes:", m.target.name(), small.len());
        if m.target == Target::Binary {
            println!("  {}", small.iter().map(|b| format!("{b:02x}")).collect::<String>());
        } else {
            println!("  {}", String::from_utf8_lossy(&small).chars().take(400).collect::<String>());
        }
        if let Err(again) = check(m.target, &small) {
            println!("  nt4:    {}", truncate(&again.ours.to_string()));
            println!("  ntcore: {}", truncate(&again.theirs.to_string()));
        }
        if let Some(dir) = &out_dir {
            std::fs::create_dir_all(dir).expect("--out");
            std::fs::write(format!("{dir}/{}-{n}.bin", m.target.name()), &small).expect("--out");
        }
    }
    std::process::exit(1);
}

fn truncate(s: &str) -> String {
    if s.len() > 600 {
        format!("{}...", &s[..s.floor_char_boundary(600)])
    } else {
        s.to_owned()
    }
}
